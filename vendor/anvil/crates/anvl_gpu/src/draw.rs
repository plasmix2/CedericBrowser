//! Per-frame draw data: rounded shapes, icons, and text, grouped into layers.
//!
//! Layer 0 is the base UI. Layer 1 is drawn above it, for dropdown menus and other popups.
//! Within a layer, all shapes draw before all text, so a layer's text never sits under its
//! own shapes.

use bytemuck::{Pod, Zeroable};

use crate::geom::Rect;
use crate::text::TextKey;
use crate::color::{with_alpha, Color};

/// Number of draw layers. Layer 0 is the base; higher layers draw on top.
pub const LAYER_COUNT: usize = 3;

/// A clip rectangle (x0, y0, x1, y1) that clips nothing.
pub const NO_CLIP: [f32; 4] = [-1.0e9, -1.0e9, 1.0e9, 1.0e9];

/// The overlap of two clip rectangles.
pub fn intersect_clip(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])]
}

/// A color ramp across a rounded rectangle, from its fill color to `to`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gradient {
    /// Along a line through the center at `angle` radians: 0 runs left to right, a quarter turn
    /// runs top to bottom.
    Linear { to: Color, angle: f32 },
    /// From the center out to the edges.
    Radial { to: Color },
    /// Flowing lit cyan-to-gold diagonal wave bands (matching `anvl.dev/hero-shader.js`).
    HeroWave { time: f32, intensity: f32 },
    /// Persona patch: a soft glowing orb with ripples in `accent`, animated by `time` (seconds) and
    /// scaled by `energy`, over the base fill color. Used by Persona's persona editor.
    Orb { time: f32, energy: f32, accent: Color },
}

/// How a rounded rectangle looks: a fill, an optional 1 px border, and a corner radius.
#[derive(Clone, Copy, Debug)]
pub struct RectStyle {
    pub fill: Color,
    pub radius: f32,
    pub border: Color,
    pub border_width: f32,
    /// Width of a soft edge in pixels. Zero is a crisp shape. A soft shape is a drop shadow.
    pub softness: f32,
    pub gradient: Option<Gradient>,
}

impl RectStyle {
    pub const fn fill(fill: Color) -> Self {
        Self {
            fill,
            radius: 0.0,
            border: [0.0; 4],
            border_width: 0.0,
            softness: 0.0,
            gradient: None,
        }
    }

    /// Fills with a linear ramp from the fill color to `to`, at `angle` radians.
    pub const fn linear(mut self, to: Color, angle: f32) -> Self {
        self.gradient = Some(Gradient::Linear { to, angle });
        self
    }

    /// Fills with a radial ramp from the fill color at the center to `to` at the edges.
    pub const fn radial(mut self, to: Color) -> Self {
        self.gradient = Some(Gradient::Radial { to });
        self
    }

    /// Fills with the animated ANVL website hero wave-band shader over the base fill color.
    pub const fn hero_wave(mut self, time: f32, intensity: f32) -> Self {
        self.gradient = Some(Gradient::HeroWave { time, intensity });
        self
    }

    /// Fills with an animated glowing orb in `accent` over the base fill (Persona patch).
    pub const fn orb(mut self, time: f32, energy: f32, accent: Color) -> Self {
        self.gradient = Some(Gradient::Orb { time, energy, accent });
        self
    }

    /// Feathers the edge over `px` pixels, for drop shadows (ADL-V3 section 3.3).
    pub const fn soft(mut self, px: f32) -> Self {
        self.softness = px;
        self
    }

    pub const fn radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    /// Border drawn inside the shape edge, so it adds no size.
    pub const fn border(mut self, color: Color, width: f32) -> Self {
        self.border = color;
        self.border_width = width;
        self
    }
}

