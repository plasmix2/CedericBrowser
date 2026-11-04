//! Drawing primitives for the native UI: a tiny-skia canvas in physical pixels, shapes, vector
//! icons, and text through cosmic-text. Everything takes logical coordinates and scales by `s`.

use cosmic_text::{Attrs, Buffer, Color as TextColor, Family, FontSystem, Metrics, Shaping, SwashCache, Weight};
use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke,
    Transform,
};

/// An sRGB color with straight (not premultiplied) alpha.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

impl Rgba {
    pub const fn hex(v: u32) -> Self {
        Self((v >> 16) as u8, (v >> 8) as u8, v as u8, 255)
    }
    pub fn with_alpha(self, a: f32) -> Self {
        Self(self.0, self.1, self.2, (self.3 as f32 * a.clamp(0.0, 1.0)).round() as u8)
    }
    /// Linear blend from `self` to `o` by `t`.
    pub fn mix(self, o: Self, t: f32) -> Self {
        let l = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t.clamp(0.0, 1.0)).round() as u8;
        Self(l(self.0, o.0), l(self.1, o.1), l(self.2, o.2), l(self.3, o.3))
    }
    fn paint(self) -> Paint<'static> {
        let mut p = Paint::default();
        p.set_color(Color::from_rgba8(self.0, self.1, self.2, self.3));
        p.anti_alias = true;
        p
    }
}

#[derive(Clone, Copy)]
pub struct Theme {
    pub strip: Rgba,
    pub toolbar: Rgba,
    pub tab_hover: Rgba,
    pub fg: Rgba,
    pub dim: Rgba,
    pub faint: Rgba,
    pub field: Rgba,
    pub field_hover: Rgba,
    pub border: Rgba,
    pub accent: Rgba,
    pub accent2: Rgba,
    pub btn_hover: Rgba,
    pub btn_press: Rgba,
    pub close: Rgba,
    pub warn: Rgba,
    pub selection: Rgba,
}

impl Theme {
    pub fn new(dark: bool) -> Self {
        if dark {
            Self {
                strip: Rgba::hex(0x18191d),
                toolbar: Rgba::hex(0x26272c),
                tab_hover: Rgba::hex(0x222328),
                fg: Rgba::hex(0xececf0),
                dim: Rgba::hex(0x9a9ca6),
                faint: Rgba::hex(0x5d606a),
                field: Rgba::hex(0x18191d),
                field_hover: Rgba::hex(0x1d1e22),
                border: Rgba::hex(0x34363d),
                accent: Rgba::hex(0xff6a3d),
                accent2: Rgba::hex(0xffb03d),
                btn_hover: Rgba(255, 255, 255, 20),
                btn_press: Rgba(255, 255, 255, 33),
                close: Rgba::hex(0xe81123),
                warn: Rgba::hex(0xf0a03c),
                selection: Rgba(255, 106, 61, 102),
            }
        } else {
            Self {
                strip: Rgba::hex(0xdfe1e6),
                toolbar: Rgba::hex(0xf7f7f9),
                tab_hover: Rgba::hex(0xe9eaee),
                fg: Rgba::hex(0x1d1e22),
                dim: Rgba::hex(0x5c5f68),
                faint: Rgba::hex(0x9fa2ab),
                field: Rgba::hex(0xe9eaee),
                field_hover: Rgba::hex(0xe2e3e8),
                border: Rgba::hex(0xd2d4da),
                accent: Rgba::hex(0xff6a3d),
                accent2: Rgba::hex(0xffb03d),
                btn_hover: Rgba(0, 0, 0, 15),
                btn_press: Rgba(0, 0, 0, 26),
                close: Rgba::hex(0xe81123),
                warn: Rgba::hex(0xd9822b),
                selection: Rgba(255, 106, 61, 90),
            }
        }
    }
}

/// The vector icons, drawn in a 24-unit grid (window controls use a 10-unit grid).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Back,
    Forward,
    Reload,
    Stop,
    Close,
    Plus,
    Lock,
    Info,
    Search,
    Globe,
    History,
    Code,
    WinMin,
    WinMax,
    WinRestore,
    WinClose,
}

