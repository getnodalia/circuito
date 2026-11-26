// SPDX-License-Identifier: MIT OR Apache-2.0
//! Device equations: pure functions of terminal voltages, with analytic derivatives.
//!
//! References (public): L. W. Nagel, "SPICE2: A Computer Program to Simulate Semiconductor
//! Circuits", ERL-M520, UC Berkeley, 1975; A. Vladimirescu, "The SPICE Book", Wiley, 1994;
//! K. Kundert, "The Designer's Guide to SPICE and Spectre", Kluwer, 1995; H. Shichman and
//! D. Hodges, IEEE J. Solid-State Circuits 3(3), 1968 (MOSFET level 1). The equations are
//! implemented from those descriptions; no simulator source was consulted.

/// Boltzmann constant over elementary charge, volts per kelvin.
pub const K_OVER_Q: f64 = 8.617_333_262e-5;

/// Thermal voltage at `temp_c` degrees Celsius.
#[must_use]
pub fn thermal_voltage(temp_c: f64) -> f64 {
    K_OVER_Q * (temp_c + 273.15)
}

/// `exp(x)` that continues linearly above `X_MAX` so it never overflows and stays monotone with a
/// continuous derivative. Returns (value, derivative).
#[must_use]
pub fn exp_lim(x: f64) -> (f64, f64) {
    const X_MAX: f64 = 80.0;
    if x <= X_MAX {
        let e = x.exp();
        (e, e)
    } else {
        let e = X_MAX.exp();
        (e * (1.0 + (x - X_MAX)), e)
    }
}

/// The critical voltage of a junction above which Newton steps are compressed.
#[must_use]
pub fn vcrit(nvt: f64, is: f64) -> f64 {
    nvt * (nvt / (std::f64::consts::SQRT_2 * is.max(1e-300))).ln()
}

/// Limit the step of a forward-biased junction voltage from `vold` to `vnew`: beyond the critical
/// voltage a large step is compressed logarithmically (the exponential would otherwise overshoot
/// by many orders of magnitude). Returns (limited voltage, whether it was changed).
#[must_use]
pub fn pnj_limit(vnew: f64, vold: f64, nvt: f64, vc: f64) -> (f64, bool) {
    if vnew > vc && (vnew - vold).abs() > 2.0 * nvt {
        if vold > 0.0 {
            let arg = 1.0 + (vnew - vold) / nvt;
            if arg > 0.0 {
                return (vold + nvt * arg.ln(), true);
            }
            return (vc, true);
        }
        return (nvt * (vnew / nvt).ln(), true);
    }
    (vnew, false)
}

/// Limit a MOSFET gate-source voltage step to a safe size.
#[must_use]
pub fn fet_limit(vnew: f64, vold: f64, vth: f64) -> (f64, bool) {
    let vtsthi = (2.0 * (vold - vth).abs()) + 2.0;
    let delta = vnew - vold;
    if delta.abs() > vtsthi {
        (vold + vtsthi.copysign(delta), true)
    } else {
        (vnew, false)
    }
}

/// Junction depletion charge and capacitance (SPICE2): `q(v)`, `c(v) = dq/dv`.
/// `cj0` zero-bias capacitance, `vj` built-in potential, `m` grading, `fc` forward-bias knee.
#[must_use]
pub fn depletion(v: f64, cj0: f64, vj: f64, m: f64, fc: f64) -> (f64, f64) {
    if cj0 == 0.0 {
        return (0.0, 0.0);
    }
    if v < fc * vj {
        let arg = 1.0 - v / vj;
        let sarg = arg.powf(-m);
        let c = cj0 * sarg;
        let q = if (m - 1.0).abs() < 1e-9 {
            -cj0 * vj * arg.ln()
        } else {
            cj0 * vj * (1.0 - arg * sarg) / (1.0 - m)
        };
        (q, c)
    } else {
        let f1 = if (m - 1.0).abs() < 1e-9 {
            -vj * (1.0 - fc).ln()
        } else {
            vj * (1.0 - (1.0 - fc).powf(1.0 - m)) / (1.0 - m)
        };
        let f2 = (1.0 - fc).powf(1.0 + m);
        let f3 = 1.0 - fc * (1.0 + m);
        let q =
            cj0 * (f1 + (f3 * (v - fc * vj) + (m / (2.0 * vj)) * (v * v - fc * fc * vj * vj)) / f2);
        let c = cj0 * (f3 + m * v / vj) / f2;
        (q, c)
    }
}

