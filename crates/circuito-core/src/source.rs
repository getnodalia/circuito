// SPDX-License-Identifier: MIT OR Apache-2.0
//! Independent source descriptions: DC value, AC small-signal magnitude and phase, and the
//! time-domain waveforms SIN, PULSE, PWL and EXP.
//!
//! The text form is SPICE's: `5`, `DC 5`, `AC 1 90`, `SIN(0 1 1k)`, `PULSE(0 5 0 1n 1n 5u 10u)`,
//! `PWL(0 0 1m 5 2m 0)`, `EXP(0 5 0 1m 5m 2m)`, and combinations such as `DC 0 AC 1 SIN(0 1 1k)`.
//! The equations follow the public SPICE definitions (Nagel, SPICE2 report; ngspice manual).

use crate::value::{format_value, parse_value, ValueError};
use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

/// Most points a PWL waveform may carry (bounds hostile input).
pub const MAX_PWL_POINTS: usize = 100_000;

/// Why a source description could not be understood.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum SourceError {
    /// A number in the description is malformed.
    #[error("source value: {0}")]
    Value(#[from] ValueError),
    /// A keyword or token is not part of the source grammar.
    #[error("source `{0}`: unexpected `{1}` (expected DC, AC, SIN, PULSE, PWL or EXP)")]
    Unexpected(String, String),
    /// A keyword has too few or too many parameters.
    #[error("{keyword} takes {expected} but {got} were given")]
    Arity {
        /// The waveform keyword.
        keyword: &'static str,
        /// A human description of the accepted count.
        expected: &'static str,
        /// How many numbers were supplied.
        got: usize,
    },
    /// A parameter has a value the waveform cannot use.
    #[error("{0}")]
    Invalid(String),
}

/// AC small-signal specification.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AcSpec {
    /// Magnitude (volts or amperes).
    pub magnitude: f64,
    /// Phase in degrees.
    pub phase_deg: f64,
}

/// A time-domain waveform.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Waveform {
    /// `SIN(vo va freq td theta phase)`.
    Sin {
        /// DC offset.
        offset: f64,
        /// Amplitude.
        amplitude: f64,
        /// Frequency in hertz.
        freq: f64,
        /// Delay in seconds.
        delay: f64,
        /// Damping factor in 1/s.
        damping: f64,
        /// Phase in degrees.
        phase_deg: f64,
    },
    /// `PULSE(v1 v2 td tr tf pw per)`; `period == 0` means a single pulse.
    Pulse {
        /// Initial value.
        v1: f64,
        /// Pulsed value.
        v2: f64,
        /// Delay.
        delay: f64,
        /// Rise time.
        rise: f64,
        /// Fall time.
        fall: f64,
        /// Width of the pulsed value.
        width: f64,
        /// Period, or 0 for a single pulse.
        period: f64,
    },
    /// `PWL(t1 v1 t2 v2 ...)` with non-decreasing times.
    Pwl {
        /// (time, value) points.
        points: Vec<(f64, f64)>,
    },
    /// `EXP(v1 v2 td1 tau1 td2 tau2)`.
    Exp {
        /// Initial value.
        v1: f64,
        /// Final value of the rising exponential.
        v2: f64,
        /// Rise delay.
        rise_delay: f64,
        /// Rise time constant.
        rise_tau: f64,
        /// Fall delay.
        fall_delay: f64,
        /// Fall time constant.
        fall_tau: f64,
    },
}

/// An independent source: optional DC, optional AC and optional waveform.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Source {
    /// Explicit DC value, if given.
    pub dc: Option<f64>,
    /// AC small-signal spec, if given.
    pub ac: Option<AcSpec>,
    /// Time-domain waveform, if given.
    pub wave: Option<Waveform>,
}

