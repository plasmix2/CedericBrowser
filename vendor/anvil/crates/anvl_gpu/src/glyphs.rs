//! The glyph atlas: every glyph the text uses, rasterized once by swash into one GPU texture.
//!
//! Glyphs are packed in shelves: rows as tall as their tallest glyph, filled left to right. When
//! the texture is full it doubles, up to [`MAX_SIZE`]. Past that, the atlas starts over and only
//! the glyphs of the current frame come back. A UI uses a few hundred glyphs, so this is rare.

use std::collections::HashMap;

use cosmic_text::{CacheKey, FontSystem, SwashCache, SwashContent};

/// Starting and largest atlas size, in pixels on a side.
const START_SIZE: u32 = 512;
pub const MAX_SIZE: u32 = 4096;
/// Empty pixels around each glyph, so linear sampling never reads a neighbour.
const PAD: u32 = 1;

/// Where a glyph sits in the atlas and how it lines up with the pen position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphSlot {
    /// Pixel rectangle in the atlas: x, y, width, height.
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Offset from the pen position to the glyph's top-left corner, from swash.
    pub left: i32,
    pub top: i32,
}

/// A row of glyphs.
#[derive(Clone, Copy, Debug)]
struct Shelf {
    y: u32,
    h: u32,
    x: u32,
}

/// Shelf packing on a square texture of `size` pixels.
#[derive(Debug)]
pub struct Packer {
    pub size: u32,
    shelves: Vec<Shelf>,
    bottom: u32,
}

impl Packer {
    pub fn new(size: u32) -> Self {
        Self {
            size,
            shelves: Vec::new(),
            bottom: 0,
        }
    }

    /// Finds room for a `w` by `h` box. `None` when it does not fit.
    pub fn place(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        let (w, h) = (w + PAD, h + PAD);
        if w > self.size || h > self.size {
            return None;
        }
        // The shortest shelf that fits wastes the least height.
        let best = self
            .shelves
            .iter_mut()
            .filter(|s| s.h >= h && s.h <= h + h / 2 + 2 && s.x + w <= self.size)
            .min_by_key(|s| s.h);
        if let Some(s) = best {
            let at = (s.x, s.y);
            s.x += w;
            return Some(at);
        }
        if self.bottom + h > self.size {
            return None;
        }
        self.shelves.push(Shelf { y: self.bottom, h, x: w });
        let at = (0, self.bottom);
        self.bottom += h;
        Some(at)
    }
}

/// What the renderer has to do after a glyph lookup.
pub enum Lookup {
    /// The glyph is in the atlas (or has no pixels, such as a space).
    Ready(Option<GlyphSlot>),
    /// The atlas is full. Grow it or start over, then look up again.
    Full,
}

/// CPU side of the atlas: the packer, the slots, and pixels waiting to be uploaded.
pub struct GlyphAtlas {
    pub packer: Packer,
    slots: HashMap<CacheKey, Option<GlyphSlot>>,
    /// Glyph bitmaps rasterized since the last upload: slot and one byte of coverage per pixel.
    pub pending: Vec<(GlyphSlot, Vec<u8>)>,
}

impl Default for GlyphAtlas {
    fn default() -> Self {
        Self::new(START_SIZE)
    }
}

impl GlyphAtlas {
    pub fn new(size: u32) -> Self {
        Self {
            packer: Packer::new(size),
            slots: HashMap::new(),
            pending: Vec::new(),
        }
    }

    pub fn size(&self) -> u32 {
        self.packer.size
    }

    /// The glyph for `key`, rasterizing and packing it the first time it is seen.
    pub fn lookup(&mut self, key: CacheKey, fonts: &mut FontSystem, swash: &mut SwashCache) -> Lookup {
        if let Some(slot) = self.slots.get(&key) {
            return Lookup::Ready(*slot);
        }
        let Some(image) = swash.get_image_uncached(fonts, key) else {
            self.slots.insert(key, None);
            return Lookup::Ready(None);
        };
        let (w, h) = (image.placement.width, image.placement.height);
        if w == 0 || h == 0 {
            self.slots.insert(key, None);
            return Lookup::Ready(None);
        }
        let Some((x, y)) = self.packer.place(w, h) else {
            return Lookup::Full;
        };
        // One coverage byte per pixel. A color glyph (emoji) keeps its alpha as the coverage.
        let coverage = match image.content {
            SwashContent::Mask => image.data,
            SwashContent::Color => image.data.chunks_exact(4).map(|p| p[3]).collect(),
            SwashContent::SubpixelMask => image
                .data
                .chunks_exact(4)
                .map(|p| ((p[0] as u16 + p[1] as u16 + p[2] as u16) / 3) as u8)
                .collect(),
        };
        let slot = GlyphSlot {
            x,
            y,
            w,
            h,
            left: image.placement.left,
            top: image.placement.top,
        };
        self.pending.push((slot, coverage));
        self.slots.insert(key, Some(slot));
        Lookup::Ready(Some(slot))
    }

