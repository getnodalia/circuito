// SPDX-License-Identifier: MIT OR Apache-2.0
//! A circuit the person opened: a `.circuito` schematic or a SPICE deck, with what the engines say
//! about it (the extracted netlist, the electrical rules check, the analyses it asks for).
//!
//! Everything here is plain data and runs without a window. Reading a file is bounded, and a
//! file that is not valid is an error with a message, never a panic.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use circuito_core::netlist::{Analysis, Netlist, Sweep};
use circuito_core::schematic::builder::{Builder, PartId};
use circuito_core::schematic::{PinRef, Point, Rotation, Schematic, MAX_FILE_BYTES};
use circuito_erc::{ErcReport, ExtractIssue, Extraction};

use crate::i18n::tr;

/// The extension of a schematic.
pub const SCHEMATIC_EXTENSIONS: [&str; 1] = ["circuito"];
/// The extension of a circuit written in Circuito's text form.
pub const SCRIPT_EXTENSIONS: [&str; 1] = ["circ"];
/// The extensions of a SPICE deck.
pub const SPICE_EXTENSIONS: [&str; 5] = ["cir", "sp", "spice", "net", "ckt"];
/// The most lines of a netlist the Netlist tab keeps (a longer one is cut, and says so).
pub const MAX_NETLIST_LINES: usize = 100_000;

/// What the file is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A drawn schematic: it has a canvas, a rules check and a netlist extracted from it.
    Schematic,
    /// A SPICE deck: only its netlist.
    Spice,
}

/// The circuits that come with the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sample {
    RcLowPass,
    SeriesRlc,
    Divider,
}

impl Sample {
    pub const ALL: [Sample; 3] = [Sample::RcLowPass, Sample::SeriesRlc, Sample::Divider];

    /// The English name (translated where it is drawn).
    pub fn label(self) -> &'static str {
        match self {
            Sample::RcLowPass => "RC low-pass filter",
            Sample::SeriesRlc => "Series RLC resonator",
            Sample::Divider => "Resistor divider",
        }
    }

    /// One line on what it shows.
    pub fn detail(self) -> &'static str {
        match self {
            Sample::RcLowPass => "Frequency and step response",
            Sample::SeriesRlc => "A resonance at 5 kHz",
            Sample::Divider => "Operating point and DC sweep",
        }
    }

    /// The schematic.
    pub fn schematic(self) -> Schematic {
        match self {
            Sample::RcLowPass => rc_low_pass(),
            Sample::SeriesRlc => series_rlc(),
            Sample::Divider => divider(),
        }
    }
}

/// A circuit, read and checked.
#[derive(Clone, Debug)]
pub struct Circuit {
    /// The file it came from (`None` for a sample).
    pub path: Option<PathBuf>,
    pub sample: Option<Sample>,
    pub kind: Kind,
    /// The title the file gives itself.
    pub title: String,
    pub schematic: Option<Schematic>,
    pub extraction: Option<Arc<Extraction>>,
    /// What simulation runs on. Absent only when a schematic could not be extracted at all.
    pub netlist: Option<Arc<Netlist>>,
    /// The netlist as SPICE text, one entry per line (cut at [`MAX_NETLIST_LINES`]).
    pub netlist_lines: Vec<String>,
    pub netlist_cut: bool,
    /// The electrical rules check (schematics only).
    pub report: Option<ErcReport>,
    /// Why a simulation cannot run, when it cannot. In the engines' words.
    pub blocked: Option<String>,
    /// What the reader said on the way (SPICE decks), in the engines' words.
    pub warnings: Vec<String>,
}

impl Circuit {
    /// The name on the tab: the file's name, or the sample's.
    pub fn name(&self) -> String {
        if let Some(path) = &self.path {
            return file_name(path);
        }
        match self.sample {
            Some(s) => tr(s.label()).to_owned(),
            None => tr("Untitled").to_owned(),
        }
    }

