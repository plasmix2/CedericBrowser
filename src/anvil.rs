//! The browser's bridge to the ANVL SDK (`vendor/anvil`): one GPU engine that draws the native
//! chrome (tab strip, toolbar, address bar) with `anvl_gpu`, text shaped and rasterised by ANVL, and
//! a `Painter` with logical-pixel drawing calls for the chrome to use.
//!
//! The engine renders **off-screen** and hands back pixels. `main.rs` still presents only the
//! toolbar strip with softbuffer (see "Things that look wrong but are deliberate" in AGENTS.md:
//! pages are child windows on top of the main window, so a GPU swap chain over the whole window
//! would flicker over them). The chrome is a short strip, so the read-back is small.

use std::collections::HashMap;

use anvil_ui::{
    Color, Fit, Frame, IconAtlas, Input, Rect, RectStyle, Renderer, TextStyle, TextureId, Ui,
};
use glam::Vec2;
use tiny_skia::Pixmap;

use crate::gfx::{Icon, Rgba};

/// The weights the chrome uses (Inter ships Regular, Medium and SemiBold in the SDK).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Weight {
    Regular,
    SemiBold,
}

impl Weight {
    fn css(self) -> u16 {
        match self {
            Weight::Regular => 400,
            Weight::SemiBold => 600,
        }
    }
}

/// The GPU, the text system and the pictures uploaded to it.
struct Inner {
    ui: Ui,
    renderer: Renderer,
    /// Uploaded pictures by caller key (favicon URL, `"logo"`, ...).
    textures: HashMap<String, TextureId>,
    clock: f32,
}

impl Inner {
    /// `None` when the machine has no usable GPU adapter (DX12 also covers WARP, so on Windows
    /// this only fails when the graphics stack is broken).
    pub fn new() -> Option<Self> {
        let icons = IconAtlas::build();
        let renderer = Renderer::new_headless(1, 1, &icons)?;
        Some(Self { ui: Ui::new(&icons), renderer, textures: HashMap::new(), clock: 0.0 })
    }

    /// Uploads `pm` under `key` unless it is already there.
    pub fn ensure_texture(&mut self, key: &str, pm: &Pixmap) {
        if !self.textures.contains_key(key) {
            let id = self.renderer.upload(&frame_from_pixmap(pm));
            self.textures.insert(key.to_owned(), id);
        }
    }

    /// Frees an uploaded picture (a replaced avatar, say).
    pub fn forget_texture(&mut self, key: &str) {
        if let Some(id) = self.textures.remove(key) {
            self.renderer.release(id);
        }
    }

    /// Width of `text` in logical pixels at `size`, the way `Painter::text` will draw it.
    pub fn text_width(&mut self, text: &str, size: f32, weight: Weight, scale: f32) -> f32 {
        self.begin(1, 1, scale);
        let w = self.ui.text_width(text, style(size, weight, [1.0; 4]));
        self.ui.end();
        w / scale
    }

    /// Draws one frame of `w` x `h` physical pixels at `scale` and returns it (RGBA8, opaque when
    /// `clear` is).
    pub fn frame(&mut self, w: u32, h: u32, scale: f32, clear: Rgba, build: impl FnOnce(&mut Painter)) -> Frame {
        let (w, h) = (w.max(1), h.max(1));
        self.renderer.resize(w, h);
        self.begin(w, h, scale);
        {
            let mut p = Painter { ui: &mut self.ui, textures: &self.textures, s: scale, backdrop: clear };
            build(&mut p);
        }
        self.ui.end();
        self.renderer.render_to_frame(&self.ui.draw, &mut self.ui.text, color(clear))
    }

    /// A transparent `w` x `h` pixmap holding a soft two-layer drop shadow for the rounded card at
    /// physical pixels (`x`, `y`, `cw`, `ch`): a wide faint ambient shadow and a tight contact one.
    #[allow(clippy::too_many_arguments)]
    pub fn shadow_pixmap(&mut self, w: u32, h: u32, scale: f32, card: (f32, f32, f32, f32), radius: f32, strength: f32) -> Option<Pixmap> {
        let (x, y, cw, ch) = (card.0 / scale, card.1 / scale, card.2 / scale, card.3 / scale);
        let r = radius / scale;
        let f = self.frame(w, h, scale, Rgba(0, 0, 0, 0), |p| {
            p.shadow(x, y, cw, ch, r, 32.0, 10.0, strength * 0.42);
            p.shadow(x, y, cw, ch, r, 8.0, 2.0, strength * 0.28);
        });
        let mut pm = Pixmap::new(w, h)?;
        copy_to_pixmap(&f, &mut pm);
        Some(pm)
    }

