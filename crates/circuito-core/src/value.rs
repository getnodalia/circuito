// SPDX-License-Identifier: MIT OR Apache-2.0
//! Engineering values with SI suffixes.
//!
//! Parsing follows SPICE: `m` is milli and `meg` is mega, `f p n u µ m k meg g t` are the prefixes
//! and `mil` is 25.4 µ. A bare trailing unit word (`V`, `A`, `F`, `H`, `s`, `Hz`, `ohm`, `W`, `deg`)
//! is accepted and ignored, so `10uF` and `5V` parse. R-notation (`4k7`, `2R2`, `1u5`) is accepted.
//! A lone `F` with no prefix is *femto*, as in SPICE (not farad): write `1F` only if you mean it.
//!
//! Numbers are built from decimal text and parsed by the standard library, so the result is the
//! correctly rounded double of the decimal value: `100n` is exactly `1e-7`.

use thiserror::Error;

/// Why a value could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ValueError {
    /// The text was empty.
    #[error("empty value")]
    Empty,
    /// The text is not a number.
    #[error("`{0}` is not a number (examples: 4.7k, 100n, 2.2u, 10meg, 4k7, 1e-9)")]
    NotANumber(String),
    /// The unit or suffix after the number is not recognised.
    #[error("`{text}`: unknown suffix `{suffix}` (use f p n u m k meg g t, optionally followed by a unit such as V, A, F, H, Hz, ohm)")]
    UnknownSuffix {
        /// The whole text.
        text: String,
        /// The part that was not understood.
        suffix: String,
    },
    /// The value is too large or too small to represent.
    #[error("`{0}` is out of range for a finite number")]
    OutOfRange(String),
}

const MAX_LEN: usize = 64;

/// (suffix text, decimal exponent). Longest suffixes first.
const SUFFIXES: [(&str, i32); 11] = [
    ("meg", 6),
    ("mil", i32::MIN), // special-cased: 25.4e-6
    ("f", -15),
    ("p", -12),
    ("n", -9),
    ("u", -6),
    ("m", -3),
    ("k", 3),
    ("g", 9),
    ("t", 12),
    ("µ", -6),
];

const UNITS: [&str; 12] = [
    "v", "a", "f", "h", "s", "hz", "ohm", "ohms", "w", "deg", "ω", "Ω",
];

fn lookup_suffix(rest: &str) -> Option<(i32, usize)> {
    let lower = rest.to_lowercase();
    for (s, e) in SUFFIXES {
        if lower.starts_with(s) {
            // `μ` (Greek mu) is also accepted.
            return Some((e, s.len()));
        }
    }
    if rest.starts_with('μ') {
        return Some((-6, 'μ'.len_utf8()));
    }
    None
}

fn is_unit(rest: &str) -> bool {
    if rest.is_empty() {
        return true;
    }
    let l = rest.to_lowercase();
    UNITS.iter().any(|u| u.to_lowercase() == l)
}

