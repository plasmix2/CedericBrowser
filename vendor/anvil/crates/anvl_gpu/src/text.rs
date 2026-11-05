//! TrueType text via cosmic-text (shaping and layout) and swash (rasterizing), with fonts
//! embedded in the binary. The glyphs are drawn by `glyphs` and the renderer.
//!
//! Typography follows `ADL-V3.md` section 2:
//!   * [`FontRole::Ui`]: Inter for labels, controls, and numbers. Regular (400), Medium (500)
//!     and SemiBold (600, display sizes only), with Regular and Medium italics. Line height 1.25.
//!   * [`FontRole::Body`]: Noto Sans Regular (400) and Medium (500) for paragraphs. Line height
//!     1.5. Body text has no italic face; italic body text uses Inter Italic.
//!   * [`FontRole::Mono`]: Fira Mono Medium for code, paths, and anything that must line up.
//!     Line height 1.5.
//!
//! The fonts are static instances of the OFL-licensed variable fonts, produced with
//! fontTools (`assets/fonts/README.md`). They are compiled in with `include_bytes!`, so the
//! app never falls back to a system font, and it renders the same on every machine.
//!
//! Shaped buffers are cached by text, size, weight, role, and wrap width. A buffer that goes
//! unused for a couple of frames is dropped.

use std::collections::HashMap;

pub use cosmic_text::Buffer as TextBuffer;

use cosmic_text::{Attrs, Buffer, Color as GlyphColor, Family, FontSystem, Metrics, Shaping, Style, SwashCache, Weight, Wrap};

/// Family names as written into the embedded fonts' name tables.
pub const UI_FAMILY: &str = "Inter";
pub const BODY_FAMILY: &str = "Noto Sans";

/// CSS-style weights used by the design system.
pub const WEIGHT_REGULAR: u16 = 400;
/// Medium, used for headings, buttons, and labels.
pub const WEIGHT_MEDIUM: u16 = 500;
/// SemiBold, for display sizes (22 px and up) only: screen titles and headline numbers.
pub const WEIGHT_SEMIBOLD: u16 = 600;

pub const MONO_FAMILY: &str = "Fira Mono";

/// Every face compiled into the binary. `assets/fonts/README.md` has their provenance.
static FACES: [&[u8]; 8] = [
    include_bytes!("../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../assets/fonts/Inter-Italic.ttf"),
    include_bytes!("../assets/fonts/Inter-MediumItalic.ttf"),
    include_bytes!("../assets/fonts/NotoSans-Regular.ttf"),
    include_bytes!("../assets/fonts/NotoSans-Medium.ttf"),
    include_bytes!("../assets/fonts/FiraMono-Medium.ttf"),
];

/// Frames an unused buffer stays cached.
const KEEP_FRAMES: u64 = 2;
/// Upper bound for layout, in pixels. Large enough for any single line.
const LAYOUT_LIMIT: f32 = 16384.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FontRole {
    #[default]
    Ui,
    Body,
    /// Fixed-width: code and aligned figures.
    Mono,
}

impl FontRole {
    /// Line height as a multiple of the font size.
    pub fn line_height(self) -> f32 {
        match self {
            FontRole::Ui => 1.25,
            FontRole::Body | FontRole::Mono => 1.5,
        }
    }

    fn family(self) -> &'static str {
        match self {
            FontRole::Ui => UI_FAMILY,
            FontRole::Body => BODY_FAMILY,
            FontRole::Mono => MONO_FAMILY,
        }
    }

    /// The nearest weight this role has a face for. Cosmic-text treats a weight its family lacks
    /// as a miss and falls back to another family, so the request has to name a real face.
    fn weight_for(self, weight: u16, italic: bool) -> u16 {
        match (self, italic) {
            (FontRole::Mono, _) => WEIGHT_MEDIUM,
            (FontRole::Ui, false) => weight.clamp(WEIGHT_REGULAR, WEIGHT_SEMIBOLD),
            _ => weight.clamp(WEIGHT_REGULAR, WEIGHT_MEDIUM),
        }
        .div_ceil(100)
            * 100
    }

    /// The family that draws this role in italic. Only Inter has italic faces.
    fn italic_family(self) -> &'static str {
        match self {
            FontRole::Mono => MONO_FAMILY,
            _ => UI_FAMILY,
        }
    }
}

/// A styled byte range in rich text. Text outside every span uses the run's own style, and its
/// color comes from the draw command. A span's color is fixed at shaping time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RichSpan {
    pub start: usize,
    pub end: usize,
    /// Straight-alpha RGBA. `None` keeps the draw command's color.
    pub color: Option<[u8; 4]>,
    pub weight: u16,
    pub role: FontRole,
    /// Size in quarter pixels. 0 keeps the run's size.
    pub size_q: u32,
    pub italic: bool,
}

impl RichSpan {
    pub fn new(range: std::ops::Range<usize>, role: FontRole, weight: u16) -> Self {
        Self {
            start: range.start,
            end: range.end,
            color: None,
            weight,
            role,
            size_q: 0,
            italic: false,
        }
    }

