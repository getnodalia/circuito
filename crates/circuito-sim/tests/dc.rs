// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! DC operating-point and sweep validation against closed-form solutions.

mod common;
use common::*;

use circuito_sim::SimError;

const VT27: f64 = 8.617_333_262e-5 * 300.15;

#[test]
fn resistor_divider_is_exact() {
    let r = run_tight("divider\nV1 in 0 DC 10\nR1 in out 1k\nR2 out 0 3k\n.op\n");
    assert_close(op(&r.table, "V(out)"), 7.5, 1e-12, 0.0, "divider");
    // SPICE convention: a source that delivers power has a negative current.
    assert_close(op(&r.table, "I(V1)"), -10.0 / 4000.0, 1e-12, 0.0, "I(V1)");
    assert_close(op(&r.table, "I(R2)"), 7.5 / 3000.0, 1e-12, 0.0, "I(R2)");
    assert_eq!(r.info.dc_method, "newton");
}

#[test]
fn current_source_flows_from_plus_through_the_source_to_minus() {
    // I1 pushes 2 mA into node 1 (from node 0, through the source).
    let r = run_tight("isrc\nI1 0 1 DC 2m\nR1 1 0 1k\n.op\n");
    assert_close(op(&r.table, "V(1)"), 2.0, 1e-12, 0.0, "V(1)");
    let r = run_tight("isrc\nI1 1 0 DC 2m\nR1 1 0 1k\n.op\n");
    assert_close(op(&r.table, "V(1)"), -2.0, 1e-12, 0.0, "reversed");
}

#[test]
fn wheatstone_bridge_matches_the_analytic_solution() {
    // 5 V across two dividers; bridge resistor R5 between the midpoints.
    let (r1, r2, r3, r4, r5) = (1000.0, 2200.0, 3300.0, 4700.0, 1500.0);
    let r = run_tight(&format!(
        "bridge\nV1 a 0 DC 5\nR1 a b {r1}\nR2 b 0 {r2}\nR3 a c {r3}\nR4 c 0 {r4}\nR5 b c {r5}\n.op\n"
    ));
    // Solve the two node equations by Cramer's rule.
    let (g1, g2, g3, g4, g5) = (1.0 / r1, 1.0 / r2, 1.0 / r3, 1.0 / r4, 1.0 / r5);
    let (a11, a12, b1) = (g1 + g2 + g5, -g5, g1 * 5.0);
    let (a21, a22, b2) = (-g5, g3 + g4 + g5, g3 * 5.0);
    let det = a11 * a22 - a12 * a21;
    let vb = (b1 * a22 - a12 * b2) / det;
    let vc = (a11 * b2 - a21 * b1) / det;
    assert_close(op(&r.table, "V(b)"), vb, 1e-10, 0.0, "V(b)");
    assert_close(op(&r.table, "V(c)"), vc, 1e-10, 0.0, "V(c)");
}

#[test]
fn controlled_sources_follow_their_definitions() {
    // VCVS: gain 4 on a 2 V control.
    let r = run_tight("e\nV1 c 0 DC 2\nR0 c 0 1k\nE1 o 0 c 0 4\nRL o 0 1k\n.op\n");
    assert_close(op(&r.table, "V(o)"), 8.0, 1e-12, 0.0, "VCVS");
    // VCCS: 2 mS * 3 V = 6 mA into 1 k from node 0 side (current flows p -> n through the source).
    let r = run_tight("g\nV1 c 0 DC 3\nR0 c 0 1k\nG1 0 o c 0 2m\nRL o 0 1k\n.op\n");
    assert_close(op(&r.table, "V(o)"), 6.0, 1e-12, 0.0, "VCCS");
    // CCCS: current gain 10 on the current through V1 (SPICE: positive current enters V1's + node).
    let r = run_tight("f\nV1 c 0 DC 1\nR0 c 0 1k\nF1 0 o V1 10\nRL o 0 100\n.op\n");
    // I(V1) = -1 mA; F pushes 10 * (-1 mA) from node 0 through the source to o: V(o) = -1 V.
    assert_close(op(&r.table, "V(o)"), -1.0, 1e-12, 0.0, "CCCS");
    // CCVS: transresistance 500 ohm.
    let r = run_tight("h\nV1 c 0 DC 1\nR0 c 0 1k\nH1 o 0 V1 500\nRL o 0 100\n.op\n");
    assert_close(op(&r.table, "V(o)"), -0.5, 1e-12, 0.0, "CCVS");
}