    fn begin(&mut self, w: u32, h: u32, scale: f32) {
        // The chrome animates itself (see `Toolbar::tick`), so ANVL gets no clock or input here.
        self.clock += 1.0 / 60.0;
        self.ui.begin(Input::default(), Vec2::new(w as f32, h as f32), scale, self.clock, 1.0 / 60.0);
    }
}

/// A handle on the one GPU engine the whole chrome shares (toolbar and every popup). Cloning it
/// shares the same device, text cache and uploaded pictures instead of starting another GPU.
#[derive(Clone)]
pub struct Engine(std::rc::Rc<std::cell::RefCell<Inner>>);

impl Engine {
    pub fn new() -> Option<Self> {
        Inner::new().map(|i| Self(std::rc::Rc::new(std::cell::RefCell::new(i))))
    }

    pub fn ensure_texture(&mut self, key: &str, pm: &Pixmap) {
        self.0.borrow_mut().ensure_texture(key, pm);
    }

    pub fn forget_texture(&mut self, key: &str) {
        self.0.borrow_mut().forget_texture(key);
    }

    pub fn text_width(&mut self, text: &str, size: f32, weight: Weight, scale: f32) -> f32 {
        self.0.borrow_mut().text_width(text, size, weight, scale)
    }

    pub fn frame(&mut self, w: u32, h: u32, scale: f32, clear: Rgba, build: impl FnOnce(&mut Painter)) -> Frame {
        self.0.borrow_mut().frame(w, h, scale, clear, build)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn shadow_pixmap(&mut self, w: u32, h: u32, scale: f32, card: (f32, f32, f32, f32), radius: f32, strength: f32) -> Option<Pixmap> {
        self.0.borrow_mut().shadow_pixmap(w, h, scale, card, radius, strength)
    }
}

fn style(size: f32, weight: Weight, c: Color) -> TextStyle {
    let mut s = TextStyle::new(size, c);
    s.weight = weight.css();
    s
}

fn color(c: Rgba) -> Color {
    [c.0 as f32 / 255.0, c.1 as f32 / 255.0, c.2 as f32 / 255.0, c.3 as f32 / 255.0]
}

/// tiny-skia keeps premultiplied pixels; ANVL wants straight alpha.
pub fn frame_from_pixmap(pm: &Pixmap) -> Frame {
    let mut rgba = Vec::with_capacity(pm.data().len());
    for p in pm.data().chunks_exact(4) {
        let a = p[3] as u32;
        let un = |c: u8| if a == 0 { 0 } else { ((c as u32 * 255 + a / 2) / a).min(255) as u8 };
        rgba.extend_from_slice(&[un(p[0]), un(p[1]), un(p[2]), p[3]]);
    }
    Frame { width: pm.width(), height: pm.height(), rgba }
}

/// Copies a rendered frame into a premultiplied tiny-skia pixmap (what `main.rs` presents).
pub fn copy_to_pixmap(frame: &Frame, pm: &mut Pixmap) {
    if frame.width != pm.width() || frame.height != pm.height() {
        return;
    }
    for (dst, src) in pm.data_mut().chunks_exact_mut(4).zip(frame.rgba.chunks_exact(4)) {
        let a = src[3] as u32;
        for k in 0..3 {
            dst[k] = ((src[k] as u32 * a + 127) / 255) as u8;
        }
        dst[3] = src[3];
    }
}

/// Drawing calls in logical pixels, backed by an ANVL `Ui`. The methods mirror `gfx::Canvas` so the
/// chrome reads the same as before.
pub struct Painter<'a> {
    ui: &'a mut Ui,
    textures: &'a HashMap<String, TextureId>,
    pub s: f32,
    /// What translucent strokes are blended against. A stroke is drawn as one shape per segment, so
    /// drawing it translucent would show every overlap; it is drawn opaque in the pre-blended color.
    pub backdrop: Rgba,
}

