//! Tab thumbnails for hover cards. A worker thread talks to the engine's loopback DevTools port
//! (the same one the docked DevTools pane uses) and screenshots the visible tab, small and as JPEG.
//! Each tab's page carries `window.__cedericTab = <id>`, which is how its target is recognised.

use std::{
    collections::HashMap,
    net::TcpStream,
    path::PathBuf,
    sync::mpsc,
    time::Duration,
};

use serde_json::{Value, json};
use tungstenite::{Message, WebSocket};

use crate::devtools;

/// Thumbnail width in CSS pixels; the card shows it at about this size.
const WIDTH: f64 = 320.0;
/// While a tab stays active, refresh its thumbnail this often.
const REFRESH: Duration = Duration::from_secs(4);

pub enum Request {
    /// The tab now on screen (`None` while nothing should be captured, e.g. minimized).
    Active(Option<u32>),
    /// Capture this tab now (it just finished loading or was just shown).
    Capture(u32),
    /// The tab was closed.
    Forget(u32),
    /// Evaluate `js` in `tab` with `userGesture: true` over CDP (needed for Picture-in-Picture
    /// when triggered from the toolbar or popup WebView rather than inside the content page).
    EvalGesture(u32, String),
}

pub fn spawn(data_dir: PathBuf, on_thumb: impl Fn(u32, String) + Send + 'static) -> mpsc::Sender<Request> {
    let (tx, rx) = mpsc::channel::<Request>();
    std::thread::spawn(move || {
        let mut targets: HashMap<u32, String> = HashMap::new();
        let mut active: Option<u32> = None;
        loop {
            let tab = match rx.recv_timeout(REFRESH) {
                Ok(Request::Active(t)) => {
                    active = t;
                    continue;
                }
                Ok(Request::Forget(t)) => {
                    targets.remove(&t);
                    continue;
                }
                Ok(Request::EvalGesture(t, js)) => {
                    if let Some(port) = devtools::port(&data_dir) {
                        let _ = targets
                            .get(&t)
                            .and_then(|id| eval_gesture(port, id, &js))
                            .or_else(|| {
                                let id = find_target(port, t)?;
                                let res = eval_gesture(port, &id, &js);
                                targets.insert(t, id);
                                res
                            });
                    }
                    continue;
                }
                Ok(Request::Capture(t)) => t,
                Err(mpsc::RecvTimeoutError::Timeout) => match active {
                    Some(t) => t,
                    None => continue,
                },
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            };
            // Only the visible tab renders; a hidden one would give a stale or blank frame.
            if active != Some(tab) {
                continue;
            }
            let Some(port) = devtools::port(&data_dir) else { continue };
            let shot = targets
                .get(&tab)
                .and_then(|id| capture(port, id))
                .or_else(|| {
                    let id = find_target(port, tab)?;
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
fn find_target(port: u16, tab: u32) -> Option<String> {
    devtools::page_targets(port)?.into_iter().find(|id| {
        Session::open(port, id)
            .and_then(|mut s| {
                s.call("Runtime.evaluate", json!({ "expression": "window.__cedericTab", "returnByValue": true }))
            })
            .and_then(|r| r["result"]["value"].as_u64())
            == Some(u64::from(tab))
    })
}

/// A small JPEG of what the tab currently shows, as a data URL.
fn capture(port: u16, target: &str) -> Option<String> {
    let mut s = Session::open(port, target)?;
    let m = s.call("Page.getLayoutMetrics", json!({}))?;
    let vp = &m["cssVisualViewport"];
    let (w, h) = (vp["clientWidth"].as_f64()?, vp["clientHeight"].as_f64()?);
    if w < 2.0 || h < 2.0 {
        return None;
    }
    let clip = json!({
        "x": vp["pageX"].as_f64().unwrap_or(0.0),
        "y": vp["pageY"].as_f64().unwrap_or(0.0),
        "width": w,
        "height": h,
        "scale": (WIDTH / w).min(1.0),
    });
    let r = s.call("Page.captureScreenshot", json!({ "format": "jpeg", "quality": 60, "clip": clip }))?;
    Some(format!("data:image/jpeg;base64,{}", r["data"].as_str()?))
}

/// Evaluates `js` in `target` with a synthesized user gesture (`userGesture: true`).
fn eval_gesture(port: u16, target: &str, js: &str) -> Option<Value> {
    let mut s = Session::open(port, target)?;
    s.call(
        "Runtime.evaluate",
        json!({
            "expression": js,
            "userGesture": true,
            "awaitPromise": true,
            "returnByValue": true,
        }),
    )
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
        for tab in 1..=3 {
            let shot = find_target(port, tab).and_then(|id| capture(port, &id));
            println!("tab {tab} thumbnail: {} bytes", shot.map_or(0, |s| s.len()));
        }
    }
}
