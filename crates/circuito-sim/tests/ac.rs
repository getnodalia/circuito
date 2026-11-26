// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Small-signal AC validation against transfer functions written out by hand.

mod common;
use common::*;

use circuito_core::netlist::Sweep;
use circuito_sim::{ac_frequencies, SimError};
use nodalia_numerics::Complex64;

const TAU: f64 = std::f64::consts::TAU;

fn j() -> Complex64 {
    Complex64::new(0.0, 1.0)
}

#[test]
fn rc_low_pass_matches_h_of_jw() {
    let (r, c): (f64, f64) = (1_591.549_430_918_953_4, 100e-9); // corner at 1000 Hz
    let res = run_tight(&format!(
        "rc\nV1 in 0 DC 0 AC 1\nR1 in out {r}\nC1 out 0 {c}\n.ac dec 10 1 100k\n"
    ));
    let out = res.table.signal("V(out)").unwrap();
    for (i, &f) in res.table.axis.iter().enumerate() {
        let h = Complex64::new(1.0, 0.0) / (Complex64::new(1.0, 0.0) + j() * (TAU * f * r * c));
        let got = out.at(i).unwrap();
        assert!(
            (got - h).norm() <= 1e-9 * h.norm() + 1e-14,
            "f={f}: {got} vs {h}"
        );
    }
    // The classical landmarks: -3.0103 dB and -45 degrees at the corner.
    let k = res
        .table
        .axis
        .iter()
        .position(|&f| (f - 1000.0).abs() < 1e-6)
        .unwrap();
    assert_close(
        out.db(k).unwrap(),
        -3.010_299_956_639_812,
        1e-9,
        1e-9,
        "corner gain",
    );
    assert_close(out.phase_deg(k).unwrap(), -45.0, 1e-9, 1e-9, "corner phase");
    // 10 points per decade from 1 Hz to 100 kHz is 51 points.
    assert_eq!(res.table.axis.len(), 51);
}

#[test]
fn series_rlc_resonance_and_quality_factor() {
    let (r, l, c): (f64, f64, f64) = (10.0, 1e-3, 1e-6);
    let f0 = 1.0 / (TAU * (l * c).sqrt());
    let res = run_tight(&format!(
        "rlc\nV1 in 0 AC 1\nL1 in a {l}\nC1 a b {c}\nR1 b 0 {r}\n.ac lin 401 {} {}\n",
        f0 * 0.5,
        f0 * 1.5
    ));
    let ir = res.table.signal("I(V1)").unwrap();
    for (i, &f) in res.table.axis.iter().enumerate() {
        let w = TAU * f;
        // I(V1) is the current into the + terminal: -V / Z.
        let z = Complex64::new(r, 0.0) + j() * (w * l) + Complex64::new(1.0, 0.0) / (j() * (w * c));
        let want = -Complex64::new(1.0, 0.0) / z;
        assert!(
            (ir.at(i).unwrap() - want).norm() <= 1e-9 * want.norm() + 1e-15,
            "f={f}"
        );
    }
    // The peak is at f0 with magnitude 1/R; the -3 dB bandwidth is f0/Q.
    let mags: Vec<f64> = (0..ir.len()).map(|i| ir.magnitude(i).unwrap()).collect();
    let (imax, &mmax) = mags
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .unwrap();
    assert_close(res.table.axis[imax], f0, 2e-3, 0.0, "resonant frequency");
    assert_close(mmax, 1.0 / r, 1e-4, 0.0, "peak current");
}

#[test]
fn transformer_with_mutual_inductance_matches_the_two_port_equations() {
    let (l1, l2, k, rl): (f64, f64, f64, f64) = (10e-3, 40e-3, 0.95, 200.0);
    let m = k * (l1 * l2).sqrt();
    let res = run_tight(&format!(
        "xfmr\nV1 in 0 AC 1\nRS in p 1\nL1 p 0 {l1}\nL2 s 0 {l2}\nK1 L1 L2 {k}\nRL s 0 {rl}\n.ac dec 5 100 100k\n"
    ));
    let out = res.table.signal("V(s)").unwrap();
    for (i, &f) in res.table.axis.iter().enumerate() {
        let w = TAU * f;
        // V1 = jw L1 I1 + jw M I2 ; V(s) = jw M I1 + jw L2 I2 = -RL I2 (current I2 flows into L2's + node).
        // Eliminate I2: I2 = -jwM I1/(jwL2 + RL); V(s) = -RL I2.
        let i1 = Complex64::new(1.0, 0.0)
            / (Complex64::new(1.0, 0.0)
                + j() * (w * l1)
                + (j() * w * m) * (-(j() * w * m) / (j() * (w * l2) + rl)));
        let i2 = -(j() * w * m) * i1 / (j() * (w * l2) + rl);
        let want = -i2 * rl;
        let got = out.at(i).unwrap();
        assert!(
            (got - want).norm() <= 1e-9 * want.norm() + 1e-14,
            "f={f}: {got} vs {want}"
        );
    }
}

#[test]
fn diode_small_signal_resistance_forms_a_divider_with_the_source_resistor() {
    // Bias 1 mA through the diode; AC rides on a current source so r_d = n Vt / I.
    let res =
        run_tight("dsig\nI1 0 a DC 1m AC 1\nD1 a 0 DM\n.model DM D(IS=1e-14)\n.ac lin 1 1k 1k\n");
    let vt = 8.617_333_262e-5 * 300.15;
    // Id = I - nothing else; r_d = Vt / (Id + Is).
    let id = 1e-3;
    let rd = vt / (id + 1e-14);
    let got = res.table.signal("V(a)").unwrap().magnitude(0).unwrap();
    assert_close(got, rd, 1e-6, 0.0, "small-signal resistance");
}

