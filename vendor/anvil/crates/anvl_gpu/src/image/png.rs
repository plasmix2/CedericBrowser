//! PNG decoding (ISO/IEC 15948) to straight-alpha RGBA8.
//!
//! Every color type (gray, RGB, palette, gray + alpha, RGBA), bit depths 1 to 16, `tRNS`
//! transparency, and Adam7 interlacing. 16-bit samples keep their high byte. Ancillary chunks
//! (gamma, color profiles, text) are skipped, so colors come out as stored.

use super::inflate::zlib_decompress;
use super::ImageError;
use crate::media::Frame;

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
/// Larger images are refused rather than allocated.
const MAX_PIXELS: u64 = 1 << 28;

pub fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(&SIGNATURE)
}

struct Header {
    width: u32,
    height: u32,
    depth: u8,
    color: u8,
    interlace: bool,
}

impl Header {
    /// Samples per pixel.
    fn channels(&self) -> usize {
        match self.color {
            0 | 3 => 1,
            4 => 2,
            2 => 3,
            _ => 4,
        }
    }

    /// Bytes from one pixel to the next, for filtering (at least 1).
    fn filter_stride(&self) -> usize {
        ((self.channels() * self.depth as usize).div_ceil(8)).max(1)
    }

    fn row_bytes(&self, width: u32) -> usize {
        (width as usize * self.channels() * self.depth as usize).div_ceil(8)
    }
}

pub fn decode(bytes: &[u8]) -> Result<Frame, ImageError> {
    if !is_png(bytes) {
        return Err(ImageError::Format("not a PNG"));
    }
    let mut at = 8;
    let mut header: Option<Header> = None;
    let mut palette: Vec<[u8; 4]> = Vec::new();
    let mut trns: Vec<u8> = Vec::new();
    let mut idat = Vec::new();
    while at + 8 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[at..at + 4].try_into().expect("4 bytes")) as usize;
        let kind = &bytes[at + 4..at + 8];
        let body = bytes.get(at + 8..at + 8 + len).ok_or(ImageError::Truncated)?;
        match kind {
            b"IHDR" => {
                if len < 13 {
                    return Err(ImageError::Format("short IHDR"));
                }
                let h = Header {
                    width: u32::from_be_bytes(body[0..4].try_into().expect("4 bytes")),
                    height: u32::from_be_bytes(body[4..8].try_into().expect("4 bytes")),
                    depth: body[8],
                    color: body[9],
                    interlace: body[12] == 1,
                };
                let ok_depth = match h.color {
                    0 => matches!(h.depth, 1 | 2 | 4 | 8 | 16),
                    3 => matches!(h.depth, 1 | 2 | 4 | 8),
                    2 | 4 | 6 => matches!(h.depth, 8 | 16),
                    _ => false,
                };
                if !ok_depth || h.width == 0 || h.height == 0 {
                    return Err(ImageError::Format("unsupported PNG color type or depth"));
                }
                if h.width as u64 * h.height as u64 > MAX_PIXELS {
                    return Err(ImageError::Format("image too large"));
                }
                header = Some(h);
            }
            b"PLTE" => {
                palette = body.chunks_exact(3).map(|c| [c[0], c[1], c[2], 255]).collect();
            }
            b"tRNS" => trns = body.to_vec(),
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        at += 12 + len;
    }
    let h = header.ok_or(ImageError::Format("no IHDR"))?;
    if h.color == 3 {
        if palette.is_empty() {
            return Err(ImageError::Format("palette image without PLTE"));
        }
        for (entry, &a) in palette.iter_mut().zip(&trns) {
            entry[3] = a;
        }
    }
    let raw = zlib_decompress(&idat, (h.row_bytes(h.width) + 1) * h.height as usize)
        .map_err(|_| ImageError::Format("damaged PNG data"))?;

    let mut rgba = vec![0u8; h.width as usize * h.height as usize * 4];
    if h.interlace {
        // Adam7: seven passes, each a smaller image on a grid.
        const PASSES: [(u32, u32, u32, u32); 7] = [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)];
        let mut off = 0;
        for (x0, y0, dx, dy) in PASSES {
            let pw = (h.width + dx - 1 - x0) / dx;
            let ph = (h.height + dy - 1 - y0) / dy;
            if pw == 0 || ph == 0 {
                continue;
            }
            let len = (h.row_bytes(pw) + 1) * ph as usize;
            let part = raw.get(off..off + len).ok_or(ImageError::Truncated)?;
            let rows = unfilter(part, &h, pw, ph)?;
            for y in 0..ph {
                for x in 0..pw {
                    let px = pixel(&rows, &h, pw, x, y, &palette, &trns);
                    let (tx, ty) = (x0 + x * dx, y0 + y * dy);
                    let i = (ty as usize * h.width as usize + tx as usize) * 4;
                    rgba[i..i + 4].copy_from_slice(&px);
                }
            }
            off += len;
        }
    } else {
        let len = (h.row_bytes(h.width) + 1) * h.height as usize;
        let part = raw.get(..len).ok_or(ImageError::Truncated)?;
        let rows = unfilter(part, &h, h.width, h.height)?;
        for y in 0..h.height {
            for x in 0..h.width {
                let px = pixel(&rows, &h, h.width, x, y, &palette, &trns);
                let i = (y as usize * h.width as usize + x as usize) * 4;
                rgba[i..i + 4].copy_from_slice(&px);
            }
        }
    }
    Ok(Frame { width: h.width, height: h.height, rgba })
}