impl Painter<'_> {
    fn r(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new(x * self.s, y * self.s, w * self.s, h * self.s)
    }

    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, c: Rgba) {
        if c.3 > 0 && w > 0.0 && h > 0.0 {
            self.ui.rect(self.r(x, y, w, h), RectStyle::fill(color(c)));
        }
    }

    pub fn rrect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, c: Rgba) {
        if c.3 > 0 && w > 0.0 && h > 0.0 {
            self.ui.rect(self.r(x, y, w, h), RectStyle::fill(color(c)).radius(radius.min(w / 2.0).min(h / 2.0) * self.s));
        }
    }

    /// A shape rounded on its top corners only (a tab). One polygon, so a translucent color does
    /// not double up where two shapes would overlap.
    pub fn rrect_top(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, c: Rgba) {
        let r = radius.min(w / 2.0).min(h);
        let mut pts = vec![[x, y + h]];
        pts.extend(arc_points(x + r, y + r, r, deg(180.0), deg(90.0)));
        pts.extend(arc_points(x + w - r, y + r, r, deg(270.0), deg(90.0)));
        pts.push([x + w, y + h]);
        self.fill_polygon(&pts, c);
    }

    /// Switches the layer new shapes go to (0 = base, 1 = blurs and veils, 2 = on top of them).
    pub fn layer(&mut self, n: usize) {
        self.ui.set_layer(n);
    }

    /// Shows what is on layer 0 beneath the box blurred; `amount` (0..1) is how much of the blur
    /// shows through, so a falling amount makes a progressive blur.
    pub fn blur_region(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, amount: f32) {
        if amount <= 0.01 || w <= 0.0 || h <= 0.0 {
            return;
        }
        let r = self.r(x, y, w, h);
        let prev = self.ui.draw.current;
        self.ui.set_layer(1);
        self.ui.draw.push_blur(anvil_ui::draw::RectInstance::new(r, RectStyle::fill([0.0, 0.0, 0.0, amount.clamp(0.0, 1.0)]).radius(radius * self.s)));
        self.ui.set_layer(prev);
    }

    /// Blurs everything drawn so far inside the box and dims it with `tint` (a locked chrome).
    pub fn veil(&mut self, x: f32, y: f32, w: f32, h: f32, tint: Rgba) {
        let r = self.r(x, y, w, h);
        self.ui.set_layer(1);
        self.ui.backdrop_blur(r, 0.0);
        self.ui.rect(r, RectStyle::fill(color(tint)));
        self.ui.set_layer(0);
    }

    /// A glowing animated orb (a custom shader) in `accent` over `base`, filling the box.
    #[allow(clippy::too_many_arguments)]
    pub fn orb(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, time: f32, energy: f32, accent: Rgba, base: Rgba) {
        let st = RectStyle::fill(color(base)).radius(radius * self.s).orb(time, energy, color(accent));
        self.ui.rect(self.r(x, y, w, h), st);
    }

    /// A rectangle filled with a left-to-right gradient from `c1` to `c2`.
    #[allow(clippy::too_many_arguments)]
    pub fn hgrad(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, c1: Rgba, c2: Rgba) {
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let st = RectStyle::fill(color(c1)).radius(radius.min(w / 2.0).min(h / 2.0) * self.s).linear(color(c2), 0.0);
        self.ui.rect(self.r(x, y, w, h), st);
    }

    /// A soft drop shadow for a rounded rectangle: `blur` px of falloff, shifted down by `dy`.
    #[allow(clippy::too_many_arguments)]
    pub fn shadow(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, blur: f32, dy: f32, alpha: f32) {
        let st = RectStyle::fill([0.0, 0.0, 0.0, alpha]).radius(radius * self.s).soft(blur * self.s);
        self.ui.rect(self.r(x, y + dy, w, h), st);
    }

    pub fn circle(&mut self, cx: f32, cy: f32, r: f32, c: Rgba) {
        self.rrect(cx - r, cy - r, r * 2.0, r * 2.0, r, c);
    }

    /// A line exactly one physical pixel thick on the pixel grid, so it stays crisp.
    pub fn hairline_h(&mut self, x: f32, y: f32, w: f32, c: Rgba) {
        let px = 1.0 / self.s;
        self.rect(x, (y * self.s).floor() / self.s, w, px, c);
    }

    pub fn hairline_v(&mut self, x: f32, y: f32, h: f32, c: Rgba) {
        let px = 1.0 / self.s;
        self.rect((x * self.s).floor() / self.s, y, px, h, c);
    }

    /// Draws the picture uploaded under `key`, stretched into the logical box. Returns false if
    /// there is none.
    pub fn image(&mut self, key: &str, x: f32, y: f32, w: f32, h: f32, alpha: f32) -> bool {
        self.picture(key, x, y, w, h, alpha, 0.0, Fit::Stretch)
    }

    /// Like `image`, but fills the box (cropping what sticks out) and rounds its corners.
    pub fn image_cover(&mut self, key: &str, x: f32, y: f32, w: f32, h: f32, radius: f32) -> bool {
        self.picture(key, x, y, w, h, 1.0, radius, Fit::Cover)
    }

    #[allow(clippy::too_many_arguments)]
    fn picture(&mut self, key: &str, x: f32, y: f32, w: f32, h: f32, alpha: f32, radius: f32, fit: Fit) -> bool {
        let Some(&id) = self.textures.get(key) else { return false };
        let r = self.r(x, y, w, h);
        let prev = self.ui.draw.opacity;
        self.ui.draw.opacity = prev * alpha.clamp(0.0, 1.0);
        self.ui.image(r, id, fit, radius);
        self.ui.draw.opacity = prev;
        true
    }

    /// The concave "foot" that joins a tab to the toolbar below it: a `r` square with a quarter
    /// disc cut out. `right` picks the side. Built from one-pixel rows, whose ends carry the curve.
    pub fn foot(&mut self, x: f32, bottom: f32, r: f32, right: bool, c: Rgba) {
        let rows = (r * self.s).round().max(1.0) as usize;
        let step = r / rows as f32;
        for i in 0..rows {
            let t = (i as f32 + 0.5) * step; // distance below the disc centre's row
            let hw = (r * r - t * t).max(0.0).sqrt();
            let y = bottom - r + i as f32 * step;
            if right {
                self.rect(x, y, r - hw, step, c);
            } else {
                self.rect(x + hw, y, r - hw, step, c);
            }
        }
    }

    /// A loading spinner: a three-quarter ring rotated by `turn` (0..1).
    pub fn spinner(&mut self, cx: f32, cy: f32, r: f32, turn: f32, c: Rgba) {
        let a0 = turn * std::f32::consts::TAU;
        let pts = arc_points(cx, cy, r, a0, std::f32::consts::TAU * 0.75);
        self.stroke(&pts, 2.0, c);
    }

    /// `c` blended onto the backdrop and made opaque (see `backdrop`).
    fn solid(&self, c: Rgba) -> Rgba {
        if c.3 == 255 || c.3 == 0 {
            return c;
        }
        let b = self.backdrop;
        Rgba(b.0, b.1, b.2, 255).mix(Rgba(c.0, c.1, c.2, 255), c.3 as f32 / 255.0)
    }

    fn stroke(&mut self, pts: &[[f32; 2]], width: f32, c: Rgba) {
        if pts.len() < 2 || c.3 == 0 {
            return;
        }
        let c = self.solid(c);
        let px: Vec<[f32; 2]> = pts.iter().map(|p| [p[0] * self.s, p[1] * self.s]).collect();
        self.ui.draw.stroke(&anvil_ui::path::Path::polyline(&px), width * self.s, color(c));
    }

    /// Draws `icon` with its grid scaled into a `size`-wide box at (x, y). The box is snapped to
    /// whole physical pixels so straight strokes stay crisp.
    pub fn icon(&mut self, icon: Icon, x: f32, y: f32, size: f32, c: Rgba) {
        self.icon_rot(icon, x, y, size, c, 0.0);
    }

    /// Like `icon`, turned by `angle` radians around the middle of its box.
    pub fn icon_rot(&mut self, icon: Icon, x: f32, y: f32, size: f32, c: Rgba, angle: f32) {
        let s = self.s;
        let (x, y) = ((x * s).round() / s, (y * s).round() / s);
        let size = ((size * s).round() / s).max(1.0 / s);
        let def = icon_def(icon);
        let k = size / def.grid;
        let fill_c = self.solid(c);
        let (sn, cs) = angle.sin_cos();
        let map = |p: [f32; 2]| {
            let (dx, dy) = (p[0] - def.grid / 2.0, p[1] - def.grid / 2.0);
            [x + (def.grid / 2.0 + dx * cs - dy * sn) * k, y + (def.grid / 2.0 + dx * sn + dy * cs) * k]
        };
        for poly in def.fills.iter() {
            self.fill_polygon(&poly.iter().map(|&p| map(p)).collect::<Vec<_>>(), fill_c);
        }
        let w = def.width * k * 1.08;
        for line in &def.strokes {
            self.stroke(&line.iter().map(|&p| map(p)).collect::<Vec<_>>(), w, c);
        }
    }

    /// Fills a polygon (even-odd) with one-pixel scan rows: exact (anti-aliased) horizontal edges,
    /// pixel-aligned vertical ones. Rows are a whole pixel tall because two half-pixel rows would
    /// blend to 75%, not 100%. Fine for the small glyphs this is used for.
    pub fn fill_polygon(&mut self, pts: &[[f32; 2]], c: Rgba) {
        if pts.len() < 3 || c.3 == 0 {
            return;
        }
        let (mut y0, mut y1) = (f32::MAX, f32::MIN);
        for p in pts {
            y0 = y0.min(p[1]);
            y1 = y1.max(p[1]);
        }
        let step = 1.0 / self.s;
        let mut y = (y0 / step).floor() * step;
        let mut xs: Vec<f32> = Vec::new();
        while y < y1 {
            let sy = y + step / 2.0;
            xs.clear();
            for i in 0..pts.len() {
                let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                if (a[1] <= sy) != (b[1] <= sy) {
                    xs.push(a[0] + (sy - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
                }
            }
            xs.sort_by(|a, b| a.total_cmp(b));
            for pair in xs.chunks_exact(2) {
                self.rect(pair[0], y, pair[1] - pair[0], step, c);
            }
            y += step;
        }
    }

    /// Single-line text with its top-left at (x, y), clipped at `clip_r` (logical x) with an
    /// ellipsis when it does not fit. Returns the width drawn.
    pub fn text(&mut self, text: &str, x: f32, y: f32, size: f32, weight: Weight, c: Rgba, clip_r: f32) -> f32 {
        if text.is_empty() || c.3 == 0 {
            return 0.0;
        }
        let st = style(size, weight, color(c));
        let (px, py) = (x * self.s, y * self.s);
        let max_w = (clip_r - x) * self.s;
        if clip_r >= 1.0e6 {
            return self.ui.text(px, py, text, st) / self.s;
        }
        self.ui.text_fit(px, py, text, st, max_w) / self.s
    }

    /// Like `text`, but never adds an ellipsis: whatever does not fit is cut off. For text that
    /// is being edited.
    pub fn text_cut(&mut self, text: &str, x: f32, y: f32, size: f32, weight: Weight, c: Rgba, clip: (f32, f32)) -> f32 {
        if text.is_empty() || c.3 == 0 {
            return 0.0;
        }
        let st = style(size, weight, color(c));
        let clip = Rect::new(clip.0 * self.s, 0.0, (clip.1 - clip.0) * self.s, 1.0e5);
        self.ui.text_clipped(x * self.s, y * self.s, text, st, clip) / self.s
    }

    /// Width of `text` in logical pixels.
    pub fn text_width(&mut self, text: &str, size: f32, weight: Weight) -> f32 {
        self.ui.text_width(text, style(size, weight, [1.0; 4])) / self.s
    }

    /// Height of one line of text at `size`, in logical pixels.
    pub fn line_height(&mut self, size: f32, weight: Weight) -> f32 {
        self.ui.measure("Ag", style(size, weight, [1.0; 4])).1.y / self.s
    }

}

// ---------------------------------------------------------------------------------------------
// Chrome icons: Tabler Icons (see `tabler.rs`), parsed into polylines for ANVL's GPU strokes.

struct IconDef {
    strokes: Vec<Vec<[f32; 2]>>,
    fills: Vec<Vec<[f32; 2]>>,
    /// Stroke width in grid units.
    width: f32,
    /// Side of the drawing grid.
    grid: f32,
}

const ALL_ICONS: [Icon; 35] = [
    Icon::Back, Icon::Forward, Icon::Reload, Icon::Stop, Icon::Close, Icon::Plus, Icon::Lock, Icon::Info,
    Icon::Search, Icon::Globe, Icon::Star, Icon::StarFilled, Icon::Bookmarks, Icon::Pip, Icon::History,
    Icon::Code, Icon::WinMin, Icon::WinMax, Icon::WinRestore, Icon::WinClose,
    Icon::ExternalLink, Icon::Copy, Icon::Link, Icon::Photo, Icon::Printer, Icon::Maximize, Icon::UserCircle, Icon::Users, Icon::Settings, Icon::ChevronRight, Icon::Check, Icon::Logout, Icon::ShieldLock, Icon::Trash, Icon::Sparkles,
];

fn icon_def(icon: Icon) -> &'static IconDef {
    static CACHE: std::sync::OnceLock<Vec<IconDef>> = std::sync::OnceLock::new();
    let defs = CACHE.get_or_init(|| {
        ALL_ICONS
            .iter()
            .map(|&ic| {
                let strokes: Vec<Vec<[f32; 2]>> = crate::tabler::paths(ic).iter().flat_map(|d| parse_path(d)).collect();
                let fills = if ic == Icon::StarFilled { strokes.clone() } else { Vec::new() };
                IconDef { strokes, fills, width: 2.0, grid: 24.0 }
            })
            .collect()
    });
    &defs[icon as usize]
}

