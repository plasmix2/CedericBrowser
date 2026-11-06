//! BMP decoding: uncompressed 24- and 32-bit, and 1-, 4- and 8-bit with a palette. Rows may be
//! stored bottom-up (the usual case) or top-down.

use super::ImageError;
use crate::media::Frame;

pub fn is_bmp(bytes: &[u8]) -> bool {
    bytes.starts_with(b"BM")
}

fn u16_at(b: &[u8], at: usize) -> Result<u16, ImageError> {
    b.get(at..at + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or(ImageError::Truncated)
}

fn u32_at(b: &[u8], at: usize) -> Result<u32, ImageError> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or(ImageError::Truncated)
}

pub fn decode(b: &[u8]) -> Result<Frame, ImageError> {
    if !is_bmp(b) {
        return Err(ImageError::Format("not a BMP"));
    }
    let data_at = u32_at(b, 10)? as usize;
    let header_size = u32_at(b, 14)? as usize;
    let width = u32_at(b, 18)? as i32;
    let height = u32_at(b, 22)? as i32;
    let bpp = u16_at(b, 28)?;
    let compression = u32_at(b, 30)?;
    // 0 is uncompressed. 3 (bit fields) is allowed for 32-bit images in the usual BGRA layout.
    if !(compression == 0 || (compression == 3 && bpp == 32)) {
        return Err(ImageError::Format("compressed BMP"));
    }
    if width <= 0 || height == 0 {
        return Err(ImageError::Format("bad BMP size"));
    }
    let (w, h) = (width as u32, height.unsigned_abs());
    if w as u64 * h as u64 > 1 << 28 {
        return Err(ImageError::Format("image too large"));
    }
    let top_down = height < 0;
    let palette: Vec<[u8; 4]> = if bpp <= 8 {
        let count = match u32_at(b, 46)? {
            0 => 1usize << bpp,
            n => n as usize,
        };
        let at = 14 + header_size;
        (0..count)
            .map(|i| b.get(at + i * 4..at + i * 4 + 4).map(|c| [c[2], c[1], c[0], 255]).ok_or(ImageError::Truncated))
            .collect::<Result<_, _>>()?
    } else {
        Vec::new()
    };
    let stride = ((w as usize * bpp as usize).div_ceil(32)) * 4;
    let mut rgba = vec![0u8; w as usize * h as usize * 4];
    // A 32-bit BMP whose alpha bytes are all zero is opaque, not invisible.
    let mut any_alpha = false;
    for y in 0..h as usize {
        let src_row = if top_down { y } else { h as usize - 1 - y };
        let row = b.get(data_at + src_row * stride..data_at + (src_row + 1) * stride).ok_or(ImageError::Truncated)?;
        for x in 0..w as usize {
            let px = match bpp {
                32 => {
                    any_alpha |= row[x * 4 + 3] != 0;
                    [row[x * 4 + 2], row[x * 4 + 1], row[x * 4], row[x * 4 + 3]]
                }
                24 => [row[x * 3 + 2], row[x * 3 + 1], row[x * 3], 255],
                1 | 4 | 8 => {
                    let bit = x * bpp as usize;
                    let v = (row[bit / 8] >> (8 - bpp as usize - bit % 8)) & ((1u16 << bpp) - 1) as u8;
                    *palette.get(v as usize).ok_or(ImageError::Format("palette index out of range"))?
                }
                _ => return Err(ImageError::Format("unsupported BMP bit depth")),
            };
            let i = (y * w as usize + x) * 4;
            rgba[i..i + 4].copy_from_slice(&px);
        }
    }
    if bpp == 32 && !any_alpha {
        for px in rgba.chunks_exact_mut(4) {
            px[3] = 255;
        }
    }
    Ok(Frame { width: w, height: h, rgba })
}
