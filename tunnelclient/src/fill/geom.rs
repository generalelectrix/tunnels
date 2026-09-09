//! Triangles, and the two ways a list of them gets walked.
//!
//! Figures arrive as contours and leave as triangles, so this is where the
//! renderer's own geometry vocabulary lives. `tunnels_sprites` has no triangles
//! in it — it ships loops and leaves the winding rule to whoever fills them.

use tunnels_sprites::Point;

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

    /// Every vertex in order, three to a triangle.
    pub fn points(&self) -> &[Point] {
        &self.0
    }
}

/// Indices into a run of shared vertices, in the narrowest width that
/// addresses them.
///
/// **The width is a property of one run and not of the library**, because the
/// two ends of the library are three orders of magnitude apart: the smallest
/// figure refines to a thousand vertices at the coarsest density and the
/// largest to 354,089 at the finest, which no `u16` reaches. One width for all
/// of them is either `u32` everywhere or a cap on how finely a figure may be
/// refined.
///
/// Indices are two-thirds of what a mesh weighs — a triangle costs three of
/// them against the two stored coordinates of about half a vertex — so this is
/// the term worth narrowing first. It halves the coarse densities, where every
/// mesh fits, and does almost nothing at the finest, where the meshes that do
/// not fit hold nine tenths of the triangles.
pub enum Indices {
    Narrow(Vec<u16>),
    Wide(Vec<u32>),
}

impl Default for Indices {
    /// An empty run, which addresses no vertices and so needs no width.
    fn default() -> Self {
        Self::Narrow(Vec::new())
    }
}

impl Indices {
    /// The indices of `flat`, narrowed if every one of them fits.
    pub fn of(flat: Vec<u32>, vertices: usize) -> Self {
        if vertices > usize::from(u16::MAX) + 1 {
            return Self::Wide(flat);
        }
        Self::Narrow(flat.into_iter().map(|i| i as u16).collect())
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Narrow(i) => i.len(),
            Self::Wide(i) => i.len(),
        }
    }

    /// What these indices hold, counting what is allocated rather than what
    /// is used, since the difference is memory either way.
    pub fn bytes(&self) -> usize {
        match self {
            Self::Narrow(i) => i.capacity() * size_of::<u16>(),
            Self::Wide(i) => i.capacity() * size_of::<u32>(),
        }
    }

    /// Runs of whole triangles, each within `max_vertices`.
    pub fn batches(&self, max_vertices: usize) -> impl Iterator<Item = IndexBatch<'_>> {
        let run = (max_vertices / 3 * 3).max(3);
        // Only one of the two is ever populated; the other contributes no
        // batches, which is what lets both widths come back as one iterator.
        let (narrow, wide) = match self {
            Self::Narrow(i) => (i.as_slice(), [].as_slice()),
            Self::Wide(i) => ([].as_slice(), i.as_slice()),
        };
        narrow
            .chunks(run)
            .map(IndexBatch::Narrow)
            .chain(wide.chunks(run).map(IndexBatch::Wide))
    }
}

/// A run of whole triangles from a refined mesh, small enough for one call
/// into the backend.
///
/// A refined mesh shares vertices between the triangles that use them, so what
/// gets batched is indices rather than points.
///
/// Both stored widths read back as `u32`, which is what keeps the choice of
/// width inside the mesh: a draw addresses a vertex the same way whichever
/// width the mesh it came from was narrow enough to use.
#[derive(Copy, Clone)]
pub enum IndexBatch<'a> {
    Narrow(&'a [u16]),
    Wide(&'a [u32]),
}

impl<'a> IndexBatch<'a> {
    /// Each triangle's three vertex indices.
    pub fn triangles(self) -> impl Iterator<Item = [u32; 3]> + 'a {
        let (narrow, wide) = self.split();
        narrow
            .as_chunks::<3>()
            .0
            .iter()
            .map(|&[a, b, c]| [u32::from(a), u32::from(b), u32::from(c)])
            .chain(wide.as_chunks::<3>().0.iter().copied())
    }

    /// Every index in the batch, for a draw that does not need the grouping.
    pub fn indices(self) -> impl Iterator<Item = u32> + 'a {
        let (narrow, wide) = self.split();
        narrow
            .iter()
            .map(|&i| u32::from(i))
            .chain(wide.iter().copied())
    }

    /// The batch as both widths, one of which is always empty.
    ///
    /// Reading it this way is what lets a walk over either width come back as
    /// one iterator rather than two the caller has to match on.
    fn split(self) -> (&'a [u16], &'a [u32]) {
        match self {
            Self::Narrow(i) => (i, &[]),
            Self::Wide(i) => (&[], i),
        }
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
    fn a_list_reads_back_as_the_triangles_that_went_in() {
        let mut list = TriangleList::default();
        for i in 0..10 {
            let p = Point::new(i as f32, 0.0);
            list.push(Triangle::new(p, p, p));
        }
        assert_eq!(list.triangles().count(), 10);
        // The flat form and the grouped one are the same vertices in the same
        // order, which is what lets a draw take either.
        assert_eq!(list.points().len(), 30);
        let flattened: Vec<Point> = list.triangles().flat_map(|t| t.points()).collect();
        assert_eq!(flattened, list.points());
    }
}
