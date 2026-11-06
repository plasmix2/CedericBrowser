//! Vector paths for strokes: lines, polylines, and quadratic and cubic Bézier curves.
//!
//! A path is a list of subpaths. [`Path::segments`] flattens curves into straight segments fine
//! enough that no point is more than `tolerance` pixels from the true curve. The renderer draws
//! each segment as an anti-aliased capsule, so joins and ends are round. Arcs and circles have
//! their own exact shape in the renderer ([`crate::draw::DrawList::arc`]).

/// A point in pixels.
pub type Point = [f32; 2];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path {
    subpaths: Vec<Vec<Point>>,
    /// Where the current subpath started, for `close`.
    start: Option<Point>,
}

impl Path {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts a new subpath at `p`.
    pub fn move_to(mut self, p: Point) -> Self {
        self.subpaths.push(vec![p]);
        self.start = Some(p);
        self
    }

    pub fn line_to(mut self, p: Point) -> Self {
        self.current().push(p);
        self
    }

    /// A quadratic Bézier from the current point through control `c` to `p`.
    pub fn quad_to(self, c: Point, p: Point) -> Self {
        let from = self.last();
        // Exact degree elevation: the same curve as a cubic.
        let c1 = lerp2(from, c, 2.0 / 3.0);
        let c2 = lerp2(p, c, 2.0 / 3.0);
        self.cubic_to(c1, c2, p)
    }

    /// A cubic Bézier from the current point with controls `c1` and `c2` to `p`. Stored as
    /// control points and flattened when drawn.
    pub fn cubic_to(mut self, c1: Point, c2: Point, p: Point) -> Self {
        let from = self.last();
        let pts = flatten_cubic(from, c1, c2, p, 0.25);
        self.current().extend(pts.into_iter().skip(1));
        self
    }

    /// Joins the current subpath back to where it started.
    pub fn close(mut self) -> Self {
        if let Some(s) = self.start {
            self.current().push(s);
        }
        self
    }

    /// A polyline through `points`.
    pub fn polyline(points: &[Point]) -> Self {
        let mut path = Self::new();
        if let Some((first, rest)) = points.split_first() {
            path = path.move_to(*first);
            for p in rest {
                path = path.line_to(*p);
            }
        }
        path
    }

    /// The straight segments that draw this path.
    pub fn segments(&self) -> Vec<(Point, Point)> {
        let mut out = Vec::new();
        for sub in &self.subpaths {
            if sub.len() == 1 {
                // A lone point draws as a dot.
                out.push((sub[0], sub[0]));
            }
            for w in sub.windows(2) {
                out.push((w[0], w[1]));
            }
        }
        out
    }

    fn current(&mut self) -> &mut Vec<Point> {
        if self.subpaths.is_empty() {
            self.subpaths.push(vec![[0.0, 0.0]]);
            self.start = Some([0.0, 0.0]);
        }
        self.subpaths.last_mut().expect("a subpath exists")
    }

    fn last(&self) -> Point {
        self.subpaths.last().and_then(|s| s.last().copied()).unwrap_or([0.0, 0.0])
    }
}

fn lerp2(a: Point, b: Point, t: f32) -> Point {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

/// Points along a cubic Bézier, including both ends, with at most `tolerance` pixels of error.
/// The step count comes from the curve's second differences, so flat curves stay cheap.
pub fn flatten_cubic(p0: Point, p1: Point, p2: Point, p3: Point, tolerance: f32) -> Vec<Point> {
    let dd = |a: Point, b: Point, c: Point| {
        let x = a[0] - 2.0 * b[0] + c[0];
        let y = a[1] - 2.0 * b[1] + c[1];
        (x * x + y * y).sqrt()
    };
    let bend = dd(p0, p1, p2).max(dd(p1, p2, p3));
    let n = ((bend * 0.75 / tolerance.max(1e-3)).sqrt().ceil() as usize).clamp(1, 256);
    (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            [
                a * p0[0] + b * p1[0] + c * p2[0] + d * p3[0],
                a * p0[1] + b * p1[1] + c * p2[1] + d * p3[1],
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polylines_and_closed_shapes_give_their_segments() {
        let tri = Path::new().move_to([0.0, 0.0]).line_to([10.0, 0.0]).line_to([5.0, 8.0]).close();
        assert_eq!(tri.segments().len(), 3);
        assert_eq!(Path::polyline(&[[0.0, 0.0], [1.0, 1.0], [2.0, 0.0]]).segments().len(), 2);
        assert_eq!(Path::new().move_to([3.0, 3.0]).segments(), vec![([3.0, 3.0], [3.0, 3.0])]);
    }

    #[test]
    fn curves_flatten_within_tolerance_and_hit_their_ends() {
        let pts = flatten_cubic([0.0, 0.0], [0.0, 100.0], [100.0, 100.0], [100.0, 0.0], 0.25);
        assert_eq!(pts[0], [0.0, 0.0]);
        assert_eq!(*pts.last().unwrap(), [100.0, 0.0]);
        assert!(pts.len() > 8, "a strong bend needs many steps: {}", pts.len());
        // The curve's midpoint (t = 0.5) is (50, 75); the flattened line passes within tolerance.
        let seg_dist = |a: Point, b: Point, p: Point| {
            let (bx, by) = (b[0] - a[0], b[1] - a[1]);
            let t = (((p[0] - a[0]) * bx + (p[1] - a[1]) * by) / (bx * bx + by * by)).clamp(0.0, 1.0);
            ((a[0] + bx * t - p[0]).powi(2) + (a[1] + by * t - p[1]).powi(2)).sqrt()
        };
        let near = pts.windows(2).map(|w| seg_dist(w[0], w[1], [50.0, 75.0])).fold(f32::MAX, f32::min);
        assert!(near <= 0.3, "{near}");
        // A straight "curve" needs one step.
        assert_eq!(flatten_cubic([0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [3.0, 0.0], 0.25).len(), 2);
    }

    #[test]
    fn a_quadratic_is_the_same_curve_as_its_cubic() {
        let q = Path::new().move_to([0.0, 0.0]).quad_to([50.0, 100.0], [100.0, 0.0]);
        let segs = q.segments();
        let mid = segs.iter().map(|s| s.1).min_by(|a, b| (a[0] - 50.0).abs().total_cmp(&(b[0] - 50.0).abs())).unwrap();
        // A quadratic's peak is half its control height.
        assert!((mid[1] - 50.0).abs() < 2.0, "{mid:?}");
    }
}
