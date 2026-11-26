// SPDX-License-Identifier: MIT OR Apache-2.0
//! Turns a board into Gerber files, one per layer.

use super::writer::{ApKey, Header, Image, ObjAttrs};
use super::{GerberFile, GerberOptions, GerberSet};
use crate::artwork::{collect_artwork, ArtGeom, ArtPrim};
use crate::board::{Board, Footprint, Pad, PadKind, PadShape};
use crate::error::{PcbError, Result};
use crate::geom::arc::Arc;
use crate::geom::point::Point;
use crate::geom::region::Region;
use crate::geom::shape::Core;
use crate::layer::{Layer, Side};
use crate::transform::{pad_center, pad_copper_layers, pad_shape};
use crate::units::{check_coord, format_deg, format_mm, Nm};

/// The Gerber layers of a board, in the order `write_all` returns them.
pub fn layer_list(board: &Board) -> Vec<Layer> {
    let mut v = board.copper_stack();
    v.extend([
        Layer::FMask,
        Layer::BMask,
        Layer::FPaste,
        Layer::BPaste,
        Layer::FSilk,
        Layer::BSilk,
        Layer::EdgeCuts,
    ]);
    v
}

fn sanitize(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        "board".into()
    } else {
        s
    }
}

fn function_of(board: &Board, layer: Layer) -> Result<(String, &'static str)> {
    let n = board.copper_layers as usize;
    Ok(match layer {
        Layer::FCu | Layer::BCu | Layer::InnerCu(_) => {
            let idx = layer.copper_index(board.copper_layers).ok_or_else(|| {
                PcbError::Invalid(format!("the board has no layer {}", layer.name()))
            })?;
            let pos = if idx == 0 {
                "Top"
            } else if idx + 1 == n {
                "Bot"
            } else {
                "Inr"
            };
            (format!("Copper,L{},{pos}", idx + 1), "Positive")
        }
        Layer::FMask => ("Soldermask,Top".into(), "Negative"),
        Layer::BMask => ("Soldermask,Bot".into(), "Negative"),
        Layer::FPaste => ("Paste,Top".into(), "Positive"),
        Layer::BPaste => ("Paste,Bot".into(), "Positive"),
        Layer::FSilk => ("Legend,Top".into(), "Positive"),
        Layer::BSilk => ("Legend,Bot".into(), "Positive"),
        Layer::EdgeCuts => ("Profile,NP".into(), "Positive"),
        other => {
            return Err(PcbError::Unsupported(format!(
            "Gerber output of layer {} (copper, mask, paste, silkscreen and profile are written)",
            other.name()
        )))
        }
    })
}

/// Writes every Gerber layer of the board. The profile is skipped, with a warning, when the board
/// has no outline.
pub fn write_all(board: &Board, opts: &GerberOptions) -> Result<GerberSet> {
    board.validate()?;
    let mut set = GerberSet::default();
    for l in layer_list(board) {
        if l == Layer::EdgeCuts && board.outline.is_none() && !has_edge_graphics(board) {
            set.warnings
                .push("the board has no outline, so no profile file was written".into());
            continue;
        }
        let (f, w) = write_layer(board, l, opts)?;
        set.files.push(f);
        set.warnings.extend(w);
    }
    set.warnings.sort();
    set.warnings.dedup();
    Ok(set)
}

fn has_edge_graphics(board: &Board) -> bool {
    board.graphics.iter().any(|g| g.layer == Layer::EdgeCuts)
        || board
            .footprints
            .iter()
            .any(|f| f.graphics.iter().any(|g| g.layer == Layer::EdgeCuts))
}

