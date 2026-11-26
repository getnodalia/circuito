// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Transient validation against analytic solutions, with the error bounds stated.

mod common;
use common::*;

use circuito_sim::{simulate, Control, Integration, Options, SimError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const STEP: &str = "PULSE(0 1 0 1p 1p 1 2)";

/// Largest absolute error of signal `name` against `f(t)` over all samples with `t >= from`.
fn max_error(t: &circuito_sim::Table, name: &str, from: f64, f: impl Fn(f64) -> f64) -> f64 {
    let y = col(t, name);
    t.axis
        .iter()
        .zip(&y)
        .filter(|(&x, _)| x >= from)
        .map(|(&x, &v)| (v - f(x)).abs())
        .fold(0.0, f64::max)
}

#[test]
fn rc_step_response_stays_inside_the_stated_error_and_improves_with_the_tolerance() {
    let (r, c) = (1e3, 1e-6);
    let tau = r * c;
    let text = format!(
        "rc\nV1 in 0 {STEP}\nR1 in out {r}\nC1 out 0 {c}\n.tran {tau} {}\n",
        6.0 * tau
    );
    let exact = |t: f64| 1.0 - (-t / tau).exp();
    let run_err = |reltol: f64| {
        let res = run_with(
            &text,
            &Options {
                reltol,
                ..Options::default()
            },
        )
        .unwrap();
        (
            max_error(&res.table, "V(out)", 1e-9, exact),
            res.info.accepted_steps,
        )
    };
    // Measured on this circuit (1 V swing, TRTOL 7): about 4e-4 at the default tolerance, where the
    // SPICE rule hmax = tstop/50 sets the step, and 5e-5 at 1e-6.
    let (e3, n3) = run_err(1e-3);
    let (e6, n6) = run_err(1e-6);
    assert!(e3 < 1e-3, "default tolerance: {e3}");
    assert!(e6 < 1e-4 && e6 < e3 / 4.0, "tight tolerance: {e6} vs {e3}");
    assert!(
        n6 > n3,
        "a tighter tolerance takes more steps ({n3} -> {n6})"
    );
}

#[test]
fn fixed_step_trapezoidal_is_second_order_and_backward_euler_first_order() {
    // Discharge of an RC from 1 V: v = exp(-t/tau). With a loose reltol the step is the user's
    // tmax, never the error controller's, so halving it must quarter (TR) or halve (BE) the error.
    let (r, c): (f64, f64) = (1e3, 1e-6);
    let tau = r * c;
    let err = |tmax: f64, method: Integration| {
        let text = format!(
            "rc\nR1 a 0 {r}\nC1 a 0 {c} IC=1\n.tran {tmax} {} 0 {tmax} UIC\n",
            6.0 * tau
        );
        let res = run_with(
            &text,
            &Options {
                reltol: 0.5,
                method,
                ..Options::default()
            },
        )
        .unwrap();
        max_error(&res.table, "V(a)", 0.0, |t| (-t / tau).exp())
    };
    let (a, b, c4) = (
        err(tau / 10.0, Integration::Trapezoidal),
        err(tau / 20.0, Integration::Trapezoidal),
        err(tau / 40.0, Integration::Trapezoidal),
    );
    assert!(
        a / b > 3.2 && a / b < 4.8,
        "halving h should quarter the error: {a:e} {b:e} {}",
        a / b
    );
    assert!(b / c4 > 3.2 && b / c4 < 4.8, "{b:e} {c4:e} {}", b / c4);
    let (d1, d2) = (
        err(tau / 10.0, Integration::BackwardEuler),
        err(tau / 20.0, Integration::BackwardEuler),
    );
    assert!(
        d1 / d2 > 1.7 && d1 / d2 < 2.3,
        "backward Euler is first order: {d1:e} {d2:e} {}",
        d1 / d2
    );
}

#[test]
fn rl_step_current_rises_exponentially() {
    let (r, l) = (50.0, 10e-3);
    let tau = l / r;
    let text = format!(
        "rl\nV1 in 0 {STEP}\nR1 in a {r}\nL1 a 0 {l}\n.tran {} {}\n",
        tau / 20.0,
        6.0 * tau
    );
    let res = run_with(
        &text,
        &Options {
            reltol: 1e-5,
            ..Options::default()
        },
    )
    .unwrap();
    let e = max_error(&res.table, "I(L1)", 1e-9, |t| {
        (1.0 / r) * (1.0 - (-t / tau).exp())
    });
    assert!(e < 1e-5 * (1.0 / r) * 10.0, "error {e}");
}

#[test]
fn series_rlc_step_matches_the_underdamped_closed_form() {
    let (r, l, c): (f64, f64, f64) = (20.0, 1e-3, 1e-6);
    let alpha = r / (2.0 * l);
    let w0 = 1.0 / (l * c).sqrt();
    let wd = (w0 * w0 - alpha * alpha).sqrt();
    let text = format!("rlc\nV1 in 0 {STEP}\nR1 in a {r}\nL1 a b {l}\nC1 b 0 {c}\n.tran 2u 1.5m\n");
    let res = run_with(
        &text,
        &Options {
            reltol: 1e-5,
            ..Options::default()
        },
    )
    .unwrap();
    let exact =
        |t: f64| 1.0 - (-alpha * t).exp() * ((wd * t).cos() + (alpha / wd) * (wd * t).sin());
    let e = max_error(&res.table, "V(b)", 1e-9, exact);
    // The peak overshoot of this circuit is exp(-pi alpha / wd) = ~0.63; require 3e-4 absolute.
    assert!(e < 1e-3, "error {e}");
    let peak = col(&res.table, "V(b)").into_iter().fold(0.0, f64::max);
    let want_peak = 1.0 + (-std::f64::consts::PI * alpha / wd).exp();
    assert_close(peak, want_peak, 2e-3, 0.0, "overshoot");
}

#[test]
fn initial_conditions_with_uic_discharge_a_capacitor_and_an_inductor() {
    let (r, c) = (1e3, 1e-6);
    let res = run_with(
        &format!("uic\nR1 a 0 {r}\nC1 a 0 {c} IC=5\n.tran 50u 5m UIC\n"),
        &Options {
            reltol: 1e-5,
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(res.info.dc_method, "uic (initial conditions)");
    assert_close(
        res.table.signal("V(a)").unwrap().real(0).unwrap(),
        5.0,
        1e-9,
        0.0,
        "initial voltage",
    );
    let e = max_error(&res.table, "V(a)", 0.0, |t| 5.0 * (-t / (r * c)).exp());
    assert!(e < 5e-4, "error {e}");
    // Inductor with an initial current, shorted through a resistor: i = I0 exp(-R t / L).
    let (r, l) = (10.0, 1e-3);
    let res = run_with(
        &format!("uicl\nL1 a 0 {l} IC=2\nR1 a 0 {r}\n.tran 5u 500u UIC\n"),
        &Options {
            reltol: 1e-5,
            ..Options::default()
        },
    )
    .unwrap();
    // The inductor current flows a -> 0 through L1 and must come back through R1 (0 -> a).
    let e = max_error(&res.table, "I(L1)", 0.0, |t| 2.0 * (-r * t / l).exp());
    assert!(e < 2e-4, "error {e}");
}

#[test]
fn lc_tank_keeps_its_amplitude_under_the_trapezoidal_rule() {
    let (l, c, v0): (f64, f64, f64) = (1e-3, 1e-6, 3.0);
    let w = 1.0 / (l * c).sqrt();
    let period = std::f64::consts::TAU / w;
    let text = format!(
        "lc\nL1 a 0 {l}\nC1 a 0 {c} IC={v0}\n.tran {} {} UIC\n",
        period / 200.0,
        20.0 * period
    );
    let res = run_with(
        &text,
        &Options {
            reltol: 1e-5,
            ..Options::default()
        },
    )
    .unwrap();
    let e = max_error(&res.table, "V(a)", 0.0, |t| v0 * (w * t).cos());
    assert!(e < 0.02 * v0, "error {e} over 20 periods");
    // Energy: no visible numerical damping, the last peak is within 1 % of the first.
    let y = col(&res.table, "V(a)");
    let last = y[y.len() * 9 / 10..]
        .iter()
        .fold(0.0f64, |m, v| m.max(v.abs()));
    assert!((last - v0).abs() < 0.01 * v0, "late peak {last}");
}

#[test]
fn sine_through_an_rc_filter_agrees_with_the_ac_analysis() {
    let (r, c, f) = (1e3, 159.154_943_091_895_35e-9, 1000.0); // corner at 1 kHz
    let text = format!("sine\nV1 in 0 SIN(0 1 {f})\nR1 in out {r}\nC1 out 0 {c}\n.tran 2u 12m\n");
    let res = run_with(
        &text,
        &Options {
            reltol: 1e-5,
            ..Options::default()
        },
    )
    .unwrap();
    // After the start-up transient the output is a steady sine of amplitude 1/sqrt(2), lagging 45 degrees.
    let w = std::f64::consts::TAU * f;
    let amp = 1.0 / (1.0 + (w * r * c).powi(2)).sqrt();
    let phase = -(w * r * c).atan();
    let steady = |t: f64| amp * (w * t + phase).sin();
    let e = max_error(&res.table, "V(out)", 8e-3, steady);
    assert!(e < 5e-3, "steady-state error {e}");
}

#[test]
fn source_breakpoints_are_hit_exactly() {
    let res = run("bp\nV1 in 0 PULSE(0 5 100u 10u 20u 200u 500u)\nR1 in out 1k\nC1 out 0 10n\n.tran 5u 1.2m\n");
    for edge in [100e-6, 110e-6, 310e-6, 330e-6, 600e-6, 610e-6] {
        assert!(
            res.table.axis.iter().any(|&t| (t - edge).abs() < 1e-12),
            "no time point at {edge}; nearest {:?}",
            res.table
                .axis
                .iter()
                .map(|&t| (t - edge).abs())
                .fold(f64::INFINITY, f64::min)
        );
    }
    // The axis is strictly increasing and ends at tstop.
    assert!(res.table.axis.windows(2).all(|w| w[1] > w[0]));
    assert_close(*res.table.axis.last().unwrap(), 1.2e-3, 1e-12, 0.0, "tstop");
}

#[test]
fn varactor_charge_balance_checks_the_junction_capacitance_model() {
    // Reverse-bias a diode through 10 k from 0 to -4 V. The charge delivered by the resistor
    // equals the junction depletion charge q(-4) - q(0) of the SPICE2 model.
    let (cjo, vj, m) = (50e-12, 0.8, 0.5);
    let text = format!("vc\nV1 in 0 PWL(0 0 1u -4 1 -4)\nR1 in a 10k\nD1 a 0 DM\n.model DM D(IS=1e-14 CJO={cjo} VJ={vj} M={m})\n.tran 50n 20u\n");
    let res = run_with(
        &text,
        &Options {
            reltol: 1e-5,
            ..Options::default()
        },
    )
    .unwrap();
    let t = &res.table.axis;
    let i = col(&res.table, "I(R1)");
    let mut q = 0.0;
    for k in 1..t.len() {
        q += 0.5 * (i[k] + i[k - 1]) * (t[k] - t[k - 1]);
    }
    // q(v) = cjo vj (1 - (1 - v/vj)^(1-m)) / (1 - m); charge on the anode plate going from 0 to -4 V.
    let qd = |v: f64| cjo * vj * (1.0 - (1.0 - v / vj).powf(1.0 - m)) / (1.0 - m);
    let want = -(qd(-4.0) - qd(0.0)); // current into the anode node is (vin - va)/R; sign follows
                                      // I(R1) flows in -> a, negative while the node is pulled below ground.
    assert_close(q, -want, 5e-3, 1e-13, "delivered charge");
}

#[test]
fn trapezoidal_and_backward_euler_agree_on_a_rectifier() {
    let text = "rect\nV1 in 0 SIN(0 5 1k)\nD1 in a DM\nC1 a 0 10u\nR1 a 0 1k\n.model DM D(IS=1e-14 RS=1)\n.tran 10u 6m\n";
    let tr = run_with(
        text,
        &Options {
            reltol: 1e-5,
            ..Options::default()
        },
    )
    .unwrap();
    let be = run_with(
        text,
        &Options {
            reltol: 1e-6,
            method: Integration::BackwardEuler,
            ..Options::default()
        },
    )
    .unwrap();
    for &t in &[1.0e-3, 2.5e-3, 3.3e-3, 4.1e-3, 5.9e-3] {
        let (a, b) = (at_time(&tr.table, "V(a)", t), at_time(&be.table, "V(a)", t));
        assert!((a - b).abs() < 5e-3, "t={t}: {a} vs {b}");
    }
    // The held output sits one diode drop (about 0.7 V plus the resistive drop) below the peak.
    let peak = col(&tr.table, "V(a)").into_iter().fold(0.0, f64::max);
    assert!(peak > 3.6 && peak < 4.5, "peak {peak}");
}

#[test]
fn a_switch_closes_when_its_control_crosses_the_threshold() {
    // The control ramps 0 -> 5 V over 5 ms; the switch (VT=2.5) closes at 2.5 ms and charges C.
    let text = "sw\nVC c 0 PWL(0 0 5m 5 10m 5)\nV1 a 0 DC 10\nS1 a b c 0 SM\nR1 b out 10\nC1 out 0 1u\nRL out 0 1meg\n.model SM SW(VT=2.5 VH=0 RON=1 ROFF=1e9)\n.tran 20u 5m\n";
    let res = run_with(
        text,
        &Options {
            reltol: 1e-4,
            ..Options::default()
        },
    )
    .unwrap();
    let y = col(&res.table, "V(out)");
    let first_up = res
        .table
        .axis
        .iter()
        .zip(&y)
        .find(|(_, &v)| v > 0.05)
        .map(|(&t, _)| t)
        .unwrap();
    // After closing, V(out) reaches 0.05 V within about 5 time constants of 11 us: a fraction of a ms.
    assert!(
        (2.5e-3 - 1e-6..2.5e-3 + 60e-6).contains(&first_up),
        "closing seen at {first_up}"
    );
    let end = *y.last().unwrap();
    assert!(end > 9.9 * 1e6 / (1e6 + 11.0) - 0.02, "charged to {end}");
}

#[test]
fn progress_is_monotonic_and_cancellation_stops_the_run() {
    let n = parse("p\nV1 in 0 SIN(0 1 1k)\nR1 in out 1k\nC1 out 0 1u\n.tran 5u 20m\n");
    let a = n.analyses[0].clone();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let ctl = Control::new().with_progress(Arc::new(move |f| {
        if let Ok(mut g) = sink.lock() {
            g.push(f);
        }
    }));
    simulate(&n, &a, &Options::default(), &ctl).unwrap();
    let v = seen.lock().unwrap().clone();
    assert!(v.len() >= 3, "{v:?}");
    assert!(v.windows(2).all(|w| w[1] >= w[0]) && (v.last().copied().unwrap() - 1.0).abs() < 1e-12);

    let flag = Arc::new(AtomicBool::new(true));
    let ctl = Control::new().with_cancel(flag.clone());
    assert_eq!(
        simulate(&n, &a, &Options::default(), &ctl).unwrap_err(),
        SimError::Cancelled
    );
    flag.store(false, Ordering::Relaxed);
    assert!(simulate(&n, &a, &Options::default(), &ctl).is_ok());
}

#[test]
fn too_many_points_is_an_error_not_an_out_of_memory() {
    let n = parse("big\nV1 in 0 DC 1\nR1 in 0 1k\n.tran 1p 1\n");
    let a = n.analyses[0].clone();
    let e = simulate(&n, &a, &Options::default(), &Control::new()).unwrap_err();
    assert!(matches!(e, SimError::TooLarge(_)), "{e}");
}

#[test]
fn a_dc_circuit_stays_put_in_transient() {
    let res = run("dc\nV1 in 0 DC 5\nR1 in out 1k\nR2 out 0 1k\nC1 out 0 1u\n.tran 1u 100u\n");
    for v in col(&res.table, "V(out)") {
        assert_close(v, 2.5, 1e-9, 1e-9, "steady value");
    }
}