/// Diode model parameters (already scaled by area).
#[derive(Debug, Clone, Copy)]
pub struct DiodeP {
    /// Saturation current (A).
    pub is: f64,
    /// Emission coefficient.
    pub n: f64,
    /// Series resistance (ohm).
    pub rs: f64,
    /// Transit time (s).
    pub tt: f64,
    /// Zero-bias junction capacitance (F).
    pub cjo: f64,
    /// Junction potential (V).
    pub vj: f64,
    /// Grading coefficient.
    pub m: f64,
    /// Forward-bias depletion coefficient.
    pub fc: f64,
    /// Reverse breakdown voltage (positive), if any.
    pub bv: Option<f64>,
    /// Current at breakdown (A).
    pub ibv: f64,
    /// Thermal voltage (V).
    pub vt: f64,
}

/// Diode junction current and conductance at junction voltage `v`, without `gmin`.
/// Returns (i, g). Includes the reverse breakdown branch when `bv` is set.
#[must_use]
pub fn diode_iv(p: &DiodeP, v: f64) -> (f64, f64) {
    let nvt = p.n * p.vt;
    let (e, de) = exp_lim(v / nvt);
    let mut i = p.is * (e - 1.0);
    let mut g = p.is * de / nvt;
    if let Some(bv) = p.bv {
        // Breakdown: current -IBV at v = -BV, rising exponentially beyond it.
        let (eb, deb) = exp_lim(-(v + bv) / nvt);
        i -= p.ibv * eb;
        g += p.ibv * deb / nvt;
    }
    (i, g)
}

/// Diode charge and capacitance at junction voltage `v`: depletion plus diffusion (`tt * i`).
#[must_use]
pub fn diode_q(p: &DiodeP, v: f64) -> (f64, f64) {
    let (qd, cd) = depletion(v, p.cjo, p.vj, p.m, p.fc);
    let nvt = p.n * p.vt;
    let (e, de) = exp_lim(v / nvt);
    let idiff = p.is * (e - 1.0);
    let gdiff = p.is * de / nvt;
    (
        qd + p.tt * idiff.max(0.0),
        cd + p.tt * if idiff > 0.0 { gdiff } else { 0.0 },
    )
}

/// Bipolar transistor parameters (already scaled by area), for an NPN; PNP flips signs outside.
#[derive(Debug, Clone, Copy)]
pub struct BjtP {
    /// Transport saturation current.
    pub is: f64,
    /// Forward beta.
    pub bf: f64,
    /// Reverse beta.
    pub br: f64,
    /// Forward emission coefficient.
    pub nf: f64,
    /// Reverse emission coefficient.
    pub nr: f64,
    /// Forward Early voltage (inverse is stored: 1/VAF, 0 for none).
    pub inv_vaf: f64,
    /// Reverse Early voltage (inverse).
    pub inv_var: f64,
    /// Thermal voltage.
    pub vt: f64,
    /// Base-emitter depletion capacitance.
    pub cje: f64,
    /// Base-emitter junction potential.
    pub vje: f64,
    /// Base-emitter grading.
    pub mje: f64,
    /// Base-collector depletion capacitance.
    pub cjc: f64,
    /// Base-collector junction potential.
    pub vjc: f64,
    /// Base-collector grading.
    pub mjc: f64,
    /// Forward-bias depletion coefficient.
    pub fc: f64,
    /// Forward transit time.
    pub tf: f64,
    /// Reverse transit time.
    pub tr: f64,
}

/// Terminal currents and their partial derivatives for an NPN at junction voltages
/// `vbe`, `vbc`: collector current `ic`, base current `ib` (emitter current is `-(ic+ib)`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BjtEval {
    /// Collector current.
    pub ic: f64,
    /// Base current.
    pub ib: f64,
    /// d ic / d vbe.
    pub gc_be: f64,
    /// d ic / d vbc.
    pub gc_bc: f64,
    /// d ib / d vbe.
    pub gb_be: f64,
    /// d ib / d vbc.
    pub gb_bc: f64,
    /// Forward diode current `If`.
    pub i_f: f64,
    /// Reverse diode current `Ir`.
    pub i_r: f64,
}

