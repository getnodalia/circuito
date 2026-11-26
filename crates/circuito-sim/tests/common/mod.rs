// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(dead_code)]
//! Shared helpers for the validation tests.

use circuito_sim::{simulate, AnalysisResult, Control, Options, SimError, Table};

/// Parse a netlist (title line first).
pub fn parse(text: &str) -> circuito_core::netlist::Netlist {
    match circuito_spice::parse(text) {
        Ok(p) => p.netlist,
        Err(e) => panic!("test netlist does not parse: {e}\n{text}"),
    }
}

/// Options with tight tolerances so analytic comparisons are limited by the model, not by
/// the stopping rule.
pub fn tight() -> Options {
    Options {
        reltol: 1e-9,
        vntol: 1e-12,
        abstol: 1e-15,
        gmin: 1e-15,
        ..Options::default()
    }
}

/// Run the first analysis card of the netlist.
pub fn run_with(text: &str, opts: &Options) -> Result<AnalysisResult, SimError> {
    let n = parse(text);
    let a = n
        .analyses
        .first()
        .cloned()
        .unwrap_or(circuito_core::netlist::Analysis::Op);
    simulate(&n, &a, opts, &Control::new())
}

pub fn run(text: &str) -> AnalysisResult {
    match run_with(text, &Options::default()) {
        Ok(r) => r,
        Err(e) => panic!("simulation failed: {e}"),
    }
}

pub fn run_tight(text: &str) -> AnalysisResult {
    match run_with(text, &tight()) {
        Ok(r) => r,
        Err(e) => panic!("simulation failed: {e}"),
    }
}

/// The only sample of a signal in a `.op` table.
pub fn op(t: &Table, name: &str) -> f64 {
    match t.signal(name) {
        Some(s) => s.real(0).unwrap_or(f64::NAN),
        None => panic!(
            "no signal {name}; have {:?}",
            t.signals.iter().map(|s| &s.name).collect::<Vec<_>>()
        ),
    }
}

/// A real signal's samples.
pub fn col(t: &Table, name: &str) -> Vec<f64> {
    match t.signal(name) {
        Some(s) => (0..s.len())
            .map(|i| s.real(i).unwrap_or(f64::NAN))
            .collect(),
        None => panic!("no signal {name}"),
    }
}

pub fn assert_close(got: f64, want: f64, rel: f64, abs: f64, what: &str) {
    let tol = rel * want.abs() + abs;
    assert!(
        (got - want).abs() <= tol,
        "{what}: got {got:e}, want {want:e} (tolerance {tol:e})"
    );
}

/// Linear interpolation of a signal at time `t`.
pub fn at_time(t: &Table, name: &str, time: f64) -> f64 {
    let y = col(t, name);
    let x = &t.axis;
    let i = x.partition_point(|&a| a <= time);
    if i == 0 {
        return y[0];
    }
    if i >= x.len() {
        return y[y.len() - 1];
    }
    let (x0, x1, y0, y1) = (x[i - 1], x[i], y[i - 1], y[i]);
    y0 + (y1 - y0) * (time - x0) / (x1 - x0)
}
