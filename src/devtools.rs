//! Docked DevTools. WebView2 only offers DevTools as a separate window, so the browser runs with
//! a loopback-only debugging port (picked by the OS) and loads the DevTools frontend that the
//! engine itself serves into a pane next to the page.

use std::{
    io::{Read, Write},
    net::TcpStream,
    path::Path,
    time::Duration,
};

use serde::Deserialize;

/// Invisible marker put in a tab's title so the tab can be found in the target list.
pub const MARKER: char = '\u{2063}';

#[derive(Deserialize)]
struct Target {
    id: String,
    title: String,
    #[serde(rename = "type")]
    kind: String,
}

/// The port WebView2 picked for `--remote-debugging-port=0`.
pub fn port(data_dir: &Path) -> Option<u16> {
    let text = std::fs::read_to_string(data_dir.join("EBWebView").join("DevToolsActivePort")).ok()?;
    text.lines().next()?.trim().parse().ok()
}

fn get(port: u16, path: &str) -> Option<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(Duration::from_millis(500))).ok()?;
    write!(stream, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").ok()?;
    let mut buf = String::new();
    stream.read_to_string(&mut buf).ok()?;
    buf.split_once("\r\n\r\n").map(|(_, body)| body.to_owned())
}

/// Finds the page whose title is `marker` and returns the docked DevTools URL for it.
/// Retries briefly because the engine updates its target list asynchronously.
pub fn frontend_url(data_dir: &Path, marker: &str) -> Option<String> {
    let port = port(data_dir)?;
    for _ in 0..40 {
        if let Some(body) = get(port, "/json/list") {
            let targets: Vec<Target> = serde_json::from_str(&body).unwrap_or_default();
            if let Some(t) = targets.iter().find(|t| t.kind == "page" && t.title == marker) {
                return Some(format!(
                    "http://127.0.0.1:{port}/devtools/inspector.html?ws=127.0.0.1:{port}/devtools/page/{}",
                    t.id
                ));
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    None
}
