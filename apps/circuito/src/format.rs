// SPDX-License-Identifier: MIT OR Apache-2.0
//! Numbers as an engineer reads them: SI prefixes, three significant digits, and the tick values
//! of a plot's axes.

/// The SI prefix for a power of ten that is a multiple of three.
fn prefix(exp3: i32) -> &'static str {
    match exp3 {
        -15 => "f",
        -12 => "p",
        -9 => "n",
        -6 => "µ",
        -3 => "m",
        3 => "k",
        6 => "M",
        9 => "G",
        12 => "T",
        _ => "",
    }
}

/// `value` split into a mantissa of at most three significant digits (already rounded, as text)
/// and its SI prefix. `None` for NaN and infinities.
fn split(value: f64) -> Option<(String, &'static str)> {
    if !value.is_finite() {
        return None;
    }
    if value == 0.0 {
        return Some(("0".into(), ""));
    }
    let mut exp3 = ((value.abs().log10() / 3.0).floor() as i32 * 3).clamp(-15, 12);
    loop {
        let m = value / 10f64.powi(exp3);
        let decimals = if m.abs() >= 99.95 {
            0
        } else if m.abs() >= 9.995 {
            1
        } else {
            2
        };
        let text = format!("{m:.decimals$}");
        // 999.7 rounds to 1000: move to the next prefix instead of printing four digits.
        if text.trim_start_matches('-').len() >= 4 && !text.contains('.') && exp3 < 12 {
            exp3 += 3;
            continue;
        }
        let text = if text.contains('.') {
            text.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            text
        };
        let text = if text == "-0" { "0".to_string() } else { text };
        let p = if text == "0" { "" } else { prefix(exp3) };
        return Some((text, p));
    }
}

/// `value` with an SI prefix and `unit`: `1.59 kHz`, `100 µs`, `-6.88 V`. Not finite: `—`.
pub fn eng(value: f64, unit: &str) -> String {
    match split(value) {
        Some((m, p)) => format!("{m} {p}{unit}").trim_end().to_string(),
        None => "—".to_string(),
    }
}

/// An axis label: the number and its prefix without a unit (`1k`, `100µ`, `0`).
pub fn axis_label(value: f64) -> String {
    match split(value) {
        Some((m, p)) => format!("{m}{p}"),
        None => "—".to_string(),
    }
}

/// Nicely spaced tick values inside `[lo, hi]`, about `target` of them: steps of 1, 2 or 5 times
/// a power of ten.
pub fn linear_ticks(lo: f64, hi: f64, target: usize) -> Vec<f64> {
    if !(lo.is_finite() && hi.is_finite()) || hi <= lo {
        return if lo.is_finite() { vec![lo] } else { Vec::new() };
    }
    let target = target.clamp(2, 40) as f64;
    let raw = (hi - lo) / target;
    let mag = 10f64.powf(raw.log10().floor());
    let norm = raw / mag;
    let step = mag
        * if norm <= 1.0 {
            1.0
        } else if norm <= 2.0 {
            2.0
        } else if norm <= 5.0 {
            5.0
        } else {
            10.0
        };
    let first = (lo / step).ceil();
    let mut ticks = Vec::new();
    let mut k = first;
    while ticks.len() < 64 {
        let v = k * step;
        if v > hi + step * 1e-9 {
            break;
        }
        // Clean the float noise of k * step.
        ticks.push(if v.abs() < step * 1e-9 { 0.0 } else { v });
        k += 1.0;
    }
    ticks
}

/// One tick of a logarithmic axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LogTick {
    pub value: f64,
    /// A power of ten (labelled); the others are the 2..9 multiples (grid only).
    pub major: bool,
}

