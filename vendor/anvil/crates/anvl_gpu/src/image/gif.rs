//! GIF decoding: the first frame, with its global or local palette, transparency, and
//! interlacing. Pixels the frame does not cover are transparent.

use super::ImageError;
use crate::media::Frame;

pub fn is_gif(bytes: &[u8]) -> bool {
    bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")
}

fn read_palette(b: &[u8], at: usize, flags: u8) -> Result<(Vec<[u8; 3]>, usize), ImageError> {
    if flags & 0x80 == 0 {
        return Ok((Vec::new(), at));
    }
    let n = 2usize << (flags & 7);
    let raw = b.get(at..at + n * 3).ok_or(ImageError::Truncated)?;
    Ok((raw.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect(), at + n * 3))
}

pub fn decode(b: &[u8]) -> Result<Frame, ImageError> {
    if !is_gif(b) || b.len() < 13 {
        return Err(ImageError::Format("not a GIF"));
    }
    let width = u16::from_le_bytes([b[6], b[7]]) as u32;
    let height = u16::from_le_bytes([b[8], b[9]]) as u32;
    if width == 0 || height == 0 {
        return Err(ImageError::Format("bad GIF size"));
    }
    let (global, mut at) = read_palette(b, 13, b[10])?;
    let mut transparent: Option<u8> = None;
    loop {
        match *b.get(at).ok_or(ImageError::Truncated)? {
            0x21 => {
                // Extension. A graphic control extension (0xF9) names the transparent index.
                let label = *b.get(at + 1).ok_or(ImageError::Truncated)?;
                at += 2;
                if label == 0xf9 && b.get(at) == Some(&4) {
                    let block = b.get(at + 1..at + 5).ok_or(ImageError::Truncated)?;
                    if block[0] & 1 != 0 {
                        transparent = Some(block[3]);
                    }
                }
                at = skip_blocks(b, at)?;
            }
            0x2c => {
                let d = b.get(at + 1..at + 10).ok_or(ImageError::Truncated)?;
                let fx = u16::from_le_bytes([d[0], d[1]]) as u32;
                let fy = u16::from_le_bytes([d[2], d[3]]) as u32;
                let fw = u16::from_le_bytes([d[4], d[5]]) as u32;
                let fh = u16::from_le_bytes([d[6], d[7]]) as u32;
                let flags = d[8];
                let (local, next) = read_palette(b, at + 10, flags)?;
                let palette = if local.is_empty() { &global } else { &local };
                let min_code = *b.get(next).ok_or(ImageError::Truncated)?;
                let (data, _) = gather_blocks(b, next + 1)?;
                let indices = lzw(&data, min_code, (fw * fh) as usize)?;
                let mut rgba = vec![0u8; (width * height * 4) as usize];
                let rows: Vec<u32> = if flags & 0x40 != 0 {
                    // Interlaced: every 8th row from 0, every 8th from 4, every 4th from 2, then
                    // every 2nd from 1.
                    [(0, 8), (4, 8), (2, 4), (1, 2)].iter().flat_map(|&(s, step)| (s..fh).step_by(step)).collect()
                } else {
                    (0..fh).collect()
                };
                for (i, &idx) in indices.iter().enumerate().take((fw * fh) as usize) {
                    let (x, row) = (fx + i as u32 % fw, rows[i / fw as usize]);
                    let y = fy + row;
                    if x >= width || y >= height {
                        continue;
                    }
                    // A transparent pixel keeps its palette color, so scaling does not darken edges.
                    let c = palette.get(idx as usize).copied().unwrap_or([0, 0, 0]);
                    let alpha = if Some(idx) == transparent { 0 } else { 255 };
                    let o = ((y * width + x) * 4) as usize;
                    rgba[o..o + 4].copy_from_slice(&[c[0], c[1], c[2], alpha]);
                }
                return Ok(Frame { width, height, rgba });
            }
            0x3b => return Err(ImageError::Format("GIF has no image")),
            _ => return Err(ImageError::Format("bad GIF block")),
        }
    }
}

fn skip_blocks(b: &[u8], mut at: usize) -> Result<usize, ImageError> {
    loop {
        let n = *b.get(at).ok_or(ImageError::Truncated)? as usize;
        at += 1 + n;
        if n == 0 {
            return Ok(at);
        }
    }
}

fn gather_blocks(b: &[u8], mut at: usize) -> Result<(Vec<u8>, usize), ImageError> {
    let mut out = Vec::new();
    loop {
        let n = *b.get(at).ok_or(ImageError::Truncated)? as usize;
        if n == 0 {
            return Ok((out, at + 1));
        }
        out.extend_from_slice(b.get(at + 1..at + 1 + n).ok_or(ImageError::Truncated)?);
        at += 1 + n;
    }
}

/// GIF's variable-width LZW, least significant bit first.
fn lzw(data: &[u8], min_code: u8, expected: usize) -> Result<Vec<u8>, ImageError> {
    if !(2..=11).contains(&min_code) {
        return Err(ImageError::Format("bad LZW code size"));
    }
    let clear = 1u16 << min_code;
    let end = clear + 1;
    // Each entry: the previous code and the last byte. Roots point at themselves.
    let mut prefix: Vec<u16> = (0..clear + 2).collect();
    let mut suffix: Vec<u8> = (0..clear + 2).map(|c| c as u8).collect();
    let mut first: Vec<u8> = suffix.clone();
    let mut width = min_code as u32 + 1;
    let mut out = Vec::with_capacity(expected);
    let mut prev: Option<u16> = None;
    let (mut bits, mut nbits, mut pos) = (0u32, 0u32, 0usize);
    let mut stack = Vec::new();
    loop {
        while nbits < width {
            let Some(&byte) = data.get(pos) else {
                return Ok(out);
            };
            bits |= (byte as u32) << nbits;
            nbits += 8;
            pos += 1;
        }
        let code = (bits & ((1 << width) - 1)) as u16;
        bits >>= width;
        nbits -= width;
        if code == clear {
            prefix.truncate(clear as usize + 2);
            suffix.truncate(clear as usize + 2);
            first.truncate(clear as usize + 2);
            width = min_code as u32 + 1;
            prev = None;
            continue;
        }
        if code == end {
            return Ok(out);
        }
        let Some(p) = prev else {
            if code >= clear {
                return Err(ImageError::Format("bad LZW stream"));
            }
            out.push(code as u8);
            prev = Some(code);
            continue;
        };
        let known = (code as usize) < prefix.len();
        if !known && code as usize != prefix.len() {
            return Err(ImageError::Format("bad LZW stream"));
        }
        // The new entry is the previous string plus the first byte of this one (or of itself).
        let head = if known { first[code as usize] } else { first[p as usize] };
        if prefix.len() < 4096 {
            prefix.push(p);
            suffix.push(head);
            first.push(first[p as usize]);
        }
        let mut c = code;
        stack.clear();
        while c >= clear {
            stack.push(suffix[c as usize]);
            c = prefix[c as usize];
        }
        stack.push(c as u8);
        out.extend(stack.iter().rev());
        if prefix.len() == 1 << width && width < 12 {
            width += 1;
        }
        prev = Some(code);
        if out.len() >= expected {
            return Ok(out);
        }
    }
}