/// A canvas in physical pixels with a logical-to-physical scale `s`.
pub struct Canvas<'a> {
    pub pm: &'a mut Pixmap,
    pub s: f32,
}

impl Canvas<'_> {
    fn t(&self) -> Transform {
        Transform::from_scale(self.s, self.s)
    }

    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, c: Rgba) {
        if let Some(r) = Rect::from_xywh(x, y, w, h) {
            self.pm.fill_rect(r, &c.paint(), self.t(), None);
        }
    }

    /// A line exactly one physical pixel thick, snapped to the pixel grid so it stays crisp.
    pub fn hairline_h(&mut self, x: f32, y: f32, w: f32, c: Rgba) {
        let py = (y * self.s).floor();
        if let Some(r) = Rect::from_xywh((x * self.s).round(), py, (w * self.s).round(), 1.0) {
            self.pm.fill_rect(r, &c.paint(), Transform::identity(), None);
        }
    }

    pub fn hairline_v(&mut self, x: f32, y: f32, h: f32, c: Rgba) {
        let px = (x * self.s).floor();
        if let Some(r) = Rect::from_xywh(px, (y * self.s).round(), 1.0, (h * self.s).round()) {
            self.pm.fill_rect(r, &c.paint(), Transform::identity(), None);
        }
    }

    /// Rounded rectangle with per-corner radii (top-left, top-right, bottom-right, bottom-left).
    pub fn rrect(&mut self, x: f32, y: f32, w: f32, h: f32, r: [f32; 4], c: Rgba) {
        if let Some(p) = rrect_path(x, y, w, h, r) {
            self.pm.fill_path(&p, &c.paint(), FillRule::Winding, self.t(), None);
        }
    }

    pub fn circle(&mut self, cx: f32, cy: f32, r: f32, c: Rgba) {
        if let Some(p) = PathBuilder::from_circle(cx, cy, r) {
            self.pm.fill_path(&p, &c.paint(), FillRule::Winding, self.t(), None);
        }
    }

    pub fn fill(&mut self, p: &Path, c: Rgba) {
        self.pm.fill_path(p, &c.paint(), FillRule::Winding, self.t(), None);
    }

    pub fn stroke(&mut self, p: &Path, width: f32, c: Rgba) {
        let st = Stroke { width, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Stroke::default() };
        self.pm.stroke_path(p, &c.paint(), &st, self.t(), None);
    }

    /// The concave "foot" that joins a tab to the toolbar below it. `right` picks the side.
    pub fn foot(&mut self, x: f32, bottom: f32, r: f32, right: bool, c: Rgba) {
        let k = 0.552_284_8 * r;
        let mut pb = PathBuilder::new();
        if right {
            // Filled on the left (touching the tab), curving up to the right.
            pb.move_to(x, bottom - r);
            pb.line_to(x, bottom);
            pb.line_to(x + r, bottom);
            pb.cubic_to(x + r - k, bottom, x, bottom - r + k, x, bottom - r);
        } else {
            pb.move_to(x + r, bottom - r);
            pb.line_to(x + r, bottom);
            pb.line_to(x, bottom);
            pb.cubic_to(x + k, bottom, x + r, bottom - r + k, x + r, bottom - r);
        }
        pb.close();
        if let Some(p) = pb.finish() {
            self.fill(&p, c);
        }
    }

    /// Draws `icon` with its grid scaled into a `size`-wide box at (x, y).
    pub fn icon(&mut self, icon: Icon, x: f32, y: f32, size: f32, c: Rgba) {
        let grid = if matches!(icon, Icon::WinMin | Icon::WinMax | Icon::WinRestore | Icon::WinClose) { 10.0 } else { 24.0 };
        let k = size / grid;
        let (paths, width) = icon_paths(icon);
        let st = Stroke { width: width * k, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Stroke::default() };
        let t = Transform::from_row(k, 0.0, 0.0, k, x, y).post_scale(self.s, self.s);
        for p in paths.iter().flatten() {
            self.pm.stroke_path(p, &c.paint(), &st, t, None);
        }
        if icon == Icon::Info {
            // The dot of the "i".
            if let Some(p) = PathBuilder::from_circle(12.0, 8.0, 1.2) {
                self.pm.fill_path(&p, &c.paint(), FillRule::Winding, t, None);
            }
        }
    }

    /// A loading spinner: a 3/4 ring rotated by `turn` (0..1).
    pub fn spinner(&mut self, cx: f32, cy: f32, r: f32, turn: f32, c: Rgba) {
        let a0 = turn * std::f32::consts::TAU;
        if let Some(p) = arc_path(cx, cy, r, a0, a0 + std::f32::consts::TAU * 0.75) {
            self.stroke(&p, 2.0, c);
        }
    }

    /// Draws an image scaled into the logical box (x, y, w, h).
    pub fn image(&mut self, img: &Pixmap, x: f32, y: f32, w: f32, h: f32, alpha: f32) {
        let sx = w * self.s / img.width() as f32;
        let sy = h * self.s / img.height() as f32;
        let paint = PixmapPaint { opacity: alpha, quality: tiny_skia::FilterQuality::Bicubic, ..PixmapPaint::default() };
        let t = Transform::from_row(sx, 0.0, 0.0, sy, x * self.s, y * self.s);
        self.pm.draw_pixmap(0, 0, img.as_ref(), &paint, t, None);
    }

    /// Draws a laid-out text buffer with its top-left at logical (x, y). Glyphs past `clip_r`
    /// are dropped and the last `fade` logical pixels before it fade out.
    pub fn text(&mut self, fonts: &mut Fonts, buf: &mut Buffer, x: f32, y: f32, c: Rgba, clip_r: f32, fade: f32) {
        let (ox, oy) = ((x * self.s).round() as i32, (y * self.s).round() as i32);
        let clip = (clip_r * self.s) as i32;
        let fade_px = (fade * self.s).max(1.0);
        let color = TextColor::rgba(c.0, c.1, c.2, c.3);
        let (w, h) = (self.pm.width() as i32, self.pm.height() as i32);
        let data = self.pm.data_mut();
        buf.draw(&mut fonts.system, &mut fonts.cache, color, |gx, gy, gw, gh, col| {
            for py in oy + gy..oy + gy + gh as i32 {
                for px in ox + gx..ox + gx + gw as i32 {
                    if px < 0 || py < 0 || px >= w || py >= h || px >= clip {
                        continue;
                    }
                    let fade_a = if fade > 0.0 { ((clip - px) as f32 / fade_px).clamp(0.0, 1.0) } else { 1.0 };
                    blend(data, (py * w + px) as usize, col.r(), col.g(), col.b(), col.a() as f32 * fade_a / 255.0);
                }
            }
        });
    }
}