/// Solve `f(v) = 0` on `[lo, hi]` by bisection (f must change sign).
fn bisect(f: impl Fn(f64) -> f64, mut lo: f64, mut hi: f64) -> f64 {
    let flo = f(lo);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if (f(mid) > 0.0) == (flo > 0.0) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

#[test]
fn diode_with_resistor_matches_the_shockley_equation() {
    let r = run_tight("d\nV1 in 0 DC 5\nR1 in a 1k\nD1 a 0 DM\n.model DM D(IS=1e-14)\n.op\n");
    let vd = bisect(
        |v| 1e-14 * ((v / VT27).exp() - 1.0) - (5.0 - v) / 1000.0,
        0.0,
        1.0,
    );
    assert_close(op(&r.table, "V(a)"), vd, 1e-9, 1e-9, "diode drop");
    assert_close(
        op(&r.table, "I(D1)"),
        (5.0 - vd) / 1000.0,
        1e-8,
        1e-12,
        "diode current",
    );
}

#[test]
fn diode_series_resistance_uses_an_internal_node() {
    let r =
        run_tight("d\nV1 in 0 DC 5\nR1 in a 100\nD1 a 0 DM\n.model DM D(IS=1e-14 RS=10)\n.op\n");
    // i = Is(exp(vj/Vt)-1); v(a) = vj + i*RS; i = (5 - v(a))/100.
    let i = bisect(
        |i: f64| {
            let vj = VT27 * (i / 1e-14 + 1.0).ln();
            i - (5.0 - vj - i * 10.0) / 100.0
        },
        0.0,
        0.05,
    );
    assert_close(op(&r.table, "I(D1)"), i, 1e-8, 1e-12, "current with RS");
    let vj = VT27 * (i / 1e-14 + 1.0).ln();
    assert_close(
        op(&r.table, "V(a)"),
        vj + i * 10.0,
        1e-8,
        1e-9,
        "anode voltage",
    );
}

#[test]
fn a_large_forward_step_converges_through_junction_limiting() {
    // 100 V across three series diodes: Newton alone would overshoot by hundreds of volts.
    let r = run_tight("d3\nV1 in 0 DC 100\nR1 in a 1k\nD1 a b DM\nD2 b c DM\nD3 c 0 DM\n.model DM D(IS=1e-14)\n.op\n");
    let i = bisect(
        |i: f64| i - (100.0 - 3.0 * VT27 * (i / 1e-14 + 1.0).ln()) / 1000.0,
        0.0,
        0.1,
    );
    assert_close(op(&r.table, "I(R1)"), i, 1e-8, 1e-12, "series current");
    assert_close(
        op(&r.table, "V(a)"),
        3.0 * VT27 * (i / 1e-14 + 1.0).ln(),
        1e-8,
        1e-9,
        "three drops",
    );
}

#[test]
fn zener_breakdown_clamps_near_bv() {
    // Reverse-biased through 1 k from 12 V; IBV = 5 mA at BV = 5.1 V.
    let r = run_tight(
        "z\nV1 in 0 DC 12\nR1 in k 1k\nD1 0 k DZ\n.model DZ D(IS=1e-14 BV=5.1 IBV=5m)\n.op\n",
    );
    // i = IBV*exp(-(v + BV)/Vt) with v = -(Vk): solve with the cathode at node k.
    let f = |vk: f64| {
        let ireverse = 5e-3 * ((vk - 5.1) / VT27).exp(); // current through the diode, k to 0
        ireverse - (12.0 - vk) / 1000.0
    };
    let vk = bisect(f, 4.0, 6.0);
    assert_close(op(&r.table, "V(k)"), vk, 1e-7, 1e-7, "zener voltage");
    assert!((op(&r.table, "V(k)") - 5.1).abs() < 0.2);
}

fn ebers_moll_ic(vbe: f64, vce: f64, is: f64, bf: f64, br: f64, vaf: f64) -> f64 {
    let (vbc, vcb) = (vbe - vce, vce - vbe);
    let _ = vcb;
    let i_f = is * ((vbe / VT27).exp() - 1.0);
    let i_r = is * ((vbc / VT27).exp() - 1.0);
    let k = 1.0 - vbc / vaf;
    let _ = bf;
    (i_f - i_r) * k - i_r / br
}

#[test]
fn bjt_common_emitter_matches_ebers_moll_with_early_effect() {
    let (is, bf, br, vaf) = (1e-14, 150.0, 2.0, 80.0);
    let r = run_tight(&format!(
        "ce\nVCC vcc 0 DC 10\nVB b 0 DC 0.68\nRC vcc c 2k\nQ1 c b 0 QM\n.model QM NPN(IS={is} BF={bf} BR={br} VAF={vaf})\n.op\n"
    ));
    let vce = bisect(
        |vce| ebers_moll_ic(0.68, vce, is, bf, br, vaf) - (10.0 - vce) / 2000.0,
        0.0,
        10.0,
    );
    assert_close(op(&r.table, "V(c)"), vce, 1e-8, 1e-8, "V(c)");
    // Base current: IB = If/BF + Ir/BR.
    let i_f = is * ((0.68f64 / VT27).exp() - 1.0);
    let i_r = is * (((0.68 - vce) / VT27).exp() - 1.0);
    assert_close(
        -op(&r.table, "I(VB)"),
        i_f / bf + i_r / br,
        1e-8,
        1e-15,
        "base current",
    );
}

#[test]
fn bjt_in_saturation_and_pnp_mirror() {
    // Hard saturation: tiny RC means vce collapses; the result must match the full Ebers-Moll
    // equations, including the reverse junction.
    let (is, bf, br) = (1e-14, 100.0, 1.0);
    let r = run_tight(&format!(
        "sat\nVCC vcc 0 DC 5\nVB b 0 DC 0.75\nRC vcc c 10\nQ1 c b 0 QM\n.model QM NPN(IS={is} BF={bf} BR={br})\n.op\n"
    ));
    let vce = bisect(
        |vce| ebers_moll_ic(0.75, vce, is, bf, br, f64::INFINITY) - (5.0 - vce) / 10.0,
        0.0,
        5.0,
    );
    assert_close(op(&r.table, "V(c)"), vce, 1e-7, 1e-7, "saturated V(c)");
    // The PNP version of the same circuit with every voltage negated gives the negated answer.
    let p = run_tight(&format!(
        "pnp\nVCC vcc 0 DC -5\nVB b 0 DC -0.75\nRC vcc c 10\nQ1 c b 0 QM\n.model QM PNP(IS={is} BF={bf} BR={br})\n.op\n"
    ));
    assert_close(op(&p.table, "V(c)"), -vce, 1e-7, 1e-7, "PNP mirror");
}

#[test]
fn mosfet_saturation_and_triode_match_the_square_law() {
    // NMOS: KP=2e-5, W/L = 100, VTO=1, LAMBDA=0.02 -> beta = 2 mA/V^2.
    let model = ".model NM NMOS(VTO=1 KP=2e-5 LAMBDA=0.02 W=1m L=10u)";
    let beta = 2e-5 * 100.0;
    // Saturation: Vgs = 3, Rd = 1k from 10 V. id = beta/2 (vgs-vto)^2 (1+lambda vds).
    let r = run_tight(&format!(
        "nm\nVDD vdd 0 DC 10\nVG g 0 DC 3\nRD vdd d 1k\nM1 d g 0 0 NM\n{model}\n.op\n"
    ));
    // 10 - 1000*id = vds; id = 2e-3*(1+0.02 vds)... solve the linear equation in vds.
    let id0 = 0.5 * beta * 4.0;
    let vds = (10.0 - 1000.0 * id0) / (1.0 + 1000.0 * id0 * 0.02);
    assert!(vds > 2.0, "test setup must be in saturation, vds = {vds}");
    assert_close(op(&r.table, "V(d)"), vds, 1e-9, 1e-9, "saturation vds");
    // Triode: big gate drive and a larger load.
    let r = run_tight(&format!(
        "nm\nVDD vdd 0 DC 10\nVG g 0 DC 8\nRD vdd d 5k\nM1 d g 0 0 NM\n{model}\n.op\n"
    ));
    // id = beta*((vgs-vto) vds - vds^2/2)(1+lambda vds); fixed point with Rd.
    let vds = bisect(
        |v| beta * (7.0 * v - 0.5 * v * v) * (1.0 + 0.02 * v) - (10.0 - v) / 5000.0,
        0.0,
        7.0,
    );
    assert!(vds < 7.0);
    assert_close(op(&r.table, "V(d)"), vds, 1e-9, 1e-9, "triode vds");
}

#[test]
fn pmos_mirrors_nmos_and_body_effect_raises_the_threshold() {
    let pm = ".model PM PMOS(VTO=-1 KP=2e-5 LAMBDA=0 W=1m L=10u)";
    let r = run_tight(&format!(
        "pm\nVDD vdd 0 DC 10\nVG g 0 DC 7\nRD d 0 1k\nM1 d g vdd vdd PM\n{pm}\n.op\n"
    ));
    // |vgs| = 3, |vto| = 1: id = 1 mA/V^2 * 4 = 4 mA... beta/2*(2)^2 = 4e-3, vd = 4 V (saturated as vsd = 6 > 2).
    assert_close(op(&r.table, "V(d)"), 4.0, 1e-9, 1e-9, "PMOS drain");
    // Body effect: source at 2 V, bulk grounded: vth = vto + gamma(sqrt(phi+vsb) - sqrt(phi)).
    let (gamma, phi) = (0.5, 0.65);
    let r = run_tight(&format!(
        "body\nVDD vdd 0 DC 10\nVG g 0 DC 5\nM1 vdd g s 0 NB\nRS s 0 1k\n.model NB NMOS(VTO=1 KP=2e-5 GAMMA={gamma} PHI={phi} W=1m L=10u)\n.op\n"
    ));
    let vs = op(&r.table, "V(s)");
    let vth = 1.0 + gamma * ((phi + vs).sqrt() - phi.sqrt());
    let id = 0.5 * 2e-3 * (5.0 - vs - vth).powi(2);
    assert_close(
        vs / 1000.0,
        id,
        1e-8,
        1e-12,
        "source follower current balance",
    );
}

#[test]
fn opamp_gain_follows_the_finite_gain_and_output_resistance_formulas() {
    let (a, rout) = (1e5, 75.0);
    // Inverting amplifier: the output stage drives Rf, so Rout takes part. Unknowns vn, vo:
    //   vn (1/Rin + 1/Rf) - vo/Rf = Vin/Rin          (node n, no current into the input)
    //   vo (1 + Rout/Rf) + (A - Rout/Rf) vn = 0       (vo = -A vn - Rout (vo - vn)/Rf)
    let (rin, rf, vin) = (1000.0, 10_000.0, 0.1);
    let r = run_tight(&format!(
        "inv\nVIN in 0 DC {vin}\nRIN in n {rin}\nRF n out {rf}\nU1 0 n out OA\n.model OA OPAMP(GAIN={a} ROUT={rout})\n.op\n"
    ));
    let (a11, a12, b1) = (1.0 / rin + 1.0 / rf, -1.0 / rf, vin / rin);
    let (a21, a22) = (a - rout / rf, 1.0 + rout / rf);
    let det = a11 * a22 - a12 * a21;
    let vo = (a11 * 0.0 - a21 * b1) / det;
    assert_close(
        op(&r.table, "V(out)"),
        vo,
        1e-9,
        1e-12,
        "inverting amplifier",
    );
    // With Rout = 0 the closed form of the finite-gain amplifier would be -(Rf/Rin) Vin / (1 + (1 + Rf/Rin)/A);
    // 75 ohm into 11 k changes it by under 1e-5 relative, so the two agree to that level.
    let textbook = -(rf / rin) * vin / (1.0 + (1.0 + rf / rin) / a);
    assert_close(
        op(&r.table, "V(out)"),
        textbook,
        2e-5,
        0.0,
        "textbook formula",
    );
    // Non-inverting: vn = k vo with k = Rg/(Rg+Rf); vo (1 + Rout/Rg k + ... ) solved directly.
    let (rg, rf) = (1000.0, 9000.0);
    let r = run_tight(&format!(
        "noninv\nVIN in 0 DC 0.1\nRG n 0 {rg}\nRF n out {rf}\nU1 in n out OA\n.model OA OPAMP(GAIN={a} ROUT={rout})\n.op\n"
    ));
    let k = rg / (rg + rf);
    // vo = A (vin - vn) - Rout * vn / Rg with vn = k vo.
    let vo = a * 0.1 / (1.0 + a * k + rout * k / rg);
    assert_close(
        op(&r.table, "V(out)"),
        vo,
        1e-9,
        1e-12,
        "non-inverting amplifier",
    );
}

#[test]
fn opamp_saturates_at_its_rails_and_ideal_mode_is_near_exact() {
    let r = run_tight("cmp\nVIN in 0 DC 1\nRL out 0 100k\nU1 in 0 out OA\n.model OA OPAMP(GAIN=1e5 ROUT=75 VMAX=12 VMIN=-10)\n.op\n");
    let v = op(&r.table, "V(out)");
    assert!(v > 11.9 && v <= 12.0 + 1e-6, "positive rail, got {v}");
    let r = run_tight("cmp\nVIN in 0 DC -1\nRL out 0 100k\nU1 in 0 out OA\n.model OA OPAMP(GAIN=1e5 ROUT=75 VMAX=12 VMIN=-10)\n.op\n");
    let v = op(&r.table, "V(out)");
    assert!((-10.0 - 1e-6..-9.9).contains(&v), "negative rail, got {v}");
    // An ideal op-amp gives the textbook gain to better than 1e-8.
    let r = run_tight("ideal\nVIN in 0 DC 0.25\nRIN in n 2k\nRF n out 6k\nU1 0 n out OA\n.model OA OPAMP(IDEAL=1)\n.op\n");
    assert_close(
        op(&r.table, "V(out)"),
        -0.75,
        1e-8,
        0.0,
        "ideal inverting amplifier",
    );
}

#[test]
fn a_switch_closes_with_hysteresis_and_conducts_through_ron() {
    let m = ".model SM SW(VT=2.5 VH=0.5 RON=10 ROFF=1e9)";
    let on = run_tight(&format!(
        "sw\nVC c 0 DC 5\nV1 a 0 DC 10\nS1 a b c 0 SM\nRL b 0 90\n{m}\n.op\n"
    ));
    assert_close(op(&on.table, "V(b)"), 9.0, 1e-9, 0.0, "closed switch");
    let off = run_tight(&format!(
        "sw\nVC c 0 DC 1\nV1 a 0 DC 10\nS1 a b c 0 SM\nRL b 0 90\n{m}\n.op\n"
    ));
    assert!(op(&off.table, "V(b)").abs() < 1e-6, "open switch");
}

#[test]
fn dc_sweep_traces_the_diode_curve_and_is_monotonic() {
    let r = run_tight(
        "sweep\nV1 in 0 DC 0\nR1 in a 100\nD1 a 0 DM\n.model DM D(IS=1e-14)\n.dc V1 0 5 0.25\n",
    );
    assert_eq!(r.table.axis.len(), 21);
    assert_close(r.table.axis[4], 1.0, 1e-12, 0.0, "axis");
    let i = col(&r.table, "I(D1)");
    for k in 0..i.len() {
        let vin = r.table.axis[k];
        let vd = bisect(
            |v| 1e-14 * ((v / VT27).exp() - 1.0) - (vin - v) / 100.0,
            -1.0,
            1.0f64.max(vin),
        );
        let want = (vin - vd) / 100.0;
        assert_close(i[k], want, 1e-7, 1e-10, &format!("I at {vin}"));
        if k > 0 {
            assert!(i[k] >= i[k - 1] - 1e-15);
        }
    }
    // A descending sweep is accepted.
    let r = run("rev\nV1 in 0 DC 0\nR1 in 0 1k\n.dc V1 5 1 1\n");
    assert_eq!(r.table.axis, vec![5.0, 4.0, 3.0, 2.0, 1.0]);
}

#[test]
fn temperature_moves_the_diode_voltage_by_about_minus_two_millivolts_per_kelvin() {
    let at = |t: f64| {
        let text =
            format!("t\nI1 0 a DC 1m\nD1 a 0 DM\n.model DM D(IS=1e-14)\n.options temp={t}\n.op\n");
        let n = parse(&text);
        let opts = circuito_sim::Options::from_netlist(&n).unwrap();
        let a = n.analyses[0].clone();
        let r = circuito_sim::simulate(&n, &a, &opts, &circuito_sim::Control::new()).unwrap();
        op(&r.table, "V(a)")
    };
    let (v27, v127) = (at(27.0), at(127.0));
    let slope = (v127 - v27) / 100.0;
    assert!(slope < -1.5e-3 && slope > -2.6e-3, "slope {slope}");
    // At 27 C the drop is Vt ln(I/Is + 1).
    assert_close(
        v27,
        VT27 * (1e-3f64 / 1e-14 + 1.0).ln(),
        1e-6,
        1e-6,
        "27 C drop",
    );
}

#[test]
fn newton_failure_falls_back_to_gmin_or_source_stepping() {
    // With only 5 iterations allowed, plain Newton cannot finish; stepping can.
    let opts = circuito_sim::Options { itl1: 5, ..tight() };
    let r = run_with("hard\nV1 in 0 DC 20\nR1 in a 100\nD1 a b DM\nD2 b c DM\nD3 c 0 DM\n.model DM D(IS=1e-14)\n.op\n", &opts);
    let r = r.expect("fallback strategies must find the solution");
    assert_ne!(r.info.dc_method, "newton", "{:?}", r.info);
    let i = bisect(
        |i: f64| i - (20.0 - 3.0 * VT27 * (i / 1e-14 + 1.0).ln()) / 100.0,
        0.0,
        0.2,
    );
    assert_close(
        op(&r.table, "I(R1)"),
        i,
        1e-7,
        1e-10,
        "current after stepping",
    );
}

#[test]
fn topology_errors_name_the_elements_involved() {
    let e = run_with("loop\nV1 a 0 DC 5\nV2 a 0 DC 3\nR1 a 0 1k\n.op\n", &tight()).unwrap_err();
    match &e {
        SimError::VoltageLoop { elements, .. } => {
            assert!(
                elements.contains(&"V1".to_string()) && elements.contains(&"V2".to_string()),
                "{elements:?}"
            );
        }
        other => panic!("wrong error {other:?}"),
    }
    let e = run_with("loopL\nV1 a 0 DC 5\nL1 a 0 1m\n.op\n", &tight()).unwrap_err();
    assert!(
        matches!(&e, SimError::VoltageLoop { inductors, .. } if inductors.contains("inductor")),
        "{e}"
    );
    let e = run_with(
        "float\nV1 a 0 DC 5\nR1 a 0 1k\nC1 a b 1u\nC2 b 0 1u\n.op\n",
        &tight(),
    )
    .unwrap_err();
    assert!(
        matches!(&e, SimError::NoDcPath { node } if node.eq_ignore_ascii_case("b")),
        "{e}"
    );
    let e = run_with(
        "cut\nV1 a 0 DC 5\nR1 a 0 1k\nI1 x 0 DC 1m\nI2 y x DC 1m\nR3 y z 1k\nR4 z 0 1k\n.op\n",
        &tight(),
    )
    .unwrap_err();
    assert!(matches!(&e, SimError::NoDcPath { .. }), "{e}");
    let e = run_with("noground\nV1 a b DC 5\nR1 a b 1k\n.op\n", &tight()).unwrap_err();
    assert!(matches!(e, SimError::NoGround), "{e}");
}

#[test]
fn unknown_sweep_source_and_bad_models_are_reported() {
    let n = parse("x\nV1 a 0 DC 1\nR1 a 0 1k\n");
    let a = circuito_core::netlist::Analysis::Dc {
        source: "V9".into(),
        start: 0.0,
        stop: 1.0,
        step: 0.5,
    };
    // `simulate` validates the analysis against the netlist's elements first.
    let e = circuito_sim::simulate(&n, &a, &tight(), &circuito_sim::Control::new());
    assert!(e.is_err());
    let e = run_with(
        "bad\nV1 a 0 DC 1\nD1 a 0 DM\n.model DM D(IS=-1)\n.op\n",
        &tight(),
    )
    .unwrap_err();
    assert!(
        matches!(&e, SimError::BadModel { model, message } if model == "DM" && message.contains("IS")),
        "{e}"
    );
}

#[test]
fn unused_model_parameters_are_reported_as_warnings() {
    let r = run("w\nV1 a 0 DC 1\nR1 a b 1k\nD1 b 0 DM\n.model DM D(IS=1e-14 XYZ=3)\n.op\n");
    assert!(
        r.info.warnings.iter().any(|w| w.contains("XYZ")),
        "{:?}",
        r.info.warnings
    );
}
