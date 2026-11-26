// SPDX-License-Identifier: MIT OR Apache-2.0
//! Results turned into what a plot draws: panels of traces over an axis, with ranges that suit
//! them. No drawing happens here, so the numbers can be tested.

use std::collections::HashSet;

use circuito_sim::{AnalysisResult, Kind, Signal, Unit, Values};

/// What a panel measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quantity {
    Voltage,
    Current,
}

/// How the signal is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The value itself (`.dc`, `.tran`).
    Value,
    /// Magnitude in decibels (`.ac`).
    Gain,
    /// Phase in degrees (`.ac`).
    Phase,
}

/// One trace.
#[derive(Clone, Debug, PartialEq)]
pub struct Series {
    pub name: String,
    /// Which colour it keeps whatever else is shown: its place among all the signals.
    pub slot: usize,
    pub points: Vec<(f64, f64)>,
}

/// Traces sharing an axis.
#[derive(Clone, Debug, PartialEq)]
pub struct Panel {
    pub quantity: Quantity,
    pub role: Role,
    pub series: Vec<Series>,
    /// The x range of the data.
    pub x: (f64, f64),
    /// The y range to draw, padded.
    pub y: (f64, f64),
}

impl Panel {
    /// The English title (translated where it is drawn).
    pub fn title(&self) -> &'static str {
        match (self.role, self.quantity) {
            (Role::Gain, Quantity::Voltage) => "Gain",
            (Role::Gain, Quantity::Current) => "Current gain",
            (Role::Phase, _) => "Phase",
            (Role::Value, Quantity::Voltage) => "Voltage",
            (Role::Value, Quantity::Current) => "Current",
        }
    }

    /// The unit of the y axis.
    pub fn y_unit(&self) -> &'static str {
        match (self.role, self.quantity) {
            (Role::Gain, _) => "dB",
            (Role::Phase, _) => "°",
            (Role::Value, Quantity::Voltage) => "V",
            (Role::Value, Quantity::Current) => "A",
        }
    }
}

/// Everything a result shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Plot {
    pub kind: Kind,
    /// The x axis is logarithmic (frequency).
    pub x_log: bool,
    /// The unit of the x axis.
    pub x_unit: &'static str,
    pub panels: Vec<Panel>,
}

/// A signal as the legend lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignalInfo {
    pub name: String,
    pub quantity: Quantity,
    pub slot: usize,
}

fn quantity(unit: Unit) -> Quantity {
    match unit {
        Unit::Volt => Quantity::Voltage,
        Unit::Ampere => Quantity::Current,
    }
}

/// The signals of a result, in the engine's order.
pub fn signals(result: &AnalysisResult) -> Vec<SignalInfo> {
    result
        .table
        .signals
        .iter()
        .enumerate()
        .map(|(slot, s)| SignalInfo {
            name: s.name.clone(),
            quantity: quantity(s.unit),
            slot,
        })
        .collect()
}

/// What starts hidden: branch currents, which would crowd the voltages.
pub fn default_hidden(result: &AnalysisResult) -> HashSet<String> {
    result
        .table
        .signals
        .iter()
        .filter(|s| s.unit == Unit::Ampere)
        .map(|s| s.name.clone())
        .collect()
}

/// The operating point: each signal and its value.
pub fn operating_point(result: &AnalysisResult) -> Vec<(String, f64, Unit)> {
    result
        .table
        .signals
        .iter()
        .filter_map(|s| s.real(0).map(|v| (s.name.clone(), v, s.unit)))
        .collect()
}

/// Add a multiple of 360 degrees where the phase jumps, so a trace stays continuous.
pub fn unwrap_phase(values: &mut [f64]) {
    let mut offset = 0.0;
    let mut previous: Option<f64> = None;
    for v in values.iter_mut() {
        let raw = *v;
        if let Some(p) = previous {
            let jump = raw - p;
            if jump > 180.0 {
                offset -= 360.0 * (jump / 360.0).round().max(1.0);
            } else if jump < -180.0 {
                offset += 360.0 * (-jump / 360.0).round().max(1.0);
            }
        }
        previous = Some(raw);
        *v = raw + offset;
    }
}

