//! JPEG decoding: baseline and extended sequential Huffman JPEG (ITU T.81), the kind cameras
//! and most tools write. Grayscale, YCbCr with any chroma subsampling, Adobe CMYK, and restart
//! intervals. Progressive and arithmetic-coded JPEGs are refused with a clear error. EXIF
//! rotation is not applied.

use super::ImageError;
use crate::media::Frame;

pub fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xff, 0xd8])
}

/// Natural (row-major) index of each zigzag position.
const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57,
    50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

#[derive(Clone, Default)]
struct Huffman {
    /// For each code length 1..=16: the largest code, the index of its first symbol, and the
    /// first code of that length. Annex F.2.2.3.
    maxcode: [i32; 18],
    valptr: [i32; 17],
    mincode: [i32; 17],
    values: Vec<u8>,
}

impl Huffman {
    fn new(counts: &[u8; 16], values: Vec<u8>) -> Self {
        let mut h = Huffman { values, ..Default::default() };
        let (mut code, mut k) = (0i32, 0i32);
        for len in 1..=16 {
            let n = counts[len - 1] as i32;
            if n == 0 {
                h.maxcode[len] = -1;
            } else {
                h.valptr[len] = k;
                h.mincode[len] = code;
                code += n;
                k += n;
                h.maxcode[len] = code - 1;
            }
            code <<= 1;
        }
        h.maxcode[17] = i32::MAX;
        h
    }
}

#[derive(Clone)]
struct Component {
    id: u8,
    h: usize,
    v: usize,
    tq: usize,
    dc: usize,
    ac: usize,
    /// Decoded samples at this component's resolution, padded to whole MCUs.
    plane: Vec<u8>,
    stride: usize,
    pred: i32,
}

/// The entropy-coded bit reader. It strips stuffed zero bytes and stops at markers.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    bits: u32,
    nbits: u32,
}

impl<'a> Reader<'a> {
    fn bit(&mut self) -> u32 {
        if self.nbits == 0 {
            let mut byte = 0;
            if self.pos < self.data.len() {
                byte = self.data[self.pos];
                if byte == 0xff {
                    let next = self.data.get(self.pos + 1).copied().unwrap_or(0);
                    if next == 0 {
                        self.pos += 2;
                    } else {
                        // A marker: feed zeros rather than read past it.
                        byte = 0;
                    }
                } else {
                    self.pos += 1;
                }
            }
            self.bits = byte as u32;
            self.nbits = 8;
        }
        self.nbits -= 1;
        (self.bits >> self.nbits) & 1
    }

    fn receive(&mut self, n: u32) -> i32 {
        let mut v = 0i32;
        for _ in 0..n {
            v = (v << 1) | self.bit() as i32;
        }
        v
    }

    /// Sign-extends an `n`-bit magnitude category value (F.2.2.1 EXTEND).
    fn extend(v: i32, n: u32) -> i32 {
        if n == 0 {
            0
        } else if v < 1 << (n - 1) {
            v - (1 << n) + 1
        } else {
            v
        }
    }

    fn decode(&mut self, h: &Huffman) -> Result<u8, ImageError> {
        let mut code = self.bit() as i32;
        for len in 1..=16 {
            if code <= h.maxcode[len] {
                let i = (h.valptr[len] + code - h.mincode[len]) as usize;
                return h.values.get(i).copied().ok_or(ImageError::Format("bad Huffman table"));
            }
            code = (code << 1) | self.bit() as i32;
        }
        Err(ImageError::Format("bad Huffman code"))
    }

    /// Skips to just after the next RSTn marker and clears the bit buffer.
    fn restart(&mut self) {
        self.nbits = 0;
        while self.pos + 1 < self.data.len() {
            if self.data[self.pos] == 0xff && (0xd0..=0xd7).contains(&self.data[self.pos + 1]) {
                self.pos += 2;
                return;
            }
            self.pos += 1;
        }
    }
}

fn be16(b: &[u8], at: usize) -> Result<usize, ImageError> {
    b.get(at..at + 2).map(|s| u16::from_be_bytes([s[0], s[1]]) as usize).ok_or(ImageError::Truncated)
}

