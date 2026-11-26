// SPDX-License-Identifier: MIT OR Apache-2.0
//! Board graphics and text as drawing primitives in board coordinates.
//!
//! Footprint graphics and texts are stored for a front-side footprint; a footprint on the back is
//! mirrored (`x -> -x`) and its elements move to the opposite layer, as in `Placement`.
//! Footprint text may use `${REF}` and `${VALUE}`.

use crate::board::{Board, Footprint, Graphic, GraphicKind, TextItem};
use crate::drc::violation::{ObjKind, ObjRef};
use crate::error::Result;
use crate::font::render_text;
use crate::geom::arc::Arc;
use crate::geom::point::Point;
use crate::geom::region::Region;
use crate::geom::shape::Shape;
use crate::layer::{Layer, Side};
use crate::transform::Placement;
use crate::units::Nm;

/// Geometry of one primitive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArtGeom {
    /// A stroked line.
    Line { a: Point, b: Point },
    /// A stroked arc (a full circle when start equals end).
    Arc(Arc),
    /// A stroked circle outline.
    Circle { center: Point, radius: Nm },
    /// A filled disc of the given diameter.
    Disc { center: Point, diameter: Nm },
    /// A filled polygon (its outline is also stroked when `width > 0`).
    Polygon(Region),
    /// A stroked polygon outline.
    PolyOutline(Region),
}

/// One primitive on one layer, stroked with a round pen of `width` nm.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtPrim {
    pub layer: Layer,
    pub geom: ArtGeom,
    pub width: Nm,
    pub owner: ObjRef,
}

/// All non-pad, non-track artwork of a board.
#[derive(Clone, Debug, Default)]
pub struct Artwork {
    pub prims: Vec<ArtPrim>,
    /// Characters the stroke font could not draw.
    pub unsupported_chars: usize,
}

fn expand_text(t: &TextItem, fp: Option<&Footprint>) -> String {
    match fp {
        Some(f) => t
            .text
            .replace("${REF}", &f.reference)
            .replace("${VALUE}", &f.value),
        None => t.text.clone(),
    }
}

fn graphic_prim(g: &Graphic, fp: Option<&Footprint>) -> Result<ArtPrim> {
    let pl = fp.map(Placement::of);
    let map = |p: Point| -> Result<Point> {
        match &pl {
            Some(pl) => pl.apply(p),
            None => Ok(p),
        }
    };
    let layer = match fp {
        Some(f) if f.side == Side::Back => g.layer.flipped(),
        _ => g.layer,
    };
    let geom = match &g.kind {
        GraphicKind::Line { a, b } => ArtGeom::Line {
            a: map(*a)?,
            b: map(*b)?,
        },
        GraphicKind::Circle { center, radius } => {
            let c = map(*center)?;
            if g.filled {
                ArtGeom::Disc {
                    center: c,
                    diameter: radius.saturating_mul(2).saturating_add(g.width),
                }
            } else {
                ArtGeom::Circle {
                    center: c,
                    radius: *radius,
                }
            }
        }
        GraphicKind::Arc(a) => ArtGeom::Arc(*a),
        GraphicKind::Polygon(r) => {
            let mapped = Region::new(
                r.outer
                    .iter()
                    .map(|p| map(*p))
                    .collect::<Result<Vec<_>>>()?,
                r.holes
                    .iter()
                    .map(|h| h.iter().map(|p| map(*p)).collect::<Result<Vec<_>>>())
                    .collect::<Result<Vec<_>>>()?,
            )?;
            if g.filled {
                ArtGeom::Polygon(mapped)
            } else {
                ArtGeom::PolyOutline(mapped)
            }
        }
    };
    Ok(ArtPrim {
        layer,
        geom,
        width: g.width,
        owner: ObjRef {
            kind: ObjKind::Graphic,
            id: g.id,
        },
    })
}

fn text_prims(t: &TextItem, fp: Option<&Footprint>, out: &mut Artwork) -> Result<()> {
    let text = expand_text(t, fp);
    let st = render_text(
        &text, t.position, t.height, t.rotation, t.mirrored, t.justify,
    )?;
    out.unsupported_chars += st.unsupported;
    let pl = fp.map(Placement::of);
    let layer = match fp {
        Some(f) if f.side == Side::Back => t.layer.flipped(),
        _ => t.layer,
    };
    for (a, b) in st.strokes {
        let (a, b) = match &pl {
            Some(pl) => (pl.apply(a)?, pl.apply(b)?),
            None => (a, b),
        };
        out.prims.push(ArtPrim {
            layer,
            geom: ArtGeom::Line { a, b },
            width: t.thickness,
            owner: ObjRef {
                kind: ObjKind::Text,
                id: t.id,
            },
        });
    }
    Ok(())
}

/// Collects the graphics and texts of the board and of every footprint.
pub fn collect_artwork(board: &Board) -> Result<Artwork> {
    let mut out = Artwork::default();
    for g in &board.graphics {
        out.prims.push(graphic_prim(g, None)?);
    }
    for t in &board.texts {
        text_prims(t, None, &mut out)?;
    }
    for fp in &board.footprints {
        for g in &fp.graphics {
            out.prims.push(graphic_prim(g, Some(fp))?);
        }
        for t in &fp.texts {
            text_prims(t, Some(fp), &mut out)?;
        }
    }
    Ok(out)
}

impl ArtPrim {
    /// Clearance shapes for this primitive (arcs and circles approximated within `tolerance` nm).
    pub fn shapes(&self, tolerance: Nm) -> Result<Vec<Shape>> {
        let w = self.width;
        Ok(match &self.geom {
            ArtGeom::Line { a, b } => vec![Shape::segment(*a, *b, w)],
            ArtGeom::Arc(a) => polyline_shapes(&a.approximate(tolerance)?, w),
            ArtGeom::Circle { center, radius } => {
                let c = Arc {
                    start: Point::new(center.x.saturating_add(*radius), center.y),
                    end: Point::new(center.x.saturating_add(*radius), center.y),
                    center: *center,
                    ccw: true,
                };
                polyline_shapes(&c.approximate(tolerance)?, w)
            }
            ArtGeom::Disc { center, diameter } => vec![Shape::point(*center, *diameter)],
            ArtGeom::Polygon(r) => vec![Shape::region(r.clone(), w)],
            ArtGeom::PolyOutline(r) => r.edges().map(|(a, b)| Shape::segment(a, b, w)).collect(),
        })
    }

    /// A representative point.
    pub fn at(&self) -> Point {
        match &self.geom {
            ArtGeom::Line { a, b } => Point::new(
                ((a.x as i128 + b.x as i128) / 2) as i64,
                ((a.y as i128 + b.y as i128) / 2) as i64,
            ),
            ArtGeom::Arc(a) => a.start,
            ArtGeom::Circle { center, .. } | ArtGeom::Disc { center, .. } => *center,
            ArtGeom::Polygon(r) | ArtGeom::PolyOutline(r) => {
                r.outer.first().copied().unwrap_or(Point::ORIGIN)
            }
        }
    }
}

/// Segments of a polyline as shapes of the given diameter.
pub fn polyline_shapes(pts: &[Point], diam: Nm) -> Vec<Shape> {
    pts.windows(2)
        .map(|w| Shape::segment(w[0], w[1], diam))
        .collect()
}

/// Reference designator of a footprint for messages.
pub fn fp_label(fp: &Footprint) -> String {
    if fp.reference.is_empty() {
        format!("footprint {}", fp.id.0)
    } else {
        fp.reference.clone()
    }
}