/// Writes one Gerber layer. Returns the file and any warnings about what could not be drawn.
pub fn write_layer(
    board: &Board,
    layer: Layer,
    opts: &GerberOptions,
) -> Result<(GerberFile, Vec<String>)> {
    board.validate()?;
    opts.validate()?;
    let (function, polarity) = function_of(board, layer)?;
    let mut cx = Cx {
        board,
        opts,
        img: Image::new(),
        warnings: Vec::new(),
    };
    match layer {
        Layer::FCu | Layer::BCu | Layer::InnerCu(_) => cx.copper(layer)?,
        Layer::FMask => cx.mask(Side::Front)?,
        Layer::BMask => cx.mask(Side::Back)?,
        Layer::FPaste => cx.paste(Side::Front)?,
        Layer::BPaste => cx.paste(Side::Back)?,
        Layer::FSilk | Layer::BSilk => cx.silk(layer)?,
        Layer::EdgeCuts => cx.profile()?,
        _ => {}
    }
    let hdr = Header {
        file_function: function.clone(),
        polarity,
        creation_date: opts.creation_date.clone(),
    };
    let content = cx.img.render(&hdr)?;
    let file = GerberFile {
        file_name: format!(
            "{}-{}.gbr",
            sanitize(&board.name),
            layer.name().replace('.', "_")
        ),
        function,
        polarity: polarity.to_string(),
        content,
    };
    Ok((file, cx.warnings))
}

struct Cx<'a> {
    board: &'a Board,
    opts: &'a GerberOptions,
    img: Image,
    warnings: Vec<String>,
}

/// How a pad is drawn.
pub(crate) enum PadArt {
    Flash {
        def: String,
        at: Point,
    },
    /// A stadium drawn as one stroke of a round aperture (an oval at an angle).
    Stroke {
        a: Point,
        b: Point,
        width: Nm,
    },
    /// A polygon, grown by `stroke` nm all round when `stroke > 0`.
    Poly {
        region: Region,
        stroke: Nm,
    },
}

fn mm(v: Nm) -> String {
    format_mm(v)
}