#[test]
fn bjt_common_emitter_gain_matches_the_derivatives_of_ebers_moll() {
    let (is, bf, br, vaf) = (1e-14, 100.0, 1.0, 60.0);
    let res = run_tight(&format!(
        "ce\nVCC vcc 0 DC 10\nVB b 0 DC 0.66 AC 1\nRC vcc c 3k\nQ1 c b 0 QM\n.model QM NPN(IS={is} BF={bf} BR={br} VAF={vaf})\n.ac lin 1 1k 1k\n"
    ));
    let _ = bf;
    let vt = 8.617_333_262e-5 * 300.15;
    let ic = |vbe: f64, vce: f64| {
        let vbc = vbe - vce;
        let i_f = is * ((vbe / vt).exp() - 1.0);
        let i_r = is * ((vbc / vt).exp() - 1.0);
        (i_f - i_r) * (1.0 - vbc / vaf) - i_r / br
    };
    // Operating point from the test's own bisection.
    let (mut lo, mut hi) = (0.0, 10.0);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if ic(0.66, mid) - (10.0 - mid) / 3000.0 > 0.0 {
            hi = mid
        } else {
            lo = mid
        }
    }
    let vce = 0.5 * (lo + hi);
    let h = 1e-6;
    let gm = (ic(0.66 + h, vce) - ic(0.66 - h, vce)) / (2.0 * h);
    let go = (ic(0.66, vce + h) - ic(0.66, vce - h)) / (2.0 * h);
    // KCL at the collector: (vc - vcc)/RC + ic = 0 -> dvc (1/RC + go) = -gm dvb.
    let gain = -gm / (1.0 / 3000.0 + go);
    let got = res.table.signal("V(c)").unwrap().at(0).unwrap();
    assert_close(got.re, gain, 1e-5, 0.0, "voltage gain");
    assert!(
        got.im.abs() < 1e-9 * gain.abs(),
        "no capacitances, so no phase shift: {got}"
    );
}

#[test]
fn junction_capacitance_adds_a_pole_at_the_expected_frequency() {
    // Reverse-biased diode: C(v) = CJO / (1 - v/VJ)^M at v = -4 V.
    let (cjo, vj, m) = (10e-12, 0.8, 0.5);
    let res = run_tight(&format!(
        "cj\nV1 in 0 DC -4 AC 1\nR1 in a 10k\nD1 a 0 DM\n.model DM D(IS=1e-14 CJO={cjo} VJ={vj} M={m})\n.ac lin 1 1meg 1meg\n"
    ));
    // The diode is almost open (gmin and Is aside), so only R and C(v) remain.
    let cap = cjo / (1.0f64 + 4.0 / vj).powf(m);
    let w = TAU * 1e6;
    let want = Complex64::new(1.0, 0.0) / (Complex64::new(1.0, 0.0) + j() * (w * 1e4 * cap));
    let got = res.table.signal("V(a)").unwrap().at(0).unwrap();
    assert!((got - want).norm() < 1e-6, "{got} vs {want}");
}

#[test]
fn frequency_grids_follow_spice_conventions() {
    let f = ac_frequencies(Sweep::Dec, 10, 1.0, 1000.0);
    assert_eq!(f.len(), 31);
    assert_close(f[10], 10.0, 1e-12, 0.0, "decade");
    assert_close(f[30], 1000.0, 1e-12, 0.0, "last");
    let f = ac_frequencies(Sweep::Oct, 3, 100.0, 800.0);
    assert_eq!(f.len(), 10);
    assert_close(f[3], 200.0, 1e-12, 0.0, "octave");
    let f = ac_frequencies(Sweep::Lin, 5, 10.0, 50.0);
    assert_eq!(f, vec![10.0, 20.0, 30.0, 40.0, 50.0]);
    assert_eq!(ac_frequencies(Sweep::Lin, 1, 7.0, 9.0), vec![7.0]);
    assert_eq!(ac_frequencies(Sweep::Dec, 5, 50.0, 50.0), vec![50.0]);
    // A range that is not a whole number of decades stops at the last point not above fstop.
    let f = ac_frequencies(Sweep::Dec, 1, 1.0, 50.0);
    assert_eq!(f.len(), 2);
}

#[test]
fn ac_without_a_source_asks_for_one() {
    let e = run_with("noac\nV1 a 0 DC 1\nR1 a 0 1k\n.ac dec 5 1 1k\n", &tight()).unwrap_err();
    assert!(
        matches!(&e, SimError::Invalid(m) if m.contains("AC")),
        "{e}"
    );
}

#[test]
fn inductor_and_capacitor_are_short_and_open_at_the_operating_point() {
    // At DC the inductor is a short: the AC analysis must linearise about that, not about zero.
    let res = run_tight(
        "op\nV1 in 0 DC 5 AC 1\nL1 in a 1m\nR1 a b 100\nR2 b 0 100\nC1 b 0 1n\n.ac lin 1 1 1\n",
    );
    let b = res.table.signal("V(b)").unwrap().at(0).unwrap();
    assert!((b.re - 0.5).abs() < 1e-6 && b.im.abs() < 1e-4, "{b}");
}
