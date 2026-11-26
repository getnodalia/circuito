// SPDX-License-Identifier: MIT OR Apache-2.0
//! 256-bit products and exact rational squared distances.
//!
//! A squared distance between integer shapes is a ratio `num/den` of `u128` values (for example the
//! squared distance from a point to a segment interior is `cross^2 / len^2`). Comparing two such
//! ratios, or a ratio with an integer threshold, needs products up to 256 bits; they are computed
//! exactly here.

use std::cmp::Ordering;

/// Full 128 x 128 -> 256 bit product, returned as `(high, low)`.
pub fn mul_wide(a: u128, b: u128) -> (u128, u128) {
    let mask: u128 = (1u128 << 64) - 1;
    let (a0, a1) = (a & mask, a >> 64);
    let (b0, b1) = (b & mask, b >> 64);
    let p00 = a0 * b0;
    let p01 = a0 * b1;
    let p10 = a1 * b0;
    let p11 = a1 * b1;
    let mid = (p00 >> 64) + (p01 & mask) + (p10 & mask);
    let lo = (p00 & mask) | ((mid & mask) << 64);
    let hi = p11 + (p01 >> 64) + (p10 >> 64) + (mid >> 64);
    (hi, lo)
}

/// Compares `a*b` with `c*d` exactly.
pub fn cmp_products(a: u128, b: u128, c: u128, d: u128) -> Ordering {
    mul_wide(a, b).cmp(&mul_wide(c, d))
}

/// A non-negative rational `num/den` with `den > 0`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Ratio {
    pub num: u128,
    pub den: u128,
}

impl Ratio {
    pub const ZERO: Ratio = Ratio { num: 0, den: 1 };

    pub fn int(n: u128) -> Ratio {
        Ratio { num: n, den: 1 }
    }

    pub fn is_zero(self) -> bool {
        self.num == 0
    }

    /// Exact comparison.
    pub fn cmp_ratio(self, o: Ratio) -> Ordering {
        cmp_products(self.num, o.den, o.num, self.den)
    }

    /// The smaller of two ratios.
    pub fn min(self, o: Ratio) -> Ratio {
        if self.cmp_ratio(o) == Ordering::Greater {
            o
        } else {
            self
        }
    }

    /// Compares `4 * self` (that is, the square of twice the distance) with the integer `t2`.
    /// Used to compare a distance `d` with a threshold `t2 / 2`: `d < t2/2` iff `4 d^2 < t2^2`.
    pub fn cmp_double_dist_sq(self, t2: u128) -> Ordering {
        // 4*num/den  vs  t2*t2   <=>   4*num  vs  t2*t2*den
        let tt = t2.saturating_mul(t2);
        mul_wide(self.num, 4).cmp(&mul_wide(tt, self.den))
    }

    /// `floor(sqrt(4 * self))`, that is `floor(2 d)` for a distance `d`.
    pub fn floor_double_dist(self) -> u128 {
        let q = self.num / self.den;
        let r = self.num % self.den;
        // 4*self = 4q + 4r/den, with 4r < 4*den <= 2^66 for the denominators used here.
        let frac = r.saturating_mul(4) / self.den;
        crate::units::isqrt(q.saturating_mul(4).saturating_add(frac))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_product_matches_known_values() {
        assert_eq!(mul_wide(0, 12345), (0, 0));
        assert_eq!(mul_wide(u128::MAX, 1), (0, u128::MAX));
        // (2^128-1)^2 = 2^256 - 2^129 + 1
        assert_eq!(mul_wide(u128::MAX, u128::MAX), (u128::MAX - 1, 1));
        assert_eq!(mul_wide(1 << 64, 1 << 64), (1, 0));
        assert_eq!(
            mul_wide((1 << 100) + 7, (1 << 90) + 3),
            (1 << 62, (7u128 << 90) + (3u128 << 100) + 21)
        );
    }

    #[test]
    fn ratio_ordering() {
        let a = Ratio { num: 1, den: 3 };
        let b = Ratio { num: 2, den: 6 };
        let c = Ratio { num: 1, den: 2 };
        assert_eq!(a.cmp_ratio(b), Ordering::Equal);
        assert_eq!(a.cmp_ratio(c), Ordering::Less);
        // d^2 = 25/4 -> d = 2.5 -> 2d = 5
        let r = Ratio { num: 25, den: 4 };
        assert_eq!(r.cmp_double_dist_sq(5), Ordering::Equal);
        assert_eq!(r.cmp_double_dist_sq(6), Ordering::Less);
        assert_eq!(r.cmp_double_dist_sq(4), Ordering::Greater);
        assert_eq!(r.floor_double_dist(), 5);
        assert_eq!(Ratio { num: 2, den: 1 }.floor_double_dist(), 2); // 2*sqrt(2)=2.83
    }
}
