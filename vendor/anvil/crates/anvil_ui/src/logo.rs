//! The ANVL logo, drawn from its polygons.
//!
//! The coordinates are copied from `HorizontalLogoANVL.svg` (a 640 x 160 frame), so the blade and
//! the wordmark here match the brand files point for point. The square squircle mark is copied
//! the same way from `LogoANVL.svg` (a 256 x 256 frame). Both are built into the engine: no SVG
//! is read at runtime.
//!
//! Shapes are filled by `anvl_gpu::raster` at the exact physical size the UI needs, so the
//! diagonal edges stay anti-aliased at any scale factor. The result is a straight-alpha
//! [`Frame`] that the renderer uploads once and `Ui::image` draws.

use crate::media::Frame;
use crate::raster::{rounded_rect, Canvas, FillRule};
use crate::theme::anvl;

/// A polygon as `(x, y)` points in the 640 x 160 logo frame.
pub type Poly = &'static [(f32, f32)];

/// Left heavy slash of the blade.
pub const BLADE_SLASH: Poly = &[(106.0, 44.0), (126.0, 44.0), (94.0, 116.0), (74.0, 116.0)];
/// Right counter hook of the blade.
pub const BLADE_HOOK: Poly = &[
    (131.0, 59.0),
    (152.0, 105.0),
    (143.0, 116.0),
    (104.0, 116.0),
    (111.0, 100.0),
    (131.0, 100.0),
    (121.0, 79.0),
];
/// Wordmark letters, left to right: A, N, V, L. They share the blade's baseline (y 44 to 116).
pub const WORDMARK: [Poly; 4] = [
    &[(262.0, 44.0), (282.0, 44.0), (314.0, 116.0), (291.0, 116.0), (272.0, 70.0), (253.0, 116.0), (230.0, 116.0)],
    &[
        (326.0, 44.0),
        (346.0, 44.0),
        (382.0, 89.0),
        (382.0, 44.0),
        (402.0, 44.0),
        (402.0, 116.0),
        (382.0, 116.0),
        (346.0, 71.0),
        (346.0, 116.0),
        (326.0, 116.0),
    ],
    &[(414.0, 44.0), (437.0, 44.0), (456.0, 90.0), (475.0, 44.0), (498.0, 44.0), (466.0, 116.0), (446.0, 116.0)],
    &[(510.0, 44.0), (532.0, 44.0), (532.0, 98.0), (576.0, 98.0), (576.0, 116.0), (510.0, 116.0)],
];

/// Vertical divider between the blade and the wordmark: x, y0, y1, stroke width.
pub const DIVIDER: (f32, f32, f32, f32) = (194.0, 38.0, 122.0, 2.0);

/// Frame rows the logo occupies. The divider is the tallest part (y 38 to 122).
pub const CROP_Y: (f32, f32) = (38.0, 122.0);
/// Horizontal extent of the blade alone.
pub const MARK_X: (f32, f32) = (74.0, 152.0);
/// Horizontal extent of blade, divider, and wordmark together.
pub const LOCKUP_X: (f32, f32) = (74.0, 576.0);

/// Height of the cropped logo in frame units.
pub const CROP_H: f32 = CROP_Y.1 - CROP_Y.0;

/// Which part of the logo to draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogoPart {
    /// The blade only.
    Blade,
    /// Blade, divider, and the ANVL wordmark.
    Lockup,
}

impl LogoPart {
    fn x_range(self) -> (f32, f32) {
        match self {
            LogoPart::Blade => MARK_X,
            LogoPart::Lockup => LOCKUP_X,
        }
    }

    /// Width over height of the cropped part.
    pub fn aspect(self) -> f32 {
        let (x0, x1) = self.x_range();
        (x1 - x0) / CROP_H
    }
}

/// Rasterizes a logo part so it is `px_height` pixels tall. Blade and wordmark use `TEXT`, the
/// divider uses the border color.
pub fn polygon_frame(part: LogoPart, px_height: u32) -> Frame {
    let px_h = px_height.max(1);
    let k = px_h as f32 / CROP_H;
    let (x0, x1) = part.x_range();
    let px_w = (((x1 - x0) * k).ceil() as u32).max(1);
    let mut canvas = Canvas::new(px_w, px_h);
    let map = |poly: &[(f32, f32)]| -> Vec<[f32; 2]> { poly.iter().map(|&(x, y)| [(x - x0) * k, (y - CROP_Y.0) * k]).collect() };
    canvas.fill(&[map(BLADE_SLASH)], anvl::TEXT, FillRule::NonZero);
    canvas.fill(&[map(BLADE_HOOK)], anvl::TEXT, FillRule::NonZero);
    if part == LogoPart::Lockup {
        let (dx, dy0, dy1, dw) = DIVIDER;
        let quad = [(dx - dw * 0.5, dy0), (dx + dw * 0.5, dy0), (dx + dw * 0.5, dy1), (dx - dw * 0.5, dy1)];
        canvas.fill(&[map(&quad)], anvl::BORDER, FillRule::NonZero);
        for letter in WORDMARK {
            canvas.fill(&[map(letter)], anvl::TEXT, FillRule::NonZero);
        }
    }
    canvas.into_frame()
}