/// Source-over blend of a cosmic-text color into premultiplied RGBA pixel `i`.
pub fn blend_px(data: &mut [u8], i: usize, c: TextColor) {
    blend(data, i, c.r(), c.g(), c.b(), c.a() as f32 / 255.0);
}

/// Source-over blend of a straight-alpha color into premultiplied RGBA pixel `i`.
fn blend(data: &mut [u8], i: usize, r: u8, g: u8, b: u8, a: f32) {
    if a <= 0.0 {
        return;
    }
    let p = &mut data[i * 4..i * 4 + 4];
    let inv = 1.0 - a;
    p[0] = (r as f32 * a + p[0] as f32 * inv) as u8;
    p[1] = (g as f32 * a + p[1] as f32 * inv) as u8;
    p[2] = (b as f32 * a + p[2] as f32 * inv) as u8;
    p[3] = (255.0 * a + p[3] as f32 * inv) as u8;
}

pub fn rrect_path(x: f32, y: f32, w: f32, h: f32, r: [f32; 4]) -> Option<Path> {
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let lim = (w.min(h)) / 2.0;
    let [tl, tr, br, bl] = r.map(|v| v.min(lim));
    let k = 0.552_284_8;
    let mut pb = PathBuilder::new();
    pb.move_to(x + tl, y);
    pb.line_to(x + w - tr, y);
    pb.cubic_to(x + w - tr + tr * k, y, x + w, y + tr - tr * k, x + w, y + tr);
    pb.line_to(x + w, y + h - br);
    pb.cubic_to(x + w, y + h - br + br * k, x + w - br + br * k, y + h, x + w - br, y + h);
    pb.line_to(x + bl, y + h);
    pb.cubic_to(x + bl - bl * k, y + h, x, y + h - bl + bl * k, x, y + h - bl);
    pb.line_to(x, y + tl);
    pb.cubic_to(x, y + tl - tl * k, x + tl - tl * k, y, x + tl, y);
    pb.close();
    pb.finish()
}