/// GPU instance for one rounded rectangle. Layout matches `shaders.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct RectInstance {
    /// x, y, width, height
    pub rect: [f32; 4],
    pub fill: [f32; 4],
    pub border: [f32; 4],
    /// radius, border width, softness, unused
    pub params: [f32; 4],
    /// x0, y0, x1, y1: pixels outside are not drawn.
    pub clip: [f32; 4],
    /// The gradient's end color, and its kind (0 none, 1 linear, 2 radial) and angle.
    pub fill2: [f32; 4],
    pub grad: [f32; 4],
}

impl RectInstance {
    pub fn new(r: Rect, s: RectStyle) -> Self {
        let (fill2, grad) = match s.gradient {
            None => (s.fill, [0.0; 4]),
            Some(Gradient::Linear { to, angle }) => (to, [1.0, angle, 0.0, 0.0]),
            Some(Gradient::Radial { to }) => (to, [2.0, 0.0, 0.0, 0.0]),
            Some(Gradient::HeroWave { time, intensity }) => (s.fill, [3.0, time, intensity, 0.0]),
            Some(Gradient::Orb { time, energy, accent }) => (accent, [4.0, time, energy, 0.0]),
        };
        Self {
            clip: NO_CLIP,
            fill2,
            grad,
            rect: [r.x(), r.y(), r.w(), r.h()],
            fill: s.fill,
            border: if s.border_width > 0.0 {
                s.border
            } else {
                with_alpha(s.border, 0.0)
            },
            params: [s.radius, s.border_width, s.softness, 0.0],
        }
    }
}

/// GPU instance for one icon quad sampled from the icon atlas.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct IconInstance {
    pub rect: [f32; 4],
    /// u0, v0, u1, v1 in the atlas
    pub uv: [f32; 4],
    pub tint: [f32; 4],
    /// x: rotation around the icon's center, in radians. Other lanes are unused.
    pub params: [f32; 4],
    /// x0, y0, x1, y1: pixels outside are not drawn.
    pub clip: [f32; 4],
}

/// A texture the renderer holds: a decoded image or a video frame. `index` is the renderer's
/// slot for it. The size is kept here so layout can fit the picture without a lookup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextureId {
    pub index: u32,
    pub width: u32,
    pub height: u32,
}

/// GPU instance for one textured quad. Layout matches the image entry points in `shaders.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct ImageInstance {
    pub rect: [f32; 4],
    /// u0, v0, u1, v1 in the texture
    pub uv: [f32; 4],
    /// Multiplies the texel color. White leaves the picture as it is.
    pub tint: [f32; 4],
    /// x: corner radius in pixels. Other lanes are unused.
    pub params: [f32; 4],
    /// x0, y0, x1, y1: pixels outside are not drawn.
    pub clip: [f32; 4],
}

/// GPU instance for one stroke piece: a line segment with round ends, or an arc.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct PathInstance {
    /// Segment: start x, y, end x, y. Arc: center x, y, radius, unused.
    pub a: [f32; 4],
    /// Stroke width, kind (0 segment, 1 arc), arc start angle, arc sweep (radians).
    pub b: [f32; 4],
    pub color: [f32; 4],
    pub clip: [f32; 4],
}

/// One image to draw: which texture, and where.
#[derive(Clone, Copy, Debug)]
pub struct ImageCmd {
    pub texture: u32,
    pub instance: ImageInstance,
}

/// A text run to draw. The shaped buffer lives in the `TextSystem` cache.
#[derive(Clone, Debug)]
pub struct TextCmd {
    pub key: TextKey,
    pub x: f32,
    pub y: f32,
    pub color: Color,
    /// Pixels outside this rectangle are clipped.
    pub clip: Rect,
}

/// Draw commands for one layer. Backdrop blurs draw first, then shapes, images, icons, and text.
#[derive(Default)]
pub struct Layer {
    /// Rounded areas that show the layers beneath, blurred (ADL-V3 section 3.5). Only `rect`,
    /// the radius, and the fill alpha (how much of the blur shows) are used.
    pub blurs: Vec<RectInstance>,
    pub rects: Vec<RectInstance>,
    /// Strokes, drawn after the layer's shapes.
    pub paths: Vec<PathInstance>,
    pub images: Vec<ImageCmd>,
    pub icons: Vec<IconInstance>,
    pub texts: Vec<TextCmd>,
}