/// The ticks of a logarithmic axis over `[lo, hi]` (both positive): every power of ten, and the
/// 2 to 9 multiples as minor ticks when the axis spans few decades.
pub fn log_ticks(lo: f64, hi: f64) -> Vec<LogTick> {
    if !(lo.is_finite() && hi.is_finite()) || lo <= 0.0 || hi <= lo {
        return Vec::new();
    }
    let (d0, d1) = (lo.log10().floor() as i32, hi.log10().ceil() as i32);
    let minor = d1 - d0 <= 6;
    let mut out = Vec::new();
    for d in d0..=d1 {
        let base = 10f64.powi(d);
        for m in 1..=9 {
            let v = base * f64::from(m);
            if v < lo * (1.0 - 1e-9) || v > hi * (1.0 + 1e-9) {
                continue;
            }
            if m == 1 || minor {
                out.push(LogTick {
                    value: v,
                    major: m == 1,
                });
            }
        }
        if out.len() > 200 {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engineering_values_read_naturally() {
        for (v, unit, want) in [
            (1590.0, "Hz", "1.59 kHz"),
            (1e-4, "s", "100 µs"),
            (0.0, "V", "0 V"),
            (-6.875, "V", "-6.88 V"),
            (10.0, "V", "10 V"),
            (6.875, "V", "6.88 V"),
            (0.0005, "A", "500 µA"),
            (999.7, "Hz", "1 kHz"),
            (1e6, "Hz", "1 MHz"),
            (2.2e-9, "F", "2.2 nF"),
            (123456.0, "Hz", "123 kHz"),
            (1.0, "", "1"),
        ] {
            assert_eq!(eng(v, unit), want, "{v}");
        }
    }

    #[test]
    fn values_that_are_not_numbers_say_so() {
        assert_eq!(eng(f64::NAN, "V"), "—");
        assert_eq!(eng(f64::INFINITY, "V"), "—");
        assert_eq!(eng(f64::NEG_INFINITY, ""), "—");
        assert_eq!(axis_label(f64::NAN), "—");
    }

    #[test]
    fn extreme_values_do_not_panic_or_lose_the_sign() {
        assert_eq!(eng(-1e-20, "V"), "0 V");
        assert!(eng(1e30, "V").ends_with("TV"));
        assert!(eng(-f64::MAX, "V").starts_with('-'));
        assert!(!eng(f64::MIN_POSITIVE, "V").is_empty());
    }

    #[test]
    fn axis_labels_have_no_unit() {
        assert_eq!(axis_label(1000.0), "1k");
        assert_eq!(axis_label(0.0001), "100µ");
        assert_eq!(axis_label(0.0), "0");
        assert_eq!(axis_label(2.5), "2.5");
    }

    #[test]
    fn linear_ticks_are_nice_and_inside() {
        let t = linear_ticks(0.0, 1.0, 5);
        assert_eq!(t.len(), 6);
        for (got, want) in t.iter().zip([0.0, 0.2, 0.4, 0.6, 0.8, 1.0]) {
            assert!((got - want).abs() < 1e-12, "{t:?}");
        }
        let t = linear_ticks(-0.3, 1.2, 8);
        assert!(t.iter().all(|v| (-0.3..=1.2 + 1e-9).contains(v)));
        assert!(t.contains(&0.0));
        assert!(t.len() >= 4 && t.len() <= 10, "{t:?}");
        assert_eq!(linear_ticks(3.0, 3.0, 5), vec![3.0]);
        assert!(linear_ticks(f64::NAN, 1.0, 5).is_empty());
        assert!(linear_ticks(0.0, f64::INFINITY, 5).len() <= 1);
        assert!(linear_ticks(0.0, 1e300, 5).len() <= 64);
    }

    #[test]
    fn log_ticks_cover_decades() {
        let t = log_ticks(10.0, 100_000.0);
        let majors: Vec<f64> = t.iter().filter(|t| t.major).map(|t| t.value).collect();
        assert_eq!(majors, vec![10.0, 100.0, 1000.0, 10000.0, 100000.0]);
        assert!(t.iter().any(|t| !t.major && (t.value - 200.0).abs() < 1e-9));
        assert!(log_ticks(0.0, 10.0).is_empty());
        assert!(log_ticks(-1.0, 10.0).is_empty());
        assert!(log_ticks(10.0, 1.0).is_empty());
        // Many decades: only the powers of ten.
        let wide = log_ticks(1e-9, 1e9);
        assert!(wide.iter().all(|t| t.major));
    }
}
