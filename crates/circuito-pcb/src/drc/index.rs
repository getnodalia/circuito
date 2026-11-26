// SPDX-License-Identifier: MIT OR Apache-2.0
//! A uniform grid over bounding boxes for broad-phase pair finding.
//!
//! Build is `O(n + total cells covered)`. Reporting every pair of overlapping boxes costs
//! `O(sum over cells of k^2)` for `k` boxes per cell, which for boards with bounded local density
//! is linear in the number of boxes. Boxes covering more than [`BIG_CELLS`] cells are kept in a side
//! list and compared with everything (`O(big x n)`). Iteration order is deterministic.

use std::collections::BTreeMap;

use crate::geom::point::Rect;
use crate::units::Nm;

/// Boxes spanning more cells than this go to the side list.
pub const BIG_CELLS: i64 = 256;

#[derive(Copy, Clone)]
struct Range {
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
}

pub struct Grid {
    cell: i64,
    cells: BTreeMap<(i64, i64), Vec<u32>>,
    boxes: Vec<Rect>,
    ranges: Vec<Range>,
    big: Vec<u32>,
}

impl Grid {
    /// Builds a grid with square cells of `cell` nm (at least 1) over `boxes`.
    pub fn new(boxes: Vec<Rect>, cell: Nm) -> Grid {
        let cell = cell.max(1);
        let mut cells: BTreeMap<(i64, i64), Vec<u32>> = BTreeMap::new();
        let mut ranges = Vec::with_capacity(boxes.len());
        let mut big = Vec::new();
        for (i, b) in boxes.iter().enumerate() {
            let r = Range {
                x0: b.min.x.div_euclid(cell),
                y0: b.min.y.div_euclid(cell),
                x1: b.max.x.div_euclid(cell),
                y1: b.max.y.div_euclid(cell),
            };
            ranges.push(r);
            let n = (r.x1 - r.x0 + 1).saturating_mul(r.y1 - r.y0 + 1);
            if n > BIG_CELLS {
                big.push(i as u32);
                continue;
            }
            for x in r.x0..=r.x1 {
                for y in r.y0..=r.y1 {
                    cells.entry((x, y)).or_default().push(i as u32);
                }
            }
        }
        Grid {
            cell,
            cells,
            boxes,
            ranges,
            big,
        }
    }

    /// A good cell size for the boxes: about twice the mean extent, within `[min, 50 mm]`.
    pub fn choose_cell(boxes: &[Rect], min: Nm) -> Nm {
        if boxes.is_empty() {
            return min.max(1);
        }
        let sum: i128 = boxes
            .iter()
            .map(|b| ((b.max.x - b.min.x).max(b.max.y - b.min.y)) as i128)
            .sum();
        let mean = (sum / boxes.len() as i128) as i64;
        (mean.saturating_mul(2)).clamp(min.max(1), 50_000_000)
    }

    pub fn len(&self) -> usize {
        self.boxes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.boxes.is_empty()
    }

    pub fn rect(&self, i: usize) -> Rect {
        self.boxes[i.min(self.boxes.len().saturating_sub(1))]
    }

    /// Calls `f(i, j)` with `i < j` once for every pair of boxes that intersect.
    pub fn for_each_pair(&self, mut f: impl FnMut(usize, usize)) {
        for (&(cx, cy), list) in &self.cells {
            for (k, &a) in list.iter().enumerate() {
                let ra = self.ranges[a as usize];
                for &b in &list[k + 1..] {
                    let rb = self.ranges[b as usize];
                    // report in the lowest cell shared by both ranges only
                    if ra.x0.max(rb.x0) != cx || ra.y0.max(rb.y0) != cy {
                        continue;
                    }
                    if self.boxes[a as usize].intersects(self.boxes[b as usize]) {
                        f(a.min(b) as usize, a.max(b) as usize);
                    }
                }
            }
        }
        if self.big.is_empty() {
            return;
        }
        let mut big_pos: Vec<Option<usize>> = vec![None; self.boxes.len()];
        for (k, &b) in self.big.iter().enumerate() {
            big_pos[b as usize] = Some(k);
        }
        for (k, &b) in self.big.iter().enumerate() {
            let bb = self.boxes[b as usize];
            for (i, (other, pos)) in self.boxes.iter().zip(&big_pos).enumerate() {
                if i == b as usize {
                    continue;
                }
                // a pair of big boxes is reported once, from the earlier one
                if matches!(pos, Some(p) if *p < k) {
                    continue;
                }
                if bb.intersects(*other) {
                    f((b as usize).min(i), (b as usize).max(i));
                }
            }
        }
    }