/// The shape to draw for a pad grown by `grow` on every side. `None` when nothing is left.
pub(crate) fn pad_art(fp: &Footprint, pad: &Pad, grow: Nm) -> Result<Option<PadArt>> {
    let center = pad_center(fp, pad)?;
    let g2 = grow
        .checked_mul(2)
        .ok_or(PcbError::Overflow("pad margin"))?;
    let mirrored = fp.side == Side::Back;
    let eff = ((fp.rotation as i64)
        + if mirrored {
            -(pad.rotation as i64)
        } else {
            pad.rotation as i64
        })
    .rem_euclid(360_000);
    let orth = eff % 90_000 == 0;
    let quarter = (eff / 90_000) % 2 == 1;
    let oriented = |w: Nm, h: Nm| if quarter { (h, w) } else { (w, h) };
    let oval = |w: Nm, h: Nm| -> Result<Option<PadArt>> {
        if w == h {
            return Ok(Some(PadArt::Flash {
                def: format!("C,{}", mm(w)),
                at: center,
            }));
        }
        if orth {
            let (w, h) = oriented(w, h);
            return Ok(Some(PadArt::Flash {
                def: format!("O,{}X{}", mm(w), mm(h)),
                at: center,
            }));
        }
        let Some(shape) = pad_shape(fp, pad, grow)? else {
            return Ok(None);
        };
        Ok(Some(match shape.core {
            Core::Segment(a, b) => PadArt::Stroke {
                a,
                b,
                width: shape.diam,
            },
            Core::Point(c) => PadArt::Flash {
                def: format!("C,{}", mm(shape.diam)),
                at: c,
            },
            Core::Region(_) => return Ok(None),
        }))
    };
    match &pad.shape {
        PadShape::Circle => {
            let d = pad
                .size
                .w
                .checked_add(g2)
                .ok_or(PcbError::Overflow("pad size"))?;
            if d <= 0 {
                return Ok(None);
            }
            Ok(Some(PadArt::Flash {
                def: format!("C,{}", mm(d)),
                at: center,
            }))
        }
        PadShape::Rect => {
            let w = pad
                .size
                .w
                .checked_add(g2)
                .ok_or(PcbError::Overflow("pad size"))?;
            let h = pad
                .size
                .h
                .checked_add(g2)
                .ok_or(PcbError::Overflow("pad size"))?;
            if w <= 0 || h <= 0 {
                return Ok(None);
            }
            let def = if orth {
                let (w, h) = oriented(w, h);
                format!("R,{}X{}", mm(w), mm(h))
            } else {
                format!("RECTROT,{}X{}X{}", mm(w), mm(h), format_deg(eff))
            };
            Ok(Some(PadArt::Flash { def, at: center }))
        }
        PadShape::Oval => {
            let w = pad
                .size
                .w
                .checked_add(g2)
                .ok_or(PcbError::Overflow("pad size"))?;
            let h = pad
                .size
                .h
                .checked_add(g2)
                .ok_or(PcbError::Overflow("pad size"))?;
            if w <= 0 || h <= 0 {
                return Ok(None);
            }
            oval(w, h)
        }
        PadShape::RoundRect { corner_radius } => {
            let w = pad
                .size
                .w
                .checked_add(g2)
                .ok_or(PcbError::Overflow("pad size"))?;
            let h = pad
                .size
                .h
                .checked_add(g2)
                .ok_or(PcbError::Overflow("pad size"))?;
            let r = corner_radius
                .checked_add(grow)
                .ok_or(PcbError::Overflow("pad corner radius"))?;
            if w <= 0 || h <= 0 || r < 0 {
                return Ok(None);
            }
            let r = r.min(w.min(h) / 2);
            if r == 0 {
                let def = if orth {
                    let (w, h) = oriented(w, h);
                    format!("R,{}X{}", mm(w), mm(h))
                } else {
                    format!("RECTROT,{}X{}X{}", mm(w), mm(h), format_deg(eff))
                };
                return Ok(Some(PadArt::Flash { def, at: center }));
            }
            if 2 * r >= w.min(h) {
                return oval(w, h);
            }
            Ok(Some(PadArt::Flash {
                def: format!("RRECT,{}X{}X{}X{}", mm(w), mm(h), mm(r), format_deg(eff)),
                at: center,
            }))
        }
        PadShape::Custom { .. } => {
            let Some(shape) = pad_shape(fp, pad, grow)? else {
                return Ok(None);
            };
            match shape.core {
                Core::Region(region) => Ok(Some(PadArt::Poly {
                    region,
                    stroke: shape.diam,
                })),
                _ => Ok(None),
            }
        }
    }
}

fn ring_lines(img: &mut Image, ring: &[Point], ap: &ApKey) {
    for (i, a) in ring.iter().enumerate() {
        let b = ring.get((i + 1) % ring.len()).copied().unwrap_or(*a);
        img.line(ap.clone(), *a, b, ObjAttrs::default());
    }
}

fn emit_pad(img: &mut Image, art: PadArt, func: Option<&str>, attrs: ObjAttrs) {
    match art {
        PadArt::Flash { def, at } => img.flash(ApKey::new(def, func), at, attrs),
        PadArt::Stroke { a, b, width } => {
            img.line(ApKey::new(format!("C,{}", mm(width)), func), a, b, attrs)
        }
        PadArt::Poly { region, stroke } => {
            img.region(region.outer.clone(), region.holes.clone(), attrs);
            if stroke > 0 {
                let ap = ApKey::new(format!("C,{}", mm(stroke)), func);
                ring_lines(img, &region.outer, &ap);
                for h in &region.holes {
                    ring_lines(img, h, &ap);
                }
            }
        }
    }
}

