//! Tab favicons for the native tab strip: fetched once per URL on a background thread and decoded
//! from PNG or ICO into an image the toolbar can draw. Anything else falls back to a globe icon.

use std::{io::Cursor, sync::mpsc, time::Duration};

use tiny_skia::{IntSize, Pixmap};
use ureq::{
    Agent,
    tls::{TlsConfig, TlsProvider},
};

/// Icons larger than this are not worth downloading for a 16px slot.
const MAX_BYTES: u64 = 512 * 1024;

pub fn spawn(on_done: impl Fn(String, Option<Pixmap>) + Send + 'static) -> mpsc::Sender<String> {
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let agent: Agent = Agent::config_builder()
            .tls_config(TlsConfig::builder().provider(TlsProvider::NativeTls).build())
            .timeout_global(Some(Duration::from_secs(4)))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) Cederic")
            .build()
            .into();
        while let Ok(url) = rx.recv() {
            let img = bytes(&agent, &url).and_then(|b| decode(&b));
            on_done(url, img);
        }
    });
    tx
}

fn bytes(agent: &Agent, url: &str) -> Option<Vec<u8>> {
    if let Some(rest) = url.strip_prefix("data:") {
        let (meta, data) = rest.split_once(',')?;
        return meta.ends_with(";base64").then(|| base64_decode(data))?;
    }
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return None;
    }
    let mut resp = agent.get(url).call().ok()?;
    resp.body_mut().with_config().limit(MAX_BYTES).read_to_vec().ok()
}

fn decode(b: &[u8]) -> Option<Pixmap> {
    if b.starts_with(b"\x89PNG") {
        return Pixmap::decode_png(b).ok();
    }
    if b.starts_with(&[0, 0, 1, 0]) {
        let dir = ico::IconDir::read(Cursor::new(b)).ok()?;
        // The smallest entry that is still sharp at 32px (16px at 200% scaling), else the largest.
        let entry = dir
            .entries()
            .iter()
            .filter(|e| e.width() >= 32)
            .min_by_key(|e| e.width())
            .or_else(|| dir.entries().iter().max_by_key(|e| e.width()))?;
        let img = entry.decode().ok()?;
        let mut rgba = img.rgba_data().to_vec();
        for p in rgba.chunks_exact_mut(4) {
            let a = p[3] as u16;
            for c in &mut p[..3] {
                *c = ((*c as u16 * a + 127) / 255) as u8;
            }
        }
        return Pixmap::from_vec(rgba, IntSize::from_wh(img.width(), img.height())?);
    }
    None
}

pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    };
    let clean: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=').collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for chunk in clean.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            n |= val(c)? << (18 - 6 * i);
        }
        let bytes = n.to_be_bytes();
        out.extend_from_slice(&bytes[1..chunk.len()]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn decodes_base64() {
        assert_eq!(super::base64_decode("aGVsbG8gd29ybGQ=").unwrap(), b"hello world");
        assert_eq!(super::base64_decode("YQ==").unwrap(), b"a");
    }
}