pub fn decode(b: &[u8]) -> Result<Frame, ImageError> {
    if !is_jpeg(b) {
        return Err(ImageError::Format("not a JPEG"));
    }
    let mut qt = [[0u16; 64]; 4];
    let mut dc_tables: [Huffman; 4] = Default::default();
    let mut ac_tables: [Huffman; 4] = Default::default();
    let mut comps: Vec<Component> = Vec::new();
    let (mut width, mut height) = (0usize, 0usize);
    let mut restart_interval = 0usize;
    let mut adobe_transform: Option<u8> = None;
    let mut at = 2;
    loop {
        // Find the next marker.
        while at < b.len() && b[at] != 0xff {
            at += 1;
        }
        while at < b.len() && b[at] == 0xff {
            at += 1;
        }
        let marker = *b.get(at).ok_or(ImageError::Truncated)?;
        at += 1;
        match marker {
            0xd8 | 0x01 | 0xd0..=0xd7 => continue,
            0xd9 => return Err(ImageError::Format("JPEG has no image data")),
            _ => {}
        }
        let len = be16(b, at)?;
        let seg = b.get(at + 2..at + len).ok_or(ImageError::Truncated)?;
        match marker {
            0xdb => {
                let mut p = 0;
                while p < seg.len() {
                    let (precision, id) = (seg[p] >> 4, (seg[p] & 3) as usize);
                    p += 1;
                    for &z in ZIGZAG.iter() {
                        let v = if precision == 0 {
                            p += 1;
                            *seg.get(p - 1).ok_or(ImageError::Truncated)? as u16
                        } else {
                            p += 2;
                            be16(seg, p - 2)? as u16
                        };
                        qt[id][z] = v;
                    }
                }
            }
            0xc4 => {
                let mut p = 0;
                while p < seg.len() {
                    let (class, id) = (seg[p] >> 4, (seg[p] & 3) as usize);
                    let counts: [u8; 16] = seg.get(p + 1..p + 17).ok_or(ImageError::Truncated)?.try_into().expect("16 bytes");
                    let n: usize = counts.iter().map(|&c| c as usize).sum();
                    let values = seg.get(p + 17..p + 17 + n).ok_or(ImageError::Truncated)?.to_vec();
                    let table = Huffman::new(&counts, values);
                    if class == 0 {
                        dc_tables[id] = table;
                    } else {
                        ac_tables[id] = table;
                    }
                    p += 17 + n;
                }
            }
            0xc0 | 0xc1 => {
                if seg[0] != 8 {
                    return Err(ImageError::Format("12-bit JPEG"));
                }
                height = be16(seg, 1)?;
                width = be16(seg, 3)?;
                let n = seg[5] as usize;
                if width == 0 || height == 0 || !(n == 1 || n == 3 || n == 4) {
                    return Err(ImageError::Format("bad JPEG frame"));
                }
                if width * height > 1 << 28 {
                    return Err(ImageError::Format("image too large"));
                }
                for i in 0..n {
                    let c = seg.get(6 + i * 3..9 + i * 3).ok_or(ImageError::Truncated)?;
                    let (h, v) = ((c[1] >> 4) as usize, (c[1] & 15) as usize);
                    if !(1..=4).contains(&h) || !(1..=4).contains(&v) {
                        return Err(ImageError::Format("bad sampling factor"));
                    }
                    comps.push(Component { id: c[0], h, v, tq: (c[2] & 3) as usize, dc: 0, ac: 0, plane: Vec::new(), stride: 0, pred: 0 });
                }
            }
            0xc2 => return Err(ImageError::Format("progressive JPEG is not supported yet")),
            0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf => {
                return Err(ImageError::Format("lossless, hierarchical or arithmetic JPEG"))
            }
            0xdd => restart_interval = be16(seg, 0)?,
            0xee => {
                if seg.starts_with(b"Adobe") && seg.len() >= 12 {
                    adobe_transform = Some(seg[11]);
                }
            }
            0xda => {
                if comps.is_empty() {
                    return Err(ImageError::Format("scan before frame"));
                }
                let ns = seg[0] as usize;
                for i in 0..ns {
                    let (id, tables) = (seg[1 + i * 2], seg[2 + i * 2]);
                    let c = comps.iter_mut().find(|c| c.id == id).ok_or(ImageError::Format("scan names an unknown component"))?;
                    c.dc = (tables >> 4) as usize & 3;
                    c.ac = (tables & 15) as usize & 3;
                }
                if ns != comps.len() {
                    return Err(ImageError::Format("multi-scan sequential JPEG"));
                }
                let data = &b[at + len..];
                decode_scan(data, &mut comps, &qt, &dc_tables, &ac_tables, width, height, restart_interval)?;
                return Ok(to_rgba(&comps, width, height, adobe_transform));
            }
            _ => {}
        }
        at += len;
    }
}

