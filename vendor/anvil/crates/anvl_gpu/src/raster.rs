//! A small CPU rasterizer for filled shapes: polygons made of one or more contours, filled with
//! the nonzero or even-odd rule, anti-aliased, and composited into an RGBA [`Frame`].
//!
//! It draws pictures that are made once at an exact pixel size and then uploaded, such as the
//! logos, so their diagonal edges stay sharp at any scale factor. Coverage is exact across each
//! pixel and sampled on 16 sub-scanlines down it.

use crate::color::Color;
use crate::media::Frame;

/// How overlapping contours combine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillRule {
    /// Inside where the contours wind around a point a nonzero number of times.
    NonZero,
    /// Inside where a ray from the point crosses an odd number of edges. A contour inside
    /// another cuts a hole, which is how a ring is drawn.
    EvenOdd,
}

const SUB: usize = 16;

/// A picture being drawn: premultiplied RGBA in 0..1, row by row.
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    px: Vec<[f32; 4]>,
}

impl Canvas {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height, px: vec![[0.0; 4]; (width * height) as usize] }
    }

    /// Fills the area the `contours` enclose with `color`, over what is already drawn.
    pub fn fill(&mut self, contours: &[Vec<[f32; 2]>], color: Color, rule: FillRule) {
        let (w, h) = (self.width as usize, self.height as usize);
        // Edges with their winding direction, skipping horizontal ones.
        let mut edges: Vec<([f32; 2], [f32; 2], i32)> = Vec::new();
        for c in contours {
            for i in 0..c.len() {
                let (a, b) = (c[i], c[(i + 1) % c.len()]);
                if a[1] != b[1] {
                    edges.push(if a[1] < b[1] { (a, b, 1) } else { (b, a, -1) });
                }
            }
        }
        let mut cover = vec![0f32; w + 1];
        let mut xs: Vec<(f32, i32)> = Vec::new();
        for y in 0..h {
            cover.iter_mut().for_each(|c| *c = 0.0);
            let mut any = false;
            for s in 0..SUB {
                let sy = y as f32 + (s as f32 + 0.5) / SUB as f32;
                xs.clear();
                for (a, b, dir) in &edges {
                    if sy >= a[1] && sy < b[1] {
                        let t = (sy - a[1]) / (b[1] - a[1]);
                        xs.push((a[0] + (b[0] - a[0]) * t, *dir));
                    }
                }
                if xs.is_empty() {
                    continue;
                }
                xs.sort_by(|p, q| p.0.total_cmp(&q.0));
                let mut wind = 0;
                for i in 0..xs.len() - 1 {
                    wind += xs[i].1;
                    let inside = match rule {
                        FillRule::NonZero => wind != 0,
                        FillRule::EvenOdd => (i + 1) % 2 == 1,
                    };
                    if inside {
                        span(&mut cover, xs[i].0, xs[i + 1].0, w);
                        any = true;
                    }
                }
            }
            if !any {
                continue;
            }
            for (x, c) in cover.iter().take(w).enumerate() {
                let a = (c / SUB as f32).clamp(0.0, 1.0) * color[3];
                if a <= 0.0 {
                    continue;
                }
                let p = &mut self.px[y * w + x];
                let src = [color[0] * a, color[1] * a, color[2] * a, a];
                for k in 0..4 {
                    p[k] = src[k] + p[k] * (1.0 - a);
                }
            }
        }
    }

    /// The picture as straight-alpha RGBA8.
    pub fn into_frame(self) -> Frame {
        let mut rgba = Vec::with_capacity(self.px.len() * 4);
        for p in &self.px {
            let a = p[3];
            let un = |c: f32| if a > 0.0 { (c / a * 255.0).round().clamp(0.0, 255.0) as u8 } else { 0 };
            rgba.extend_from_slice(&[un(p[0]), un(p[1]), un(p[2]), (a * 255.0).round() as u8]);
        }
        Frame { width: self.width, height: self.height, rgba }
    }
}

