// SPDX-License-Identifier: MIT OR Apache-2.0
//! Command-line parsing. Hand-written: the grammar is tiny and a parser dependency is not worth it.

/// How `sim` prints its results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Aligned columns for people.
    Table,
    /// Comma-separated values.
    Csv,
    /// One JSON document.
    Json,
}

/// A parsed command line.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// `--help`, `-h` or no arguments.
    Help,
    /// `--version`.
    Version,
    /// `sim`.
    Sim {
        file: String,
        format: Format,
        output: Option<String>,
        analysis: Option<usize>,
    },
    /// `erc`.
    Erc {
        file: String,
        json: bool,
        deny_warnings: bool,
    },
    /// `netlist`.
    Netlist {
        file: String,
        output: Option<String>,
    },
    /// `drc`.
    Drc {
        file: String,
        profile: Option<String>,
        json: bool,
        deny_warnings: bool,
    },
    /// `gerber`.
    Gerber { file: String, out_dir: String },
    /// `import-kicad`.
    ImportKicad { file: String, output: String },
}

/// What was wrong with the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageError(pub String);

fn usage<T>(msg: impl Into<String>) -> Result<T, UsageError> {
    Err(UsageError(msg.into()))
}

/// Parse `args` (without the program name).
///
/// # Errors
/// An unknown command or option, a missing value or file, or a repeated option.
pub fn parse(args: &[String]) -> Result<Command, UsageError> {
    let Some(first) = args.first() else {
        return Ok(Command::Help);
    };
    let rest = args.get(1..).unwrap_or(&[]);
    match first.as_str() {
        "-h" | "--help" | "help" => Ok(Command::Help),
        "-V" | "--version" => Ok(Command::Version),
        "sim" => parse_sim(rest),
        "erc" => parse_erc(rest),
        "netlist" => parse_netlist(rest),
        "drc" => parse_drc(rest),
        "gerber" => parse_gerber(rest),
        "import-kicad" => parse_import_kicad(rest),
        other => usage(format!(
            "unknown command `{other}` (commands: sim, erc, netlist, drc, gerber, import-kicad; try `circuito --help`)"
        )),
    }
}

/// Split `--name=value` into its parts.
fn split_eq(a: &str) -> (&str, Option<&str>) {
    match a.split_once('=') {
        Some((n, v)) if a.starts_with("--") => (n, Some(v)),
        _ => (a, None),
    }
}

struct Cursor<'a> {
    args: &'a [String],
    i: usize,
}

impl<'a> Cursor<'a> {
    fn next(&mut self) -> Option<&'a str> {
        let a = self.args.get(self.i)?;
        self.i += 1;
        Some(a.as_str())
    }

    fn value(&mut self, name: &str, inline: Option<&'a str>) -> Result<&'a str, UsageError> {
        match inline {
            Some(v) => Ok(v),
            None => self
                .next()
                .ok_or_else(|| UsageError(format!("{name} needs a value"))),
        }
    }
}

fn set_once<T>(slot: &mut Option<T>, name: &str, v: T) -> Result<(), UsageError> {
    if slot.is_some() {
        return usage(format!("{name} given twice"));
    }
    *slot = Some(v);
    Ok(())
}

fn parse_sim(args: &[String]) -> Result<Command, UsageError> {
    let mut c = Cursor { args, i: 0 };
    let (mut file, mut format, mut output, mut analysis) = (None, None, None, None);
    while let Some(a) = c.next() {
        let (name, inline) = split_eq(a);
        match name {
            "--format" | "-f" => {
                let v = c.value(name, inline)?;
                let f = match v {
                    "table" => Format::Table,
                    "csv" => Format::Csv,
                    "json" => Format::Json,
                    _ => return usage(format!("unknown format `{v}` (table, csv or json)")),
                };
                set_once(&mut format, "--format", f)?;
            }
            "--csv" => set_once(&mut format, "--format", Format::Csv)?,
            "--json" => set_once(&mut format, "--format", Format::Json)?,
            "--output" | "-o" => {
                let v = c.value(name, inline)?;
                set_once(&mut output, "--output", v.to_string())?;
            }
            "--analysis" | "-a" => {
                let v = c.value(name, inline)?;
                let n: usize =
                    v.parse().ok().filter(|n| *n >= 1).ok_or_else(|| {
                        UsageError(format!("--analysis wants 1, 2, 3...; got `{v}`"))
                    })?;
                set_once(&mut analysis, "--analysis", n)?;
            }
            _ if name.starts_with('-') && name != "-" => {
                return usage(format!("unknown option `{name}` for `sim`"));
            }
            _ => set_once(&mut file, "the input file", a.to_string())?,
        }
    }
    let file =
        file.ok_or_else(|| UsageError("`sim` needs an input file (or `-` for stdin)".into()))?;
    Ok(Command::Sim {
        file,
        format: format.unwrap_or(Format::Table),
        output,
        analysis,
    })
}

fn parse_erc(args: &[String]) -> Result<Command, UsageError> {
    let mut c = Cursor { args, i: 0 };
    let (mut file, mut json, mut deny) = (None, false, false);
    while let Some(a) = c.next() {
        match a {
            "--json" => json = true,
            "--deny-warnings" => deny = true,
            _ if a.starts_with('-') && a != "-" => {
                return usage(format!("unknown option `{a}` for `erc`"));
            }
            _ => set_once(&mut file, "the input file", a.to_string())?,
        }
    }
    let file = file.ok_or_else(|| {
        UsageError("`erc` needs a .circuito or .circ file (or `-` for stdin)".into())
    })?;
    Ok(Command::Erc {
        file,
        json,
        deny_warnings: deny,
    })
}