/// Circular arc from angle `a0` to `a1` (radians, y down, clockwise), as cubic segments.
fn arc_path(cx: f32, cy: f32, r: f32, a0: f32, a1: f32) -> Option<Path> {
    let mut pb = PathBuilder::new();
    push_arc(&mut pb, cx, cy, r, a0, a1, true);
    pb.finish()
}

fn push_arc(pb: &mut PathBuilder, cx: f32, cy: f32, r: f32, a0: f32, a1: f32, start: bool) {
    let n = ((a1 - a0).abs() / (std::f32::consts::FRAC_PI_2)).ceil().max(1.0) as usize;
    let step = (a1 - a0) / n as f32;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    let pt = |a: f32| (cx + r * a.cos(), cy + r * a.sin());
    let (sx, sy) = pt(a0);
    if start {
        pb.move_to(sx, sy);
    } else {
        pb.line_to(sx, sy);
    }
    for i in 0..n {
        let (b0, b1) = (a0 + step * i as f32, a0 + step * (i + 1) as f32);
        let (x0, y0) = pt(b0);
        let (x1, y1) = pt(b1);
        pb.cubic_to(
            x0 - k * r * b0.sin(),
            y0 + k * r * b0.cos(),
            x1 + k * r * b1.sin(),
            y1 - k * r * b1.cos(),
            x1,
            y1,
        );
    }
}

fn poly(points: &[(f32, f32)]) -> Option<Path> {
    let mut pb = PathBuilder::new();
    let (first, rest) = points.split_first()?;
    pb.move_to(first.0, first.1);
    for p in rest {
        pb.line_to(p.0, p.1);
    }
    pb.finish()
}

fn rect_outline(x: f32, y: f32, w: f32, h: f32) -> Option<Path> {
    let mut pb = PathBuilder::new();
    pb.push_rect(Rect::from_xywh(x, y, w, h)?);
    pb.finish()
}

