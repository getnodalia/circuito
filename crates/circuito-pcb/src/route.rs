// SPDX-License-Identifier: MIT OR Apache-2.0
//! A small exact connector helper. This is **not** an autorouter: it draws the one or two
//! segments that join two points at angles of 0, 45 or 90 degrees, and checks the result against
//! the board with the design rules.
//!
//! All arithmetic is integer: the legs are exactly horizontal, vertical or diagonal at 45 degrees,
//! and the end points are exactly the points asked for.

use crate::board::{Board, Id, NetId, Track, TrackGeom};
use crate::drc::{run_drc, Rules, Violation, ViolationCode};
use crate::error::{PcbError, Result};
use crate::geom::point::Point;
use crate::layer::Layer;
use crate::units::{check_coord, check_positive, Nm};

/// How the two points are joined.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ConnectorShape {
    /// Horizontal leg first, then vertical (a right angle).
    HorizontalVertical,
    /// Vertical leg first, then horizontal (a right angle).
    VerticalHorizontal,
    /// A straight leg along the longer axis, then a 45 degree leg into the end point.
    StraightThenDiagonal,
    /// A 45 degree leg out of the start point, then a straight leg along the longer axis.
    DiagonalThenStraight,
}

/// The corner points of the connector from `a` to `b`, `a` and `b` included. Points that already
/// share a row, a column or a 45 degree diagonal are joined by a single segment, whatever the shape.
pub fn connector_points(a: Point, b: Point, shape: ConnectorShape) -> Result<Vec<Point>> {
    for p in [a, b] {
        check_coord(p.x, "connector point")?;
        check_coord(p.y, "connector point")?;
    }
    if a == b {
        return Err(PcbError::Invalid(
            "a connector needs two different points".into(),
        ));
    }
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    if dx == 0 || dy == 0 || dx.abs() == dy.abs() {
        return Ok(vec![a, b]);
    }
    let (sx, sy) = (dx.signum(), dy.signum());
    let m = dx.abs().min(dy.abs());
    let mid = match shape {
        ConnectorShape::HorizontalVertical => Point::new(b.x, a.y),
        ConnectorShape::VerticalHorizontal => Point::new(a.x, b.y),
        ConnectorShape::DiagonalThenStraight => Point::new(a.x + sx * m, a.y + sy * m),
        ConnectorShape::StraightThenDiagonal => {
            if dx.abs() > dy.abs() {
                Point::new(b.x - sx * m, a.y)
            } else {
                Point::new(a.x, b.y - sy * m)
            }
        }
    };
    Ok(vec![a, mid, b])
}

/// The result of [`connect`]: the new tracks (not added to the board) and what the design rules
/// say about them.
#[derive(Clone, Debug, PartialEq)]
pub struct Connection {
    pub tracks: Vec<Track>,
    /// Violations that involve a new track: clearance, shorts, width, edge, keepout. Dangling ends
    /// and unconnected nets are not reported, because the connector is one piece of a route.
    pub violations: Vec<Violation>,
}

impl Connection {
    /// True when no violation was found.
    pub fn is_clean(&self) -> bool {
        self.violations.is_empty()
    }
}

/// What to connect.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ConnectorSpec {
    pub net: Option<NetId>,
    pub layer: Layer,
    pub width: Nm,
    pub from: Point,
    pub to: Point,
    pub shape: ConnectorShape,
}

/// Builds the tracks joining `spec.from` and `spec.to` and checks them against the board with
/// `rules`. The board is not changed; add `tracks` to it when the result is acceptable.
pub fn connect(board: &Board, spec: &ConnectorSpec, rules: &Rules) -> Result<Connection> {
    let ConnectorSpec {
        net,
        layer,
        width,
        from,
        to,
        shape,
    } = *spec;
    check_positive(width, "track width")?;
    if layer.copper_index(board.copper_layers).is_none() {
        return Err(PcbError::Invalid(format!(
            "the board has no copper layer {}",
            layer.name()
        )));
    }
    if let Some(n) = net {
        if board.net(n).is_none() {
            return Err(PcbError::Invalid(format!("unknown net {}", n.0)));
        }
    }
    let pts = connector_points(from, to, shape)?;
    let first = board.next_id().0;
    let tracks: Vec<Track> = pts
        .windows(2)
        .enumerate()
        .filter_map(|(i, w)| {
            Some(Track {
                id: Id(first.checked_add(i as u64)?),
                net,
                layer,
                width,
                geom: TrackGeom::Segment {
                    start: *w.first()?,
                    end: *w.get(1)?,
                },
            })
        })
        .collect();
    let mut trial = board.clone();
    trial.tracks.extend(tracks.iter().cloned());
    let report = run_drc(&trial, rules)?;
    let violations = report
        .violations
        .into_iter()
        .filter(|v| {
            !matches!(
                v.code,
                ViolationCode::TrackDangling
                    | ViolationCode::Unconnected
                    | ViolationCode::ZoneUnfilled
            ) && v
                .objects
                .iter()
                .any(|o| tracks.iter().any(|t| t.id == o.id))
        })
        .collect();
    Ok(Connection { tracks, violations })
}