impl Cx<'_> {
    fn net_name(&self, id: Option<crate::board::NetId>) -> Option<String> {
        id.and_then(|i| self.board.net(i)).map(|n| n.name.clone())
    }

    fn pad_attrs(&self, fp: &Footprint, pad: &Pad) -> ObjAttrs {
        let net = Some(self.net_name(pad.net).unwrap_or_else(|| "N/C".to_string()));
        if fp.reference.is_empty() {
            ObjAttrs {
                net,
                comp: None,
                pin: None,
            }
        } else {
            ObjAttrs {
                net,
                comp: Some(fp.reference.clone()),
                pin: Some((fp.reference.clone(), pad.number.clone())),
            }
        }
    }

    fn copper(&mut self, layer: Layer) -> Result<()> {
        let b = self.board;
        for z in b.zones.iter().filter(|z| z.layer == layer) {
            if z.filled.is_empty() {
                self.warnings.push(format!(
                    "zone {} on {} has no fill data and was not written; fill the zone first",
                    z.id.0,
                    layer.name()
                ));
                continue;
            }
            let attrs = ObjAttrs {
                net: self.net_name(z.net),
                ..ObjAttrs::default()
            };
            for r in &z.filled {
                self.img
                    .region(r.outer.clone(), r.holes.clone(), attrs.clone());
            }
        }
        for t in b.tracks.iter().filter(|t| t.layer == layer) {
            let ap = ApKey::new(format!("C,{}", mm(t.width)), Some("Conductor"));
            let attrs = ObjAttrs {
                net: self.net_name(t.net),
                ..ObjAttrs::default()
            };
            match &t.geom {
                crate::board::TrackGeom::Segment { start, end } => {
                    self.img.line(ap, *start, *end, attrs)
                }
                crate::board::TrackGeom::Arc(a) => self.img.arc(ap, *a, attrs),
            }
        }
        let n = b.copper_layers;
        let here = layer.copper_index(n);
        for v in &b.vias {
            let (Some(i), Some(a), Some(z)) = (here, v.from.copper_index(n), v.to.copper_index(n))
            else {
                continue;
            };
            if i < a.min(z) || i > a.max(z) {
                continue;
            }
            self.img.flash(
                ApKey::new(format!("C,{}", mm(v.diameter)), Some("ViaPad")),
                v.position,
                ObjAttrs {
                    net: self.net_name(v.net),
                    ..ObjAttrs::default()
                },
            );
        }
        for fp in &b.footprints {
            for pad in &fp.pads {
                if !pad_copper_layers(fp, pad, n).contains(&layer) {
                    continue;
                }
                let Some(art) = pad_art(fp, pad, 0)? else {
                    continue;
                };
                let func = if pad.kind == PadKind::Smd {
                    "SMDPad,CuDef"
                } else {
                    "ComponentPad"
                };
                let attrs = self.pad_attrs(fp, pad);
                emit_pad(&mut self.img, art, Some(func), attrs);
            }
        }
        Ok(())
    }

    fn mask(&mut self, side: Side) -> Result<()> {
        let b = self.board;
        for fp in &b.footprints {
            for pad in &fp.pads {
                if !pad.mask {
                    continue;
                }
                let on_side = match pad.kind {
                    PadKind::Smd => fp.side == side,
                    PadKind::ThroughHole | PadKind::Npth => true,
                };
                if !on_side {
                    continue;
                }
                let m = pad.mask_margin.unwrap_or(self.opts.mask_margin);
                if let Some(art) = pad_art(fp, pad, m)? {
                    emit_pad(&mut self.img, art, None, ObjAttrs::default());
                }
            }
        }
        let want = side.copper();
        for v in b.vias.iter().filter(|v| !v.tented) {
            if v.from != want && v.to != want {
                continue;
            }
            let d = v
                .diameter
                .checked_add(self.opts.mask_margin.max(0).saturating_mul(2))
                .ok_or(PcbError::Overflow("via mask opening"))?;
            self.img.flash(
                ApKey::new(format!("C,{}", mm(d)), None),
                v.position,
                ObjAttrs::default(),
            );
        }
        Ok(())
    }

    fn paste(&mut self, side: Side) -> Result<()> {
        for fp in self.board.footprints.iter().filter(|f| f.side == side) {
            for pad in &fp.pads {
                if pad.kind != PadKind::Smd || !pad.paste {
                    continue;
                }
                if let Some(art) = pad_art(fp, pad, pad.paste_margin.unwrap_or(0))? {
                    emit_pad(&mut self.img, art, None, ObjAttrs::default());
                }
            }
        }
        Ok(())
    }

    fn silk(&mut self, layer: Layer) -> Result<()> {
        let art = collect_artwork(self.board)?;
        if art.unsupported_chars > 0 {
            self.warnings.push(format!(
                "{} text characters are not in the stroke font and were drawn as boxes",
                art.unsupported_chars
            ));
        }
        for p in art.prims.iter().filter(|p| p.layer == layer) {
            self.art(p, None, None)?;
        }
        Ok(())
    }

    fn profile(&mut self) -> Result<()> {
        let w = self.opts.profile_width;
        let ap = ApKey::new(format!("C,{}", mm(w)), Some("Profile"));
        if let Some(o) = &self.board.outline {
            let loops = std::iter::once(&o.outer).chain(o.cutouts.iter());
            for l in loops {
                for s in l {
                    match s {
                        crate::board::OutlineSeg::Line { start, end } => {
                            self.img.line(ap.clone(), *start, *end, ObjAttrs::default())
                        }
                        crate::board::OutlineSeg::Arc(a) => {
                            self.img.arc(ap.clone(), *a, ObjAttrs::default())
                        }
                    }
                }
            }
        }
        let art = collect_artwork(self.board)?;
        for p in art.prims.iter().filter(|p| p.layer == Layer::EdgeCuts) {
            self.art(p, Some("Profile"), Some(w))?;
        }
        Ok(())
    }

    /// Draws one artwork primitive. `fallback` replaces a zero stroke width.
    fn art(&mut self, p: &ArtPrim, func: Option<&str>, fallback: Option<Nm>) -> Result<()> {
        let width = if p.width > 0 {
            p.width
        } else {
            fallback.unwrap_or(0)
        };
        let ap = (width > 0).then(|| ApKey::new(format!("C,{}", mm(width)), func));
        let none = ObjAttrs::default;
        let mut skipped = false;
        match &p.geom {
            ArtGeom::Line { a, b } => match &ap {
                Some(ap) => self.img.line(ap.clone(), *a, *b, none()),
                None => skipped = true,
            },
            ArtGeom::Arc(a) => match &ap {
                Some(ap) => self.img.arc(ap.clone(), *a, none()),
                None => skipped = true,
            },
            ArtGeom::Circle { center, radius } => match &ap {
                Some(ap) => {
                    let start = Point::new(
                        center
                            .x
                            .checked_add(*radius)
                            .ok_or(PcbError::Overflow("circle"))?,
                        center.y,
                    );
                    check_coord(start.x, "circle")?;
                    self.img.arc(
                        ap.clone(),
                        Arc {
                            start,
                            end: start,
                            center: *center,
                            ccw: true,
                        },
                        none(),
                    );
                }
                None => skipped = true,
            },
            ArtGeom::Disc { center, diameter } => {
                if *diameter > 0 {
                    self.img.flash(
                        ApKey::new(format!("C,{}", mm(*diameter)), func),
                        *center,
                        none(),
                    );
                }
            }
            ArtGeom::Polygon(r) => {
                self.img.region(r.outer.clone(), r.holes.clone(), none());
                if let Some(ap) = &ap {
                    ring_lines(&mut self.img, &r.outer, ap);
                    for h in &r.holes {
                        ring_lines(&mut self.img, h, ap);
                    }
                }
            }
            ArtGeom::PolyOutline(r) => match &ap {
                Some(ap) => {
                    ring_lines(&mut self.img, &r.outer, ap);
                    for h in &r.holes {
                        ring_lines(&mut self.img, h, ap);
                    }
                }
                None => skipped = true,
            },
        }
        if skipped {
            self.warnings
                .push("a stroked drawing with zero width was not written".into());
        }
        Ok(())
    }
}
