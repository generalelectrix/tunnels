//! Triangles, and the two ways a list of them gets walked.
//!
//! Figures arrive as contours and leave as triangles, so this is where the
//! renderer's own geometry vocabulary lives. `tunnels_shapes` has no triangles
//! in it — it ships loops and leaves the winding rule to whoever fills them.

use tunnels_shapes::Point;

/// Three points bounding a filled region.
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(transparent)]
pub struct Triangle([Point; 3]);

impl Triangle {
    pub const fn new(a: Point, b: Point, c: Point) -> Self {
        Self([a, b, c])
    }

    pub const fn points(self) -> [Point; 3] {
        self.0
    }

    /// The longest edge: which one, and how long.
    ///
    /// Ties go to the earliest edge, which is what makes a split deterministic
    /// — an isosceles triangle refined twice has to divide the same way both
    /// times or the shared vertices stop matching.
    fn longest(self) -> (usize, f32) {
        let p = self.0;
        let lengths = [
            p[0].distance(p[1]),
            p[1].distance(p[2]),
            p[2].distance(p[0]),
        ];
        let cut = (0..3).fold(
            0,
            |best, i| if lengths[i] > lengths[best] { i } else { best },
        );
        (cut, lengths[cut])
    }

    /// How long the longest edge is, which is what a refinement bounds.
    pub fn longest_edge(self) -> f32 {
        self.longest().1
    }

    /// How close this triangle comes to the origin.
    ///
    /// Angular phase varies without bound at the centre — one triangle
    /// spanning the origin covers every angle there is — so a refinement
    /// tightens its limit as this shrinks.
    pub fn nearest_radius(self) -> f32 {
        self.0.iter().map(|p| p.radius()).fold(f32::MAX, f32::min)
    }

    /// Halve this triangle across the midpoint of its longest edge.
    ///
    /// Splitting the longest edge refines a long thin triangle along its
    /// length rather than shattering it in both directions, which matters
    /// because a fill tessellator emits plenty of them.
    pub fn split_longest(self) -> [Self; 2] {
        let p = self.0;
        let (i, j, k) = match self.longest().0 {
            0 => (0, 1, 2),
            1 => (1, 2, 0),
            _ => (2, 0, 1),
        };
        let mid = p[i].midpoint(p[j]);
        [Self([p[i], mid, p[k]]), Self([mid, p[j], p[k]])]
    }
}

/// A run of triangles, three points at a time.
///
/// The grouping is the meaning: a list whose length is not a multiple of three
/// is not a coarser figure, it is nonsense. Points only go in a triangle at a
/// time, so the invariant holds by construction and nothing downstream has to
/// check it.
#[derive(Debug, Default, Clone)]
pub struct TriangleList(Vec<Point>);

impl TriangleList {
    pub fn push(&mut self, triangle: Triangle) {
        self.0.extend_from_slice(&triangle.points());
    }

    pub fn triangles(&self) -> impl Iterator<Item = Triangle> + '_ {
        self.0
            .as_chunks::<3>()
            .0
            .iter()
            .map(|&[a, b, c]| Triangle::new(a, b, c))
    }

    /// Runs of whole triangles, each within `max_vertices`.
    ///
    /// The backend takes a bounded number of vertices per call, and a run that
    /// ended mid-triangle would draw a torn one. Rounding the cap down to a
    /// multiple of three is this type's problem rather than every caller's.
    pub fn batches(&self, max_vertices: usize) -> impl Iterator<Item = &[Point]> {
        self.0.chunks((max_vertices / 3 * 3).max(3))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A run of whole triangles from a refined mesh, small enough for one call
/// into the backend.
///
/// A refined mesh shares vertices between the triangles that use them, so what
/// gets batched is indices rather than points.
#[derive(Copy, Clone)]
pub struct IndexBatch<'a>(&'a [u32]);

impl<'a> IndexBatch<'a> {
    pub(super) const fn new(indices: &'a [u32]) -> Self {
        Self(indices)
    }

    /// Each triangle's three vertex indices.
    pub fn triangles(self) -> impl Iterator<Item = [u32; 3]> + 'a {
        self.0.as_chunks::<3>().0.iter().copied()
    }

    /// Every index in the batch, for a draw that does not need the grouping.
    pub fn indices(self) -> &'a [u32] {
        self.0
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn a_split_halves_the_longest_edge_and_shares_the_new_vertex() {
        // Longest edge is the second, from (4,0) to (0,3): length 5.
        let tri = Triangle::new(
            Point::new(0.0, 0.0),
            Point::new(4.0, 0.0),
            Point::new(0.0, 3.0),
        );
        assert_eq!(tri.longest_edge(), 5.0);
        assert_eq!(tri.nearest_radius(), 0.0, "one corner is the origin");

        let halves = tri.split_longest();
        let mid = Point::new(2.0, 1.5);
        // Both halves carry the midpoint, and they carry the same one — which
        // is what lets a refinement weld them back together by value.
        assert!(halves[0].points().contains(&mid));
        assert!(halves[1].points().contains(&mid));
        // Between them they still cover the original: the untouched corner is
        // in both, and each keeps one end of the edge that was cut.
        let corner = Point::new(0.0, 0.0);
        assert!(halves.iter().all(|h| h.points().contains(&corner)));
    }

    #[test]
    fn a_list_batches_without_tearing_a_triangle() {
        let mut list = TriangleList::default();
        for i in 0..10 {
            let p = Point::new(i as f32, 0.0);
            list.push(Triangle::new(p, p, p));
        }
        assert_eq!(list.triangles().count(), 10);

        // A cap that is not a multiple of three is rounded down, so no batch
        // ever ends part way through a triangle.
        let batches: Vec<usize> = list.batches(8).map(<[Point]>::len).collect();
        assert_eq!(batches, vec![6, 6, 6, 6, 6], "30 points in runs of six");
        assert!(
            batches.iter().all(|n| n % 3 == 0),
            "a batch ended mid-triangle"
        );
        // A cap below one triangle still yields whole triangles.
        assert!(list.batches(1).all(|b| b.len() == 3));
    }
}