    /// The analyses the circuit asks for, in order.
    pub fn analyses(&self) -> &[Analysis] {
        self.netlist.as_deref().map_or(&[], |n| &n.analyses)
    }

    /// Read a file: a schematic by its extension or its first character, otherwise a deck.
    ///
    /// # Errors
    /// The file cannot be read, is not text, or is not a valid schematic or deck.
    pub fn open(path: &Path) -> Result<Circuit, String> {
        let mut buf = Vec::new();
        std::fs::File::open(path)
            .and_then(|f| f.take(MAX_FILE_BYTES as u64 + 1).read_to_end(&mut buf))
            .map_err(|e| e.to_string())?;
        if buf.len() > MAX_FILE_BYTES {
            return Err(format!("the file is larger than {MAX_FILE_BYTES} bytes"));
        }
        let text = String::from_utf8(buf).map_err(|_| "the file is not UTF-8 text".to_owned())?;
        Circuit::from_text(Some(path), &text)
    }

    /// Read `text` as a schematic or a SPICE deck.
    ///
    /// # Errors
    /// The text is not valid.
    pub fn from_text(path: Option<&Path>, text: &str) -> Result<Circuit, String> {
        let by_extension = path
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        let script = by_extension
            .as_deref()
            .is_some_and(|e| SCRIPT_EXTENSIONS.contains(&e));
        let schematic = by_extension
            .as_deref()
            .is_some_and(|e| SCHEMATIC_EXTENSIONS.contains(&e))
            || (!script && text.trim_start().starts_with('{'));
        if script {
            let s = circuito_script::compile(text).map_err(|e| e.to_string())?;
            let mut circuit = Circuit::from_schematic(s);
            circuit.path = path.map(Path::to_path_buf);
            Ok(circuit)
        } else if schematic {
            let s = Schematic::from_json(text).map_err(|e| e.to_string())?;
            let mut circuit = Circuit::from_schematic(s);
            circuit.path = path.map(Path::to_path_buf);
            Ok(circuit)
        } else {
            let parsed = circuito_spice::parse(text).map_err(|e| e.to_string())?;
            let warnings = parsed
                .warnings
                .iter()
                .map(|w| {
                    if w.line == 0 {
                        w.message.clone()
                    } else {
                        format!("{}: {}", w.line, w.message)
                    }
                })
                .collect();
            let (netlist_lines, netlist_cut) = lines_of(text);
            Ok(Circuit {
                path: path.map(Path::to_path_buf),
                sample: None,
                kind: Kind::Spice,
                title: parsed.netlist.title.clone(),
                schematic: None,
                extraction: None,
                netlist: Some(Arc::new(parsed.netlist)),
                netlist_lines,
                netlist_cut,
                report: None,
                blocked: None,
                warnings,
            })
        }
    }

    /// A drawn schematic: its rules check and its extracted netlist.
    pub fn from_schematic(schematic: Schematic) -> Circuit {
        let report = circuito_erc::check(&schematic);
        let (extraction, blocked, text) = match circuito_erc::extract(&schematic) {
            Ok(x) => {
                let blocked = blocked_by(&x.issues);
                let text =
                    circuito_spice::write(&x.netlist).unwrap_or_else(|_| x.netlist.to_string());
                (Some(Arc::new(x)), blocked, text)
            }
            Err(e) => (None, Some(e.to_string()), String::new()),
        };
        let (netlist_lines, netlist_cut) = lines_of(&text);
        Circuit {
            path: None,
            sample: None,
            kind: Kind::Schematic,
            title: schematic.title.clone(),
            netlist: extraction.as_ref().map(|x| Arc::new(x.netlist.clone())),
            extraction,
            schematic: Some(schematic),
            netlist_lines,
            netlist_cut,
            report: Some(report),
            blocked,
            warnings: Vec::new(),
        }
    }

    /// A new, empty circuit that asks for an operating point.
    pub fn blank() -> Circuit {
        let mut s = Schematic::new("");
        s.analyses = vec![Analysis::Op];
        Circuit::from_schematic(s)
    }

