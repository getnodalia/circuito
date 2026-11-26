// SPDX-License-Identifier: MIT OR Apache-2.0
//! Exact integer geometry: points, distances, regions, arcs and clearance shapes.

pub mod arc;
pub mod dist;
pub mod point;
pub mod region;
pub mod shape;
pub mod wide;

pub use arc::Arc;
pub use point::{Point, Rect};
pub use region::{Loc, Region};
pub use shape::{Core, Shape};
