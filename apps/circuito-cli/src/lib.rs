// SPDX-License-Identifier: MIT OR Apache-2.0
//! The `circuito` command, as a library so the tests can run it in process.
//!
//! Exit codes: 0 success, 1 the input or the analysis failed (or the check found errors),
//! 2 the command line was wrong.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::todo,
    clippy::unimplemented,
    clippy::unreachable
)]

pub mod args;
pub mod load;
pub mod output;
mod pcb;

use args::{Command, Format};
use circuito_erc::Severity;
use circuito_sim::{simulate, AnalysisResult, Control, Options};
use std::io::{self, Read, Write};

const USAGE: &str = "\
circuito: simulate circuits, check schematics and boards, export netlists and Gerber files

USAGE
  circuito sim <file> [--format table|csv|json] [-o <out>] [-a <n>]
  circuito erc <file.circuito|file.circ> [--json] [--deny-warnings]
  circuito netlist <file.circuito|file.circ> [-o <out>]
  circuito drc <board.circuito-pcb> [--profile <name>] [--json] [--deny-warnings]
  circuito gerber <board.circuito-pcb> --out <dir>
  circuito --help | --version

COMMANDS
  sim      Run the analysis cards (.op .dc .ac .tran) of a SPICE netlist or a .circuito
           schematic. --csv and --json are shorthands for --format. -a N runs only the N-th
           card (1-based). Without -o the results go to standard output.
  erc      Electrical rules check of a schematic. Exit 1 if it has errors (or warnings with
           --deny-warnings).
  netlist  Write the SPICE netlist extracted from a schematic. Exit 1 if some parts could not
           be extracted (they are listed on standard error; the rest is still written).
  drc      Design rules check of a board against a manufacturer profile (generic-2layer or
           generic-4layer; by default the one that fits the board's copper layers). Exit 1 if it
           has errors (or warnings with --deny-warnings).
  gerber   Write the manufacturing files of a board into <dir>: Gerber RS-274X/X2 for every
           layer, Excellon drills, pick-and-place and bill of materials CSV.

A file name of - reads standard input. Exit codes: 0 ok, 1 failed, 2 bad command line.
";

pub(crate) fn fail(err: &mut dyn Write, msg: &str) -> i32 {
    let _ = writeln!(err, "circuito: {msg}");
    1
}

/// Run the command line `args` (without the program name). Returns the exit code.
pub fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    run_with_stdin(args, &mut io::stdin().lock(), out, err)
}