    /// Whether the drawing can be edited (a SPICE deck has none).
    pub fn editable(&self) -> bool {
        self.schematic.is_some()
    }

    /// Work out again what the drawing says: the rules check, the netlist and what simulation
    /// needs. The file, the sample and what the reader said stay as they were.
    pub fn rederive(&mut self) {
        let Some(schematic) = self.schematic.take() else {
            return;
        };
        let (path, sample, warnings) = (
            self.path.take(),
            self.sample.take(),
            std::mem::take(&mut self.warnings),
        );
        *self = Circuit::from_schematic(schematic);
        self.path = path;
        self.sample = sample;
        self.warnings = warnings;
    }

    /// One of the circuits that come with the app.
    pub fn sample(sample: Sample) -> Circuit {
        let mut circuit = Circuit::from_schematic(sample.schematic());
        circuit.sample = Some(sample);
        circuit
    }

    /// The name of the net a pin is on (`part` and `pin` are indexes on `sheet`).
    pub fn net_of_pin(&self, sheet: usize, part: usize, pin: usize) -> Option<&str> {
        let x = self.extraction.as_deref()?;
        let key = PinRef { sheet, part, pin };
        let net = *x.resolution.pin_net.get(&key)?;
        x.net_names.get(net).map(String::as_str)
    }
}

fn blocked_by(issues: &[ExtractIssue]) -> Option<String> {
    let first = issues.first()?;
    Some(if issues.len() == 1 {
        first.message.clone()
    } else {
        format!("{} (+{})", first.message, issues.len() - 1)
    })
}

fn lines_of(text: &str) -> (Vec<String>, bool) {
    let mut lines: Vec<String> = text
        .lines()
        .take(MAX_NETLIST_LINES + 1)
        .map(str::to_owned)
        .collect();
    let cut = lines.len() > MAX_NETLIST_LINES;
    lines.truncate(MAX_NETLIST_LINES);
    (lines, cut)
}

/// A path's last component.
pub fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

// --- The samples --------------------------------------------------------------------------------

fn pt(b: &Builder, part: PartId, pin: &str) -> Point {
    // The pins of the library parts used below always exist; a missing one lands at the origin
    // and the rules check says so.
    b.pin(part, pin).unwrap_or(Point::new(0, 0))
}

fn rc_low_pass() -> Schematic {
    let mut b = Builder::new("RC low-pass filter, fc = 1/(2 pi R C) = 1.59 kHz");
    let v1 = b.place("V", "V1", "PULSE(0 1 0 1n 1n 5m 10m) AC 1", 0, 0);
    let r1 = b.place("R", "R1", "1k", 14, -4);
    b.rotate(r1, Rotation::R90);
    let c1 = b.place("C", "C1", "100n", 26, 0);
    b.wire(&[pt(&b, v1, "1"), pt(&b, r1, "1")]);
    b.wire(&[pt(&b, r1, "2"), pt(&b, c1, "1")]);
    b.label_pin(v1, "1", "in");
    b.label_pin(c1, "1", "out");
    b.power_pin(v1, "2", "GND");
    b.power_pin(c1, "2", "GND");
    b.schematic.analyses = vec![
        Analysis::Ac {
            sweep: Sweep::Dec,
            points: 20,
            fstart: 10.0,
            fstop: 100e3,
        },
        Analysis::Tran {
            tstep: 10e-6,
            tstop: 1e-3,
            tstart: 0.0,
            tmax: None,
            uic: false,
        },
    ];
    b.build()
}

