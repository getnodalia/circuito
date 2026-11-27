// SPDX-License-Identifier: MIT OR Apache-2.0
//! The `drc` and `gerber` commands: the PCB engine on a `.circuito-pcb` board.

use crate::{fail, load};
use circuito_pcb::assembly::{self, AssemblyOptions};
use circuito_pcb::board::Board;
use circuito_pcb::drc::{self, CapabilityProfile, Severity, Violation};
use circuito_pcb::excellon::{self, DrillOptions};
use circuito_pcb::gerber::{self, GerberOptions};
use circuito_pcb::units::format_mm;
use std::io::{self, Read, Write};
use std::path::Path;

fn load_board(file: &str, stdin: &mut dyn Read) -> Result<Board, String> {
    let text = load::read_text(file, stdin)?;
    Board::from_json(&text).map_err(|e| format!("{file}: {e}"))
}

/// The built-in profile named `name`: the full name (`generic-2layer-6-6`) or its short form
/// (`generic-2layer`).
fn find_profile(name: &str) -> Option<CapabilityProfile> {
    let short = format!("{name}-");
    CapabilityProfile::builtin()
        .into_iter()
        .find(|p| p.name == name || p.name.starts_with(&short))
}

fn profile_names() -> String {
    CapabilityProfile::builtin()
        .iter()
        .map(|p| p.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// A two-layer board is checked with the two-layer profile, anything bigger with the four-layer one.
fn default_profile(board: &Board) -> CapabilityProfile {
    if board.copper_layers <= 2 {
        CapabilityProfile::generic_2layer()
    } else {
        CapabilityProfile::generic_4layer()
    }
}

fn severity_word(s: Severity) -> &'static str {
    match s {
        Severity::Error => "error",
        Severity::Warning => "warning",
    }
}

fn describe(v: &Violation) -> String {
    let mut place = String::new();
    if let Some(l) = v.layer {
        place.push_str(&format!(" {}", l.name()));
    }
    if let Some(p) = v.locations.first() {
        place.push_str(&format!(" at {},{} mm", format_mm(p.x), format_mm(p.y)));
    }
    format!(
        "{}[{}]{place}: {}",
        severity_word(v.severity),
        v.code.as_str(),
        v.message
    )
}

pub(crate) fn cmd_drc(
    file: &str,
    profile: Option<&str>,
    json: bool,
    deny_warnings: bool,
    stdin: &mut dyn Read,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let board = match load_board(file, stdin) {
        Ok(b) => b,
        Err(e) => return fail(err, &e),
    };
    let profile = match profile {
        None => default_profile(&board),
        Some(n) => match find_profile(n) {
            Some(p) => p,
            None => {
                return fail(
                    err,
                    &format!("unknown profile `{n}` (built in: {})", profile_names()),
                )
            }
        },
    };
    let report = match drc::run_drc(&board, &profile.rules()) {
        Ok(r) => r,
        Err(e) => return fail(err, &format!("{file}: {e}")),
    };
    let mut violations = report.violations.clone();
    match drc::check_profile(&board, &profile) {
        Ok(v) => violations.extend(v),
        Err(e) => return fail(err, &format!("{file}: {e}")),
    }
    let count = |s: Severity| violations.iter().filter(|v| v.severity == s).count();
    let (errors, warnings) = (count(Severity::Error), count(Severity::Warning));
    let printed = if json {
        let doc = serde_json::json!({
            "file": file,
            "profile": profile.name,
            "errors": errors,
            "warnings": warnings,
            "truncated": report.truncated,
            "stats": report.stats,
            "violations": violations,
        });
        serde_json::to_string_pretty(&doc)
            .map_err(io::Error::other)
            .and_then(|t| writeln!(out, "{t}"))
    } else {
        (|| {
            for v in &violations {
                writeln!(out, "{file}: {}", describe(v))?;
            }
            if report.truncated {
                writeln!(
                    out,
                    "{file}: the list was cut short; more findings were dropped"
                )?;
            }
            writeln!(
                out,
                "{file}: {errors} error(s), {warnings} warning(s) against {}",
                profile.name
            )
        })()
    };
    if let Err(e) = printed {
        return fail(err, &format!("cannot write the report: {e}"));
    }
    i32::from(errors > 0 || (deny_warnings && warnings > 0))
}

/// A file name that stays inside the output directory.
fn safe_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && !name.contains(['/', '\\', '\0']) && name != ".."
}

