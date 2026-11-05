//! Pure safe-Rust authenticated encryption (ChaCha20-Poly1305, RFC 8439) and hashing
//! (SHA-256, FIPS 180-4) for encrypting local search history at rest.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

const MAGIC: &[u8; 4] = b"CED1";
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
const HEADER_LEN: usize = 4 + NONCE_LEN + TAG_LEN;

/// Computes the SHA-256 digest of `data`.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];

    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];

    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity(data.len() + 72);
    padded.extend_from_slice(data);
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in padded.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..(i + 1) * 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

fn quarter_round(state: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    state[a] = state[a].wrapping_add(state[b]);
    state[d] = (state[d] ^ state[a]).rotate_left(16);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_left(12);
    state[a] = state[a].wrapping_add(state[b]);
    state[d] = (state[d] ^ state[a]).rotate_left(8);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_left(7);
}

fn chacha20_block(key: &[u8; 32], counter: u32, nonce: &[u8; 12]) -> [u8; 64] {
    let mut state = [0u32; 16];
    state[0] = 0x61707865;
    state[1] = 0x3320646e;
    state[2] = 0x79622d32;
    state[3] = 0x6b206574;
    for i in 0..8 {
        let off = i * 4;
        state[4 + i] = u32::from_le_bytes([key[off], key[off + 1], key[off + 2], key[off + 3]]);
    }
    state[12] = counter;
    state[13] = u32::from_le_bytes([nonce[0], nonce[1], nonce[2], nonce[3]]);
    state[14] = u32::from_le_bytes([nonce[4], nonce[5], nonce[6], nonce[7]]);
    state[15] = u32::from_le_bytes([nonce[8], nonce[9], nonce[10], nonce[11]]);

    let mut working = state;
    for _ in 0..10 {
        quarter_round(&mut working, 0, 4, 8, 12);
        quarter_round(&mut working, 1, 5, 9, 13);
        quarter_round(&mut working, 2, 6, 10, 14);
        quarter_round(&mut working, 3, 7, 11, 15);
        quarter_round(&mut working, 0, 5, 10, 15);
        quarter_round(&mut working, 1, 6, 11, 12);
        quarter_round(&mut working, 2, 7, 8, 13);
        quarter_round(&mut working, 3, 4, 9, 14);
    }

    let mut out = [0u8; 64];
    for i in 0..16 {
        let word = working[i].wrapping_add(state[i]);
        out[i * 4..(i + 1) * 4].copy_from_slice(&word.to_le_bytes());
    }
    out
}

fn chacha20_xor(key: &[u8; 32], counter: u32, nonce: &[u8; 12], data: &mut [u8]) {
    for (i, chunk) in data.chunks_mut(64).enumerate() {
        let block = chacha20_block(key, counter.wrapping_add(i as u32), nonce);
        for (b, k) in chunk.iter_mut().zip(block.iter()) {
            *b ^= *k;
        }
    }
}