impl Layer {
    fn clear(&mut self) {
        self.blurs.clear();
        self.rects.clear();
        self.paths.clear();
        self.images.clear();
        self.icons.clear();
        self.texts.clear();
    }
}

/// Everything the renderer needs for one frame.
pub struct DrawList {
    pub layers: [Layer; LAYER_COUNT],
    /// Layer that new commands go to.
    pub current: usize,
    /// Multiplies the alpha of every command pushed while it is set. Screen transitions and
    /// fading surfaces use it, so widgets need no alpha of their own. 1 is fully opaque.
    pub opacity: f32,
    /// Every command pushed is clipped to this rectangle (x0, y0, x1, y1).
    pub clip: [f32; 4],
}

impl Default for DrawList {
    fn default() -> Self {
        Self {
            layers: Default::default(),
            current: 0,
            opacity: 1.0,
            clip: NO_CLIP,
        }
    }
}

impl DrawList {
    pub fn clear(&mut self) {
        for layer in &mut self.layers {
            layer.clear();
        }
        self.current = 0;
        self.opacity = 1.0;
        self.clip = NO_CLIP;
    }

    /// Clips what is pushed next to `r` as well as to the current clip. Returns the clip to give
    /// back to [`DrawList::pop_clip`] when the clipped content is done.
    pub fn push_clip(&mut self, r: Rect) -> [f32; 4] {
        let prev = self.clip;
        self.clip = intersect_clip(prev, [r.x(), r.y(), r.right(), r.bottom()]);
        prev
    }

    /// Restores the clip from before a [`DrawList::push_clip`].
    pub fn pop_clip(&mut self, prev: [f32; 4]) {
        self.clip = prev;
    }

    /// A stroke along `path`, `width` pixels wide, with round joins and ends.
    pub fn stroke(&mut self, path: &crate::path::Path, width: f32, color: Color) {
        for (p, q) in path.segments() {
            self.push_path(PathInstance {
                a: [p[0], p[1], q[0], q[1]],
                b: [width, 0.0, 0.0, 0.0],
                color,
                clip: NO_CLIP,
            });
        }
    }

    /// An arc of a circle around `center`, from `start` through `sweep` radians (clockwise on
    /// screen), `width` pixels wide with round ends. A sweep of a full turn is a ring.
    pub fn arc(&mut self, center: [f32; 2], radius: f32, start: f32, sweep: f32, width: f32, color: Color) {
        self.push_path(PathInstance {
            a: [center[0], center[1], radius, 0.0],
            b: [width, 1.0, start, sweep.clamp(-std::f32::consts::TAU, std::f32::consts::TAU)],
            color,
            clip: NO_CLIP,
        });
    }

    pub fn push_path(&mut self, mut p: PathInstance) {
        p.color[3] *= self.opacity;
        p.clip = intersect_clip(p.clip, self.clip);
        self.layers[self.current].paths.push(p);
    }

    pub fn push_rect(&mut self, mut r: RectInstance) {
        if self.opacity < 1.0 {
            r.fill[3] *= self.opacity;
            r.fill2[3] *= self.opacity;
            r.border[3] *= self.opacity;
        }
        r.clip = intersect_clip(r.clip, self.clip);
        self.layers[self.current].rects.push(r);
    }

    /// A backdrop blur on the current layer. Layer 0 has nothing beneath it, so it is ignored.
    pub fn push_blur(&mut self, mut r: RectInstance) {
        if self.current == 0 {
            return;
        }
        r.fill[3] *= self.opacity;
        r.clip = intersect_clip(r.clip, self.clip);
        self.layers[self.current].blurs.push(r);
    }

    /// The first layer with a backdrop blur, if any. Layers under it are what gets blurred.
    pub fn first_blur_layer(&self) -> Option<usize> {
        self.layers.iter().position(|l| !l.blurs.is_empty())
    }