/// Parse an engineering value such as `4.7k`, `100n`, `2.2µ`, `10meg`, `4k7`, `1e-9`.
///
/// # Errors
/// Returns a [`ValueError`] that names the offending text.
pub fn parse_value(text: &str) -> Result<f64, ValueError> {
    let t = text.trim();
    if t.is_empty() {
        return Err(ValueError::Empty);
    }
    if t.len() > MAX_LEN {
        return Err(ValueError::NotANumber(format!(
            "{}…",
            &t[..t.floor_char_boundary(24)]
        )));
    }
    let bytes = t.as_bytes();
    let mut i = 0;
    let mut mant = String::new();
    if matches!(bytes.first(), Some(b'+') | Some(b'-')) {
        if bytes[0] == b'-' {
            mant.push('-');
        }
        i += 1;
    }
    let int_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        mant.push(bytes[i] as char);
        i += 1;
    }
    let int_digits = i - int_start;
    let mut frac_digits = 0;
    let mut has_dot = false;
    if i < bytes.len() && bytes[i] == b'.' {
        has_dot = true;
        mant.push('.');
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            mant.push(bytes[i] as char);
            i += 1;
            frac_digits += 1;
        }
    }
    if int_digits + frac_digits == 0 {
        return Err(ValueError::NotANumber(t.to_string()));
    }
    // Exponent: `e` followed by optional sign and digits. `e` is not a suffix, so no clash.
    let mut exp10: i32 = 0;
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        let mut j = i + 1;
        let mut neg = false;
        if j < bytes.len() && (bytes[j] == b'+' || bytes[j] == b'-') {
            neg = bytes[j] == b'-';
            j += 1;
        }
        let ds = j;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        if j == ds {
            return Err(ValueError::NotANumber(t.to_string()));
        }
        let e: i32 = t[ds..j]
            .parse()
            .map_err(|_| ValueError::OutOfRange(t.to_string()))?;
        exp10 = if neg { -e } else { e };
        i = j;
    }
    let rest = &t[i..];
    let mut scale_exp: i32 = 0;
    let mut mil = false;
    let mut tail = rest;
    if !rest.is_empty() {
        if let Some((e, len)) = lookup_suffix(rest) {
            let after = &rest[len..];
            // R-notation: digits after the letter, e.g. 4k7. Only when there was no dot/exponent.
            if !after.is_empty()
                && after.bytes().all(|b| b.is_ascii_digit())
                && !has_dot
                && exp10 == 0
                && e != i32::MIN
            {
                mant.push('.');
                mant.push_str(after);
                scale_exp = e;
                tail = "";
            } else {
                if e == i32::MIN {
                    mil = true;
                } else {
                    scale_exp = e;
                }
                tail = after;
            }
        } else if (rest.starts_with('R') || rest.starts_with('r'))
            && rest.len() > 1
            && rest[1..].bytes().all(|b| b.is_ascii_digit())
            && !has_dot
            && exp10 == 0
        {
            mant.push('.');
            mant.push_str(&rest[1..]);
            tail = "";
        }
        if !is_unit(tail) {
            let suffix = rest.to_string();
            return Err(ValueError::UnknownSuffix {
                text: t.to_string(),
                suffix,
            });
        }
    }
    let total = exp10.saturating_add(scale_exp);
    let s = format!("{mant}e{total}");
    let mut v: f64 = s
        .parse()
        .map_err(|_| ValueError::NotANumber(t.to_string()))?;
    if mil {
        v *= 25.4e-6;
    }
    if !v.is_finite() {
        return Err(ValueError::OutOfRange(t.to_string()));
    }
    Ok(v)
}