/// Ebers-Moll transport model with Early effect (the Gummel-Poon core without high-injection
/// and leakage terms): `ic = (If - Ir) * K - Ir/BR`, `ib = If/BF + Ir/BR` with
/// `K = 1 - vbc/VAF - vbe/VAR` (the inverse base-charge factor).
#[must_use]
pub fn bjt_eval(p: &BjtP, vbe: f64, vbc: f64) -> BjtEval {
    let (ef, def) = exp_lim(vbe / (p.nf * p.vt));
    let (er, der) = exp_lim(vbc / (p.nr * p.vt));
    let i_f = p.is * (ef - 1.0);
    let i_r = p.is * (er - 1.0);
    let gf = p.is * def / (p.nf * p.vt);
    let gr = p.is * der / (p.nr * p.vt);
    let mut k = 1.0 - vbc * p.inv_vaf - vbe * p.inv_var;
    let (mut dk_be, mut dk_bc) = (-p.inv_var, -p.inv_vaf);
    if k < 1e-3 {
        k = 1e-3;
        dk_be = 0.0;
        dk_bc = 0.0;
    }
    let itr = i_f - i_r;
    BjtEval {
        ic: itr * k - i_r / p.br,
        ib: i_f / p.bf + i_r / p.br,
        gc_be: gf * k + itr * dk_be,
        gc_bc: -gr * k + itr * dk_bc - gr / p.br,
        gb_be: gf / p.bf,
        gb_bc: gr / p.br,
        i_f,
        i_r,
    }
}

/// Base-emitter junction charge and capacitance.
#[must_use]
pub fn bjt_q_be(p: &BjtP, vbe: f64) -> (f64, f64) {
    let (q, c) = depletion(vbe, p.cje, p.vje, p.mje, p.fc);
    let (e, de) = exp_lim(vbe / (p.nf * p.vt));
    let i_f = p.is * (e - 1.0);
    let g_f = p.is * de / (p.nf * p.vt);
    (
        q + p.tf * i_f.max(0.0),
        c + p.tf * if i_f > 0.0 { g_f } else { 0.0 },
    )
}

/// Base-collector junction charge and capacitance.
#[must_use]
pub fn bjt_q_bc(p: &BjtP, vbc: f64) -> (f64, f64) {
    let (q, c) = depletion(vbc, p.cjc, p.vjc, p.mjc, p.fc);
    let (e, de) = exp_lim(vbc / (p.nr * p.vt));
    let i_r = p.is * (e - 1.0);
    let g_r = p.is * de / (p.nr * p.vt);
    (
        q + p.tr * i_r.max(0.0),
        c + p.tr * if i_r > 0.0 { g_r } else { 0.0 },
    )
}

/// MOSFET level 1 parameters (for an N-channel; P-channel flips signs outside).
#[derive(Debug, Clone, Copy)]
pub struct MosP {
    /// Zero-bias threshold voltage (already polarity-adjusted: positive for enhancement).
    pub vto: f64,
    /// Transconductance parameter times W/L, `beta` (A/V^2).
    pub beta: f64,
    /// Body-effect coefficient (sqrt(V)).
    pub gamma: f64,
    /// Surface potential (V).
    pub phi: f64,
    /// Channel-length modulation (1/V).
    pub lambda: f64,
}

/// Drain current and derivatives for a MOSFET with `vds >= 0` (Shichman-Hodges).
/// Returns (id, gm = did/dvgs, gds = did/dvds, gmbs = did/dvbs).
#[must_use]
pub fn mos_forward(p: &MosP, vgs: f64, vds: f64, vbs: f64) -> (f64, f64, f64, f64) {
    // Threshold with body effect: vth = vto + gamma (sqrt(phi - vbs) - sqrt(phi)). The square root
    // is continued by its tangent line once the bulk is forward biased past phi/2, so the model
    // stays defined, continuous and differentiable there (it is not physical in that region).
    let vb0 = 0.5 * p.phi;
    let (sarg, dsarg) = if vbs <= vb0 {
        let s = (p.phi - vbs).sqrt();
        (s, -0.5 / s)
    } else {
        let s0 = (p.phi - vb0).sqrt();
        let d0 = -0.5 / s0;
        let s = s0 + d0 * (vbs - vb0);
        if s > 1e-2 {
            (s, d0)
        } else {
            (1e-2, 0.0)
        }
    };
    let vth = p.vto + p.gamma * (sarg - p.phi.sqrt());
    let eta = -p.gamma * dsarg;
    let vov = vgs - vth;
    if vov <= 0.0 {
        return (0.0, 0.0, 0.0, 0.0);
    }
    let clm = 1.0 + p.lambda * vds;
    let (id, gm, gds) = if vds < vov {
        // Triode.
        let core = vov * vds - 0.5 * vds * vds;
        let id = p.beta * core * clm;
        let gm = p.beta * vds * clm;
        let gds = p.beta * ((vov - vds) * clm + core * p.lambda);
        (id, gm, gds)
    } else {
        // Saturation.
        let core = 0.5 * vov * vov;
        let id = p.beta * core * clm;
        let gm = p.beta * vov * clm;
        let gds = p.beta * core * p.lambda;
        (id, gm, gds)
    };
    (id, gm, gds, gm * eta)
}