/// Computes the RFC 8439 Poly1305 authentication tag using 26-bit limb arithmetic.
fn poly1305_mac(msg: &[u8], key: &[u8; 32]) -> [u8; 16] {
    let t0 = u32::from_le_bytes([key[0], key[1], key[2], key[3]]);
    let t1 = u32::from_le_bytes([key[4], key[5], key[6], key[7]]);
    let t2 = u32::from_le_bytes([key[8], key[9], key[10], key[11]]);
    let t3 = u32::from_le_bytes([key[12], key[13], key[14], key[15]]);

    let r0 = (t0 & 0x03ff_ffff) as u64;
    let r1 = (((t0 >> 26) | (t1 << 6)) & 0x03ff_ff03) as u64;
    let r2 = (((t1 >> 20) | (t2 << 12)) & 0x03ff_c0ff) as u64;
    let r3 = (((t2 >> 14) | (t3 << 18)) & 0x03f0_3fff) as u64;
    let r4 = ((t3 >> 8) & 0x000f_ffff) as u64;

    let s1 = r1 * 5;
    let s2 = r2 * 5;
    let s3 = r3 * 5;
    let s4 = r4 * 5;

    let mut h0 = 0u64;
    let mut h1 = 0u64;
    let mut h2 = 0u64;
    let mut h3 = 0u64;
    let mut h4 = 0u64;

    for chunk in msg.chunks(16) {
        let mut block = [0u8; 17];
        block[..chunk.len()].copy_from_slice(chunk);
        block[chunk.len()] = 1;
        let hibit = if chunk.len() == 16 { 1u64 << 24 } else { 0u64 };

        let m0 = u32::from_le_bytes([block[0], block[1], block[2], block[3]]);
        let m1 = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
        let m2 = u32::from_le_bytes([block[8], block[9], block[10], block[11]]);
        let m3 = u32::from_le_bytes([block[12], block[13], block[14], block[15]]);

        h0 += (m0 & 0x03ff_ffff) as u64;
        h1 += (((m0 >> 26) | (m1 << 6)) & 0x03ff_ffff) as u64;
        h2 += (((m1 >> 20) | (m2 << 12)) & 0x03ff_ffff) as u64;
        h3 += (((m2 >> 14) | (m3 << 18)) & 0x03ff_ffff) as u64;
        h4 += ((m3 >> 8) as u64) | hibit;

        let d0 = h0 * r0 + h1 * s4 + h2 * s3 + h3 * s2 + h4 * s1;
        let mut d1 = h0 * r1 + h1 * r0 + h2 * s4 + h3 * s3 + h4 * s2;
        let mut d2 = h0 * r2 + h1 * r1 + h2 * r0 + h3 * s4 + h4 * s3;
        let mut d3 = h0 * r3 + h1 * r2 + h2 * r1 + h3 * r0 + h4 * s4;
        let mut d4 = h0 * r4 + h1 * r3 + h2 * r2 + h3 * r1 + h4 * r0;

        let mut c = d0 >> 26;
        h0 = d0 & 0x03ff_ffff;
        d1 += c;
        c = d1 >> 26;
        h1 = d1 & 0x03ff_ffff;
        d2 += c;
        c = d2 >> 26;
        h2 = d2 & 0x03ff_ffff;
        d3 += c;
        c = d3 >> 26;
        h3 = d3 & 0x03ff_ffff;
        d4 += c;
        c = d4 >> 26;
        h4 = d4 & 0x03ff_ffff;
        h0 += c * 5;
        c = h0 >> 26;
        h0 &= 0x03ff_ffff;
        h1 += c;
    }

    let mut c = h1 >> 26;
    h1 &= 0x03ff_ffff;
    h2 += c;
    c = h2 >> 26;
    h2 &= 0x03ff_ffff;
    h3 += c;
    c = h3 >> 26;
    h3 &= 0x03ff_ffff;
    h4 += c;
    c = h4 >> 26;
    h4 &= 0x03ff_ffff;
    h0 += c * 5;
    c = h0 >> 26;
    h0 &= 0x03ff_ffff;
    h1 += c;

    let mut g0 = h0.wrapping_add(5);
    c = g0 >> 26;
    g0 &= 0x03ff_ffff;
    let mut g1 = h1.wrapping_add(c);
    c = g1 >> 26;
    g1 &= 0x03ff_ffff;
    let mut g2 = h2.wrapping_add(c);
    c = g2 >> 26;
    g2 &= 0x03ff_ffff;
    let mut g3 = h3.wrapping_add(c);
    c = g3 >> 26;
    g3 &= 0x03ff_ffff;
    let g4 = h4.wrapping_add(c).wrapping_sub(1 << 26);

    let mask = (g4 >> 63).wrapping_sub(1);
    g0 &= mask;
    g1 &= mask;
    g2 &= mask;
    g3 &= mask;
    let nmask = !mask;
    h0 = (h0 & nmask) | g0;
    h1 = (h1 & nmask) | g1;
    h2 = (h2 & nmask) | g2;
    h3 = (h3 & nmask) | g3;
    h4 = (h4 & nmask) | (g4 & mask);

    let w0 = (h0 | (h1 << 26)) & 0xffff_ffff;
    let w1 = ((h1 >> 6) | (h2 << 20)) & 0xffff_ffff;
    let w2 = ((h2 >> 12) | (h3 << 14)) & 0xffff_ffff;
    let w3 = ((h3 >> 18) | (h4 << 8)) & 0xffff_ffff;

    let pad0 = u32::from_le_bytes([key[16], key[17], key[18], key[19]]) as u64;
    let pad1 = u32::from_le_bytes([key[20], key[21], key[22], key[23]]) as u64;
    let pad2 = u32::from_le_bytes([key[24], key[25], key[26], key[27]]) as u64;
    let pad3 = u32::from_le_bytes([key[28], key[29], key[30], key[31]]) as u64;

    let mut f = w0 + pad0;
    let out0 = f as u32;
    f = w1 + pad1 + (f >> 32);
    let out1 = f as u32;
    f = w2 + pad2 + (f >> 32);
    let out2 = f as u32;
    f = w3 + pad3 + (f >> 32);
    let out3 = f as u32;

    let mut tag = [0u8; 16];
    tag[0..4].copy_from_slice(&out0.to_le_bytes());
    tag[4..8].copy_from_slice(&out1.to_le_bytes());
    tag[8..12].copy_from_slice(&out2.to_le_bytes());
    tag[12..16].copy_from_slice(&out3.to_le_bytes());
    tag
}