/// Paths and stroke width (in grid units) for each icon.
fn icon_paths(icon: Icon) -> (Vec<Option<Path>>, f32) {
    use std::f32::consts::PI;
    let deg = |d: f32| d * PI / 180.0;
    match icon {
        Icon::Back => (vec![poly(&[(19.0, 12.0), (5.0, 12.0)]), poly(&[(11.0, 18.0), (5.0, 12.0), (11.0, 6.0)])], 1.9),
        Icon::Forward => (vec![poly(&[(5.0, 12.0), (19.0, 12.0)]), poly(&[(13.0, 18.0), (19.0, 12.0), (13.0, 6.0)])], 1.9),
        Icon::Reload => {
            // Open circle with an arrow head at its top-right end.
            let mut pb = PathBuilder::new();
            push_arc(&mut pb, 12.0, 12.0, 8.0, deg(0.0), deg(315.0), true);
            pb.line_to(20.0, 8.5);
            (vec![pb.finish(), poly(&[(20.0, 4.0), (20.0, 8.5), (15.5, 8.5)])], 1.9)
        }
        Icon::Stop => (vec![poly(&[(17.0, 7.0), (7.0, 17.0)]), poly(&[(7.0, 7.0), (17.0, 17.0)])], 1.9),
        Icon::Close => (vec![poly(&[(6.0, 6.0), (18.0, 18.0)]), poly(&[(18.0, 6.0), (6.0, 18.0)])], 2.4),
        Icon::Plus => (vec![poly(&[(12.0, 5.0), (12.0, 19.0)]), poly(&[(5.0, 12.0), (19.0, 12.0)])], 1.9),
        Icon::Lock => {
            let mut shackle = PathBuilder::new();
            shackle.move_to(8.0, 11.0);
            shackle.line_to(8.0, 8.0);
            push_arc(&mut shackle, 12.0, 8.0, 4.0, deg(180.0), deg(360.0), false);
            shackle.line_to(16.0, 11.0);
            (vec![rrect_path(5.0, 11.0, 14.0, 10.0, [2.0; 4]), shackle.finish()], 1.9)
        }
        Icon::Info => (vec![PathBuilder::from_circle(12.0, 12.0, 9.0), poly(&[(12.0, 16.0), (12.0, 11.0)])], 1.9),
        Icon::Search => (vec![PathBuilder::from_circle(11.0, 11.0, 7.0), poly(&[(20.0, 20.0), (16.0, 16.0)])], 1.9),
        Icon::Globe => (
            vec![
                PathBuilder::from_circle(12.0, 12.0, 9.0),
                poly(&[(3.0, 12.0), (21.0, 12.0)]),
                Rect::from_xywh(7.5, 3.0, 9.0, 18.0).and_then(PathBuilder::from_oval),
            ],
            1.9,
        ),
        Icon::History => {
            let mut pb = PathBuilder::new();
            push_arc(&mut pb, 12.0, 12.0, 9.0, deg(180.0), deg(180.0 + 312.0), true);
            (vec![pb.finish(), poly(&[(3.0, 4.0), (3.0, 8.5), (7.5, 8.5)]), poly(&[(12.0, 7.0), (12.0, 12.0), (15.0, 14.0)])], 1.9)
        }
        Icon::Code => (vec![poly(&[(8.0, 7.0), (3.0, 12.0), (8.0, 17.0)]), poly(&[(16.0, 7.0), (21.0, 12.0), (16.0, 17.0)])], 1.9),
        Icon::WinMin => (vec![poly(&[(0.0, 5.0), (10.0, 5.0)])], 1.0),
        Icon::WinMax => (vec![rect_outline(0.5, 0.5, 9.0, 9.0)], 1.0),
        Icon::WinRestore => (vec![rect_outline(0.5, 2.5, 7.0, 7.0), poly(&[(2.5, 2.5), (2.5, 0.5), (9.5, 0.5), (9.5, 7.5), (7.5, 7.5)])], 1.0),
        Icon::WinClose => (vec![poly(&[(0.0, 0.0), (10.0, 10.0)]), poly(&[(10.0, 0.0), (0.0, 10.0)])], 1.0),
    }
}

/// Fonts and the glyph cache. Only the handful of system fonts the UI needs are loaded, which
/// keeps startup fast compared with scanning every installed font.
pub struct Fonts {
    pub system: FontSystem,
    pub cache: SwashCache,
}

pub const UI_FAMILY: &str = "Segoe UI";

impl Fonts {
    pub fn load() -> Self {
        let mut db = cosmic_text::fontdb::Database::new();
        let dir = std::env::var_os("WINDIR").map_or_else(|| "C:\\Windows".into(), std::path::PathBuf::from).join("Fonts");
        for f in [
            "segoeui.ttf", "seguisb.ttf", "segoeuib.ttf", "seguisym.ttf", "seguiemj.ttf",
            "msyh.ttc", "YuGothM.ttc", "malgun.ttf", "Nirmala.ttf", "ebrima.ttf", "tahoma.ttf",
        ] {
            let _ = db.load_font_file(dir.join(f));
        }
        let locale = sys_locale().unwrap_or_else(|| "en-US".into());
        Self { system: FontSystem::new_with_locale_and_db(locale, db), cache: SwashCache::new() }
    }

    /// A single-line text buffer in physical pixels.
    pub fn line(&mut self, text: &str, size: f32, weight: Weight, s: f32) -> Buffer {
        let mut b = Buffer::new(&mut self.system, Metrics::new(size * s, size * 1.35 * s));
        b.set_wrap(cosmic_text::Wrap::None);
        b.set_size(None, None);
        b.set_text(text, &attrs(weight), Shaping::Advanced, None);
        b.shape_until_scroll(&mut self.system, false);
        b
    }
}

pub fn attrs(weight: Weight) -> Attrs<'static> {
    Attrs::new().family(Family::Name(UI_FAMILY)).weight(weight)
}

fn sys_locale() -> Option<String> {
    std::env::var("LANG").ok().filter(|l| !l.is_empty())
}

/// Logical width of a laid-out single-line buffer.
pub fn text_width(b: &Buffer, s: f32) -> f32 {
    b.layout_runs().map(|r| r.line_w).fold(0.0, f32::max) / s
}
