//! Search suggestions, fetched on a background thread over one kept-alive HTTPS connection.

use std::{sync::mpsc, time::Duration};

use ureq::{
    Agent,
    tls::{TlsConfig, TlsProvider},
};

const ENDPOINT: &str = "https://search.brave.com/api/suggest";

pub struct Request {
    pub seq: u64,
    pub query: String,
}

pub struct Response {
    pub seq: u64,
    pub items: Vec<String>,
}

/// Starts the worker. Only the newest queued query is fetched, so fast typing never
/// builds up a backlog. An empty query just warms the connection.
pub fn spawn(on_result: impl Fn(Response) + Send + 'static) -> mpsc::Sender<Request> {
    let (tx, rx) = mpsc::channel::<Request>();
    std::thread::spawn(move || {
        let agent: Agent = Agent::config_builder()
            .tls_config(TlsConfig::builder().provider(TlsProvider::NativeTls).build())
            .timeout_global(Some(Duration::from_secs(3)))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36")
            .build()
            .into();
        let mut cache: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
        while let Ok(mut req) = rx.recv() {
            while let Ok(newer) = rx.try_recv() {
                req = newer;
            }
            let trimmed = req.query.trim().to_lowercase();
            if trimmed.is_empty() {
                let _ = fetch(&agent, "a");
                continue;
            }
            if let Some(cached) = cache.get(&trimmed) {
                on_result(Response { seq: req.seq, items: cached.clone() });
                continue;
            }
            let items = fetch(&agent, &trimmed).unwrap_or_default();
            if cache.len() >= 256 {
                cache.clear();
            }
            cache.insert(trimmed, items.clone());
            on_result(Response { seq: req.seq, items });
        }
    });
    tx
}

fn fetch(agent: &Agent, query: &str) -> Option<Vec<String>> {
    let body = agent
        .get(ENDPOINT)
        .query("q", query)
        .query("rich", "false")
        .call()
        .ok()?
        .body_mut()
        .read_to_string()
        .ok()?;
    // OpenSearch format: ["query", ["suggestion", ...]]
    let (_, items): (String, Vec<String>) = serde_json::from_str(&body).ok()?;
    Some(items.into_iter().take(7).collect())
}