/// The squircle app mark, from `LogoANVL.svg` (256 x 256): a dark rounded square with a 4-unit
/// border ring, and the blade on top. Its two polygons are the blade at the mark's scale.
pub const MARK_SLASH: Poly = &[(116.0, 52.0), (152.0, 52.0), (88.0, 204.0), (52.0, 204.0)];
pub const MARK_HOOK: Poly = &[(164.0, 84.0), (204.0, 180.0), (184.0, 204.0), (108.0, 204.0), (122.0, 170.0), (162.0, 170.0), (144.0, 128.0)];

/// Rasterizes the squircle mark into a `px` by `px` frame: the window icon, the launcher icon,
/// and the phone header.
pub fn mark_frame(px: u32) -> Frame {
    let px = px.max(1);
    let k = px as f32 / 256.0;
    let mut canvas = Canvas::new(px, px);
    canvas.fill(&[rounded_rect(0.0, 0.0, 256.0 * k, 256.0 * k, 56.0 * k)], anvl::BG, FillRule::NonZero);
    // The border is a 4-unit stroke centered on (4, 4, 248, 248) with radius 52: a ring.
    let outer = rounded_rect(2.0 * k, 2.0 * k, 252.0 * k, 252.0 * k, 54.0 * k);
    let inner = rounded_rect(6.0 * k, 6.0 * k, 244.0 * k, 244.0 * k, 50.0 * k);
    canvas.fill(&[outer, inner], anvl::BORDER, FillRule::EvenOdd);
    let scale = |poly: Poly| -> Vec<[f32; 2]> { poly.iter().map(|&(x, y)| [x * k, y * k]).collect() };
    canvas.fill(&[scale(MARK_SLASH)], anvl::TEXT, FillRule::NonZero);
    canvas.fill(&[scale(MARK_HOOK)], anvl::TEXT, FillRule::NonZero);
    canvas.into_frame()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ink(frame: &Frame) -> usize {
        frame.rgba.chunks_exact(4).filter(|p| p[3] > 128).count()
    }

    #[test]
    fn polygons_stay_inside_the_crop() {
        for poly in [BLADE_SLASH, BLADE_HOOK].into_iter().chain(WORDMARK) {
            for &(x, y) in poly {
                assert!((LOCKUP_X.0..=LOCKUP_X.1).contains(&x), "x {x}");
                assert!((44.0..=116.0).contains(&y), "y {y}");
            }
        }
    }

    #[test]
    fn blade_and_lockup_rasterize_with_ink() {
        let blade = polygon_frame(LogoPart::Blade, 84);
        assert_eq!(blade.height, 84);
        assert!(ink(&blade) > 500);
        let lockup = polygon_frame(LogoPart::Lockup, 84);
        assert!(lockup.width > blade.width * 5);
        assert!(ink(&lockup) > ink(&blade) * 2);
    }

    #[test]
    fn the_mark_has_a_dark_body_a_border_and_a_light_blade() {
        let f = mark_frame(128);
        let at = |x: u32, y: u32| &f.rgba[((y * 128 + x) * 4) as usize..((y * 128 + x) * 4 + 4) as usize];
        assert_eq!(at(64, 20), &[0x09, 0x09, 0x0b, 255], "body");
        assert_eq!(at(1, 64), &[0x27, 0x27, 0x2a, 255], "border ring");
        assert_eq!(at(0, 0)[3], 0, "the rounded corner is clear");
        assert_eq!(at(60, 50), &[0xf4, 0xf4, 0xf5, 255], "blade");
    }

    #[test]
    fn straight_alpha_keeps_the_color() {
        let blade = polygon_frame(LogoPart::Blade, 168);
        let solid = blade.rgba.chunks_exact(4).find(|p| p[3] == 255).expect("a solid pixel");
        assert_eq!(&solid[..3], &[0xf4, 0xf4, 0xf5]);
    }
}
