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
    Color, Fit, Frame, Icon as AnvilIcon, IconAtlas, Input, Rect, RectStyle, Renderer, TextStyle, TextureId, Ui,
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

/// The GPU, the text system and the pictures uploaded to it for one drawing surface.
pub struct Engine {
    ui: Ui,
    renderer: Renderer,
    /// Uploaded pictures by caller key (favicon URL, `"logo"`, ...).
    textures: HashMap<String, TextureId>,
    clock: f32,
}

impl Engine {
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
            p.shadow(x, y, cw, ch, r, 44.0, 14.0, strength * 0.7);
            p.shadow(x, y, cw, ch, r, 12.0, 3.0, strength * 0.5);
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

    /// A rounded rectangle filled with a linear gradient from `c1` to `c2` at `angle` radians, with
    /// an optional border.
    #[allow(clippy::too_many_arguments)]
    pub fn rrect_gradient(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, c1: Rgba, c2: Rgba, angle: f32, border: Option<(Rgba, f32)>) {
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let mut st = RectStyle::fill(color(c1)).radius(radius.min(w / 2.0).min(h / 2.0) * self.s).linear(color(c2), angle);
        if let Some((bc, bw)) = border {
            st = st.border(color(bc), bw * self.s);
        }
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
        let s = self.s;
        let (x, y) = ((x * s).round() / s, (y * s).round() / s);
        let size = ((size * s).round() / s).max(1.0 / s);
        if let Some(a) = atlas_icon(icon) {
            // ANVL's own Lucide icon: a texture, so translucency is exact.
            if c.3 > 0 {
                self.ui.icon(Rect::new(x * s, y * s, size * s, size * s), a, color(c));
            }
            return;
        }
        let def = icon_def(icon);
        let k = size / def.grid;
        let fill_c = self.solid(c);
        let map = |p: [f32; 2]| [x + p[0] * k, y + p[1] * k];
        for poly in &def.fills {
            self.fill_polygon(&poly.iter().map(|&p| map(p)).collect::<Vec<_>>(), fill_c);
        }
        let w = def.width * k * if def.grid > 20.0 { 1.2 } else { 1.0 };
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

/// The chrome icons that ANVL's own atlas has an exact match for.
fn atlas_icon(icon: Icon) -> Option<AnvilIcon> {
    Some(match icon {
        Icon::Back => AnvilIcon::ArrowLeft,
        Icon::Forward => AnvilIcon::ArrowRight,
        Icon::Plus => AnvilIcon::Plus,
        Icon::Close | Icon::Stop => AnvilIcon::X,
        Icon::Search => AnvilIcon::Search,
        Icon::Info => AnvilIcon::Info,
        Icon::Code => AnvilIcon::Code,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------------------------
// Chrome icons: the same drawings the tiny-skia UI used, as polylines for ANVL's GPU strokes.

struct IconDef {
    strokes: Vec<Vec<[f32; 2]>>,
    fills: Vec<Vec<[f32; 2]>>,
    /// Stroke width in grid units.
    width: f32,
    /// Side of the drawing grid.
    grid: f32,
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

fn ring(cx: f32, cy: f32, r: f32) -> Vec<[f32; 2]> {
    arc_points(cx, cy, r, 0.0, std::f32::consts::TAU)
}

fn deg(d: f32) -> f32 {
    d.to_radians()
}

fn line(pts: &[(f32, f32)]) -> Vec<[f32; 2]> {
    pts.iter().map(|&(x, y)| [x, y]).collect()
}

fn closed(pts: &[(f32, f32)]) -> Vec<[f32; 2]> {
    let mut v = line(pts);
    if let Some(&f) = v.first() {
        v.push(f);
    }
    v
}

fn rrect_outline(x: f32, y: f32, w: f32, h: f32, r: f32) -> Vec<[f32; 2]> {
    let mut v = Vec::new();
    for (cx, cy, a0) in [(x + w - r, y + r, -90.0), (x + w - r, y + h - r, 0.0), (x + r, y + h - r, 90.0), (x + r, y + r, 180.0)] {
        v.extend(arc_points(cx, cy, r, deg(a0), deg(90.0)));
    }
    if let Some(&f) = v.first() {
        v.push(f);
    }
    v
}

fn icon_def(icon: Icon) -> IconDef {
    use std::f32::consts::PI;
    let mut d = IconDef { strokes: Vec::new(), fills: Vec::new(), width: 1.9, grid: 24.0 };
    match icon {
        Icon::Back => {
            d.strokes = vec![line(&[(19.0, 12.0), (5.0, 12.0)]), line(&[(11.0, 18.0), (5.0, 12.0), (11.0, 6.0)])];
        }
        Icon::Forward => {
            d.strokes = vec![line(&[(5.0, 12.0), (19.0, 12.0)]), line(&[(13.0, 18.0), (19.0, 12.0), (13.0, 6.0)])];
        }
        Icon::Reload => {
            let mut a = arc_points(12.0, 12.0, 8.0, 0.0, deg(315.0));
            a.push([20.0, 8.5]);
            d.strokes = vec![a, line(&[(20.0, 4.0), (20.0, 8.5), (15.5, 8.5)])];
        }
        Icon::Stop => d.strokes = vec![line(&[(17.0, 7.0), (7.0, 17.0)]), line(&[(7.0, 7.0), (17.0, 17.0)])],
        Icon::Close => {
            d.strokes = vec![line(&[(6.0, 6.0), (18.0, 18.0)]), line(&[(18.0, 6.0), (6.0, 18.0)])];
            d.width = 2.4;
        }
        Icon::Plus => d.strokes = vec![line(&[(12.0, 5.0), (12.0, 19.0)]), line(&[(5.0, 12.0), (19.0, 12.0)])],
        Icon::Lock => {
            let mut shackle = line(&[(8.0, 11.0), (8.0, 8.0)]);
            shackle.extend(arc_points(12.0, 8.0, 4.0, deg(180.0), PI));
            shackle.push([16.0, 11.0]);
            d.strokes = vec![rrect_outline(5.0, 11.0, 14.0, 10.0, 2.0), shackle];
        }
        Icon::Info => {
            d.strokes = vec![ring(12.0, 12.0, 9.0), line(&[(12.0, 16.0), (12.0, 11.0)])];
            d.fills = vec![ring(12.0, 8.0, 1.2)];
        }
        Icon::Search => d.strokes = vec![ring(11.0, 11.0, 7.0), line(&[(20.0, 20.0), (16.0, 16.0)])],
        Icon::Globe => {
            let oval: Vec<[f32; 2]> = ring(0.0, 0.0, 1.0).into_iter().map(|p| [12.0 + p[0] * 4.5, 12.0 + p[1] * 9.0]).collect();
            d.strokes = vec![ring(12.0, 12.0, 9.0), line(&[(3.0, 12.0), (21.0, 12.0)]), oval];
        }
        Icon::Star | Icon::StarFilled => {
            let star = [
                (12.0, 3.2), (14.8, 8.9), (21.0, 9.8), (16.5, 14.2), (17.6, 20.4),
                (12.0, 17.4), (6.4, 20.4), (7.5, 14.2), (3.0, 9.8), (9.2, 8.9),
            ];
            d.strokes = vec![closed(&star)];
            d.width = 1.8;
            if icon == Icon::StarFilled {
                d.fills = vec![line(&star)];
            }
        }
        Icon::Bookmarks => {
            d.strokes = vec![closed(&[(6.0, 4.0), (18.0, 4.0), (18.0, 20.5), (12.0, 16.5), (6.0, 20.5)])];
        }
        Icon::Pip => {
            d.strokes = vec![line(&[(21.0, 10.0), (21.0, 5.5), (3.0, 5.5), (3.0, 18.5), (10.5, 18.5)])];
            d.fills = vec![rrect_outline(12.5, 12.5, 8.5, 6.5, 1.5)];
        }
        Icon::History => {
            d.strokes = vec![
                arc_points(12.0, 12.0, 9.0, deg(180.0), deg(312.0)),
                line(&[(3.0, 4.0), (3.0, 8.5), (7.5, 8.5)]),
                line(&[(12.0, 7.0), (12.0, 12.0), (15.0, 14.0)]),
            ];
        }
        Icon::Code => d.strokes = vec![line(&[(8.0, 7.0), (3.0, 12.0), (8.0, 17.0)]), line(&[(16.0, 7.0), (21.0, 12.0), (16.0, 17.0)])],
        Icon::WinMin => {
            d.grid = 10.0;
            d.width = 1.0;
            d.strokes = vec![line(&[(0.0, 5.0), (10.0, 5.0)])];
        }
        Icon::WinMax => {
            d.grid = 10.0;
            d.width = 1.0;
            d.strokes = vec![closed(&[(0.5, 0.5), (9.5, 0.5), (9.5, 9.5), (0.5, 9.5)])];
        }
        Icon::WinRestore => {
            d.grid = 10.0;
            d.width = 1.0;
            d.strokes = vec![
                closed(&[(0.5, 2.5), (7.5, 2.5), (7.5, 9.5), (0.5, 9.5)]),
                line(&[(2.5, 2.5), (2.5, 0.5), (9.5, 0.5), (9.5, 7.5), (7.5, 7.5)]),
            ];
        }
        Icon::WinClose => {
            d.grid = 10.0;
            d.width = 1.0;
            d.strokes = vec![line(&[(0.0, 0.0), (10.0, 10.0)]), line(&[(10.0, 0.0), (0.0, 10.0)])];
        }
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

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