/// Adds the horizontal coverage of [x0, x1) to the pixels it crosses.
fn span(cover: &mut [f32], x0: f32, x1: f32, w: usize) {
    let (x0, x1) = (x0.max(0.0), x1.min(w as f32));
    if x1 <= x0 {
        return;
    }
    let (i0, i1) = (x0.floor() as usize, x1.floor() as usize);
    if i0 == i1 {
        cover[i0] += x1 - x0;
        return;
    }
    cover[i0] += (i0 + 1) as f32 - x0;
    for c in cover.iter_mut().take(i1).skip(i0 + 1) {
        *c += 1.0;
    }
    if i1 < w {
        cover[i1] += x1 - i1 as f32;
    }
}

/// The outline of a rounded rectangle as one contour, its corners flattened finely enough for
/// any size the logo is drawn at.
pub fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Vec<[f32; 2]> {
    let r = r.min(w * 0.5).min(h * 0.5).max(0.0);
    let steps = 16;
    let mut out = Vec::with_capacity(steps * 4 + 4);
    let corners = [(x + w - r, y + r, -0.25f32), (x + w - r, y + h - r, 0.0), (x + r, y + h - r, 0.25), (x + r, y + r, 0.5)];
    for (cx, cy, start) in corners {
        for i in 0..=steps {
            let a = (start + 0.25 * i as f32 / steps as f32) * std::f32::consts::TAU;
            out.push([cx + r * a.cos(), cy + r * a.sin()]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha(f: &Frame, x: u32, y: u32) -> u8 {
        f.rgba[((y * f.width + x) * 4 + 3) as usize]
    }

    #[test]
    fn a_square_fills_exactly_and_edges_are_partial() {
        let mut c = Canvas::new(10, 10);
        c.fill(&[vec![[2.0, 2.0], [7.5, 2.0], [7.5, 8.0], [2.0, 8.0]]], [1.0, 0.0, 0.0, 1.0], FillRule::NonZero);
        let f = c.into_frame();
        assert_eq!(alpha(&f, 4, 4), 255);
        assert_eq!(&f.rgba[(4 * 10 + 4) * 4..(4 * 10 + 4) * 4 + 3], &[255, 0, 0]);
        assert_eq!(alpha(&f, 1, 4), 0);
        assert_eq!(alpha(&f, 7, 4), 128, "half of column 7 is inside");
        assert_eq!(alpha(&f, 4, 8), 0);
    }

    #[test]
    fn even_odd_cuts_holes_and_nonzero_does_not() {
        let outer = rounded_rect(0.0, 0.0, 20.0, 20.0, 0.0);
        let inner = vec![[5.0, 5.0], [15.0, 5.0], [15.0, 15.0], [5.0, 15.0]];
        let mut ring = Canvas::new(20, 20);
        ring.fill(&[outer.clone(), inner.clone()], [1.0; 4], FillRule::EvenOdd);
        let ring = ring.into_frame();
        assert_eq!(alpha(&ring, 10, 10), 0);
        assert_eq!(alpha(&ring, 2, 10), 255);
        let mut solid = Canvas::new(20, 20);
        solid.fill(&[outer, inner], [1.0; 4], FillRule::NonZero);
        assert_eq!(alpha(&solid.into_frame(), 10, 10), 255);
    }

    #[test]
    fn a_diagonal_edge_is_anti_aliased() {
        let mut c = Canvas::new(8, 8);
        c.fill(&[vec![[0.0, 0.0], [8.0, 8.0], [0.0, 8.0]]], [1.0; 4], FillRule::NonZero);
        let f = c.into_frame();
        let on = alpha(&f, 3, 3);
        assert!((120..=136).contains(&on), "the diagonal splits its pixels: {on}");
        assert_eq!(alpha(&f, 1, 6), 255);
        assert_eq!(alpha(&f, 6, 1), 0);
    }
}