fn points(
    axis: &[f64],
    values: impl Iterator<Item = Option<f64>>,
    positive_x: bool,
) -> Vec<(f64, f64)> {
    axis.iter()
        .zip(values)
        .filter_map(|(x, y)| {
            let y = y?;
            (x.is_finite() && y.is_finite() && (!positive_x || *x > 0.0)).then_some((*x, y))
        })
        .collect()
}

fn range(series: &[Series], pick: impl Fn(&(f64, f64)) -> f64) -> Option<(f64, f64)> {
    let mut all = series.iter().flat_map(|s| s.points.iter()).map(&pick);
    let first = all.next()?;
    Some(all.fold((first, first), |(lo, hi), v| (lo.min(v), hi.max(v))))
}

/// A y range with room around the data (and some range at all for a flat trace).
pub fn padded(lo: f64, hi: f64) -> (f64, f64) {
    if !(lo.is_finite() && hi.is_finite()) {
        return (0.0, 1.0);
    }
    let span = hi - lo;
    if span <= f64::EPSILON * lo.abs().max(hi.abs()).max(1e-300) {
        let pad = if lo == 0.0 { 1.0 } else { lo.abs() * 0.1 };
        return (lo - pad, hi + pad);
    }
    (lo - span * 0.06, hi + span * 0.06)
}

fn panel(quantity: Quantity, role: Role, series: Vec<Series>) -> Option<Panel> {
    if series.iter().all(|s| s.points.is_empty()) {
        return None;
    }
    let x = range(&series, |p| p.0)?;
    let y = range(&series, |p| p.1)?;
    Some(Panel {
        quantity,
        role,
        series,
        x,
        y: padded(y.0, y.1),
    })
}

fn value_of(s: &Signal, i: usize, role: Role) -> Option<f64> {
    match role {
        Role::Value => s.real(i),
        Role::Gain => s.db(i),
        Role::Phase => s.phase_deg(i),
    }
}

/// The traces of `result` that are not in `hidden`. An `.op` has no plot: the panels are empty.
pub fn build(result: &AnalysisResult, hidden: &HashSet<String>) -> Plot {
    let table = &result.table;
    let ac = result.kind == Kind::Ac;
    let x_unit = match result.kind {
        Kind::Ac => "Hz",
        Kind::Tran => "s",
        Kind::Dc => {
            if table.axis_name.to_ascii_uppercase().starts_with('I') {
                "A"
            } else {
                "V"
            }
        }
        Kind::Op => "",
    };
    let mut panels = Vec::new();
    if result.kind != Kind::Op {
        let roles: &[Role] = if ac {
            &[Role::Gain, Role::Phase]
        } else {
            &[Role::Value]
        };
        for q in [Quantity::Voltage, Quantity::Current] {
            for role in roles {
                let series: Vec<Series> = table
                    .signals
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| quantity(s.unit) == q && !hidden.contains(&s.name))
                    .map(|(slot, s)| {
                        let values = (0..s.len()).map(|i| value_of(s, i, *role));
                        let mut pts = points(&table.axis, values, ac);
                        if *role == Role::Phase {
                            let mut ys: Vec<f64> = pts.iter().map(|p| p.1).collect();
                            unwrap_phase(&mut ys);
                            for (p, y) in pts.iter_mut().zip(ys) {
                                p.1 = y;
                            }
                        }
                        Series {
                            name: s.name.clone(),
                            slot,
                            points: pts,
                        }
                    })
                    .collect();
                if let Some(p) = panel(q, *role, series) {
                    panels.push(p);
                }
            }
        }
    }
    // Complex data never reaches a Value panel, and real data never a Gain one.
    debug_assert!(table.signals.iter().all(|s| matches!(
        (&s.values, ac),
        (Values::Complex(_), true) | (Values::Real(_), false)
    )));
    Plot {
        kind: result.kind,
        x_log: ac,
        x_unit,
        panels,
    }
}