    pub fn push_icon(&mut self, mut i: IconInstance) {
        i.tint[3] *= self.opacity;
        i.clip = intersect_clip(i.clip, self.clip);
        self.layers[self.current].icons.push(i);
    }

    pub fn push_image(&mut self, texture: u32, mut instance: ImageInstance) {
        instance.tint[3] *= self.opacity;
        instance.clip = intersect_clip(instance.clip, self.clip);
        self.layers[self.current]
            .images
            .push(ImageCmd { texture, instance });
    }

    pub fn push_text(&mut self, mut t: TextCmd) {
        t.color[3] *= self.opacity;
        let c = intersect_clip([t.clip.x(), t.clip.y(), t.clip.right(), t.clip.bottom()], self.clip);
        t.clip = Rect::new(c[0], c[1], (c[2] - c[0]).max(0.0), (c[3] - c[1]).max(0.0));
        self.layers[self.current].texts.push(t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opacity_scales_every_command_and_resets_each_frame() {
        let mut d = DrawList::default();
        d.opacity = 0.5;
        d.push_rect(RectInstance::new(Rect::new(0.0, 0.0, 1.0, 1.0), RectStyle::fill([1.0; 4])));
        assert_eq!(d.layers[0].rects[0].fill[3], 0.5);
        d.clear();
        assert_eq!(d.opacity, 1.0);
        d.push_rect(RectInstance::new(Rect::new(0.0, 0.0, 1.0, 1.0), RectStyle::fill([1.0; 4])));
        assert_eq!(d.layers[0].rects[0].fill[3], 1.0);
    }

    #[test]
    fn blurs_need_a_layer_beneath_and_fade_with_opacity() {
        let mut d = DrawList::default();
        let r = RectInstance::new(Rect::new(0.0, 0.0, 4.0, 4.0), RectStyle::fill([0.0, 0.0, 0.0, 1.0]));
        d.push_blur(r);
        assert_eq!(d.first_blur_layer(), None);
        d.current = 2;
        d.opacity = 0.25;
        d.push_blur(r);
        assert_eq!(d.first_blur_layer(), Some(2));
        assert_eq!(d.layers[2].blurs[0].fill[3], 0.25);
    }

    #[test]
    fn the_clip_stack_narrows_and_restores() {
        let mut d = DrawList::default();
        let outer = d.push_clip(Rect::new(0.0, 0.0, 100.0, 100.0));
        let inner = d.push_clip(Rect::new(50.0, -10.0, 100.0, 30.0));
        d.push_rect(RectInstance::new(Rect::new(0.0, 0.0, 10.0, 10.0), RectStyle::fill([1.0; 4])));
        assert_eq!(d.layers[0].rects[0].clip, [50.0, 0.0, 100.0, 20.0]);
        d.pop_clip(inner);
        assert_eq!(d.clip, [0.0, 0.0, 100.0, 100.0]);
        d.pop_clip(outer);
        assert_eq!(d.clip, NO_CLIP);
    }

    #[test]
    fn gradients_reach_their_lanes() {
        let r = RectInstance::new(Rect::new(0.0, 0.0, 4.0, 4.0), RectStyle::fill([1.0; 4]).linear([0.0, 0.0, 0.0, 1.0], 1.5));
        assert_eq!(r.grad, [1.0, 1.5, 0.0, 0.0]);
        assert_eq!(r.fill2, [0.0, 0.0, 0.0, 1.0]);
        let plain = RectInstance::new(Rect::new(0.0, 0.0, 4.0, 4.0), RectStyle::fill([0.5; 4]));
        assert_eq!(plain.grad[0], 0.0);
    }

    #[test]
    fn soft_style_reaches_the_shader_lane() {
        let r = RectInstance::new(Rect::new(0.0, 0.0, 4.0, 4.0), RectStyle::fill([0.0; 4]).soft(12.0));
        assert_eq!(r.params[2], 12.0);
    }
}