fn series_rlc() -> Schematic {
    let mut b = Builder::new("Series RLC: f0 = 1/(2 pi sqrt(LC)) = 5.03 kHz, Q = 31.6");
    let v1 = b.place("V", "V1", "PULSE(0 1 0 1n 1n 1 2) AC 1", 0, 0);
    let r1 = b.place("R", "R1", "10", 14, -4);
    b.rotate(r1, Rotation::R90);
    let l1 = b.place("L", "L1", "10m", 28, -4);
    b.rotate(l1, Rotation::R90);
    let c1 = b.place("C", "C1", "100n", 40, 0);
    b.wire(&[pt(&b, v1, "1"), pt(&b, r1, "1")]);
    b.wire(&[pt(&b, r1, "2"), pt(&b, l1, "1")]);
    b.wire(&[pt(&b, l1, "2"), pt(&b, c1, "1")]);
    b.label_pin(v1, "1", "in");
    b.label_pin(c1, "1", "out");
    b.power_pin(v1, "2", "GND");
    b.power_pin(c1, "2", "GND");
    b.schematic.analyses = vec![
        Analysis::Ac {
            sweep: Sweep::Dec,
            points: 40,
            fstart: 100.0,
            fstop: 100e3,
        },
        Analysis::Tran {
            tstep: 2e-6,
            tstop: 2e-3,
            tstart: 0.0,
            tmax: None,
            uic: false,
        },
    ];
    b.build()
}

fn divider() -> Schematic {
    let mut b = Builder::new("Resistor divider: Vout = Vin R2 / (R1 + R2)");
    let v1 = b.place("V", "V1", "10", 0, 0);
    let r1 = b.place("R", "R1", "1k", 14, -4);
    b.rotate(r1, Rotation::R90);
    let r2 = b.place("R", "R2", "2.2k", 26, 0);
    b.wire(&[pt(&b, v1, "1"), pt(&b, r1, "1")]);
    b.wire(&[pt(&b, r1, "2"), pt(&b, r2, "1")]);
    b.label_pin(v1, "1", "in");
    b.label_pin(r2, "1", "out");
    b.power_pin(v1, "2", "GND");
    b.power_pin(r2, "2", "GND");
    b.schematic.analyses = vec![
        Analysis::Op,
        Analysis::Dc {
            source: "V1".into(),
            start: 0.0,
            stop: 10.0,
            step: 0.5,
        },
    ];
    b.build()
}

/// Whether `path` is a kind of file Circuito writes (a schematic or a circuit as code).
pub fn is_writable_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|e| {
            SCHEMATIC_EXTENSIONS.contains(&e.as_str()) || SCRIPT_EXTENSIONS.contains(&e.as_str())
        })
}

/// `path` as the person named it, with `.circuito` added when it has no extension of ours.
pub fn with_extension_if_missing(path: &Path) -> PathBuf {
    if is_writable_extension(path) {
        path.to_path_buf()
    } else {
        let mut name = path.as_os_str().to_owned();
        name.push(".circuito");
        PathBuf::from(name)
    }
}

/// The text a file at `path` holds for `schematic`: JSON, or the text form for `.circ`.
///
/// # Errors
/// The text form describes one sheet only; a schematic with several is saved as JSON.
pub fn serialise(schematic: &Schematic, path: &Path) -> Result<String, String> {
    let script = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|e| SCRIPT_EXTENSIONS.contains(&e.as_str()));
    if script {
        if schematic.sheets.len() != 1 {
            return Err(
                tr("Several sheets cannot be written as code; save as .circuito.").to_owned(),
            );
        }
        let options = circuito_script::PrintOptions { layout: true };
        Ok(circuito_script::print(
            &circuito_script::design_of(schematic),
            options,
        ))
    } else {
        schematic.to_json().map_err(|e| e.to_string())
    }
}

