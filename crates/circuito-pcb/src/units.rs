// SPDX-License-Identifier: MIT OR Apache-2.0
//! Units and limits. All lengths are integer nanometres (`i64`); the Y axis points up, as in Gerber.
//!
//! Coordinates are limited to `±MAX_COORD` (2^30 nm, about 1.07 m) and sizes (widths, diameters,
//! clearances) to `0..=MAX_DIM`. With these limits every exact predicate in [`crate::geom`] fits in
//! `i128`/`u128` plus the 256-bit comparison in `geom::wide`, so no coordinate arithmetic wraps.

use crate::error::{PcbError, Result};

/// Nanometres.
pub type Nm = i64;

/// One millimetre in nm.
pub const MM: Nm = 1_000_000;
/// One mil (thousandth of an inch) in nm: exactly 25.4 µm.
pub const MIL: Nm = 25_400;
/// Largest absolute coordinate accepted (2^30 nm, ~1.07 m).
pub const MAX_COORD: Nm = 1 << 30;
/// Largest size (width, diameter, clearance, margin) accepted.
pub const MAX_DIM: Nm = 1 << 30;

/// Checks a coordinate is within `±MAX_COORD`.
pub fn check_coord(v: Nm, what: &str) -> Result<Nm> {
    if (-MAX_COORD..=MAX_COORD).contains(&v) {
        Ok(v)
    } else {
        Err(PcbError::range(
            what,
            format!("{v} nm is outside ±{MAX_COORD} nm"),
        ))
    }
}

/// Checks a size is within `0..=MAX_DIM`.
pub fn check_dim(v: Nm, what: &str) -> Result<Nm> {
    if (0..=MAX_DIM).contains(&v) {
        Ok(v)
    } else {
        Err(PcbError::range(
            what,
            format!("{v} nm is outside 0..={MAX_DIM} nm"),
        ))
    }
}

/// Checks a strictly positive size.
pub fn check_positive(v: Nm, what: &str) -> Result<Nm> {
    if (1..=MAX_DIM).contains(&v) {
        Ok(v)
    } else {
        Err(PcbError::range(
            what,
            format!("{v} nm is outside 1..={MAX_DIM} nm"),
        ))
    }
}

/// Formats nanometres as millimetres with exactly six decimals: `1500000` becomes `1.500000`.
/// The conversion is exact because 1 nm is 1e-6 mm.
pub fn format_mm(nm: Nm) -> String {
    let neg = nm < 0;
    let a = nm.unsigned_abs();
    let s = format!("{}.{:06}", a / 1_000_000, a % 1_000_000);
    if neg {
        format!("-{s}")
    } else {
        s
    }
}

/// Formats millidegrees as degrees with three decimals (`90000` becomes `90.000`).
pub fn format_deg(mdeg: i64) -> String {
    let neg = mdeg < 0;
    let a = mdeg.unsigned_abs();
    let s = format!("{}.{:03}", a / 1000, a % 1000);
    if neg {
        format!("-{s}")
    } else {
        s
    }
}

/// Parses a decimal millimetre string such as `-12.345678` into nm, exactly. At most six decimals.
pub fn parse_mm(s: &str) -> Result<Nm> {
    let bad = || PcbError::Parse {
        what: "millimetre value",
        detail: format!("`{s}` is not a decimal number with at most 6 decimals"),
    };
    let t = s.trim();
    let (neg, body) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let (ip, fp) = match body.split_once('.') {
        Some((a, b)) => (a, b),
        None => (body, ""),
    };
    if (ip.is_empty() && fp.is_empty()) || fp.len() > 6 {
        return Err(bad());
    }
    if !ip.bytes().all(|c| c.is_ascii_digit()) || !fp.bytes().all(|c| c.is_ascii_digit()) {
        return Err(bad());
    }
    if ip.len() > 12 {
        return Err(bad());
    }
    let ipv: i64 = if ip.is_empty() {
        0
    } else {
        ip.parse().map_err(|_| bad())?
    };
    let mut fpv: i64 = if fp.is_empty() {
        0
    } else {
        fp.parse().map_err(|_| bad())?
    };
    for _ in fp.len()..6 {
        fpv *= 10;
    }
    let v = ipv
        .checked_mul(1_000_000)
        .and_then(|v| v.checked_add(fpv))
        .ok_or_else(bad)?;
    Ok(if neg { -v } else { v })
}

/// Integer square root (floor) of a `u128`.
pub fn isqrt(n: u128) -> u128 {
    if n < 2 {
        return n;
    }
    // Start from a power of two above the root and descend bit by bit.
    let mut bit: u128 = 1u128 << ((128 - n.leading_zeros()).div_ceil(2));
    let mut res: u128 = 0;
    while bit != 0 {
        let cand = res | bit;
        if let Some(sq) = cand.checked_mul(cand) {
            if sq <= n {
                res = cand;
            }
        }
        bit >>= 1;
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mm_round_trip() {
        for v in [
            0,
            1,
            -1,
            999_999,
            1_000_000,
            -1_500_000,
            123_456_789,
            MAX_COORD,
            -MAX_COORD,
        ] {
            assert_eq!(parse_mm(&format_mm(v)).unwrap(), v);
        }
        assert_eq!(format_mm(1_500_000), "1.500000");
        assert_eq!(format_mm(-5), "-0.000005");
    }

    #[test]
    fn parse_mm_rejects_garbage() {
        for s in [
            "",
            ".",
            "1.2345678",
            "abc",
            "1e3",
            "--1",
            "1.-2",
            "99999999999999999999",
        ] {
            assert!(parse_mm(s).is_err(), "{s}");
        }
        assert_eq!(parse_mm("2").unwrap(), 2_000_000);
        assert_eq!(parse_mm(".5").unwrap(), 500_000);
    }

    #[test]
    fn isqrt_exact() {
        for n in [
            0u128,
            1,
            2,
            3,
            4,
            15,
            16,
            17,
            1 << 100,
            (1 << 100) - 1,
            u128::MAX,
        ] {
            let r = isqrt(n);
            assert!(r.checked_mul(r).is_none_or(|s| s <= n));
            let r1 = r + 1;
            assert!(r1.checked_mul(r1).is_none_or(|s| s > n));
        }
    }
}