/// Undoes the per-row filters. Returns the rows without their filter bytes.
fn unfilter(data: &[u8], h: &Header, w: u32, rows: u32) -> Result<Vec<u8>, ImageError> {
    let stride = h.row_bytes(w);
    let bpp = h.filter_stride();
    let mut out = vec![0u8; stride * rows as usize];
    for y in 0..rows as usize {
        let src = &data[y * (stride + 1)..(y + 1) * (stride + 1)];
        let (filter, line) = (src[0], &src[1..]);
        let (done, rest) = out.split_at_mut(y * stride);
        let prev = if y == 0 { None } else { Some(&done[(y - 1) * stride..]) };
        let cur = &mut rest[..stride];
        for i in 0..stride {
            let a = if i >= bpp { cur[i - bpp] as i16 } else { 0 };
            let b = prev.map_or(0, |p| p[i] as i16);
            let c = if i >= bpp { prev.map_or(0, |p| p[i - bpp] as i16) } else { 0 };
            let pred = match filter {
                0 => 0,
                1 => a,
                2 => b,
                3 => (a + b) / 2,
                4 => {
                    let p = a + b - c;
                    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
                    if pa <= pb && pa <= pc {
                        a
                    } else if pb <= pc {
                        b
                    } else {
                        c
                    }
                }
                _ => return Err(ImageError::Format("bad PNG filter")),
            };
            cur[i] = line[i].wrapping_add(pred as u8);
        }
    }
    Ok(out)
}

/// Sample `n` of pixel (x, y), scaled to 8 bits.
fn sample(rows: &[u8], h: &Header, w: u32, x: u32, y: u32, n: usize) -> (u8, u16) {
    let stride = h.row_bytes(w);
    let row = &rows[y as usize * stride..];
    let d = h.depth as usize;
    let index = x as usize * h.channels() + n;
    match d {
        16 => {
            let v = u16::from_be_bytes([row[index * 2], row[index * 2 + 1]]);
            ((v >> 8) as u8, v)
        }
        8 => (row[index], row[index] as u16),
        _ => {
            let bit = index * d;
            let byte = row[bit / 8];
            let shift = 8 - d - bit % 8;
            let v = (byte >> shift) & ((1 << d) - 1);
            // Scale 1, 2 or 4 bits up to 8: 1 -> 255, 3 (of 2 bits) -> 255.
            ((v as u16 * 255 / ((1u16 << d) - 1)) as u8, v as u16)
        }
    }
}

fn pixel(rows: &[u8], h: &Header, w: u32, x: u32, y: u32, palette: &[[u8; 4]], trns: &[u8]) -> [u8; 4] {
    let s = |n| sample(rows, h, w, x, y, n);
    // A tRNS color key, compared at the image's own bit depth.
    let key = |n: usize| trns.get(n * 2..n * 2 + 2).map(|k| u16::from_be_bytes([k[0], k[1]]));
    match h.color {
        0 => {
            let (g, raw) = s(0);
            let a = if key(0) == Some(raw) { 0 } else { 255 };
            [g, g, g, a]
        }
        2 => {
            let ((r, rr), (g, gr), (b, br)) = (s(0), s(1), s(2));
            let a = if key(0) == Some(rr) && key(1) == Some(gr) && key(2) == Some(br) { 0 } else { 255 };
            [r, g, b, a]
        }
        3 => {
            let (_, index) = s(0);
            palette.get(index as usize).copied().unwrap_or([0, 0, 0, 255])
        }
        4 => {
            let (g, _) = s(0);
            [g, g, g, s(1).0]
        }
        _ => [s(0).0, s(1).0, s(2).0, s(3).0],
    }
}
