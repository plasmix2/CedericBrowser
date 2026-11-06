//! Finding a tab's page on the engine's loopback-only debugging port (picked by the OS). Persona's
//! own DevTools (`inspector.rs`) and the tab snapshots (`thumbs.rs`) talk to pages through it.

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
    // The engine keeps the connection open after replying, so read exactly Content-Length bytes
    // instead of waiting for it to close.
    let mut buf = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    loop {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else { continue };
        let head = String::from_utf8_lossy(&buf[..end]);
        let len = head
            .lines()
            .find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("content-length")))
            .and_then(|(_, v)| v.trim().parse::<usize>().ok())?;
        if buf.len() >= end + 4 + len {
            return String::from_utf8(buf[end + 4..end + 4 + len].to_vec()).ok();
        }
    }
    None
}

/// Ids of all page targets (tabs, the toolbar, popups).
pub fn page_targets(port: u16) -> Option<Vec<String>> {
    let targets: Vec<Target> = serde_json::from_str(&get(port, "/json/list")?).ok()?;
    Some(targets.into_iter().filter(|t| t.kind == "page").map(|t| t.id).collect())
}

fn host(url: &str) -> &str {
    url.split_once("://").map_or("", |(_, r)| r.split(['/', '?', '#']).next().unwrap_or(""))
}

/// Finds the tab's page target: by the marker title, else by a unique URL, else by a unique host
/// (pages may change their URL or title themselves). Returns the debugging port and the target
/// id. Retries briefly because the engine updates its target list asynchronously.
pub fn find_page(data_dir: &Path, marker: &str, url: &str) -> Option<(u16, String)> {
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
                return Some((port, id));
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::get;
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };

    /// WebView2's DevTools server answers but keeps the connection open; reading must not wait for EOF.
    #[test]
    fn reads_response_from_keep_alive_server() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut req = [0u8; 1024];
            let _ = conn.read(&mut req);
            let body = r#"[{"id":"A","title":"t","url":"https://x/","type":"page"}]"#;
            write!(conn, "HTTP/1.1 200 OK\r\nContent-Length:{}\r\n\r\n{body}", body.len()).unwrap();
            std::thread::sleep(std::time::Duration::from_secs(2));
        });
        let start = std::time::Instant::now();
        let body = get(port, "/json/list").expect("response");
        assert!(body.starts_with("[{\"id\":\"A\""));
        assert!(start.elapsed() < std::time::Duration::from_millis(400));
        drop(server);
    }

    /// Run with a Cederic window open on Brave Search: `cargo test -- --ignored live`.
    #[test]
    #[ignore]
    fn live_lookup_finds_search_tab() {
        let dir = std::path::PathBuf::from(std::env::var("LOCALAPPDATA").unwrap()).join("Cederic");
        let found = super::find_page(&dir, "no-marker", "https://search.brave.com/search?q=x");
        println!("{found:?}");
        assert!(found.is_some());
    }
}
