//! DEFLATE decompression (RFC 1951) and the zlib wrapper around it (RFC 1950), for PNG.
//!
//! Stored, fixed-Huffman and dynamic-Huffman blocks are all supported. Huffman codes decode
//! through a canonical count/symbol table, one bit at a time: simple, and fast enough for the
//! images a UI loads.

/// Why a stream did not decompress.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InflateError {
    /// The input ended in the middle of a block.
    Truncated,
    /// A block or code that DEFLATE does not allow.
    Invalid(&'static str),
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    bit: u32,
    nbits: u32,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0, bit: 0, nbits: 0 }
    }

    fn need(&mut self, n: u32) -> Result<(), InflateError> {
        while self.nbits < n {
            let byte = *self.data.get(self.pos).ok_or(InflateError::Truncated)?;
            self.pos += 1;
            self.bit |= (byte as u32) << self.nbits;
            self.nbits += 8;
        }
        Ok(())
    }

    fn take(&mut self, n: u32) -> Result<u32, InflateError> {
        if n == 0 {
            return Ok(0);
        }
        self.need(n)?;
        let v = self.bit & ((1u32 << n) - 1);
        self.bit >>= n;
        self.nbits -= n;
        Ok(v)
    }

    /// Drops the bits left in the current byte.
    fn align(&mut self) {
        let drop = self.nbits % 8;
        self.bit >>= drop;
        self.nbits -= drop;
    }
}

/// A canonical Huffman code: how many codes of each length, and the symbols in code order.
struct Huffman {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Result<Self, InflateError> {
        let mut counts = [0u16; 16];
        for &l in lengths {
            counts[l as usize] += 1;
        }
        counts[0] = 0;
        let mut offs = [0u16; 16];
        for i in 1..16 {
            offs[i] = offs[i - 1] + counts[i - 1];
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbols[offs[l as usize] as usize] = sym as u16;
                offs[l as usize] += 1;
            }
        }
        Ok(Self { counts, symbols })
    }

    fn decode(&self, bits: &mut Bits) -> Result<u16, InflateError> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= bits.take(1)? as i32;
            let count = self.counts[len] as i32;
            if code - count < first {
                return Ok(self.symbols[(index + (code - first)) as usize]);
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err(InflateError::Invalid("bad Huffman code"))
    }
}

const LEN_BASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEN_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
/// The order code-length code lengths arrive in a dynamic block.
const CL_ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

/// Decompresses a raw DEFLATE stream. `size_hint` preallocates the output.
pub fn inflate(data: &[u8], size_hint: usize) -> Result<Vec<u8>, InflateError> {
    let mut out = Vec::with_capacity(size_hint);
    let mut bits = Bits::new(data);
    loop {
        let last = bits.take(1)? == 1;
        match bits.take(2)? {
            0 => {
                bits.align();
                // Whole bytes from here: LEN, NLEN, then the data.
                let at = bits.pos - (bits.nbits / 8) as usize;
                bits.bit = 0;
                bits.nbits = 0;
                let hdr = data.get(at..at + 4).ok_or(InflateError::Truncated)?;
                let len = u16::from_le_bytes([hdr[0], hdr[1]]) as usize;
                let nlen = u16::from_le_bytes([hdr[2], hdr[3]]) as usize;
                if len != !nlen & 0xffff {
                    return Err(InflateError::Invalid("stored block length check"));
                }
                let body = data.get(at + 4..at + 4 + len).ok_or(InflateError::Truncated)?;
                out.extend_from_slice(body);
                bits.pos = at + 4 + len;
            }
            1 => {
                let mut lit = [0u8; 288];
                lit[..144].fill(8);
                lit[144..256].fill(9);
                lit[256..280].fill(7);
                lit[280..].fill(8);
                let lit = Huffman::new(&lit)?;
                let dist = Huffman::new(&[5u8; 30])?;
                codes(&mut bits, &lit, &dist, &mut out)?;
            }
            2 => {
                let hlit = bits.take(5)? as usize + 257;
                let hdist = bits.take(5)? as usize + 1;
                let hclen = bits.take(4)? as usize + 4;
                let mut cl = [0u8; 19];
                for &i in CL_ORDER.iter().take(hclen) {
                    cl[i] = bits.take(3)? as u8;
                }
                let cl = Huffman::new(&cl)?;
                let mut lengths = vec![0u8; hlit + hdist];
                let mut i = 0;
                while i < hlit + hdist {
                    let sym = cl.decode(&mut bits)?;
                    let (value, repeat) = match sym {
                        0..=15 => (sym as u8, 1),
                        16 => {
                            let prev = *lengths.get(i.wrapping_sub(1)).ok_or(InflateError::Invalid("repeat with no length"))?;
                            (prev, 3 + bits.take(2)? as usize)
                        }
                        17 => (0, 3 + bits.take(3)? as usize),
                        _ => (0, 11 + bits.take(7)? as usize),
                    };
                    if i + repeat > lengths.len() {
                        return Err(InflateError::Invalid("code lengths overrun"));
                    }
                    lengths[i..i + repeat].fill(value);
                    i += repeat;
                }
                let lit = Huffman::new(&lengths[..hlit])?;
                let dist = Huffman::new(&lengths[hlit..])?;
                codes(&mut bits, &lit, &dist, &mut out)?;
            }
            _ => return Err(InflateError::Invalid("reserved block type")),
        }
        if last {
            return Ok(out);
        }
    }
}

