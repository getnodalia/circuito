// SPDX-License-Identifier: MIT OR Apache-2.0
//! What an analysis returns: named signals over an axis, plus the record of how they were made.

use nodalia_numerics::Complex64;

/// The numbers of one signal.
#[derive(Debug, Clone, PartialEq)]
pub enum Values {
    /// Real samples (`.op`, `.dc`, `.tran`).
    Real(Vec<f64>),
    /// Complex phasors (`.ac`).
    Complex(Vec<Complex64>),
}

/// What a signal measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// Volts.
    Volt,
    /// Amperes.
    Ampere,
}

/// One named trace, `V(out)` or `I(V1)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Signal {
    /// `V(node)` or `I(element)`, in the case the netlist used.
    pub name: String,
    /// Volts or amperes.
    pub unit: Unit,
    /// The samples, one per axis point.
    pub values: Values,
}

impl Signal {
    /// The number of samples.
    #[must_use]
    pub fn len(&self) -> usize {
        match &self.values {
            Values::Real(v) => v.len(),
            Values::Complex(v) => v.len(),
        }
    }

    /// True when there are no samples.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Sample `i` as a complex number (imaginary part 0 for a real signal).
    #[must_use]
    pub fn at(&self, i: usize) -> Option<Complex64> {
        match &self.values {
            Values::Real(v) => v.get(i).map(|&x| Complex64::new(x, 0.0)),
            Values::Complex(v) => v.get(i).copied(),
        }
    }

    /// The real part of sample `i`.
    #[must_use]
    pub fn real(&self, i: usize) -> Option<f64> {
        self.at(i).map(|c| c.re)
    }

    /// The magnitude of sample `i`.
    #[must_use]
    pub fn magnitude(&self, i: usize) -> Option<f64> {
        self.at(i).map(|c| c.norm())
    }

    /// The phase of sample `i` in degrees, in (-180, 180].
    #[must_use]
    pub fn phase_deg(&self, i: usize) -> Option<f64> {
        self.at(i).map(|c| c.arg().to_degrees())
    }

    /// The magnitude of sample `i` in decibels, `20 log10 |x|` (negative infinity for zero).
    #[must_use]
    pub fn db(&self, i: usize) -> Option<f64> {
        self.magnitude(i).map(|m| 20.0 * m.log10())
    }
}

/// Signals sharing one axis.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    /// `time`, `frequency`, the swept source's name, or `op`.
    pub axis_name: String,
    /// The axis values: seconds, hertz or the source's value; a single 0 for `.op`.
    pub axis: Vec<f64>,
    /// One signal per node voltage and branch current.
    pub signals: Vec<Signal>,
}

impl Table {
    /// A signal by name, ignoring case (`v(out)` finds `V(OUT)`).
    #[must_use]
    pub fn signal(&self, name: &str) -> Option<&Signal> {
        self.signals
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
    }
}

/// Which analysis produced a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `.op`.
    Op,
    /// `.dc`.
    Dc,
    /// `.ac`.
    Ac,
    /// `.tran`.
    Tran,
}

/// How the numbers were produced: enough to reproduce or question them.
#[derive(Debug, Clone, PartialEq)]
pub struct RunInfo {
    /// The engine and its version.
    pub engine: String,
    /// The analysis card as written back, e.g. `.tran 1u 5m`.
    pub analysis: String,
    /// How the DC operating point was found: `newton`, `gmin stepping` or `source stepping`.
    pub dc_method: String,
    /// The transient integration method, when there was one.
    pub integration: Option<String>,
    /// The tolerances used.
    pub reltol: f64,
    /// Absolute current tolerance.
    pub abstol: f64,
    /// Absolute voltage tolerance.
    pub vntol: f64,
    /// Junction conductance floor.
    pub gmin: f64,
    /// Temperature, degrees Celsius.
    pub temp: f64,
    /// Newton iterations over the whole run.
    pub newton_iterations: usize,
    /// Matrix factorisations over the whole run.
    pub factorizations: usize,
    /// Accepted transient steps.
    pub accepted_steps: usize,
    /// Rejected transient steps (truncation error or Newton failure).
    pub rejected_steps: usize,
    /// The reciprocal condition estimate of the operating-point matrix, when computed.
    pub rcond: Option<f64>,
    /// Things worth knowing that did not stop the run.
    pub warnings: Vec<String>,
}

/// A finished analysis.
#[derive(Debug, Clone, PartialEq)]
pub struct AnalysisResult {
    /// Which analysis.
    pub kind: Kind,
    /// The traces.
    pub table: Table,
    /// How they were made.
    pub info: RunInfo,
}
