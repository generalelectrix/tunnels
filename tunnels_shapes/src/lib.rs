//! Procedural figures for projection: closed contours and the fill rule that
//! decides what they enclose.
//!
//! A figure is a [`geom::Figure`] — a list of closed polylines plus a
//! [`geom::FillRule`]. Nothing here rasterises, tessellates or strokes; the
//! contours are the whole output.
//!
//! Even-odd fill is the operator these families are built with rather than an
//! option applied to them afterwards. Overlap subtracts, and every family leans
//! on that somewhere: a ring is an outer contour and an inner one with the disc
//! between them cancelled, two grids at a slight angle beat against each other
//! where they cross, and a curve traced past its own period would erase itself
//! entirely.

pub mod curve;
pub mod families;
pub mod geom;
pub mod presets;

pub use families::ShapeParams;
pub use geom::{Contour, Figure, FillRule, Point};