    pub fn color(mut self, c: crate::color::Color) -> Self {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        self.color = Some([q(c[0]), q(c[1]), q(c[2]), q(c[3])]);
        self
    }

    pub fn size(mut self, px: f32) -> Self {
        self.size_q = (px * 4.0).round() as u32;
        self
    }

    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }
}

/// Identifies a shaped text run in the cache.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TextKey {
    text: String,
    /// Size in quarter pixels, so equal sizes hash equal.
    size_q: u32,
    weight: u16,
    role: FontRole,
    italic: bool,
    /// 0 for unwrapped text, otherwise the wrap width in quarter pixels plus one.
    wrap_q: u32,
    /// Styled ranges for rich text. Empty for plain text.
    spans: Vec<RichSpan>,
}

pub struct CachedBuffer {
    pub buffer: Buffer,
    /// Widest line in pixels.
    pub width: f32,
    /// Number of laid-out lines. Always at least 1.
    pub lines: usize,
    last_used: u64,
}

pub struct TextSystem {
    font_system: FontSystem,
    swash: SwashCache,
    buffers: HashMap<TextKey, CachedBuffer>,
    frame: u64,
}

impl Default for TextSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl TextSystem {
    pub fn new() -> Self {
        // Use only the embedded fonts. Starting from an empty database keeps the system fonts
        // from ever being picked up by mistake.
        let mut font_system = FontSystem::new_with_fonts(std::iter::empty());
        for bytes in FACES {
            // Read in place from the binary: copying them would hold every font twice.
            let data: &'static [u8] = bytes;
            font_system.db_mut().load_font_source(cosmic_text::fontdb::Source::Binary(std::sync::Arc::new(data)));
        }
        Self {
            font_system,
            swash: SwashCache::new(),
            buffers: HashMap::new(),
            frame: 0,
        }
    }

    pub fn begin_frame(&mut self) {
        self.frame += 1;
    }

    pub fn end_frame(&mut self) {
        let frame = self.frame;
        self.buffers
            .retain(|_, c| c.last_used + KEEP_FRAMES >= frame);
    }

    /// Shapes `text` (or reuses the cached shaping) and returns its key.
    /// With `wrap_width`, lines break at words, and words longer than the width break by glyph.
    pub fn layout(
        &mut self,
        text: &str,
        size: f32,
        weight: u16,
        role: FontRole,
        wrap_width: Option<f32>,
    ) -> TextKey {
        self.layout_rich(text, size, weight, role, false, wrap_width, &[])
    }

    /// Like [`TextSystem::layout`], with `spans` styled their own way. Spans must be sorted,
    /// must not overlap, and must start and end on character boundaries. A span may change the
    /// font, weight, size, color, and slant. Each line's height follows its tallest span.
    #[allow(clippy::too_many_arguments)]
    pub fn layout_rich(
        &mut self,
        text: &str,
        size: f32,
        weight: u16,
        role: FontRole,
        italic: bool,
        wrap_width: Option<f32>,
        spans: &[RichSpan],
    ) -> TextKey {
        let key = TextKey {
            text: text.to_string(),
            size_q: (size * 4.0).round() as u32,
            weight,
            role,
            italic,
            wrap_q: wrap_width.map_or(0, |w| (w.max(1.0) * 4.0).ceil() as u32 + 1),
            spans: spans.to_vec(),
        };
        if let Some(cached) = self.buffers.get_mut(&key) {
            cached.last_used = self.frame;
            return key;
        }

        let buffer = build_buffer(
            &mut self.font_system,
            role,
            italic,
            text,
            size,
            weight,
            role.line_height(),
            wrap_width,
            spans,
        );
        let width = buffer
            .layout_runs()
            .map(|run| run.line_w)
            .fold(0.0_f32, f32::max);
        let lines = buffer.layout_runs().count().max(1);
        self.buffers.insert(
            key.clone(),
            CachedBuffer {
                buffer,
                width,
                lines,
                last_used: self.frame,
            },
        );
        key
    }

    /// Width in pixels of a shaped run.
    pub fn width(&self, key: &TextKey) -> f32 {
        self.buffers.get(key).map_or(0.0, |c| c.width)
    }

    /// The shaped glyphon buffer behind a key, for geometry queries such as caret placement.
    pub fn buffer(&self, key: &TextKey) -> Option<&Buffer> {
        self.buffers.get(key).map(|c| &c.buffer)
    }

    /// Number of laid-out lines for a shaped run.
    pub fn lines(&self, key: &TextKey) -> usize {
        self.buffers.get(key).map_or(1, |c| c.lines)
    }

    /// Borrows what the renderer needs at once: the font system, the glyph cache, and the
    /// shaped buffers.
    pub fn parts(
        &mut self,
    ) -> (
        &mut FontSystem,
        &mut SwashCache,
        &HashMap<TextKey, CachedBuffer>,
    ) {
        (&mut self.font_system, &mut self.swash, &self.buffers)
    }
}