    /// Doubles the texture size, keeping every glyph where it is. Returns false at the limit.
    pub fn grow(&mut self) -> bool {
        if self.packer.size >= MAX_SIZE {
            return false;
        }
        self.packer.size *= 2;
        true
    }

    /// Forgets every glyph. The caller re-uploads what it needs.
    pub fn reset(&mut self) {
        let size = self.packer.size;
        self.packer = Packer::new(size);
        self.slots.clear();
        self.pending.clear();
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

/// Cuts the glyph quad `q` (x0, y0, x1, y1) and its texture coordinates `uv` to `clip`. Returns
/// `None` when nothing is left.
pub fn clip_quad(q: [f32; 4], uv: [f32; 4], clip: [f32; 4]) -> Option<([f32; 4], [f32; 4])> {
    let x0 = q[0].max(clip[0]);
    let y0 = q[1].max(clip[1]);
    let x1 = q[2].min(clip[2]);
    let y1 = q[3].min(clip[3]);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let (qw, qh) = (q[2] - q[0], q[3] - q[1]);
    let su = (uv[2] - uv[0]) / qw;
    let sv = (uv[3] - uv[1]) / qh;
    Some((
        [x0, y0, x1, y1],
        [
            uv[0] + (x0 - q[0]) * su,
            uv[1] + (y0 - q[1]) * sv,
            uv[0] + (x1 - q[0]) * su,
            uv[1] + (y1 - q[1]) * sv,
        ],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shelves_pack_without_overlap_and_report_full() {
        let mut p = Packer::new(64);
        let mut boxes = Vec::new();
        while let Some((x, y)) = p.place(10, 12) {
            boxes.push((x, y));
        }
        assert!(boxes.len() >= 16, "packed {}", boxes.len());
        for (i, a) in boxes.iter().enumerate() {
            for b in &boxes[i + 1..] {
                let apart = a.0 + 11 <= b.0 || b.0 + 11 <= a.0 || a.1 + 13 <= b.1 || b.1 + 13 <= a.1;
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
        assert_eq!(p.place(100, 4), None);
    }

    #[test]
    fn clipping_trims_the_quad_and_its_uvs_together() {
        let (q, uv) = clip_quad([0.0, 0.0, 10.0, 10.0], [0.0, 0.0, 1.0, 1.0], [5.0, -5.0, 20.0, 20.0]).unwrap();
        assert_eq!(q, [5.0, 0.0, 10.0, 10.0]);
        assert_eq!(uv, [0.5, 0.0, 1.0, 1.0]);
        assert!(clip_quad([0.0, 0.0, 10.0, 10.0], [0.0; 4], [20.0, 0.0, 30.0, 10.0]).is_none());
    }

    #[test]
    fn real_glyphs_rasterize_into_slots_once() {
        let mut ts = crate::text::TextSystem::new();
        let key = ts.layout("Ag", 24.0, 400, crate::text::FontRole::Ui, None);
        let (fonts, swash, buffers) = ts.parts();
        let mut atlas = GlyphAtlas::default();
        let mut seen = 0;
        for run in buffers[&key].buffer.layout_runs() {
            for g in run.glyphs {
                let pg = g.physical((0.0, 0.0), 1.0);
                match atlas.lookup(pg.cache_key, fonts, swash) {
                    Lookup::Ready(Some(slot)) => {
                        assert!(slot.w > 4 && slot.h > 4);
                        seen += 1;
                    }
                    _ => panic!("a letter has pixels and fits"),
                }
            }
        }
        assert_eq!(seen, 2);
        assert_eq!(atlas.pending.len(), 2);
        // Looking the same glyph up again rasterizes nothing new.
        let run = buffers[&key].buffer.layout_runs().next().unwrap();
        let again = run.glyphs[0].physical((0.0, 0.0), 1.0).cache_key;
        assert!(matches!(atlas.lookup(again, fonts, swash), Lookup::Ready(Some(_))));
        assert_eq!(atlas.pending.len(), 2);
    }
}
