// SPDX-License-Identifier: MIT OR Apache-2.0
//! Copper connectivity: which pads of a net are joined by copper, and which are not.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use super::items::CopperKind;
use super::violation::{ObjKind, ObjRef, ViolationCode};
use super::{items_cmp, violation, Ctx, Out};
use crate::board::NetId;
use crate::geom::Point;

struct Dsu {
    parent: Vec<u32>,
}

impl Dsu {
    fn new(n: usize) -> Dsu {
        Dsu {
            parent: (0..n as u32).collect(),
        }
    }
    fn find(&mut self, mut x: u32) -> u32 {
        while self.parent[x as usize] != x {
            let p = self.parent[x as usize];
            self.parent[x as usize] = self.parent[p as usize];
            x = self.parent[x as usize];
        }
        x
    }
    fn union(&mut self, a: u32, b: u32) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent[ra.max(rb) as usize] = ra.min(rb);
        }
    }
}

/// Unions the items of every net that touch on a common layer and returns the union-find over the
/// item indices. Pads of one terminal (same footprint, same number) are always joined.
pub(crate) fn connect(ctx: &Ctx) -> Vec<u32> {
    let n = ctx.items.len();
    let mut dsu = Dsu::new(n);
    let mut first_of_group: BTreeMap<u32, u32> = BTreeMap::new();
    for (i, it) in ctx.items.iter().enumerate() {
        if let Some(g) = it.group {
            match first_of_group.get(&g) {
                Some(&f) => dsu.union(f, i as u32),
                None => {
                    first_of_group.insert(g, i as u32);
                }
            }
        }
    }
    ctx.grid.for_each_pair(|i, j| {
        let (a, b) = (&ctx.items[i], &ctx.items[j]);
        if a.mask & b.mask == 0 || a.net.is_none() || a.net != b.net {
            return;
        }
        // a zone set to "no connection" does not join pads
        if (a.thermal_none && b.kind == CopperKind::Pad)
            || (b.thermal_none && a.kind == CopperKind::Pad)
        {
            return;
        }
        if items_cmp(a, b, 0) != Ordering::Greater {
            dsu.union(i as u32, j as u32);
        }
    });
    (0..n as u32).map(|i| dsu.find(i)).collect()
}

pub(crate) fn check_unconnected(ctx: &Ctx, out: &mut Out) {
    if !ctx.rules.enabled(ViolationCode::Unconnected) {
        return;
    }
    let root = connect(ctx);
    // net -> component root -> pad item indices
    let mut nets: BTreeMap<NetId, BTreeMap<u32, Vec<usize>>> = BTreeMap::new();
    for (i, it) in ctx.items.iter().enumerate() {
        if it.kind != CopperKind::Pad {
            continue;
        }
        if let Some(n) = it.net {
            nets.entry(n)
                .or_default()
                .entry(root[i])
                .or_default()
                .push(i);
        }
    }
    for (net, comps) in nets {
        if comps.len() < 2 {
            continue;
        }
        let mut list: Vec<Vec<usize>> = comps.into_values().collect();
        // largest island first, then the one containing the lowest pad id
        list.sort_by(|a, b| {
            b.len()
                .cmp(&a.len())
                .then_with(|| min_id(ctx, a).cmp(&min_id(ctx, b)))
        });
        let Some(main) = list.first().cloned() else {
            continue;
        };
        let Some(&anchor) = main.iter().min_by_key(|&&i| ctx.items[i].obj.id) else {
            continue;
        };
        let net_name = ctx.board.net_name(Some(net));
        for island in list.iter().skip(1) {
            let Some(&p) = island.iter().min_by_key(|&&i| ctx.items[i].obj.id) else {
                continue;
            };
            let (pa, pb) = (&ctx.items[anchor], &ctx.items[p]);
            let extra = if island.len() > 1 {
                format!(" (and {} more pad(s) joined to it)", island.len() - 1)
            } else {
                String::new()
            };
            let locations: Vec<Point> = vec![pa.at, pb.at];
            out.push(violation(
                ViolationCode::Unconnected,
                None,
                locations,
                vec![
                    ObjRef {
                        kind: ObjKind::Pad,
                        id: pa.obj.id,
                    },
                    ObjRef {
                        kind: ObjKind::Pad,
                        id: pb.obj.id,
                    },
                ],
                Some(net_name.clone()),
                None,
                None,
                format!(
                    "net `{net_name}`: {} is not connected to {}{extra}",
                    pb.label, pa.label
                ),
            ));
        }
    }
}

fn min_id(ctx: &Ctx, island: &[usize]) -> Option<crate::board::Id> {
    island.iter().map(|&i| ctx.items[i].obj.id).min()
}