/// Format a value with an SI suffix using the fewest digits that read back as exactly the same
/// number: `4700 -> "4.7k"`, `1e-7 -> "100n"`, `1e6 -> "1meg"`, `0 -> "0"`.
/// `parse_value(&format_value(x)) == Ok(x)` for every finite `x`. Non-finite values format as
/// `nan`, `inf` and `-inf` (which do not parse: they are never valid circuit values).
#[must_use]
pub fn format_value(x: f64) -> String {
    if x == 0.0 {
        return "0".to_string();
    }
    if !x.is_finite() {
        return if x.is_nan() {
            "nan".into()
        } else if x > 0.0 {
            "inf".into()
        } else {
            "-inf".into()
        };
    }
    // Shortest round-trip digits via the exponent form: "-4.7e3".
    let sci = format!("{x:e}");
    let (neg, body) = match sci.strip_prefix('-') {
        Some(b) => (true, b),
        None => (false, sci.as_str()),
    };
    let (mant, exp) = body.split_once('e').unwrap_or((body, "0"));
    let exp10: i32 = exp.parse().unwrap_or(0);
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    // Outside the prefix range (below femto, at or above 1e15) the plain exponent form is clearer.
    if !(-15..15).contains(&exp10) {
        return format!("{x:e}");
    }
    // SI group exponent (multiple of 3) within the supported prefix range.
    let group = (exp10.div_euclid(3) * 3).clamp(-15, 12);
    let suffix = match group {
        -15 => "f",
        -12 => "p",
        -9 => "n",
        -6 => "u",
        -3 => "m",
        0 => "",
        3 => "k",
        6 => "meg",
        9 => "g",
        _ => "t",
    };
    // Position of the decimal point within `digits`: exp10 - group + 1 digits before the point.
    let before = exp10 - group + 1; // may exceed digits.len() (pad zeros) when group clamped
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    let nd = digits.len() as i32;
    if before >= nd {
        out.push_str(&digits);
        for _ in 0..(before - nd) {
            out.push('0');
        }
    } else if before <= 0 {
        // Only when group is clamped low (value below femto): fall back to exponent form.
        return format!("{x:e}");
    } else {
        let b = before as usize;
        out.push_str(&digits[..b]);
        out.push('.');
        out.push_str(&digits[b..]);
    }
    out.push_str(suffix);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> f64 {
        parse_value(s).unwrap()
    }

    #[test]
    fn suffixes() {
        assert_eq!(p("4.7k"), 4700.0);
        assert_eq!(p("100n"), 1e-7);
        assert_eq!(p("1u"), 1e-6);
        assert_eq!(p("2.2µ"), 2.2e-6);
        assert_eq!(p("2.2μ"), 2.2e-6);
        assert_eq!(p("10meg"), 1e7);
        assert_eq!(p("10MEG"), 1e7);
        assert_eq!(p("5m"), 5e-3);
        assert_eq!(p("22p"), 22e-12);
        assert_eq!(p("1g"), 1e9);
        assert_eq!(p("1T"), 1e12);
        assert_eq!(p("3f"), 3e-15);
        assert_eq!(p("1e-9"), 1e-9);
        assert_eq!(p("-1.5E3"), -1500.0);
        assert_eq!(p("+.5"), 0.5);
        assert!((p("1mil") - 25.4e-6).abs() < 1e-18);
    }

    #[test]
    fn r_notation() {
        assert_eq!(p("4k7"), 4700.0);
        assert_eq!(p("2R2"), 2.2);
        assert_eq!(p("0R1"), 0.1);
        assert_eq!(p("1u5"), 1.5e-6);
        assert_eq!(p("1n5"), 1.5e-9);
        assert_eq!(p("2meg2"), 2.2e6);
    }

    #[test]
    fn units_ignored() {
        assert_eq!(p("10uF"), 1e-5);
        assert_eq!(p("5V"), 5.0);
        assert_eq!(p("1kohm"), 1000.0);
        assert_eq!(p("50Hz"), 50.0);
        assert_eq!(p("100nH"), 1e-7);
        assert_eq!(p("1megHz"), 1e6);
        // A lone F is femto, as in SPICE.
        assert_eq!(p("2F"), 2e-15);
    }

    #[test]
    fn garbage_is_rejected_with_a_message() {
        for bad in [
            "", "   ", "abc", "1.2.3", "--1", "1k2k", "k", ".", "1e", "1e+", "5x", "1kk", "0x10",
            "1,5",
        ] {
            let e = parse_value(bad);
            assert!(e.is_err(), "{bad:?} should fail, got {e:?}");
            assert!(!e.unwrap_err().to_string().is_empty());
        }
        assert!(matches!(
            parse_value("1e999"),
            Err(ValueError::OutOfRange(_))
        ));
        assert!(matches!(
            parse_value("1e99999999999"),
            Err(ValueError::OutOfRange(_))
        ));
        assert!(parse_value("nan").is_err());
        assert!(parse_value("inf").is_err());
        assert!(parse_value(&"9".repeat(500)).is_err());
    }

    #[test]
    fn formatting() {
        assert_eq!(format_value(4700.0), "4.7k");
        assert_eq!(format_value(1e-7), "100n");
        assert_eq!(format_value(1e6), "1meg");
        assert_eq!(format_value(0.0), "0");
        assert_eq!(format_value(-2.2e-6), "-2.2u");
        assert_eq!(format_value(1.0), "1");
        assert_eq!(format_value(0.5), "500m");
        assert_eq!(format_value(12345.0), "12.345k");
        assert_eq!(format_value(1e14), "100t");
        assert_eq!(format_value(1e15), "1e15");
        assert_eq!(format_value(22e-12), "22p");
    }

    #[test]
    fn round_trip_exact() {
        for x in [
            1.0,
            3.0,
            4700.0,
            1e-7,
            1e-9,
            0.1,
            0.2,
            0.3,
            2.2e-6,
            1.0 / 3.0,
            6.02214076e23,
            1e-30,
            1e300,
            -4.7e3,
            9.999e-4,
            123456789.123,
            f64::MIN_POSITIVE,
            f64::MAX,
            1e-15,
            1e-16,
        ] {
            let s = format_value(x);
            let y = parse_value(&s).unwrap_or_else(|e| panic!("{s}: {e}"));
            assert_eq!(x, y, "{s}");
        }
    }

    proptest::proptest! {
        #[test]
        fn round_trip_any_finite(bits in proptest::num::u64::ANY) {
            let x = f64::from_bits(bits);
            if x.is_finite() {
                let s = format_value(x);
                let y = parse_value(&s);
                // Values below the femto range use the plain exponent form, which also parses.
                proptest::prop_assert_eq!(y.ok(), Some(x));
            }
        }

        #[test]
        fn never_panics(s in "\\PC{0,80}") {
            let _ = parse_value(&s);
        }
    }
}