impl Waveform {
    /// Value at time `t` (seconds). Always finite for finite parameters.
    #[must_use]
    pub fn value_at(&self, t: f64) -> f64 {
        match self {
            Waveform::Sin {
                offset,
                amplitude,
                freq,
                delay,
                damping,
                phase_deg,
            } => {
                let ph = phase_deg.to_radians();
                if t < *delay {
                    offset + amplitude * ph.sin()
                } else {
                    let dt = t - delay;
                    offset
                        + amplitude
                            * (-dt * damping).exp()
                            * (std::f64::consts::TAU * freq * dt + ph).sin()
                }
            }
            Waveform::Pulse {
                v1,
                v2,
                delay,
                rise,
                fall,
                width,
                period,
            } => {
                if t < *delay {
                    return *v1;
                }
                let mut tt = t - delay;
                if *period > 0.0 {
                    tt %= period;
                }
                if tt < *rise {
                    v1 + (v2 - v1) * tt / rise
                } else if tt < rise + width {
                    *v2
                } else if tt < rise + width + fall {
                    v2 + (v1 - v2) * (tt - rise - width) / fall
                } else {
                    *v1
                }
            }
            Waveform::Pwl { points } => {
                let (Some(first), Some(last)) = (points.first(), points.last()) else {
                    return 0.0;
                };
                if t < first.0 {
                    return first.1;
                }
                if t >= last.0 {
                    return last.1;
                }
                // First point with time > t; the segment is [idx-1, idx].
                let idx = points.partition_point(|p| p.0 <= t);
                let (Some(a), Some(b)) = (points.get(idx.wrapping_sub(1)), points.get(idx)) else {
                    return last.1;
                };
                if b.0 > a.0 {
                    a.1 + (b.1 - a.1) * (t - a.0) / (b.0 - a.0)
                } else {
                    b.1
                }
            }
            Waveform::Exp {
                v1,
                v2,
                rise_delay,
                rise_tau,
                fall_delay,
                fall_tau,
            } => {
                if t < *rise_delay {
                    *v1
                } else if t < *fall_delay {
                    v1 + (v2 - v1) * (1.0 - (-(t - rise_delay) / rise_tau).exp())
                } else {
                    v1 + (v2 - v1) * (1.0 - (-(t - rise_delay) / rise_tau).exp())
                        + (v1 - v2) * (1.0 - (-(t - fall_delay) / fall_tau).exp())
                }
            }
        }
    }

    /// Times in `(0, tstop]` where the waveform has a corner or a jump, in increasing order. A
    /// transient solver steps exactly onto these. At most `limit` are returned.
    #[must_use]
    pub fn breakpoints(&self, tstop: f64, limit: usize) -> Vec<f64> {
        let mut out = Vec::new();
        let push = |t: f64, out: &mut Vec<f64>| {
            if t.is_finite() && t > 0.0 && t <= tstop && out.len() < limit {
                out.push(t);
            }
        };
        match self {
            Waveform::Sin { delay, .. } => push(*delay, &mut out),
            Waveform::Pulse {
                delay,
                rise,
                fall,
                width,
                period,
                ..
            } => {
                let mut base = *delay;
                let edges = [0.0, *rise, rise + width, rise + width + fall];
                let mut guard = 0usize;
                while base <= tstop && out.len() < limit && guard < limit {
                    for e in edges {
                        push(base + e, &mut out);
                    }
                    if *period <= 0.0 {
                        break;
                    }
                    base += period;
                    guard += 1;
                }
            }
            Waveform::Pwl { points } => {
                for p in points {
                    push(p.0, &mut out);
                }
            }
            Waveform::Exp {
                rise_delay,
                fall_delay,
                ..
            } => {
                push(*rise_delay, &mut out);
                push(*fall_delay, &mut out);
            }
        }
        out.sort_by(f64::total_cmp);
        out.dedup();
        out
    }

    fn validate(&self) -> Result<(), SourceError> {
        let bad = |m: &str| Err(SourceError::Invalid(m.to_string()));
        let finite = |xs: &[f64]| xs.iter().all(|x| x.is_finite());
        match self {
            Waveform::Sin {
                offset,
                amplitude,
                freq,
                delay,
                damping,
                phase_deg,
            } => {
                if !finite(&[*offset, *amplitude, *freq, *delay, *damping, *phase_deg]) {
                    return bad("SIN parameters must be finite");
                }
                if *freq < 0.0 {
                    return bad("SIN frequency must not be negative");
                }
                if *delay < 0.0 {
                    return bad("SIN delay must not be negative");
                }
            }
            Waveform::Pulse {
                v1,
                v2,
                delay,
                rise,
                fall,
                width,
                period,
            } => {
                if !finite(&[*v1, *v2, *delay, *rise, *fall, *width, *period]) {
                    return bad("PULSE parameters must be finite");
                }
                if *delay < 0.0 || *rise < 0.0 || *fall < 0.0 || *width < 0.0 || *period < 0.0 {
                    return bad("PULSE delay, rise, fall, width and period must not be negative");
                }
                if *period > 0.0 && rise + width + fall > *period * (1.0 + 1e-12) {
                    return bad("PULSE period is shorter than rise + width + fall");
                }
            }
            Waveform::Pwl { points } => {
                if points.is_empty() {
                    return bad("PWL needs at least one (time, value) pair");
                }
                if points.len() > MAX_PWL_POINTS {
                    return bad("PWL has too many points");
                }
                let mut prev = f64::NEG_INFINITY;
                for (t, v) in points {
                    if !t.is_finite() || !v.is_finite() {
                        return bad("PWL points must be finite");
                    }
                    if *t < prev {
                        return bad("PWL times must not decrease");
                    }
                    prev = *t;
                }
            }
            Waveform::Exp {
                v1,
                v2,
                rise_delay,
                rise_tau,
                fall_delay,
                fall_tau,
            } => {
                if !finite(&[*v1, *v2, *rise_delay, *rise_tau, *fall_delay, *fall_tau]) {
                    return bad("EXP parameters must be finite");
                }
                if *rise_tau <= 0.0 || *fall_tau <= 0.0 {
                    return bad("EXP time constants must be positive");
                }
                if *fall_delay < *rise_delay {
                    return bad("EXP fall delay must not precede the rise delay");
                }
            }
        }
        Ok(())
    }
}