/// Write `content` to `dir/name` through a temporary sibling, so a failure never leaves half a file.
fn write_file(dir: &Path, name: &str, content: &str) -> Result<(), String> {
    if !safe_name(name) {
        return Err(format!("refusing to write a file called `{name}`"));
    }
    let path = dir.join(name);
    let tmp = dir.join(format!("{name}.tmp-circuito"));
    let result = std::fs::write(&tmp, content).and_then(|()| std::fs::rename(&tmp, &path));
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("cannot write {}: {e}", path.display()));
    }
    Ok(())
}

fn base_name(board_name: &str) -> String {
    let s: String = board_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        "board".into()
    } else {
        s
    }
}

pub(crate) fn cmd_gerber(
    file: &str,
    out_dir: &str,
    stdin: &mut dyn Read,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let board = match load_board(file, stdin) {
        Ok(b) => b,
        Err(e) => return fail(err, &e),
    };
    // Build everything first: nothing is written when any output cannot be produced.
    let gerbers = match gerber::write_all(&board, &GerberOptions::default()) {
        Ok(g) => g,
        Err(e) => return fail(err, &format!("{file}: gerber: {e}")),
    };
    let drills = match excellon::write_drills(&board, &DrillOptions::default()) {
        Ok(d) => d,
        Err(e) => return fail(err, &format!("{file}: drill: {e}")),
    };
    let base = base_name(&board.name);
    let pos = match assembly::pick_and_place_csv(&board, &AssemblyOptions::default()) {
        Ok(p) => p,
        Err(e) => return fail(err, &format!("{file}: pick and place: {e}")),
    };
    let bom = match assembly::bom_csv(&board) {
        Ok(b) => b,
        Err(e) => return fail(err, &format!("{file}: bill of materials: {e}")),
    };
    let dir = Path::new(out_dir);
    if let Err(e) = std::fs::create_dir_all(dir) {
        return fail(err, &format!("cannot create {out_dir}: {e}"));
    }
    let mut files: Vec<(String, &str)> = Vec::new();
    files.extend(
        gerbers
            .files
            .iter()
            .map(|f| (f.file_name.clone(), f.content.as_str())),
    );
    files.extend(
        drills
            .files
            .iter()
            .map(|f| (f.file_name.clone(), f.content.as_str())),
    );
    files.push((format!("{base}-pos.csv"), pos.as_str()));
    files.push((format!("{base}-bom.csv"), bom.as_str()));
    for w in gerbers.warnings.iter().chain(&drills.warnings) {
        let _ = writeln!(err, "circuito: warning: {file}: {w}");
    }
    for (name, content) in &files {
        if let Err(e) = write_file(dir, name, content) {
            return fail(err, &e);
        }
        let _ = writeln!(out, "{}", dir.join(name).display());
    }
    let _ = writeln!(out, "{} file(s) written to {out_dir}", files.len());
    0
}

pub(crate) fn cmd_import_kicad(
    file: &str,
    output: &str,
    stdin: &mut dyn Read,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let text = match load::read_text(file, stdin) {
        Ok(t) => t,
        Err(e) => return fail(err, &e),
    };
    let name = Path::new(file)
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| *s != "-")
        .unwrap_or("board");
    let imported = match circuito_kicad::pcb::import(&text, name) {
        Ok(i) => i,
        Err(e) => return fail(err, &format!("{file}: {e}")),
    };
    let json = match imported.board.to_json() {
        Ok(j) => j,
        Err(e) => return fail(err, &format!("{file}: {e}")),
    };
    for w in &imported.warnings {
        let _ = writeln!(err, "circuito: warning: {file}: {w}");
    }
    let path = Path::new(output);
    let (dir, name) = match (path.parent(), path.file_name().and_then(|n| n.to_str())) {
        (Some(d), Some(n)) => (
            if d.as_os_str().is_empty() {
                Path::new(".")
            } else {
                d
            },
            n,
        ),
        _ => return fail(err, &format!("{output} is not a file name")),
    };
    if let Err(e) = write_file(dir, name, &json) {
        return fail(err, &e);
    }
    let b = &imported.board;
    let _ = writeln!(
        out,
        "{output}: {} footprint(s), {} track(s), {} via(s), {} zone(s), {} net(s), {} warning(s)",
        b.footprints.len(),
        b.tracks.len(),
        b.vias.len(),
        b.zones.len(),
        b.nets.len(),
        imported.warnings.len()
    );
    0
}
