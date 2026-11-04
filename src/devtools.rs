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

/// Invisible marker put in a tab's title so the tab can be found in the target list when its
/// URL alone is ambiguous.
pub const MARKER: char = '\u{2063}';

#[derive(Deserialize)]
struct Target {
    id: String,
    title: String,
    url: String,
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

fn host(url: &str) -> &str {
    url.split_once("://").map_or("", |(_, r)| r.split(['/', '?', '#']).next().unwrap_or(""))
}

/// Finds the tab's page target and returns the docked DevTools URL for it: by the marker title,
/// else by a unique URL, else by a unique host (pages may change their URL or title themselves).
/// Retries briefly because the engine updates its target list asynchronously.
pub fn frontend_url(data_dir: &Path, marker: &str, url: &str) -> Option<String> {
    let port = port(data_dir)?;
    let unique = |ts: Vec<&Target>| (ts.len() == 1).then(|| ts[0].id.clone());
    for attempt in 0..40 {
        if let Some(body) = get(port, "/json/list") {
            let targets: Vec<Target> = serde_json::from_str(&body).unwrap_or_default();
            let pages: Vec<&Target> = targets.iter().filter(|t| t.kind == "page").collect();
            let found = pages.iter().find(|t| t.title == marker).map(|t| t.id.clone()).or_else(|| {
                // Give the marker a few tries before trusting URL matches.
                (attempt >= 4).then(|| {
                    unique(pages.iter().copied().filter(|t| t.url == url).collect()).or_else(|| {
                        unique(pages.iter().copied().filter(|t| !host(url).is_empty() && host(&t.url) == host(url)).collect())
                    })
                })?
            });
            if let Some(id) = found {
                return Some(format!(
                    "http://127.0.0.1:{port}/devtools/inspector.html?ws=127.0.0.1:{port}/devtools/page/{id}"
                ));
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    None
}