impl Source {
    /// A plain DC source.
    #[must_use]
    pub fn dc(v: f64) -> Self {
        Source {
            dc: Some(v),
            ac: None,
            wave: None,
        }
    }

    /// The value used by DC and operating-point analyses: the explicit DC value, else the
    /// waveform at `t = 0`, else 0.
    #[must_use]
    pub fn dc_value(&self) -> f64 {
        self.dc
            .or_else(|| self.wave.as_ref().map(|w| w.value_at(0.0)))
            .unwrap_or(0.0)
    }

    /// The value at time `t` for a transient analysis.
    #[must_use]
    pub fn value_at(&self, t: f64) -> f64 {
        match &self.wave {
            Some(w) => w.value_at(t),
            None => self.dc.unwrap_or(0.0),
        }
    }

    /// Parse the SPICE text form of a source.
    ///
    /// # Errors
    /// Returns a [`SourceError`] for unknown keywords, wrong parameter counts or invalid values.
    pub fn parse(spec: &str) -> Result<Source, SourceError> {
        let cleaned: String = spec
            .chars()
            .map(|c| if matches!(c, '(' | ')' | ',') { ' ' } else { c })
            .collect();
        let toks: Vec<&str> = cleaned.split_whitespace().collect();
        let mut src = Source::default();
        let mut i = 0;
        let is_kw = |t: &str| {
            matches!(
                t.to_ascii_uppercase().as_str(),
                "DC" | "AC" | "SIN" | "PULSE" | "PWL" | "EXP"
            )
        };
        let mut first = true;
        while i < toks.len() {
            let t = toks[i];
            let up = t.to_ascii_uppercase();
            if first && !is_kw(t) {
                src.dc = Some(parse_value(t)?);
                i += 1;
                first = false;
                continue;
            }
            first = false;
            // Numbers following the keyword, up to the next keyword.
            let mut j = i + 1;
            while j < toks.len() && !is_kw(toks[j]) {
                j += 1;
            }
            let args = &toks[i + 1..j];
            let nums: Result<Vec<f64>, ValueError> = args.iter().map(|a| parse_value(a)).collect();
            match up.as_str() {
                "DC" => {
                    let n = nums?;
                    if n.len() != 1 {
                        return Err(SourceError::Arity {
                            keyword: "DC",
                            expected: "1 value",
                            got: n.len(),
                        });
                    }
                    src.dc = Some(n[0]);
                }
                "AC" => {
                    let n = nums?;
                    if n.is_empty() || n.len() > 2 {
                        return Err(SourceError::Arity {
                            keyword: "AC",
                            expected: "magnitude and optional phase",
                            got: n.len(),
                        });
                    }
                    src.ac = Some(AcSpec {
                        magnitude: n[0],
                        phase_deg: n.get(1).copied().unwrap_or(0.0),
                    });
                }
                "SIN" => {
                    let n = nums?;
                    if !(3..=6).contains(&n.len()) {
                        return Err(SourceError::Arity {
                            keyword: "SIN",
                            expected: "3 to 6 values (vo va freq [td theta phase])",
                            got: n.len(),
                        });
                    }
                    let g = |k: usize| n.get(k).copied().unwrap_or(0.0);
                    src.wave = Some(Waveform::Sin {
                        offset: n[0],
                        amplitude: n[1],
                        freq: n[2],
                        delay: g(3),
                        damping: g(4),
                        phase_deg: g(5),
                    });
                }
                "PULSE" => {
                    let n = nums?;
                    if !(6..=7).contains(&n.len()) {
                        return Err(SourceError::Arity {
                            keyword: "PULSE",
                            expected: "6 or 7 values (v1 v2 td tr tf pw [per])",
                            got: n.len(),
                        });
                    }
                    src.wave = Some(Waveform::Pulse {
                        v1: n[0],
                        v2: n[1],
                        delay: n[2],
                        rise: n[3],
                        fall: n[4],
                        width: n[5],
                        period: n.get(6).copied().unwrap_or(0.0),
                    });
                }
                "PWL" => {
                    let n = nums?;
                    if n.is_empty() || n.len() % 2 != 0 {
                        return Err(SourceError::Arity {
                            keyword: "PWL",
                            expected: "an even number of values (t v pairs)",
                            got: n.len(),
                        });
                    }
                    if n.len() / 2 > MAX_PWL_POINTS {
                        return Err(SourceError::Invalid("PWL has too many points".into()));
                    }
                    src.wave = Some(Waveform::Pwl {
                        points: n.as_chunks::<2>().0.iter().map(|&[t, v]| (t, v)).collect(),
                    });
                }
                "EXP" => {
                    let n = nums?;
                    if n.len() != 6 {
                        return Err(SourceError::Arity {
                            keyword: "EXP",
                            expected: "6 values (v1 v2 td1 tau1 td2 tau2)",
                            got: n.len(),
                        });
                    }
                    src.wave = Some(Waveform::Exp {
                        v1: n[0],
                        v2: n[1],
                        rise_delay: n[2],
                        rise_tau: n[3],
                        fall_delay: n[4],
                        fall_tau: n[5],
                    });
                }
                _ => {
                    return Err(SourceError::Unexpected(
                        spec.trim().to_string(),
                        t.to_string(),
                    ))
                }
            }
            i = j;
        }
        if let Some(w) = &src.wave {
            w.validate()?;
        }
        if let Some(a) = &src.ac {
            if !a.magnitude.is_finite() || !a.phase_deg.is_finite() {
                return Err(SourceError::Invalid("AC values must be finite".into()));
            }
        }
        if let Some(d) = src.dc {
            if !d.is_finite() {
                return Err(SourceError::Invalid("DC value must be finite".into()));
            }
        }
        Ok(src)
    }
}