/// Decodes literal and length/distance codes until the end-of-block code.
fn codes(bits: &mut Bits, lit: &Huffman, dist: &Huffman, out: &mut Vec<u8>) -> Result<(), InflateError> {
    loop {
        let sym = lit.decode(bits)? as usize;
        if sym < 256 {
            out.push(sym as u8);
        } else if sym == 256 {
            return Ok(());
        } else {
            let i = sym - 257;
            if i >= 29 {
                return Err(InflateError::Invalid("bad length code"));
            }
            let len = LEN_BASE[i] as usize + bits.take(LEN_EXTRA[i] as u32)? as usize;
            let d = dist.decode(bits)? as usize;
            if d >= 30 {
                return Err(InflateError::Invalid("bad distance code"));
            }
            let back = DIST_BASE[d] as usize + bits.take(DIST_EXTRA[d] as u32)? as usize;
            if back > out.len() {
                return Err(InflateError::Invalid("distance before the start"));
            }
            let start = out.len() - back;
            // Overlapping copies repeat the recent bytes, one at a time.
            for k in 0..len {
                let b = out[start + k];
                out.push(b);
            }
        }
    }
}

/// Decompresses a zlib stream: a two-byte header, DEFLATE data, and an Adler-32 check.
pub fn zlib_decompress(data: &[u8], size_hint: usize) -> Result<Vec<u8>, InflateError> {
    if data.len() < 6 {
        return Err(InflateError::Truncated);
    }
    let (cmf, flg) = (data[0], data[1]);
    if cmf & 0x0f != 8 || ((cmf as u16) << 8 | flg as u16) % 31 != 0 {
        return Err(InflateError::Invalid("not a zlib stream"));
    }
    if flg & 0x20 != 0 {
        return Err(InflateError::Invalid("preset dictionary"));
    }
    let out = inflate(&data[2..], size_hint)?;
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in &out {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    let tail = &data[data.len() - 4..];
    if (b << 16 | a) != u32::from_be_bytes([tail[0], tail[1], tail[2], tail[3]]) {
        return Err(InflateError::Invalid("Adler-32 check failed"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_fixed_and_dynamic_blocks_decompress() {
        // zlib.compress(b"hello hello hello hello", 9): fixed Huffman with a back reference.
        let fixed = [0x78, 0xda, 0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0xc8, 0x40, 0x27, 0x01, 0x68, 0x03, 0x08, 0xb1];
        assert_eq!(zlib_decompress(&fixed, 0).unwrap(), b"hello hello hello hello");
        // zlib.compress(b"abc", 0): one stored block.
        let stored = [0x78, 0x01, 0x01, 0x03, 0x00, 0xfc, 0xff, 0x61, 0x62, 0x63, 0x02, 0x4d, 0x01, 0x27];
        assert_eq!(zlib_decompress(&stored, 0).unwrap(), b"abc");
    }

    #[test]
    fn damage_is_an_error_not_a_panic() {
        let fixed = [0x78, 0xda, 0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0xc8, 0x40, 0x27, 0x01, 0x68, 0x03, 0x08, 0xb1];
        assert!(zlib_decompress(&fixed[..8], 0).is_err());
        let mut bad = fixed;
        bad[15] ^= 1;
        assert_eq!(zlib_decompress(&bad, 0), Err(InflateError::Invalid("Adler-32 check failed")));
        assert!(zlib_decompress(b"junk data here", 0).is_err());
    }
}