/// Write `text` to `path` through a temporary sibling, so a failure never leaves half a file.
///
/// # Errors
/// The folder cannot be written to.
pub fn write_atomically(path: &Path, text: &str) -> Result<(), String> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, text)
        .and_then(|()| std::fs::rename(&tmp, path))
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            e.to_string()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use circuito_erc::Severity;

    #[test]
    fn the_examples_written_as_code_open_and_are_ready_to_simulate() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let mut seen = 0;
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("circ") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let circuit = Circuit::from_text(Some(&path), &text)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert!(circuit.schematic.is_some(), "{}", path.display());
            assert!(!circuit.analyses().is_empty(), "{}", path.display());
            seen += 1;
        }
        assert!(seen >= 3, "found {seen} examples");
    }

    #[test]
    fn every_sample_is_clean_and_ready_to_simulate() {
        for sample in Sample::ALL {
            let c = Circuit::sample(sample);
            let report = c.report.as_ref().unwrap();
            assert!(
                report.violations.is_empty(),
                "{sample:?}: {:?}",
                report.violations
            );
            assert!(c.blocked.is_none(), "{sample:?}: {:?}", c.blocked);
            assert!(!c.analyses().is_empty());
            assert!(c.netlist_lines.iter().any(|l| l.starts_with('V')));
        }
    }

    #[test]
    fn a_sample_survives_a_round_trip_through_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rc.circuito");
        let s = Sample::RcLowPass.schematic();
        std::fs::write(&path, s.to_json().unwrap()).unwrap();
        let c = Circuit::open(&path).unwrap();
        assert_eq!(c.kind, Kind::Schematic);
        assert_eq!(c.schematic.as_ref(), Some(&s));
        assert_eq!(c.name(), "rc.circuito");
        assert_eq!(c.analyses().len(), 2);
    }

    #[test]
    fn a_spice_deck_has_a_netlist_and_no_drawing() {
        let c = Circuit::from_text(
            Some(Path::new("x.cir")),
            "t\nV1 in 0 5\nR1 in out 1k\nR2 out 0 1k\n.op\n.end\n",
        )
        .unwrap();
        assert_eq!(c.kind, Kind::Spice);
        assert!(c.schematic.is_none() && c.report.is_none());
        assert_eq!(c.analyses(), &[Analysis::Op]);
        assert_eq!(c.netlist_lines.first().map(String::as_str), Some("t"));
    }

    #[test]
    fn bad_files_are_errors_not_panics() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin.circuito");
        std::fs::write(&bin, [0xff, 0xfe, 0x00]).unwrap();
        assert!(Circuit::open(&bin).is_err());
        assert!(Circuit::open(&dir.path().join("missing.cir")).is_err());
        assert!(Circuit::from_text(None, "{ \"format_version\": 1").is_err());
        assert!(Circuit::from_text(None, "{}").is_err());
        assert!(Circuit::from_text(Some(Path::new("x.circuito")), "").is_err());
        assert!(Circuit::from_text(None, "t\nR1 a b not-a-number\n").is_err());
    }

    #[test]
    fn a_schematic_with_a_part_that_cannot_be_extracted_blocks_simulation() {
        let mut b = Builder::new("broken");
        let r1 = b.place("R", "R1", "", 0, 0);
        b.power_pin(r1, "2", "GND");
        b.label_pin(r1, "1", "a");
        let c = Circuit::from_schematic(b.build());
        assert!(c.blocked.is_some());
        let report = c.report.unwrap();
        assert!(report.count(Severity::Error) + report.count(Severity::Warning) > 0);
    }

    #[test]
    fn pins_know_their_nets() {
        let c = Circuit::sample(Sample::RcLowPass);
        let s = c.schematic.as_ref().unwrap();
        let c1 = s.sheets[0]
            .parts
            .iter()
            .position(|p| p.refdes == "C1")
            .unwrap();
        assert_eq!(c.net_of_pin(0, c1, 0), Some("out"));
        assert_eq!(c.net_of_pin(0, c1, 9), None);
    }

    #[test]
    fn a_long_netlist_is_cut() {
        let mut text = String::from("t\n");
        for i in 0..(MAX_NETLIST_LINES + 10) {
            text.push_str(&format!("* {i}\n"));
        }
        let (lines, cut) = lines_of(&text);
        assert!(cut);
        assert_eq!(lines.len(), MAX_NETLIST_LINES);
    }
}