#[allow(clippy::too_many_arguments)]
fn decode_scan(
    data: &[u8],
    comps: &mut [Component],
    qt: &[[u16; 64]; 4],
    dc: &[Huffman; 4],
    ac: &[Huffman; 4],
    width: usize,
    height: usize,
    restart_interval: usize,
) -> Result<(), ImageError> {
    let hmax = comps.iter().map(|c| c.h).max().unwrap_or(1);
    let vmax = comps.iter().map(|c| c.v).max().unwrap_or(1);
    let single = comps.len() == 1;
    // A lone component is coded in its own 8x8 blocks, not in MCUs.
    let (mcux, mcuy) = if single {
        (width.div_ceil(8), height.div_ceil(8))
    } else {
        (width.div_ceil(8 * hmax), height.div_ceil(8 * vmax))
    };
    for c in comps.iter_mut() {
        let (bw, bh) = if single { (mcux, mcuy) } else { (mcux * c.h, mcuy * c.v) };
        c.stride = bw * 8;
        c.plane = vec![0u8; bw * 8 * bh * 8];
        c.pred = 0;
    }
    let idct = IdctTable::new();
    let mut r = Reader { data, pos: 0, bits: 0, nbits: 0 };
    let mut block = [0i32; 64];
    let total = mcux * mcuy;
    for m in 0..total {
        if restart_interval > 0 && m > 0 && m % restart_interval == 0 {
            r.restart();
            for c in comps.iter_mut() {
                c.pred = 0;
            }
        }
        let (mx, my) = (m % mcux, m / mcux);
        for c in comps.iter_mut() {
            let (nh, nv) = if single { (1, 1) } else { (c.h, c.v) };
            for by in 0..nv {
                for bx in 0..nh {
                    block.fill(0);
                    let t = r.decode(&dc[c.dc])? as u32;
                    let diff = Reader::extend(r.receive(t), t);
                    c.pred += diff;
                    block[0] = c.pred * qt[c.tq][0] as i32;
                    let mut k = 1;
                    while k < 64 {
                        let rs = r.decode(&ac[c.ac])?;
                        let (run, size) = ((rs >> 4) as usize, (rs & 15) as u32);
                        if size == 0 {
                            if run == 15 {
                                k += 16;
                                continue;
                            }
                            break;
                        }
                        k += run;
                        if k > 63 {
                            return Err(ImageError::Format("bad AC run"));
                        }
                        let v = Reader::extend(r.receive(size), size);
                        block[ZIGZAG[k]] = v * qt[c.tq][ZIGZAG[k]] as i32;
                        k += 1;
                    }
                    let x0 = (mx * nh + bx) * 8;
                    let y0 = (my * nv + by) * 8;
                    idct.apply(&block, &mut c.plane, y0 * c.stride + x0, c.stride);
                }
            }
        }
    }
    Ok(())
}

/// A separable floating-point inverse DCT with a precomputed cosine table.
struct IdctTable([[f32; 8]; 8]);

impl IdctTable {
    fn new() -> Self {
        let mut t = [[0f32; 8]; 8];
        for (x, row) in t.iter_mut().enumerate() {
            for (u, v) in row.iter_mut().enumerate() {
                let cu = if u == 0 { std::f32::consts::FRAC_1_SQRT_2 } else { 1.0 };
                *v = cu * ((2 * x + 1) as f32 * u as f32 * std::f32::consts::PI / 16.0).cos() / 2.0;
            }
        }
        Self(t)
    }