impl fmt::Display for Waveform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let v = format_value;
        match self {
            Waveform::Sin {
                offset,
                amplitude,
                freq,
                delay,
                damping,
                phase_deg,
            } => write!(
                f,
                "SIN({} {} {} {} {} {})",
                v(*offset),
                v(*amplitude),
                v(*freq),
                v(*delay),
                v(*damping),
                v(*phase_deg)
            ),
            Waveform::Pulse {
                v1,
                v2,
                delay,
                rise,
                fall,
                width,
                period,
            } => write!(
                f,
                "PULSE({} {} {} {} {} {} {})",
                v(*v1),
                v(*v2),
                v(*delay),
                v(*rise),
                v(*fall),
                v(*width),
                v(*period)
            ),
            Waveform::Pwl { points } => {
                write!(f, "PWL(")?;
                for (k, (t, x)) in points.iter().enumerate() {
                    if k > 0 {
                        write!(f, " ")?;
                    }
                    write!(f, "{} {}", v(*t), v(*x))?;
                }
                write!(f, ")")
            }
            Waveform::Exp {
                v1,
                v2,
                rise_delay,
                rise_tau,
                fall_delay,
                fall_tau,
            } => write!(
                f,
                "EXP({} {} {} {} {} {})",
                v(*v1),
                v(*v2),
                v(*rise_delay),
                v(*rise_tau),
                v(*fall_delay),
                v(*fall_tau)
            ),
        }
    }
}