/// At most `max` points of `points` that keep its shape: for every stretch the lowest and the
/// highest point, so a narrow peak survives. The first and last point always stay.
pub fn decimate(points: &[(f64, f64)], max: usize) -> Vec<(f64, f64)> {
    let max = max.max(4);
    if points.len() <= max {
        return points.to_vec();
    }
    let buckets = (max - 2) / 2;
    let per = points.len().div_ceil(buckets).max(1);
    let mut out = Vec::with_capacity(max);
    for chunk in points.chunks(per) {
        let mut lo = chunk[0];
        let mut hi = chunk[0];
        let (mut lo_i, mut hi_i) = (0, 0);
        for (i, p) in chunk.iter().enumerate() {
            if p.1 < lo.1 {
                lo = *p;
                lo_i = i;
            }
            if p.1 > hi.1 {
                hi = *p;
                hi_i = i;
            }
        }
        if lo_i <= hi_i {
            out.push(lo);
            if lo_i != hi_i {
                out.push(hi);
            }
        } else {
            out.push(hi);
            out.push(lo);
        }
    }
    out
}

/// The point of `points` whose x is nearest `x`.
pub fn nearest(points: &[(f64, f64)], x: f64) -> Option<(f64, f64)> {
    points
        .iter()
        .copied()
        .min_by(|a, b| (a.0 - x).abs().total_cmp(&(b.0 - x).abs()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Circuit, Sample};
    use crate::sim::{Outcome, Reply, Runner};
    use std::time::{Duration, Instant};

    fn run(sample: Sample, index: usize) -> AnalysisResult {
        let c = Circuit::sample(sample);
        let netlist = c.netlist.clone().unwrap();
        let mut runner = Runner::spawn(|| {}).unwrap();
        runner
            .submit(1, index, netlist.clone(), netlist.analyses[index].clone())
            .unwrap();
        let end = Instant::now() + Duration::from_secs(30);
        while Instant::now() < end {
            for r in runner.poll() {
                if let Reply::Done {
                    outcome: Outcome::Finished(result),
                    ..
                } = r
                {
                    return *result;
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!("no result")
    }

    #[test]
    fn an_ac_result_has_gain_and_phase_panels_over_a_log_axis() {
        let result = run(Sample::RcLowPass, 0);
        let hidden = default_hidden(&result);
        assert!(hidden
            .iter()
            .any(|n| n.to_ascii_uppercase().starts_with("I(")));
        let plot = build(&result, &hidden);
        assert!(plot.x_log);
        assert_eq!(plot.x_unit, "Hz");
        let roles: Vec<Role> = plot.panels.iter().map(|p| p.role).collect();
        assert_eq!(roles, vec![Role::Gain, Role::Phase]);
        let gain = &plot.panels[0];
        let out = gain
            .series
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case("V(out)"))
            .unwrap();
        // At the corner frequency 1/(2 pi R C) the gain is -3.0103 dB (to 0.05 dB between samples).
        let fc = 1.0 / (2.0 * std::f64::consts::PI * 1e3 * 100e-9);
        let (_, db) = nearest(&out.points, fc).unwrap();
        assert!((db + 3.0103).abs() < 0.2, "{db}");
        let phase = &plot.panels[1];
        let out_phase = phase
            .series
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case("V(out)"))
            .unwrap();
        let last = out_phase.points.last().unwrap().1;
        assert!(last < -80.0 && last > -90.0, "{last}");
        assert!(gain.y.0 < gain.y.1);
    }

    #[test]
    fn hiding_a_signal_removes_its_trace_and_keeps_the_others_colours() {
        let result = run(Sample::RcLowPass, 0);
        let mut hidden = default_hidden(&result);
        let all = build(&result, &hidden);
        let slots: Vec<usize> = all.panels[0].series.iter().map(|s| s.slot).collect();
        let first = all.panels[0].series[0].name.clone();
        hidden.insert(first.clone());
        let fewer = build(&result, &hidden);
        assert!(fewer.panels[0].series.iter().all(|s| s.name != first));
        assert_eq!(fewer.panels[0].series.len() + 1, slots.len());
        // The remaining trace keeps the slot it had.
        assert!(slots.contains(&fewer.panels[0].series[0].slot));
        // Showing a current adds a panel for it.
        let with_current = build(&result, &HashSet::new());
        assert!(with_current
            .panels
            .iter()
            .any(|p| p.quantity == Quantity::Current));
        // Hiding everything leaves nothing to draw.
        let none: HashSet<String> = signals(&result).into_iter().map(|s| s.name).collect();
        assert!(build(&result, &none).panels.is_empty());
    }

    #[test]
    fn a_transient_is_one_voltage_panel_over_a_linear_time_axis() {
        let result = run(Sample::RcLowPass, 1);
        let plot = build(&result, &default_hidden(&result));
        assert!(!plot.x_log);
        assert_eq!(plot.x_unit, "s");
        assert_eq!(plot.panels.len(), 1);
        let p = &plot.panels[0];
        assert_eq!((p.role, p.quantity), (Role::Value, Quantity::Voltage));
        assert!(p.x.0 >= 0.0 && p.x.1 <= 1.0001e-3);
        assert!(p.y.0 < 0.0 && p.y.1 > 1.0, "{:?}", p.y);
    }

    #[test]
    fn a_dc_sweep_is_linear_in_the_source_and_an_op_has_no_plot() {
        let sweep = run(Sample::Divider, 1);
        let plot = build(&sweep, &default_hidden(&sweep));
        assert_eq!((plot.x_log, plot.x_unit), (false, "V"));
        let out = plot.panels[0]
            .series
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case("V(out)"))
            .unwrap();
        let (x, y) = out.points[out.points.len() - 1];
        assert!((y - x * 2.2 / 3.2).abs() < 1e-9);
        let op = run(Sample::Divider, 0);
        assert!(build(&op, &HashSet::new()).panels.is_empty());
        let rows = operating_point(&op);
        let vin = rows
            .iter()
            .find(|r| r.0.eq_ignore_ascii_case("V(in)"))
            .unwrap();
        assert!((vin.1 - 10.0).abs() < 1e-9);
    }

    #[test]
    fn phase_is_continuous_across_the_wrap() {
        let mut v = vec![-170.0, -175.0, 180.0, 175.0, 170.0];
        unwrap_phase(&mut v);
        assert_eq!(v, vec![-170.0, -175.0, -180.0, -185.0, -190.0]);
        let mut up = vec![170.0, 178.0, -178.0, -170.0];
        unwrap_phase(&mut up);
        assert_eq!(up, vec![170.0, 178.0, 182.0, 190.0]);
        let mut none: Vec<f64> = Vec::new();
        unwrap_phase(&mut none);
    }

    #[test]
    fn ranges_have_room_and_flat_traces_get_some() {
        let (lo, hi) = padded(0.0, 10.0);
        assert!(lo < 0.0 && hi > 10.0);
        let (lo, hi) = padded(5.0, 5.0);
        assert!(lo < 5.0 && hi > 5.0);
        let (lo, hi) = padded(0.0, 0.0);
        assert_eq!((lo, hi), (-1.0, 1.0));
        assert_eq!(padded(f64::NAN, 1.0), (0.0, 1.0));
    }

    #[test]
    fn decimation_bounds_the_count_and_keeps_peaks_and_ends() {
        let mut pts: Vec<(f64, f64)> = (0..100_000).map(|i| (i as f64, 0.0)).collect();
        pts[54_321].1 = 7.0;
        pts[12_345].1 = -3.0;
        let d = decimate(&pts, 500);
        assert!(d.len() <= 500, "{}", d.len());
        assert_eq!(d.first(), pts.first());
        assert!(d.contains(&(54_321.0, 7.0)) && d.contains(&(12_345.0, -3.0)));
        assert!(d.windows(2).all(|w| w[0].0 <= w[1].0), "stays in order");
        assert_eq!(decimate(&pts[..10], 500).len(), 10);
        assert!(decimate(&[], 10).is_empty());
        assert!(decimate(&pts[..100], 0).len() <= 100);
    }

    #[test]
    fn nearest_finds_the_closest_x() {
        let pts = [(1.0, 10.0), (2.0, 20.0), (4.0, 40.0)];
        assert_eq!(nearest(&pts, 2.9), Some((2.0, 20.0)));
        assert_eq!(nearest(&pts, 3.1), Some((4.0, 40.0)));
        assert_eq!(nearest(&[], 1.0), None);
    }
}
