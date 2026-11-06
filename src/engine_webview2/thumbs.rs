//! Tab thumbnails for hover cards. A worker thread talks to the engine's loopback DevTools port
//! (the same one the docked DevTools pane uses) and screenshots the visible tab, small and as JPEG.
//! Each view's page carries `window.__cedericTab = <tag>` (set by `WryEngine::create_view`), which is how
//! its target is recognised.

use std::{
    collections::HashMap,
    net::TcpStream,
    path::PathBuf,
    sync::mpsc,
    time::Duration,
};

use serde_json::{Value, json};
use tungstenite::{Message, WebSocket};

use super::devtools;
use crate::engine::SnapshotRequest as Request;

/// Thumbnail width in CSS pixels; the card shows it at about this size.
const WIDTH: f64 = 320.0;

pub fn spawn(data_dir: PathBuf, on_thumb: Box<dyn Fn(u32, tiny_skia::Pixmap) + Send>) -> mpsc::Sender<Request> {
    let (tx, rx) = mpsc::channel::<Request>();
    std::thread::spawn(move || {
        let mut targets: HashMap<u32, String> = HashMap::new();
        let mut active: Option<u32> = None;
        while let Ok(first) = rx.recv() {
            let mut pending_capture: Option<u32> = None;
            for req in std::iter::once(first).chain(std::iter::from_fn(|| rx.try_recv().ok())) {
                match req {
                    Request::Active(t) => active = t,
                    Request::Forget(t) => {
                        targets.remove(&t);
                        if pending_capture == Some(t) {
                            pending_capture = None;
                        }
                    }
                    Request::Capture(t) => pending_capture = Some(t),
                }
            }
            let Some(tab) = pending_capture else { continue };
            // Only the visible tab renders; a hidden one would give a stale or blank frame.
            if active != Some(tab) {
                continue;
            }
            let Some(port) = devtools::port(&data_dir) else { continue };
            let shot = targets
                .get(&tab)
                .and_then(|id| capture(port, id))
                .or_else(|| {
                    let id = find_target(port, tab, &targets)?;
                    let shot = capture(port, &id);
                    targets.insert(tab, id);
                    shot
                });
            if let Some(img) = shot {
                on_thumb(tab, img);
            }
        }
    });
    tx
}

struct Session {
    ws: WebSocket<TcpStream>,
    next: u64,
}

impl Session {
    fn open(port: u16, target: &str) -> Option<Self> {
        let stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
        stream.set_read_timeout(Some(Duration::from_millis(1500))).ok()?;
        stream.set_write_timeout(Some(Duration::from_millis(1500))).ok()?;
        let url = format!("ws://127.0.0.1:{port}/devtools/page/{target}");
        let (ws, _) = tungstenite::client(url.as_str(), stream).ok()?;
        Some(Self { ws, next: 1 })
    }

    fn call(&mut self, method: &str, params: Value) -> Option<Value> {
        let id = self.next;
        self.next += 1;
        let msg = json!({ "id": id, "method": method, "params": params }).to_string();
        self.ws.send(Message::text(msg)).ok()?;
        loop {
            let Message::Text(text) = self.ws.read().ok()? else { continue };
            let v: Value = serde_json::from_str(text.as_str()).ok()?;
            if v["id"].as_u64() == Some(id) {
                return v.get("result").cloned();
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.ws.close(None);
    }
}

/// Finds the page target whose `window.__cedericTab` is `tab`.
fn find_target(port: u16, tab: u32, known: &HashMap<u32, String>) -> Option<String> {
    let all = devtools::page_targets(port)?;
    // Probe targets not yet assigned to another tab first.
    all.iter()
        .filter(|id| !known.values().any(|k| k == *id))
        .chain(all.iter().filter(|id| known.values().any(|k| k == *id)))
        .find(|id| {
            Session::open(port, id)
                .and_then(|mut s| {
                    s.call("Runtime.evaluate", json!({ "expression": "window.__cedericTab", "returnByValue": true }))
                })
                .and_then(|r| r["result"]["value"].as_u64())
                == Some(u64::from(tab))
        })
        .cloned()
}

/// A small snapshot of what the tab currently shows.
fn capture(port: u16, target: &str) -> Option<tiny_skia::Pixmap> {
    capture_at(port, target, WIDTH)
}

/// Like `capture`, scaled to at most `width` pixels wide.
///
/// The screenshot is taken at the page's own size and shrunk here: asking the engine for a scaled
/// `clip` makes it re-render the page at another size, which shows as the page resizing or flashing.
fn capture_at(port: u16, target: &str, width: f64) -> Option<tiny_skia::Pixmap> {
    let mut s = Session::open(port, target)?;
    let r = s.call("Page.captureScreenshot", json!({ "format": "png", "optimizeForSpeed": true }))?;
    let full = tiny_skia::Pixmap::decode_png(&crate::favicons::base64_decode(r["data"].as_str()?)?).ok()?;
    let k = (width as f32 / full.width() as f32).min(1.0);
    let (tw, th) = (((full.width() as f32 * k).round() as u32).max(1), ((full.height() as f32 * k).round() as u32).max(1));
    let mut small = tiny_skia::Pixmap::new(tw, th)?;
    let paint = tiny_skia::PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() };
    small.draw_pixmap(0, 0, full.as_ref(), &paint, tiny_skia::Transform::from_scale(k, k), None);
    Some(small)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drives a running Cederic through its debug port: `CEDERIC_EVAL='<js>' cargo test -- --ignored live_eval`.
    /// Evaluates the JS in every page target and prints the results, then captures tab 1.
    #[test]
    #[ignore]
    fn live_eval() {
        let dir = std::path::PathBuf::from(std::env::var("LOCALAPPDATA").unwrap()).join("Cederic");
        let port = devtools::port(&dir).expect("port");
        let js = std::env::var("CEDERIC_EVAL").unwrap_or_else(|_| "location.href".into());
        for id in devtools::page_targets(port).expect("targets") {
            let r = Session::open(port, &id)
                .and_then(|mut s| s.call("Runtime.evaluate", json!({ "expression": js, "returnByValue": true, "awaitPromise": true, "userGesture": true })));
            println!("{id}: {}", r.map(|v| v["result"]["value"].to_string()).unwrap_or_default());
        }
        // CEDERIC_SHOT=<dir>: also save a full-size PNG of every page, so a page's own
        // rendering can be inspected without capturing the screen.
        if let Ok(dir) = std::env::var("CEDERIC_SHOT") {
            for id in devtools::page_targets(port).expect("targets") {
                let Some(img) = capture_at(port, &id, 4000.0) else { continue };
                let path = std::path::Path::new(&dir).join(format!("{id}.png"));
                img.save_png(&path).unwrap();
                println!("saved {}", path.display());
            }
        }
        for tab in 1..=3 {
            let shot = find_target(port, tab, &HashMap::new()).and_then(|id| capture(port, &id));
            println!("tab {tab} thumbnail: {:?}", shot.map(|s| (s.width(), s.height())));
        }
    }
}