fn arc_points(cx: f32, cy: f32, r: f32, start: f32, sweep: f32) -> Vec<[f32; 2]> {
    let n = ((sweep.abs() / std::f32::consts::TAU) * 64.0).ceil().max(4.0) as usize;
    (0..=n)
        .map(|i| {
            let a = start + sweep * i as f32 / n as f32;
            [cx + r * a.cos(), cy + r * a.sin()]
        })
        .collect()
}

fn deg(d: f32) -> f32 {
    d.to_radians()
}

/// Parses SVG path data (M L H V C S Q A Z, absolute and relative) into polylines, one per subpath.
fn parse_path(d: &str) -> Vec<Vec<[f32; 2]>> {
    struct Rd<'a> {
        b: &'a [u8],
        i: usize,
    }
    impl Rd<'_> {
        fn skip(&mut self) {
            while self.i < self.b.len() && (self.b[self.i] == b' ' || self.b[self.i] == b',' || self.b[self.i] == b'\n') {
                self.i += 1;
            }
        }
        fn more(&mut self) -> bool {
            self.skip();
            self.i < self.b.len() && !self.b[self.i].is_ascii_alphabetic()
        }
        fn num(&mut self) -> f32 {
            self.skip();
            let st = self.i;
            if self.i < self.b.len() && (self.b[self.i] == b'-' || self.b[self.i] == b'+') {
                self.i += 1;
            }
            let mut dot = false;
            while self.i < self.b.len() {
                let c = self.b[self.i];
                if c.is_ascii_digit() {
                    self.i += 1;
                } else if c == b'.' && !dot {
                    dot = true;
                    self.i += 1;
                } else {
                    break;
                }
            }
            std::str::from_utf8(&self.b[st..self.i]).ok().and_then(|t| t.parse().ok()).unwrap_or(0.0)
        }
        fn flag(&mut self) -> bool {
            self.skip();
            let v = self.i < self.b.len() && self.b[self.i] == b'1';
            self.i += 1;
            v
        }
    }
    let mut out: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut cur: Vec<[f32; 2]> = Vec::new();
    let (mut x, mut y, mut sx, mut sy) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let mut last_c: Option<[f32; 2]> = None;
    let mut rd = Rd { b: d.as_bytes(), i: 0 };
    let mut cmd = b'M';
    while rd.i < rd.b.len() {
        rd.skip();
        if rd.i >= rd.b.len() {
            break;
        }
        if rd.b[rd.i].is_ascii_alphabetic() {
            cmd = rd.b[rd.i];
            rd.i += 1;
        } else if cmd == b'M' || cmd == b'm' {
            cmd = if cmd == b'M' { b'L' } else { b'l' };
        }
        let rel = cmd.is_ascii_lowercase();
        let (ox, oy) = if rel { (x, y) } else { (0.0, 0.0) };
        match cmd.to_ascii_uppercase() {
            b'Z' => {
                if !cur.is_empty() {
                    cur.push([sx, sy]);
                    out.push(std::mem::take(&mut cur));
                }
                x = sx;
                y = sy;
                last_c = None;
            }
            b'M' => {
                if cur.len() > 1 {
                    out.push(std::mem::take(&mut cur));
                }
                cur.clear();
                x = ox + rd.num();
                y = oy + rd.num();
                sx = x;
                sy = y;
                cur.push([x, y]);
                last_c = None;
            }
            b'L' | b'H' | b'V' => {
                while rd.more() {
                    match cmd.to_ascii_uppercase() {
                        b'H' => x = ox_for(rel, x) + rd.num(),
                        b'V' => y = ox_for(rel, y) + rd.num(),
                        _ => {
                            let nx = ox_for(rel, x) + rd.num();
                            let ny = ox_for(rel, y) + rd.num();
                            x = nx;
                            y = ny;
                        }
                    }
                    if cur.is_empty() {
                        cur.push([sx, sy]);
                    }
                    cur.push([x, y]);
                }
                last_c = None;
            }
            b'C' | b'S' | b'Q' => {
                while rd.more() {
                    let (bx, by) = (if rel { x } else { 0.0 }, if rel { y } else { 0.0 });
                    let p0 = [x, y];
                    let (c1, c2, p3);
                    match cmd.to_ascii_uppercase() {
                        b'C' => {
                            c1 = [bx + rd.num(), by + rd.num()];
                            c2 = [bx + rd.num(), by + rd.num()];
                            p3 = [bx + rd.num(), by + rd.num()];
                        }
                        b'S' => {
                            c1 = last_c.map_or(p0, |l| [2.0 * x - l[0], 2.0 * y - l[1]]);
                            c2 = [bx + rd.num(), by + rd.num()];
                            p3 = [bx + rd.num(), by + rd.num()];
                        }
                        _ => {
                            let q = [bx + rd.num(), by + rd.num()];
                            p3 = [bx + rd.num(), by + rd.num()];
                            c1 = [p0[0] + 2.0 / 3.0 * (q[0] - p0[0]), p0[1] + 2.0 / 3.0 * (q[1] - p0[1])];
                            c2 = [p3[0] + 2.0 / 3.0 * (q[0] - p3[0]), p3[1] + 2.0 / 3.0 * (q[1] - p3[1])];
                        }
                    }
                    if cur.is_empty() {
                        cur.push(p0);
                    }
                    for i in 1..=24 {
                        let t = i as f32 / 24.0;
                        let u = 1.0 - t;
                        cur.push([
                            u * u * u * p0[0] + 3.0 * u * u * t * c1[0] + 3.0 * u * t * t * c2[0] + t * t * t * p3[0],
                            u * u * u * p0[1] + 3.0 * u * u * t * c1[1] + 3.0 * u * t * t * c2[1] + t * t * t * p3[1],
                        ]);
                    }
                    x = p3[0];
                    y = p3[1];
                    last_c = Some(c2);
                }
            }
            b'A' => {
                while rd.more() {
                    let (rx, ry, rot) = (rd.num(), rd.num(), rd.num());
                    let (large, sweep) = (rd.flag(), rd.flag());
                    let (ex, ey) = (ox_for(rel, x) + rd.num(), ox_for(rel, y) + rd.num());
                    if cur.is_empty() {
                        cur.push([x, y]);
                    }
                    cur.extend(arc_to(x, y, rx, ry, rot, large, sweep, ex, ey));
                    x = ex;
                    y = ey;
                }
                last_c = None;
            }
            _ => break,
        }
    }
    if cur.len() > 1 {
        out.push(cur);
    }
    out
}