/// Drain current and derivatives for any sign of `vds`, using the source-drain symmetry of the
/// device: when `vds < 0` the roles of drain and source swap. Returns (id, gm, gds, gmbs) as
/// derivatives with respect to `vgs`, `vds`, `vbs` of the original orientation.
#[must_use]
pub fn mos_eval(p: &MosP, vgs: f64, vds: f64, vbs: f64) -> (f64, f64, f64, f64) {
    if vds >= 0.0 {
        mos_forward(p, vgs, vds, vbs)
    } else {
        // id(vgs, vds, vbs) = -f(vgs - vds, -vds, vbs - vds) with f the forward characteristic.
        let (id, gm, gds, gmbs) = mos_forward(p, vgs - vds, -vds, vbs - vds);
        (-id, -gm, gm + gds + gmbs, -gmbs)
    }
}

/// Op-amp output transfer: a linear gain `a` with smooth saturation at `vmax` / `vmin` (`vmin`
/// negative) using `f(x) = x / (1 + (|x|/V)^8)^(1/8)`, which is linear to better than 1e-9 relative
/// for `|x| < V/20`. Returns (output, d output / d vd) for differential input `vd`.
#[must_use]
pub fn opamp_transfer(a: f64, vmax: Option<f64>, vmin: Option<f64>, vd: f64) -> (f64, f64) {
    let x = a * vd;
    let lim = if x >= 0.0 { vmax } else { vmin.map(f64::abs) };
    match lim {
        None => (x, a),
        Some(v) => {
            let t = (x.abs() / v).powi(8);
            if !t.is_finite() {
                return (v.copysign(x), 0.0);
            }
            let s = 1.0 + t;
            let f = x * s.powf(-0.125);
            let df = s.powf(-1.125);
            (f, a * df)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dp() -> DiodeP {
        DiodeP {
            is: 1e-14,
            n: 1.0,
            rs: 0.0,
            tt: 0.0,
            cjo: 0.0,
            vj: 1.0,
            m: 0.5,
            fc: 0.5,
            bv: None,
            ibv: 1e-3,
            vt: thermal_voltage(27.0),
        }
    }

    fn num_deriv(f: impl Fn(f64) -> f64, x: f64) -> f64 {
        let h = 1e-6 * x.abs().max(1e-3);
        (f(x + h) - f(x - h)) / (2.0 * h)
    }

    #[test]
    fn thermal_voltage_at_room_temperature() {
        // k/q = 8.617333262e-5 V/K at 300.15 K.
        assert!((thermal_voltage(27.0) - 0.025_864_7).abs() < 1e-6);
    }

    #[test]
    fn exp_lim_is_continuous_and_finite() {
        let (a, da) = exp_lim(80.0);
        let (b, db) = exp_lim(80.0 + 1e-9);
        assert!((a - b).abs() / a < 1e-6 && (da - db).abs() / da < 1e-6);
        let (big, _) = exp_lim(1e6);
        assert!(big.is_finite());
        assert!(exp_lim(-1e6).0 == 0.0);
    }

    #[test]
    fn diode_derivative_matches_numeric() {
        let p = dp();
        for v in [-1.0, -0.1, 0.0, 0.3, 0.6, 0.7] {
            let (_, g) = diode_iv(&p, v);
            let n = num_deriv(|x| diode_iv(&p, x).0, v);
            assert!((g - n).abs() <= 1e-5 * g.abs() + 1e-18, "v={v} g={g} n={n}");
        }
    }

    #[test]
    fn diode_forward_matches_shockley() {
        let p = dp();
        let (i, _) = diode_iv(&p, 0.7);
        let want = 1e-14 * ((0.7 / thermal_voltage(27.0)).exp() - 1.0);
        assert!((i - want).abs() / want < 1e-12);
    }

    #[test]
    fn breakdown_conducts_ibv_at_bv() {
        let p = DiodeP {
            bv: Some(5.1),
            ibv: 5e-3,
            ..dp()
        };
        let (i, g) = diode_iv(&p, -5.1);
        assert!((i + 5e-3).abs() < 1e-5, "{i}");
        assert!(g > 0.0);
        let (i2, _) = diode_iv(&p, -4.0);
        assert!(i2.abs() < 1e-12);
        let (i3, _) = diode_iv(&p, -5.3);
        assert!(i3 < -5e-3 * 10.0);
    }

    #[test]
    fn depletion_charge_derivative_is_capacitance_and_continuous_at_knee() {
        for v in [-5.0, -1.0, 0.0, 0.3, 0.49, 0.51, 0.9, 2.0] {
            let (_, c) = depletion(v, 2e-12, 1.0, 0.5, 0.5);
            let n = num_deriv(|x| depletion(x, 2e-12, 1.0, 0.5, 0.5).0, v);
            assert!((c - n).abs() <= 1e-5 * c.abs(), "v={v} c={c} n={n}");
        }
        let (q1, c1) = depletion(0.5 - 1e-9, 2e-12, 1.0, 0.5, 0.5);
        let (q2, c2) = depletion(0.5 + 1e-9, 2e-12, 1.0, 0.5, 0.5);
        assert!((q1 - q2).abs() < 1e-18 && (c1 - c2).abs() / c1 < 1e-6);
        // At zero bias the capacitance is CJ0.
        assert!((depletion(0.0, 2e-12, 1.0, 0.5, 0.5).1 - 2e-12).abs() < 1e-24);
        // m = 1 uses the logarithmic charge.
        let (_, c) = depletion(-1.0, 1e-12, 1.0, 1.0, 0.5);
        assert!((c - 0.5e-12).abs() < 1e-18);
    }

    fn bp() -> BjtP {
        BjtP {
            is: 1e-14,
            bf: 100.0,
            br: 2.0,
            nf: 1.0,
            nr: 1.0,
            inv_vaf: 1.0 / 75.0,
            inv_var: 0.0,
            vt: thermal_voltage(27.0),
            cje: 0.0,
            vje: 0.75,
            mje: 0.33,
            cjc: 0.0,
            vjc: 0.75,
            mjc: 0.33,
            fc: 0.5,
            tf: 0.0,
            tr: 0.0,
        }
    }

    #[test]
    fn bjt_derivatives_match_numeric() {
        let p = bp();
        for (vbe, vbc) in [
            (0.65, -3.0),
            (0.7, 0.4),
            (0.0, -1.0),
            (-0.3, -0.3),
            (0.72, 0.68),
        ] {
            let e = bjt_eval(&p, vbe, vbc);
            let nb = |f: &dyn Fn(f64, f64) -> f64, wrt: usize| {
                let h = 1e-6;
                if wrt == 0 {
                    (f(vbe + h, vbc) - f(vbe - h, vbc)) / (2.0 * h)
                } else {
                    (f(vbe, vbc + h) - f(vbe, vbc - h)) / (2.0 * h)
                }
            };
            let ic = |a: f64, b: f64| bjt_eval(&p, a, b).ic;
            let ib = |a: f64, b: f64| bjt_eval(&p, a, b).ib;
            for (got, want) in [
                (e.gc_be, nb(&ic, 0)),
                (e.gc_bc, nb(&ic, 1)),
                (e.gb_be, nb(&ib, 0)),
                (e.gb_bc, nb(&ib, 1)),
            ] {
                assert!(
                    (got - want).abs() <= 1e-5 * want.abs() + 1e-16,
                    "{vbe},{vbc}: {got} vs {want}"
                );
            }
        }
    }

    #[test]
    fn bjt_forward_active_gain_and_early() {
        let p = bp();
        let e = bjt_eval(&p, 0.65, -5.0);
        // In forward-active the collector current is If*(1 + vcb/VAF) minus a tiny reverse part.
        let want = e.i_f * (1.0 + 5.0 / 75.0);
        assert!((e.ic - want).abs() / want < 1e-9);
        assert!((e.ic / e.ib - 100.0 * (1.0 + 5.0 / 75.0)).abs() < 1e-3);
    }

    fn mp() -> MosP {
        MosP {
            vto: 1.0,
            beta: 2e-3,
            gamma: 0.0,
            phi: 0.6,
            lambda: 0.0,
        }
    }

    #[test]
    fn mos_square_law_regions() {
        let p = mp();
        // Saturation: id = beta/2 (vgs - vto)^2.
        let (id, gm, gds, _) = mos_eval(&p, 3.0, 5.0, 0.0);
        assert!((id - 1e-3 * 4.0).abs() < 1e-15);
        assert!((gm - 2e-3 * 2.0).abs() < 1e-15);
        assert_eq!(gds, 0.0);
        // Triode: id = beta (vov vds - vds^2/2).
        let (id, _, gds, _) = mos_eval(&p, 3.0, 1.0, 0.0);
        assert!((id - 2e-3 * (2.0 - 0.5)).abs() < 1e-15);
        assert!((gds - 2e-3 * (2.0 - 1.0)).abs() < 1e-15);
        // Cutoff.
        assert_eq!(mos_eval(&p, 0.5, 5.0, 0.0).0, 0.0);
    }

    #[test]
    fn mos_is_symmetric_and_derivatives_match_numeric() {
        let p = MosP {
            gamma: 0.4,
            lambda: 0.05,
            ..mp()
        };
        let (a, ..) = mos_eval(&p, 3.0, 2.0, -1.0);
        let (b, ..) = mos_eval(&p, 3.0 - 2.0, -2.0, -1.0 - 2.0);
        assert!(
            (a + b).abs() < 1e-15,
            "swapping drain and source reverses the current"
        );
        for (vgs, vds, vbs) in [
            (3.0, 5.0, -1.0),
            (3.0, 1.0, -0.5),
            (2.0, -1.5, -0.2),
            (2.5, -4.0, 0.0),
            (3.0, 1.9, -1.0),
        ] {
            let (_, gm, gds, gmbs) = mos_eval(&p, vgs, vds, vbs);
            let h = 1e-6;
            let n_gm =
                (mos_eval(&p, vgs + h, vds, vbs).0 - mos_eval(&p, vgs - h, vds, vbs).0) / (2.0 * h);
            let n_gds =
                (mos_eval(&p, vgs, vds + h, vbs).0 - mos_eval(&p, vgs, vds - h, vbs).0) / (2.0 * h);
            let n_gmb =
                (mos_eval(&p, vgs, vds, vbs + h).0 - mos_eval(&p, vgs, vds, vbs - h).0) / (2.0 * h);
            for (g, n) in [(gm, n_gm), (gds, n_gds), (gmbs, n_gmb)] {
                assert!(
                    (g - n).abs() <= 1e-5 * n.abs() + 1e-12,
                    "{vgs},{vds},{vbs}: {g} vs {n}"
                );
            }
        }
    }

    #[test]
    fn opamp_is_linear_then_saturates() {
        let (v, g) = opamp_transfer(1e5, Some(13.0), Some(-13.0), 1e-5);
        assert!((v - 1.0).abs() < 1e-9 && (g - 1e5).abs() / 1e5 < 1e-8);
        let (v, g) = opamp_transfer(1e5, Some(13.0), Some(-13.0), 1.0);
        assert!((v - 13.0).abs() < 1e-3 && g < 1.0);
        let (v, _) = opamp_transfer(1e5, Some(13.0), Some(-10.0), -1.0);
        assert!((v + 10.0).abs() < 1e-3);
        let (v, g) = opamp_transfer(1e5, None, None, 1.0);
        assert_eq!((v, g), (1e5, 1e5));
        // Derivative check.
        for vd in [-2e-4, -1e-4, 1e-5, 1.0e-4, 1.3e-4, 1.6e-4] {
            let (_, g) = opamp_transfer(1e5, Some(13.0), Some(-13.0), vd);
            let n = num_deriv(|x| opamp_transfer(1e5, Some(13.0), Some(-13.0), x).0, vd);
            assert!(
                (g - n).abs() <= 1e-4 * n.abs() + 1e-6,
                "vd={vd}: {g} vs {n}"
            );
        }
    }

    #[test]
    fn limiting_compresses_large_forward_steps() {
        let nvt = thermal_voltage(27.0);
        let vc = vcrit(nvt, 1e-14);
        assert!(vc > 0.5 && vc < 0.9, "{vc}");
        let (v, limited) = pnj_limit(5.0, 0.6, nvt, vc);
        assert!(limited && v > 0.6 && v < 1.0, "{v}");
        let (v, limited) = pnj_limit(0.65, 0.6, nvt, vc);
        assert!(!limited && v == 0.65);
        let (v, limited) = pnj_limit(-3.0, 0.0, nvt, vc);
        assert!(!limited && v == -3.0);
        let (v, limited) = pnj_limit(5.0, 0.0, nvt, vc);
        assert!(limited && v.is_finite() && v < 5.0);
        let (v, _) = fet_limit(100.0, 0.0, 1.0);
        assert!(v < 100.0);
    }
}