#[allow(clippy::too_many_arguments)]
fn build_buffer(
    font_system: &mut FontSystem,
    role: FontRole,
    italic: bool,
    text: &str,
    size: f32,
    weight: u16,
    line_factor: f32,
    wrap_width: Option<f32>,
    spans: &[RichSpan],
) -> Buffer {
    let mut buffer = Buffer::new(font_system, Metrics::new(size, size * line_factor));
    let face = |role: FontRole, italic: bool, weight: u16| {
        let a = Attrs::new().weight(Weight(role.weight_for(weight, italic)));
        if italic {
            a.family(Family::Name(role.italic_family())).style(Style::Italic)
        } else {
            a.family(Family::Name(role.family()))
        }
    };
    let attrs = face(role, italic, weight);
    // Plain text, or rich text cut into runs: each span with its own attributes, and the gaps
    // between spans with the run's.
    let set_text = |buffer: &mut Buffer| {
        if spans.is_empty() {
            buffer.set_text(text, &attrs, Shaping::Advanced, None);
            return;
        }
        let mut runs: Vec<(&str, Attrs)> = Vec::with_capacity(spans.len() * 2 + 1);
        let mut at = 0;
        for sp in spans {
            let (a, b) = (sp.start.clamp(at, text.len()), sp.end.min(text.len()));
            if a > at {
                runs.push((&text[at..a], attrs.clone()));
            }
            if b > a {
                let mut sa = face(sp.role, sp.italic, sp.weight);
                if let Some([r, g, bl, al]) = sp.color {
                    sa = sa.color(GlyphColor::rgba(r, g, bl, al));
                }
                if sp.size_q > 0 {
                    let px = sp.size_q as f32 / 4.0;
                    sa = sa.metrics(Metrics::new(px, px * sp.role.line_height()));
                }
                runs.push((&text[a..b], sa));
                at = b;
            }
        }
        if at < text.len() {
            runs.push((&text[at..], attrs.clone()));
        }
        buffer.set_rich_text(runs, &attrs, Shaping::Advanced, None);
    };

    match wrap_width {
        Some(width) => {
            let width = width.max(1.0);
            // Wrap at words first, which reads best for prose.
            buffer.set_wrap(Wrap::Word);
            buffer.set_size(Some(width), Some(LAYOUT_LIMIT));
            set_text(&mut buffer);
            buffer.shape_until_scroll(font_system, false);
            // A single word wider than the box cannot break at word boundaries. Fall back to
            // glyph wrapping, so no line ever leaves its container.
            let overflows = buffer
                .layout_runs()
                .any(|run| run.line_w > width + 0.5);
            if overflows {
                buffer.set_wrap(Wrap::Glyph);
                buffer.shape_until_scroll(font_system, false);
            }
        }
        None => {
            buffer.set_wrap(Wrap::None);
            buffer.set_size(Some(LAYOUT_LIMIT), Some(LAYOUT_LIMIT));
            set_text(&mut buffer);
            buffer.shape_until_scroll(font_system, false);
        }
    }
    buffer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_fonts_are_registered_under_their_families() {
        let ts = TextSystem::new();
        let families: std::collections::HashSet<String> = ts
            .font_system
            .db()
            .faces()
            .flat_map(|f| f.families.iter().map(|(n, _)| n.clone()))
            .collect();
        assert!(families.contains(UI_FAMILY), "missing {UI_FAMILY}");
        assert!(families.contains(BODY_FAMILY), "missing {BODY_FAMILY}");
    }

    #[test]
    fn every_role_weight_and_slant_resolves_to_its_own_face() {
        let mut ts = TextSystem::new();
        let mut w = |weight, role, italic| {
            let k = ts.layout_rich("Anvil Wim", 20.0, weight, role, italic, None, &[]);
            ts.width(&k)
        };
        let base = w(WEIGHT_REGULAR, FontRole::Ui, false);
        assert_ne!(base, w(WEIGHT_SEMIBOLD, FontRole::Ui, false));
        assert_ne!(base, w(WEIGHT_REGULAR, FontRole::Ui, true));
        assert_ne!(w(WEIGHT_REGULAR, FontRole::Ui, true), w(WEIGHT_MEDIUM, FontRole::Ui, true));
        assert_ne!(w(WEIGHT_REGULAR, FontRole::Body, false), w(WEIGHT_MEDIUM, FontRole::Body, false));
    }

    #[test]
    fn mono_glyphs_share_one_advance() {
        let mut ts = TextSystem::new();
        let narrow = ts.layout("iiii", 20.0, WEIGHT_REGULAR, FontRole::Mono, None);
        let wide = ts.layout("WWWW", 20.0, WEIGHT_REGULAR, FontRole::Mono, None);
        assert!((ts.width(&narrow) - ts.width(&wide)).abs() < 0.01);
    }

    #[test]
    fn medium_weight_is_a_distinct_face() {
        let mut ts = TextSystem::new();
        let regular = ts.layout("Anvil", 20.0, WEIGHT_REGULAR, FontRole::Ui, None);
        let medium = ts.layout("Anvil", 20.0, WEIGHT_MEDIUM, FontRole::Ui, None);
        // Medium glyphs are slightly wider than Regular, so the widths differ.
        assert_ne!(ts.width(&regular), ts.width(&medium));
    }
}