fn poly1305_aead_tag(otk: &[u8; 32], aad: &[u8], ciphertext: &[u8]) -> [u8; 16] {
    let mut mac_data = Vec::with_capacity(aad.len() + ciphertext.len() + 48);
    mac_data.extend_from_slice(aad);
    if !aad.len().is_multiple_of(16) {
        mac_data.resize(mac_data.len() + (16 - (aad.len() % 16)), 0);
    }
    mac_data.extend_from_slice(ciphertext);
    if !ciphertext.len().is_multiple_of(16) {
        mac_data.resize(mac_data.len() + (16 - (ciphertext.len() % 16)), 0);
    }
    mac_data.extend_from_slice(&(aad.len() as u64).to_le_bytes());
    mac_data.extend_from_slice(&(ciphertext.len() as u64).to_le_bytes());
    poly1305_mac(&mac_data, otk)
}

/// Encrypts `plaintext` with ChaCha20-Poly1305 (RFC 8439), returning `(ciphertext, tag)`.
pub fn aead_seal(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], plaintext: &[u8]) -> (Vec<u8>, [u8; 16]) {
    let block0 = chacha20_block(key, 0, nonce);
    let mut otk = [0u8; 32];
    otk.copy_from_slice(&block0[..32]);

    let mut ciphertext = plaintext.to_vec();
    chacha20_xor(key, 1, nonce, &mut ciphertext);
    let tag = poly1305_aead_tag(&otk, aad, &ciphertext);
    (ciphertext, tag)
}

/// Decrypts and authenticates `ciphertext` with ChaCha20-Poly1305 (RFC 8439).
pub fn aead_open(
    key: &[u8; 32],
    nonce: &[u8; 12],
    aad: &[u8],
    ciphertext: &[u8],
    tag: &[u8; 16],
) -> Option<Vec<u8>> {
    let block0 = chacha20_block(key, 0, nonce);
    let mut otk = [0u8; 32];
    otk.copy_from_slice(&block0[..32]);

    let expected = poly1305_aead_tag(&otk, aad, ciphertext);
    // Constant-time tag verification.
    let mut diff = 0u8;
    for (a, b) in expected.iter().zip(tag.iter()) {
        diff |= a ^ b;
    }
    if diff != 0 {
        return None;
    }

    let mut plaintext = ciphertext.to_vec();
    chacha20_xor(key, 1, nonce, &mut plaintext);
    Some(plaintext)
}