    /// Calls `f(i)` once for every box intersecting `q`.
    pub fn query(&self, q: Rect, mut f: impl FnMut(usize)) {
        let qr = Range {
            x0: q.min.x.div_euclid(self.cell),
            y0: q.min.y.div_euclid(self.cell),
            x1: q.max.x.div_euclid(self.cell),
            y1: q.max.y.div_euclid(self.cell),
        };
        let ncells = (qr.x1 - qr.x0 + 1).saturating_mul(qr.y1 - qr.y0 + 1);
        if ncells > 4 * BIG_CELLS {
            // very large query: scan everything
            for (i, b) in self.boxes.iter().enumerate() {
                if b.intersects(q) {
                    f(i);
                }
            }
            return;
        }
        for x in qr.x0..=qr.x1 {
            for y in qr.y0..=qr.y1 {
                if let Some(list) = self.cells.get(&(x, y)) {
                    for &i in list {
                        let r = self.ranges[i as usize];
                        // deliver in the lowest shared cell only
                        if r.x0.max(qr.x0) == x
                            && r.y0.max(qr.y0) == y
                            && self.boxes[i as usize].intersects(q)
                        {
                            f(i as usize);
                        }
                    }
                }
            }
        }
        for &i in &self.big {
            if self.boxes[i as usize].intersects(q) {
                f(i as usize);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::point::Point;

    fn r(x0: i64, y0: i64, x1: i64, y1: i64) -> Rect {
        Rect {
            min: Point::new(x0, y0),
            max: Point::new(x1, y1),
        }
    }

    #[test]
    fn pairs_match_brute_force() {
        // deterministic pseudo-random boxes, including large ones
        let mut s: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut rnd = |m: i64| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s % m as u64) as i64
        };
        let mut boxes = Vec::new();
        for i in 0..400 {
            let x = rnd(20_000) - 10_000;
            let y = rnd(20_000) - 10_000;
            let (w, h) = if i % 50 == 0 {
                (rnd(15_000), rnd(15_000))
            } else {
                (rnd(600), rnd(600))
            };
            boxes.push(r(x, y, x + w, y + h));
        }
        for cell in [100, 700, 5_000, 1] {
            let g = Grid::new(boxes.clone(), cell);
            let mut got = Vec::new();
            g.for_each_pair(|a, b| got.push((a, b)));
            got.sort_unstable();
            let n = got.len();
            got.dedup();
            assert_eq!(n, got.len(), "duplicate pair at cell {cell}");
            let mut want = Vec::new();
            for i in 0..boxes.len() {
                for j in i + 1..boxes.len() {
                    if boxes[i].intersects(boxes[j]) {
                        want.push((i, j));
                    }
                }
            }
            assert_eq!(got, want, "cell {cell}");
            let q = r(-500, -500, 3_000, 2_000);
            let mut hits = Vec::new();
            g.query(q, |i| hits.push(i));
            hits.sort_unstable();
            let m = hits.len();
            hits.dedup();
            assert_eq!(m, hits.len());
            let want: Vec<usize> = (0..boxes.len())
                .filter(|&i| boxes[i].intersects(q))
                .collect();
            assert_eq!(hits, want);
        }
    }
}