fn parse_netlist(args: &[String]) -> Result<Command, UsageError> {
    let mut c = Cursor { args, i: 0 };
    let (mut file, mut output) = (None, None);
    while let Some(a) = c.next() {
        let (name, inline) = split_eq(a);
        match name {
            "--output" | "-o" => {
                let v = c.value(name, inline)?;
                set_once(&mut output, "--output", v.to_string())?;
            }
            _ if name.starts_with('-') && name != "-" => {
                return usage(format!("unknown option `{name}` for `netlist`"));
            }
            _ => set_once(&mut file, "the input file", a.to_string())?,
        }
    }
    let file = file.ok_or_else(|| {
        UsageError("`netlist` needs a .circuito or .circ file (or `-` for stdin)".into())
    })?;
    Ok(Command::Netlist { file, output })
}

fn parse_drc(args: &[String]) -> Result<Command, UsageError> {
    let mut c = Cursor { args, i: 0 };
    let (mut file, mut profile, mut json, mut deny) = (None, None, false, false);
    while let Some(a) = c.next() {
        let (name, inline) = split_eq(a);
        match name {
            "--profile" | "-p" => {
                let v = c.value(name, inline)?;
                set_once(&mut profile, "--profile", v.to_string())?;
            }
            "--json" => json = true,
            "--deny-warnings" => deny = true,
            _ if name.starts_with('-') && name != "-" => {
                return usage(format!("unknown option `{name}` for `drc`"));
            }
            _ => set_once(&mut file, "the input file", a.to_string())?,
        }
    }
    let file = file
        .ok_or_else(|| UsageError("`drc` needs a .circuito-pcb file (or `-` for stdin)".into()))?;
    Ok(Command::Drc {
        file,
        profile,
        json,
        deny_warnings: deny,
    })
}

fn parse_import_kicad(args: &[String]) -> Result<Command, UsageError> {
    let mut c = Cursor { args, i: 0 };
    let (mut file, mut output) = (None, None);
    while let Some(a) = c.next() {
        let (name, inline) = split_eq(a);
        match name {
            "--out" | "-o" => {
                let v = c.value(name, inline)?;
                set_once(&mut output, "--out", v.to_string())?;
            }
            _ if name.starts_with('-') && name != "-" => {
                return usage(format!("unknown option `{name}` for `import-kicad`"));
            }
            _ => set_once(&mut file, "the input file", a.to_string())?,
        }
    }
    let file = file.ok_or_else(|| {
        UsageError("`import-kicad` needs a .kicad_pcb file (or `-` for stdin)".into())
    })?;
    let output = output.ok_or_else(|| {
        UsageError("`import-kicad` needs --out <file.circuito-pcb> for the board".into())
    })?;
    Ok(Command::ImportKicad { file, output })
}

fn parse_gerber(args: &[String]) -> Result<Command, UsageError> {
    let mut c = Cursor { args, i: 0 };
    let (mut file, mut out_dir) = (None, None);
    while let Some(a) = c.next() {
        let (name, inline) = split_eq(a);
        match name {
            "--out" | "-o" => {
                let v = c.value(name, inline)?;
                set_once(&mut out_dir, "--out", v.to_string())?;
            }
            _ if name.starts_with('-') && name != "-" => {
                return usage(format!("unknown option `{name}` for `gerber`"));
            }
            _ => set_once(&mut file, "the input file", a.to_string())?,
        }
    }
    let file = file.ok_or_else(|| {
        UsageError("`gerber` needs a .circuito-pcb file (or `-` for stdin)".into())
    })?;
    let out_dir = out_dir
        .ok_or_else(|| UsageError("`gerber` needs --out <directory> for the files".into()))?;
    Ok(Command::Gerber { file, out_dir })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Result<Command, UsageError> {
        let v: Vec<String> = s.split_whitespace().map(str::to_string).collect();
        parse(&v)
    }

    #[test]
    fn parses_the_three_commands() {
        assert_eq!(p("").unwrap(), Command::Help);
        assert_eq!(
            p("sim a.cir --csv -o out.csv -a 2").unwrap(),
            Command::Sim {
                file: "a.cir".into(),
                format: Format::Csv,
                output: Some("out.csv".into()),
                analysis: Some(2)
            }
        );
        assert_eq!(
            p("erc --json --deny-warnings s.circuito").unwrap(),
            Command::Erc {
                file: "s.circuito".into(),
                json: true,
                deny_warnings: true
            }
        );
        assert_eq!(
            p("netlist s.circuito --output=x.cir").unwrap(),
            Command::Netlist {
                file: "s.circuito".into(),
                output: Some("x.cir".into())
            }
        );
    }

    #[test]
    fn parses_drc_and_gerber() {
        assert_eq!(
            p("drc b.json --profile generic-4layer --json --deny-warnings").unwrap(),
            Command::Drc {
                file: "b.json".into(),
                profile: Some("generic-4layer".into()),
                json: true,
                deny_warnings: true
            }
        );
        assert_eq!(
            p("gerber b.json --out=dir").unwrap(),
            Command::Gerber {
                file: "b.json".into(),
                out_dir: "dir".into()
            }
        );
    }

    #[test]
    fn rejects_bad_command_lines_with_a_reason() {
        for bad in [
            "frobnicate",
            "sim",
            "sim a b",
            "sim a --bogus",
            "sim a --format xml",
            "sim a --analysis 0",
            "sim a --analysis x",
            "sim a --csv --json",
            "sim a -o",
            "erc",
            "erc a --nope",
            "netlist",
            "netlist a -o x -o y",
            "drc",
            "drc a --profile",
            "drc a --nope",
            "gerber a",
            "gerber --out d",
            "gerber a --out d --out e",
        ] {
            let e = p(bad).unwrap_err();
            assert!(!e.0.is_empty(), "{bad}");
        }
    }
}