impl fmt::Display for Source {
    /// Canonical text form; `Source::parse` reads it back to an equal value.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts: Vec<String> = Vec::new();
        if let Some(d) = self.dc {
            parts.push(format!("DC {}", format_value(d)));
        }
        if let Some(a) = &self.ac {
            parts.push(format!(
                "AC {} {}",
                format_value(a.magnitude),
                format_value(a.phase_deg)
            ));
        }
        if let Some(w) = &self.wave {
            parts.push(w.to_string());
        }
        if parts.is_empty() {
            parts.push("DC 0".into());
        }
        write!(f, "{}", parts.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_forms() {
        assert_eq!(Source::parse("5").unwrap(), Source::dc(5.0));
        assert_eq!(Source::parse("DC 5").unwrap(), Source::dc(5.0));
        let s = Source::parse("dc 0 ac 1 90 sin(0 1 1k)").unwrap();
        assert_eq!(s.dc, Some(0.0));
        assert_eq!(
            s.ac,
            Some(AcSpec {
                magnitude: 1.0,
                phase_deg: 90.0
            })
        );
        assert!(matches!(s.wave, Some(Waveform::Sin { freq, .. }) if freq == 1000.0));
        let p = Source::parse("PULSE(0 5 0 1n 1n 5u 10u)").unwrap();
        assert!(matches!(p.wave, Some(Waveform::Pulse { period, .. }) if period == 10e-6));
        let w = Source::parse("PWL(0 0, 1m 5, 2m 0)").unwrap();
        assert!(matches!(w.wave, Some(Waveform::Pwl { ref points }) if points.len() == 3));
        assert!(Source::parse("EXP(0 5 0 1m 5m 2m)").is_ok());
    }

    #[test]
    fn rejects_bad_input() {
        for bad in [
            "SIN(0 1)",
            "PULSE(0 5)",
            "PWL(0 0 1)",
            "PWL()",
            "AC",
            "DC 1 2",
            "FOO(1)",
            "5 6",
            "SIN(0 1 -1k)",
            "PWL(1 0 0 1)",
            "EXP(0 1 0 0 1 1)",
            "PULSE(0 1 0 1 1 5 4)",
            "DC abc",
            "PULSE(0 1 -1 0 0 1 2)",
            "SIN(0 1 1e999)",
        ] {
            assert!(Source::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn display_round_trips() {
        for t in [
            "5",
            "DC 1.5 AC 2 45",
            "SIN(0 1 1k 1m 10 30)",
            "PULSE(0 5 1u 1n 2n 5u 10u)",
            "PWL(0 0 1m 5 1m 3 2m 0)",
            "EXP(0 5 1m 2m 5m 3m)",
            "AC 1",
            "0",
        ] {
            let s = Source::parse(t).unwrap();
            let again = Source::parse(&s.to_string()).unwrap();
            assert_eq!(s, again, "{t}");
        }
    }

    #[test]
    fn waveform_values() {
        let sin = Waveform::Sin {
            offset: 1.0,
            amplitude: 2.0,
            freq: 1.0,
            delay: 0.0,
            damping: 0.0,
            phase_deg: 0.0,
        };
        assert!((sin.value_at(0.25) - 3.0).abs() < 1e-12);
        assert!((sin.value_at(0.75) + 1.0).abs() < 1e-12);
        let pulse = Waveform::Pulse {
            v1: 0.0,
            v2: 5.0,
            delay: 1.0,
            rise: 1.0,
            fall: 1.0,
            width: 2.0,
            period: 10.0,
        };
        assert_eq!(pulse.value_at(0.5), 0.0);
        assert!((pulse.value_at(1.5) - 2.5).abs() < 1e-12);
        assert_eq!(pulse.value_at(3.0), 5.0);
        assert!((pulse.value_at(4.5) - 2.5).abs() < 1e-12);
        assert_eq!(pulse.value_at(7.0), 0.0);
        assert_eq!(pulse.value_at(12.0), 5.0 * 0.0 + pulse.value_at(2.0));
        let pwl = Waveform::Pwl {
            points: vec![(0.0, 0.0), (1.0, 2.0), (1.0, 4.0), (2.0, 4.0)],
        };
        assert_eq!(pwl.value_at(-1.0), 0.0);
        assert!((pwl.value_at(0.5) - 1.0).abs() < 1e-12);
        assert_eq!(pwl.value_at(1.0), 4.0);
        assert_eq!(pwl.value_at(5.0), 4.0);
        let e = Waveform::Exp {
            v1: 0.0,
            v2: 1.0,
            rise_delay: 0.0,
            rise_tau: 1.0,
            fall_delay: 100.0,
            fall_tau: 1.0,
        };
        assert!((e.value_at(1.0) - (1.0 - (-1.0f64).exp())).abs() < 1e-12);
    }

    #[test]
    fn breakpoints_cover_pulse_edges() {
        let pulse = Waveform::Pulse {
            v1: 0.0,
            v2: 1.0,
            delay: 0.0,
            rise: 1e-9,
            fall: 1e-9,
            width: 4e-6,
            period: 10e-6,
        };
        let b = pulse.breakpoints(25e-6, 1000);
        assert!(b.contains(&1e-9));
        assert!(b.iter().any(|t| (t - 10e-6).abs() < 1e-15));
        assert!(b.windows(2).all(|w| w[0] < w[1]));
        // Hostile: tiny period over a long run is capped by the limit.
        let tiny = Waveform::Pulse {
            v1: 0.0,
            v2: 1.0,
            delay: 0.0,
            rise: 0.0,
            fall: 0.0,
            width: 1e-12,
            period: 1e-9,
        };
        assert!(tiny.breakpoints(1.0, 500).len() <= 500);
    }
}
