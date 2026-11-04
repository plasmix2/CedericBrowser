//! Turns whatever the user typed in the address bar into an encrypted URL to load,
//! upgrades insecure `http://` URLs to `https://`, and extracts search queries from URLs.

use url::Url;

const SEARCH: &str = "https://search.brave.com/search?q=";
const ALLOWED_SCHEMES: &[&str] = &["http", "https", "file", "view-source"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedInput {
    pub url: String,
    pub query: String,
    pub engine: &'static str,
    pub is_search: bool,
}

/// Returns `None` for empty input or for schemes we refuse to load (e.g. `javascript:`).
/// Non-local `http://` URLs are automatically upgraded to `https://` so traffic on Wi-Fi stays encrypted.
#[cfg(test)]
pub fn resolve(input: &str) -> Option<String> {
    resolve_with_info(input).map(|r| r.url)
}

/// Resolves user input into a URL along with search/history metadata.
pub fn resolve_with_info(input: &str) -> Option<ResolvedInput> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }

    if let Ok(mut u) = Url::parse(s) {
        if ALLOWED_SCHEMES.contains(&u.scheme()) {
            // Upgrade non-local http:// to https:// to prevent Wi-Fi eavesdropping.
            if u.scheme() == "http"
                && let Some(host) = u.host_str()
                && !is_local(host)
            {
                let _ = u.set_scheme("https");
            }
            let url_str: String = u.into();
            if let Some((q, engine)) = extract_search(&url_str) {
                return Some(ResolvedInput {
                    url: url_str,
                    query: q,
                    engine,
                    is_search: true,
                });
            }
            return Some(ResolvedInput {
                query: s.to_owned(),
                url: url_str,
                engine: "Direct HTTPS",
                is_search: false,
            });
        }
        // "localhost:3000" parses with scheme "localhost"; fall through to host handling.
        if !looks_like_host(s) {
            // A bare "scheme:..." with no spaces is a URL we won't load; with spaces it's a query.
            return s.contains(char::is_whitespace).then(|| ResolvedInput {
                url: search(s),
                query: s.to_owned(),
                engine: "Brave Search",
                is_search: true,
            });
        }
    }

    if looks_like_host(s) {
        let scheme = if is_local(s) { "http" } else { "https" };
        if let Ok(u) = Url::parse(&format!("{scheme}://{s}")) {
            let url_str: String = u.into();
            if let Some((q, engine)) = extract_search(&url_str) {
                return Some(ResolvedInput {
                    url: url_str,
                    query: q,
                    engine,
                    is_search: true,
                });
            }
            return Some(ResolvedInput {
                query: s.to_owned(),
                url: url_str,
                engine: if scheme == "https" { "Direct HTTPS" } else { "Localhost" },
                is_search: false,
            });
        }
    }

    Some(ResolvedInput {
        url: search(s),
        query: s.to_owned(),
        engine: "Brave Search",
        is_search: true,
    })
}

/// If `url` is a non-local `http://` URL, returns its `https://` equivalent for automatic upgrade.
pub fn upgrade_to_https(url: &str) -> Option<String> {
    let mut parsed = Url::parse(url).ok()?;
    if parsed.scheme() != "http" {
        return None;
    }
    let host = parsed.host_str()?;
    if is_local(host) {
        return None;
    }
    parsed.set_scheme("https").ok()?;
    Some(parsed.into())
}

/// Determines whether a WebView navigation target scheme is safe to load.
pub fn is_allowed_navigation(url: &str) -> bool {
    if url == "about:blank" || url.starts_with("data:text/html") {
        return true;
    }
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };
    ALLOWED_SCHEMES.contains(&parsed.scheme())
}