fn ox_for(rel: bool, v: f32) -> f32 {
    if rel { v } else { 0.0 }
}

/// Points along an SVG elliptical arc from (x0, y0) to (x1, y1), excluding the start point.
#[allow(clippy::too_many_arguments)]
fn arc_to(x0: f32, y0: f32, rx: f32, ry: f32, rot_deg: f32, large: bool, sweep: bool, x1: f32, y1: f32) -> Vec<[f32; 2]> {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx < 1e-6 || ry < 1e-6 || (x0 == x1 && y0 == y1) {
        return vec![[x1, y1]];
    }
    let phi = rot_deg.to_radians();
    let (cp, sp) = (phi.cos(), phi.sin());
    let (dx, dy) = ((x0 - x1) / 2.0, (y0 - y1) / 2.0);
    let (x1p, y1p) = (cp * dx + sp * dy, -sp * dx + cp * dy);
    let lam = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lam > 1.0 {
        let k = lam.sqrt();
        rx *= k;
        ry *= k;
    }
    let num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let mut co = (num / den).max(0.0).sqrt();
    if large == sweep {
        co = -co;
    }
    let (cxp, cyp) = (co * rx * y1p / ry, -co * ry * x1p / rx);
    let (cx, cy) = (cp * cxp - sp * cyp + (x0 + x1) / 2.0, sp * cxp + cp * cyp + (y0 + y1) / 2.0);
    let ang = |ux: f32, uy: f32, vx: f32, vy: f32| (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
    let th1 = ang(1.0, 0.0, (x1p - cxp) / rx, (y1p - cyp) / ry);
    let mut dth = ang((x1p - cxp) / rx, (y1p - cyp) / ry, (-x1p - cxp) / rx, (-y1p - cyp) / ry);
    if !sweep && dth > 0.0 {
        dth -= std::f32::consts::TAU;
    } else if sweep && dth < 0.0 {
        dth += std::f32::consts::TAU;
    }
    let n = ((dth.abs() / std::f32::consts::TAU) * 64.0).ceil().max(3.0) as usize;
    (1..=n)
        .map(|i| {
            let t = th1 + dth * i as f32 / n as f32;
            let (ex, ey) = (rx * t.cos(), ry * t.sin());
            [cp * ex - sp * ey + cx, sp * ex + cp * ey + cy]
        })
        .collect()
}

/// A box-blurred copy of `src` (three passes approximate a Gaussian). `radius` is in pixels of `src`.
pub fn blur_pixmap(src: &Pixmap, radius: usize) -> Pixmap {
    let (w, h) = (src.width() as usize, src.height() as usize);
    let mut a: Vec<u8> = src.data().to_vec();
    let mut b = a.clone();
    let r = radius.max(1);
    for _ in 0..3 {
        for horizontal in [true, false] {
            let (len, lines) = if horizontal { (w, h) } else { (h, w) };
            for line in 0..lines {
                let idx = |i: usize| if horizontal { (line * w + i) * 4 } else { (i * w + line) * 4 };
                let mut acc = [0u32; 4];
                let win = |i: isize| i.clamp(0, len as isize - 1) as usize;
                for k in -(r as isize)..=(r as isize) {
                    let p = idx(win(k));
                    for c in 0..4 {
                        acc[c] += a[p + c] as u32;
                    }
                }
                let n = (2 * r + 1) as u32;
                for i in 0..len {
                    let o = idx(i);
                    for c in 0..4 {
                        b[o + c] = (acc[c] / n) as u8;
                    }
                    let (add, sub) = (idx(win(i as isize + r as isize + 1)), idx(win(i as isize - r as isize)));
                    for c in 0..4 {
                        acc[c] = acc[c] + a[add + c] as u32 - a[sub + c] as u32;
                    }
                }
            }
            std::mem::swap(&mut a, &mut b);
        }
    }
    let mut out = Pixmap::new(src.width(), src.height()).unwrap_or_else(|| src.clone());
    out.data_mut().copy_from_slice(&a);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blur_softens_an_edge_and_keeps_size() {
        let mut pm = Pixmap::new(20, 4).unwrap();
        for y in 0..4 {
            for x in 10..20 {
                let i = ((y * 20 + x) * 4) as usize;
                pm.data_mut()[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
        let out = blur_pixmap(&pm, 3);
        assert_eq!((out.width(), out.height()), (20, 4));
        let px = |x: usize| out.data()[(x) * 4];
        assert!(px(0) < 10 && px(19) > 245, "far pixels stay put");
        assert!(px(9) > 20 && px(9) < 235, "the edge is smoothed");
    }

    /// Draws a frame and checks the pixels. Skipped on machines with no GPU adapter.
    #[test]
    fn frame_draws_shapes_and_text() {
        let Some(mut e) = Engine::new() else { return };
        let f = e.frame(100, 40, 1.0, Rgba(0, 0, 0, 255), |p| {
            p.rect(0.0, 0.0, 10.0, 10.0, Rgba(255, 0, 0, 255));
            p.text("Hi", 20.0, 5.0, 13.0, Weight::Regular, Rgba(255, 255, 255, 255), 1.0e9);
        });
        let px = |x: usize, y: usize| &f.rgba[(y * 100 + x) * 4..(y * 100 + x) * 4 + 4];
        assert_eq!(px(5, 5), [255, 0, 0, 255]);
        assert_eq!(px(90, 35), [0, 0, 0, 255]);
        assert!(f.rgba.chunks_exact(4).skip(20).any(|p| p[0] > 100 && p[1] > 100), "text drew nothing");
    }

    /// The vendored shader has Persona's own orb branch: a WGSL mistake there makes every frame
    /// blank (the pipeline fails to build), so check it draws, on dark and on light.
    #[test]
    fn orb_shader_draws_on_dark_and_light() {
        let Some(mut e) = Engine::new() else { return };
        for base in [Rgba(30, 30, 34, 255), Rgba(255, 255, 255, 255)] {
            let f = e.frame(120, 80, 1.0, base, |p| {
                p.orb(0.0, 0.0, 120.0, 80.0, 0.0, 1.0, 1.0, Rgba(30, 200, 230, 255), base);
            });
            let differs = f.rgba.chunks_exact(4).any(|p| (p[0] as i32 - base.0 as i32).abs() > 30 || (p[2] as i32 - base.2 as i32).abs() > 30);
            assert!(differs, "the orb drew nothing over {base:?}");
        }
    }

    /// Renders icons, text and shapes headlessly: `CEDERIC_SHOT=<dir> cargo test render_anvil -- --ignored`.
    #[test]
    #[ignore]
    fn render_anvil() {
        let dir = std::path::PathBuf::from(std::env::var("CEDERIC_SHOT").expect("CEDERIC_SHOT"));
        let Some(mut e) = Engine::new() else { panic!("no GPU adapter") };
        for (scale, name) in [(1.0f32, "anvil-1x"), (2.0, "anvil-2x")] {
            let (w, h) = ((420.0 * scale) as u32, (130.0 * scale) as u32);
            let f = e.frame(w, h, scale, Rgba(29, 30, 34, 255), |p| {
                let fg = Rgba(236, 236, 240, 255);
                p.text("Search or type a URL — 0123 Wi-Fi", 10.0, 8.0, 13.5, Weight::Regular, fg, 1.0e9);
                for (i, ic) in [
                    Icon::Back, Icon::Forward, Icon::Reload, Icon::Stop, Icon::Lock, Icon::Info, Icon::Search, Icon::Globe,
                    Icon::Star, Icon::StarFilled, Icon::Bookmarks, Icon::Pip, Icon::History, Icon::Code, Icon::Close,
                ]
                .into_iter()
                .enumerate()
                {
                    p.icon(ic, 10.0 + (i % 8) as f32 * 40.0, 40.0 + (i / 8) as f32 * 36.0, 20.0, fg);
                }
                p.foot(350.0, 120.0, 10.0, false, Rgba(60, 60, 70, 255));
                p.foot(380.0, 120.0, 10.0, true, Rgba(60, 60, 70, 255));
                p.spinner(400.0, 20.0, 6.0, 0.1, Rgba(255, 106, 61, 255));
            });
            let mut pm = Pixmap::new(f.width, f.height).unwrap();
            copy_to_pixmap(&f, &mut pm);
            pm.save_png(dir.join(format!("{name}.png"))).unwrap();
        }
    }
}