/// Generates 32 bytes of entropy seeded from OS `RandomState` (which uses OS CSPRNG on Windows)
/// and high-resolution system clocks, without including any OS user or process identifiers.
pub fn random_bytes_32() -> [u8; 32] {
    let mut material = Vec::with_capacity(64);
    for i in 0..4u64 {
        let s = RandomState::new();
        let mut h = s.build_hasher();
        h.write_u64(i);
        if let Ok(d) = SystemTime::now().duration_since(UNIX_EPOCH) {
            h.write_u128(d.as_nanos());
        }
        material.extend_from_slice(&h.finish().to_le_bytes());
    }
    sha256(&material)
}

fn random_nonce() -> [u8; 12] {
    let r = random_bytes_32();
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(&r[..12]);
    nonce
}

/// Derives an anonymous 256-bit encryption key from the random key seed alone (no OS username or hostname).
pub fn derive_vault_key(seed: &[u8; 32]) -> [u8; 32] {
    let mut buf = Vec::with_capacity(96);
    buf.extend_from_slice(b"CedericBrowser::AnonymousEncryptedVault::v2::");
    buf.extend_from_slice(seed);
    sha256(&buf)
}

/// Encrypts a byte payload into a `CED1` envelope (`MAGIC || NONCE || TAG || CIPHERTEXT`).
pub fn encrypt_envelope(key: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
    let nonce = random_nonce();
    let (ciphertext, tag) = aead_seal(key, &nonce, MAGIC, plaintext);
    let mut out = Vec::with_capacity(HEADER_LEN + ciphertext.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&tag);
    out.extend_from_slice(&ciphertext);
    out
}

/// Decrypts and authenticates a `CED1` envelope.
pub fn decrypt_envelope(key: &[u8; 32], blob: &[u8]) -> Option<Vec<u8>> {
    if blob.len() < HEADER_LEN || &blob[..4] != MAGIC {
        return None;
    }
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&blob[4..4 + NONCE_LEN]);
    let mut tag = [0u8; TAG_LEN];
    tag.copy_from_slice(&blob[4 + NONCE_LEN..HEADER_LEN]);
    let ciphertext = &blob[HEADER_LEN..];
    aead_open(key, &nonce, MAGIC, ciphertext, &tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_nist_vector() {
        let digest = sha256(b"abc");
        let expected: [u8; 32] = [
            0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea,
            0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22, 0x23,
            0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c,
            0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00, 0x15, 0xad,
        ];
        assert_eq!(digest, expected);
    }

    #[test]
    fn rfc8439_chacha20_poly1305_vector() {
        let key: [u8; 32] = [
            0x80, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87,
            0x88, 0x89, 0x8a, 0x8b, 0x8c, 0x8d, 0x8e, 0x8f,
            0x90, 0x91, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97,
            0x98, 0x99, 0x9a, 0x9b, 0x9c, 0x9d, 0x9e, 0x9f,
        ];
        let nonce: [u8; 12] = [
            0x07, 0x00, 0x00, 0x00, 0x40, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47,
        ];
        let aad: [u8; 12] = [
            0x50, 0x51, 0x52, 0x53, 0xc0, 0xc1, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7,
        ];
        let plaintext = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";

        let (ct, tag) = aead_seal(&key, &nonce, &aad, plaintext);
        let expected_tag: [u8; 16] = [
            0x1a, 0xe1, 0x0b, 0x59, 0x4f, 0x09, 0xe2, 0x6a,
            0x7e, 0x90, 0x2e, 0xcb, 0xd0, 0x60, 0x06, 0x91,
        ];
        assert_eq!(tag, expected_tag);

        let opened = aead_open(&key, &nonce, &aad, &ct, &tag).unwrap();
        assert_eq!(opened, plaintext);
    }

    #[test]
    fn envelope_roundtrip_and_tamper_detection() {
        let key = derive_vault_key(&[42u8; 32]);
        let secret = b"rust borrow checker search at 2026-11-04T08:42:13";
        let mut blob = encrypt_envelope(&key, secret);
        assert_ne!(&blob[HEADER_LEN..], secret);
        assert_eq!(decrypt_envelope(&key, &blob).unwrap(), secret);

        // Flip a single bit in the ciphertext: authentication must fail.
        let last = blob.len() - 1;
        blob[last] ^= 0x01;
        assert!(decrypt_envelope(&key, &blob).is_none());
    }
}