/// [`run`] with an explicit standard input.
pub fn run_with_stdin(
    args: &[String],
    stdin: &mut dyn Read,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let cmd = match args::parse(args) {
        Ok(c) => c,
        Err(e) => {
            let _ = writeln!(err, "circuito: {}", e.0);
            return 2;
        }
    };
    match cmd {
        Command::Help => {
            let _ = out.write_all(USAGE.as_bytes());
            0
        }
        Command::Version => {
            let _ = writeln!(out, "circuito {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Command::Sim {
            file,
            format,
            output,
            analysis,
        } => cmd_sim(&file, format, output.as_deref(), analysis, stdin, out, err),
        Command::Erc {
            file,
            json,
            deny_warnings,
        } => cmd_erc(&file, json, deny_warnings, stdin, out, err),
        Command::Netlist { file, output } => cmd_netlist(&file, output.as_deref(), stdin, out, err),
        Command::Drc {
            file,
            profile,
            json,
            deny_warnings,
        } => pcb::cmd_drc(
            &file,
            profile.as_deref(),
            json,
            deny_warnings,
            stdin,
            out,
            err,
        ),
        Command::Gerber { file, out_dir } => pcb::cmd_gerber(&file, &out_dir, stdin, out, err),
    }
}

/// Run `f` on the output file (written atomically through a temporary sibling) or on `out`.
fn with_output(
    path: Option<&str>,
    out: &mut dyn Write,
    f: &mut dyn FnMut(&mut dyn Write) -> io::Result<()>,
) -> Result<(), String> {
    match path {
        None => f(out).map_err(|e| format!("cannot write the output: {e}")),
        Some(p) => {
            let tmp = format!("{p}.tmp-circuito");
            let result = (|| {
                let file = std::fs::File::create(&tmp)?;
                let mut w = io::BufWriter::new(file);
                f(&mut w)?;
                w.flush()?;
                drop(w);
                std::fs::rename(&tmp, p)
            })();
            if let Err(e) = result {
                let _ = std::fs::remove_file(&tmp);
                return Err(format!("cannot write {p}: {e}"));
            }
            Ok(())
        }
    }
}

fn cmd_sim(
    file: &str,
    format: Format,
    output: Option<&str>,
    which: Option<usize>,
    stdin: &mut dyn Read,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let loaded = match load::load_netlist(file, stdin) {
        Ok(l) => l,
        Err(e) => return fail(err, &e),
    };
    for w in &loaded.warnings {
        let _ = writeln!(err, "circuito: warning: {w}");
    }
    let netlist = &loaded.netlist;
    let total = netlist.analyses.len();
    if total == 0 {
        return fail(
            err,
            &format!("{file} has no analysis card; add .op, .dc, .ac or .tran"),
        );
    }
    let selected: Vec<_> = match which {
        None => netlist.analyses.iter().collect(),
        Some(n) => match n.checked_sub(1).and_then(|i| netlist.analyses.get(i)) {
            Some(a) => vec![a],
            None => {
                return fail(
                    err,
                    &format!("--analysis {n}: the file has {total} analysis card(s)"),
                )
            }
        },
    };
    let opts = match Options::from_netlist(netlist) {
        Ok(o) => o,
        Err(e) => return fail(err, &format!("{file}: {e}")),
    };
    let control = Control::new();
    let mut results: Vec<AnalysisResult> = Vec::new();
    let mut failed = false;
    for a in selected {
        match simulate(netlist, a, &opts, &control) {
            Ok(r) => results.push(r),
            Err(e) => {
                failed = true;
                let _ = writeln!(err, "circuito: {a}: {e}");
            }
        }
    }
    if failed && results.is_empty() {
        return 1;
    }
    let written = with_output(output, out, &mut |w| match format {
        Format::Table => {
            for (i, r) in results.iter().enumerate() {
                if i > 0 {
                    writeln!(w)?;
                }
                output::table(w, r)?;
            }
            Ok(())
        }
        Format::Csv => {
            for (i, r) in results.iter().enumerate() {
                if i > 0 {
                    writeln!(w)?;
                }
                output::csv(w, r)?;
            }
            Ok(())
        }
        Format::Json => {
            let doc = serde_json::json!({
                "title": netlist.title,
                "results": results.iter().map(output::result_json).collect::<Vec<_>>(),
            });
            let text = serde_json::to_string_pretty(&doc).map_err(io::Error::other)?;
            writeln!(w, "{text}")
        }
    });
    if let Err(e) = written {
        return fail(err, &e);
    }
    i32::from(failed)
}

fn cmd_erc(
    file: &str,
    json: bool,
    deny_warnings: bool,
    stdin: &mut dyn Read,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let text = match load::read_text(file, stdin) {
        Ok(t) => t,
        Err(e) => return fail(err, &e),
    };
    let schematic = match load::schematic_from_text(file, &text) {
        Ok(s) => s,
        Err(e) => return fail(err, &e),
    };
    let report = circuito_erc::check(&schematic);
    let (errors, warnings) = (
        report.count(Severity::Error),
        report.count(Severity::Warning),
    );
    let printed = if json {
        writeln!(out, "{}", report.to_json())
    } else {
        print_report(out, file, &report, errors, warnings)
    };
    if let Err(e) = printed {
        return fail(err, &format!("cannot write the report: {e}"));
    }
    i32::from(errors > 0 || (deny_warnings && warnings > 0))
}

fn print_report(
    out: &mut dyn Write,
    file: &str,
    report: &circuito_erc::ErcReport,
    errors: usize,
    warnings: usize,
) -> io::Result<()> {
    for v in &report.violations {
        let sev = match v.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        };
        let place = match (&v.sheet_name, v.position) {
            (Some(s), Some(p)) => format!(" [{s} at {},{}]", p.x, p.y),
            (Some(s), None) => format!(" [{s}]"),
            _ => String::new(),
        };
        writeln!(out, "{file}: {sev}[{}]{place}: {}", v.code, v.message)?;
    }
    writeln!(
        out,
        "{file}: {errors} error(s), {warnings} warning(s), {} note(s)",
        report.count(Severity::Info)
    )
}

fn cmd_netlist(
    file: &str,
    output: Option<&str>,
    stdin: &mut dyn Read,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let text = match load::read_text(file, stdin) {
        Ok(t) => t,
        Err(e) => return fail(err, &e),
    };
    let (extraction, _) = match load::extract_text(file, &text) {
        Ok(x) => x,
        Err(e) => return fail(err, &e),
    };
    let spice = match circuito_spice::write(&extraction.netlist) {
        Ok(s) => s,
        Err(e) => return fail(err, &format!("{file}: {e}")),
    };
    if let Err(e) = with_output(output, out, &mut |w| w.write_all(spice.as_bytes())) {
        return fail(err, &e);
    }
    if extraction.issues.is_empty() {
        0
    } else {
        let _ = writeln!(
            err,
            "circuito: {}",
            load::format_issues(file, &extraction.issues)
        );
        1
    }
}