/// Extracts the human-readable search query and engine name if `url` is a known search engine URL.
pub fn extract_search(url: &str) -> Option<(String, &'static str)> {
    let parsed = Url::parse(url).ok()?;
    if parsed.scheme() != "https" && parsed.scheme() != "http" {
        return None;
    }
    let host = parsed.host_str()?.to_ascii_lowercase();
    let path = parsed.path();

    let (engine, param) = if host == "search.brave.com" && path.starts_with("/search") {
        ("Brave Search", "q")
    } else if (host == "google.com" || host.ends_with(".google.com")) && path.starts_with("/search") {
        ("Google", "q")
    } else if host == "duckduckgo.com" || host.ends_with(".duckduckgo.com") {
        ("DuckDuckGo", "q")
    } else if (host == "bing.com" || host.ends_with(".bing.com")) && path.starts_with("/search") {
        ("Bing", "q")
    } else if host == "startpage.com" || host.ends_with(".startpage.com") {
        ("Startpage", "q")
    } else if (host == "kagi.com" || host.ends_with(".kagi.com")) && path.starts_with("/search") {
        ("Kagi", "q")
    } else if (host == "ecosia.org" || host.ends_with(".ecosia.org")) && path.starts_with("/search") {
        ("Ecosia", "q")
    } else if host.ends_with(".yahoo.com") && path.starts_with("/search") {
        ("Yahoo", "p")
    } else {
        return None;
    };

    for (k, v) in parsed.query_pairs() {
        if (k == param || (engine == "Startpage" && k == "query")) && !v.trim().is_empty() {
            return Some((v.trim().to_owned(), engine));
        }
    }
    None
}

fn search(q: &str) -> String {
    let encoded: String = url::form_urlencoded::byte_serialize(q.as_bytes()).collect();
    format!("{SEARCH}{encoded}")
}

fn host_part(s: &str) -> &str {
    let h = s.split(['/', '?', '#']).next().unwrap_or(s);
    h.rsplit_once(':')
        .filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit()))
        .map_or(h, |(host, _)| host)
}

fn looks_like_host(s: &str) -> bool {
    if s.contains(char::is_whitespace) {
        return false;
    }
    let host = host_part(s);
    is_local(s)
        || host.parse::<std::net::IpAddr>().is_ok()
        || (host.contains('.')
            && !host.starts_with('.')
            && !host.ends_with('.')
            && host.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '.'))
}

pub fn is_local(s: &str) -> bool {
    let host = host_part(s);
    host == "localhost" || host.ends_with(".localhost") || host == "127.0.0.1" || host == "[::1]"
}

#[cfg(test)]
mod tests {
    use super::{extract_search, resolve, upgrade_to_https};

    #[test]
    fn resolves_inputs() {
        assert_eq!(resolve("example.com").unwrap(), "https://example.com/");
        assert_eq!(resolve("http://example.com/login").unwrap(), "https://example.com/login");
        assert_eq!(resolve("localhost:3000/x").unwrap(), "http://localhost:3000/x");
        assert_eq!(resolve("https://rust-lang.org").unwrap(), "https://rust-lang.org/");
        assert_eq!(resolve("rust borrow checker").unwrap(), "https://search.brave.com/search?q=rust+borrow+checker");
        assert_eq!(resolve("rust").unwrap(), "https://search.brave.com/search?q=rust");
        assert_eq!(resolve("192.168.1.1").unwrap(), "https://192.168.1.1/");
        assert!(resolve("javascript:alert(1)").is_none());
        assert!(resolve("   ").is_none());
    }

    #[test]
    fn upgrades_insecure_http_urls() {
        assert_eq!(
            upgrade_to_https("http://search.brave.com/search?q=wifi"),
            Some("https://search.brave.com/search?q=wifi".into())
        );
        assert_eq!(upgrade_to_https("https://search.brave.com/search?q=wifi"), None);
        assert_eq!(upgrade_to_https("http://localhost:8080/test"), None);
        assert_eq!(upgrade_to_https("http://127.0.0.1:3000/"), None);
    }

    #[test]
    fn extracts_search_queries() {
        assert_eq!(
            extract_search("https://search.brave.com/search?q=rust+borrow+checker&source=web"),
            Some(("rust borrow checker".into(), "Brave Search"))
        );
        assert_eq!(
            extract_search("https://www.google.com/search?q=encrypted+dns"),
            Some(("encrypted dns".into(), "Google"))
        );
        assert_eq!(
            extract_search("https://duckduckgo.com/?q=wifi+privacy"),
            Some(("wifi privacy".into(), "DuckDuckGo"))
        );
        assert_eq!(extract_search("https://rust-lang.org/"), None);
    }
}