    fn apply(&self, coef: &[i32; 64], out: &mut [u8], at: usize, stride: usize) {
        let mut tmp = [0f32; 64];
        // Columns, then rows.
        for x in 0..8 {
            for y in 0..8 {
                let mut s = 0.0;
                for v in 0..8 {
                    s += self.0[y][v] * coef[v * 8 + x] as f32;
                }
                tmp[y * 8 + x] = s;
            }
        }
        for y in 0..8 {
            for x in 0..8 {
                let mut s = 0.0;
                for u in 0..8 {
                    s += self.0[x][u] * tmp[y * 8 + u];
                }
                out[at + y * stride + x] = (s + 128.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// Upsamples each component to full size and converts to RGBA.
fn to_rgba(comps: &[Component], width: usize, height: usize, adobe: Option<u8>) -> Frame {
    let hmax = comps.iter().map(|c| c.h).max().unwrap_or(1);
    let vmax = comps.iter().map(|c| c.v).max().unwrap_or(1);
    // Subsampled chroma is upsampled bilinearly with sample centers aligned, which for 2x is
    // libjpeg's "fancy" triangle filter. Full-size components read straight through.
    let sample = |c: &Component, x: usize, y: usize| -> f32 {
        if c.h == hmax && c.v == vmax {
            return c.plane[y * c.stride + x] as f32;
        }
        let cw = (width * c.h).div_ceil(hmax);
        let chh = (height * c.v).div_ceil(vmax);
        let fx = ((x as f32 + 0.5) * c.h as f32 / hmax as f32 - 0.5).clamp(0.0, (cw - 1) as f32);
        let fy = ((y as f32 + 0.5) * c.v as f32 / vmax as f32 - 0.5).clamp(0.0, (chh - 1) as f32);
        let (x0, y0) = (fx as usize, fy as usize);
        let (x1, y1) = ((x0 + 1).min(cw - 1), (y0 + 1).min(chh - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let p = |xx: usize, yy: usize| c.plane[yy * c.stride + xx] as f32;
        let top = p(x0, y0) + (p(x1, y0) - p(x0, y0)) * tx;
        let bottom = p(x0, y1) + (p(x1, y1) - p(x0, y1)) * tx;
        top + (bottom - top) * ty
    };
    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let px = match comps.len() {
                1 => {
                    let g = sample(&comps[0], x, y) as u8;
                    [g, g, g, 255]
                }
                3 if adobe == Some(0) => [
                    sample(&comps[0], x, y) as u8,
                    sample(&comps[1], x, y) as u8,
                    sample(&comps[2], x, y) as u8,
                    255,
                ],
                3 => ycc_to_rgb(sample(&comps[0], x, y), sample(&comps[1], x, y), sample(&comps[2], x, y)),
                _ => {
                    // Adobe CMYK is stored inverted. With transform 2 the first three are YCC.
                    let (c, m, ye) = if adobe == Some(2) {
                        let [r, g, b, _] = ycc_to_rgb(sample(&comps[0], x, y), sample(&comps[1], x, y), sample(&comps[2], x, y));
                        (r as f32, g as f32, b as f32)
                    } else {
                        (sample(&comps[0], x, y), sample(&comps[1], x, y), sample(&comps[2], x, y))
                    };
                    let k = sample(&comps[3], x, y) / 255.0;
                    [(c * k) as u8, (m * k) as u8, (ye * k) as u8, 255]
                }
            };
            rgba.extend_from_slice(&px);
        }
    }
    Frame { width: width as u32, height: height as u32, rgba }
}

fn ycc_to_rgb(y: f32, cb: f32, cr: f32) -> [u8; 4] {
    let (cb, cr) = (cb - 128.0, cr - 128.0);
    let c = |v: f32| v.round().clamp(0.0, 255.0) as u8;
    [c(y + 1.402 * cr), c(y - 0.344136 * cb - 0.714136 * cr), c(y + 1.772 * cb), 255]
}
