//! Encrypted search history store that records every search and the exact moment it occurred.

use crate::crypto;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_HISTORY_ENTRIES: usize = 500;
const DEDUP_WINDOW_MS: u64 = 4_000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryEntry {
    pub id: u64,
    /// The search query text (or host/address looked up).
    pub query: String,
    /// The resolved encrypted URL (`https://...`).
    pub url: String,
    /// Search engine or navigation source (e.g. "Brave Search", "Direct HTTPS").
    pub engine: String,
    /// Whether this entry was a search query vs. a direct URL navigation.
    pub is_search: bool,
    /// Exact moment in milliseconds since the Unix epoch.
    pub timestamp_ms: u64,
    /// Human-readable civil timestamp of the moment (`YYYY-MM-DD HH:MM:SS UTC`).
    pub moment: String,
}

#[derive(Serialize, Deserialize, Default)]
struct PersistedHistory {
    next_id: u64,
    entries: Vec<HistoryEntry>,
}

pub struct HistoryStore {
    file_path: PathBuf,
    key: [u8; 32],
    next_id: u64,
    entries: Vec<HistoryEntry>,
}

impl HistoryStore {
    /// Opens or initializes the encrypted search history store in `dir`.
    pub fn open(dir: &Path) -> Self {
        let _ = fs::create_dir_all(dir);
        let key_path = dir.join("history.key");
        let file_path = dir.join("history.enc");

        let seed = match fs::read(&key_path) {
            Ok(bytes) if bytes.len() == 32 => {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes);
                arr
            }
            _ => {
                let fresh = crypto::random_bytes_32();
                let _ = fs::write(&key_path, fresh);
                fresh
            }
        };
        let key = crypto::derive_vault_key(&seed);

        let (next_id, entries) = fs::read(&file_path)
            .ok()
            .and_then(|blob| crypto::decrypt_envelope(&key, &blob))
            .and_then(|plain| serde_json::from_slice::<PersistedHistory>(&plain).ok())
            .map(|p| (p.next_id.max(1), p.entries))
            .unwrap_or_else(|| (1, Vec::new()));

        Self {
            file_path,
            key,
            next_id,
            entries,
        }
    }

    pub fn entries(&self) -> &[HistoryEntry] {
        &self.entries
    }

    /// Records a search or lookup at the current moment and persists it encrypted to disk.
    pub fn record(&mut self, query: &str, url: &str, engine: &str, is_search: bool) {
        let q = query.trim();
        if q.is_empty() || url.is_empty() {
            return;
        }
        let now_ms = now_epoch_ms();
        let moment = format_utc_moment(now_ms);

        // Avoid duplicate records when an address-bar search immediately triggers a page-load event.
        if let Some(top) = self.entries.first_mut()
            && now_ms.saturating_sub(top.timestamp_ms) <= DEDUP_WINDOW_MS
            && (top.query.eq_ignore_ascii_case(q) || top.url == url)
        {
            if is_search && !top.is_search {
                top.query = q.to_owned();
                top.engine = engine.to_owned();
                top.is_search = true;
            }
            top.url = url.to_owned();
            self.save();
            return;
        }

        let id = self.next_id;
        self.next_id += 1;

        self.entries.insert(
            0,
            HistoryEntry {
                id,
                query: q.to_owned(),
                url: url.to_owned(),
                engine: engine.to_owned(),
                is_search,
                timestamp_ms: now_ms,
                moment,
            },
        );

        if self.entries.len() > MAX_HISTORY_ENTRIES {
            self.entries.truncate(MAX_HISTORY_ENTRIES);
        }
        self.save();
    }

    /// Removes a single history entry by ID and updates the encrypted file.
    pub fn remove(&mut self, id: u64) -> bool {
        let len_before = self.entries.len();
        self.entries.retain(|e| e.id != id);
        let removed = self.entries.len() != len_before;
        if removed {
            self.save();
        }
        removed
    }

    /// Clears all saved search history and removes the encrypted payload.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.save();
    }

    fn save(&self) {
        let persisted = PersistedHistory {
            next_id: self.next_id,
            entries: self.entries.clone(),
        };
        if let Ok(json) = serde_json::to_vec(&persisted) {
            let encrypted = crypto::encrypt_envelope(&self.key, &json);
            let tmp = self.file_path.with_extension("enc.tmp");
            if fs::write(&tmp, &encrypted).is_ok() {
                let _ = fs::rename(&tmp, &self.file_path);
            } else {
                let _ = fs::write(&self.file_path, &encrypted);
            }
        }
    }
}

pub fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Converts Unix epoch milliseconds into a `YYYY-MM-DD HH:MM:SS UTC` civil timestamp
/// using Howard Hinnant's civil-from-days algorithm.
pub fn format_utc_moment(epoch_ms: u64) -> String {
    let total_secs = (epoch_ms / 1000) as i64;
    let days = total_secs.div_euclid(86_400);
    let secs_of_day = total_secs.rem_euclid(86_400) as u32;

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };

    let hour = secs_of_day / 3600;
    let min = (secs_of_day % 3600) / 60;
    let sec = secs_of_day % 60;

    format!("{year:04}-{m:02}-{d:02} {hour:02}:{min:02}:{sec:02} UTC")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_civil_moments_accurately() {
        // 2026-11-04T13:42:13Z = 1793799733000 ms
        assert_eq!(format_utc_moment(1_793_799_733_000), "2026-11-04 13:42:13 UTC");
        assert_eq!(format_utc_moment(0), "1970-01-01 00:00:00 UTC");
    }

    #[test]
    fn encrypted_store_roundtrips_and_deduplicates() {
        let dir = std::env::temp_dir().join(format!("cederic_test_{}", now_epoch_ms()));
        let _ = fs::remove_dir_all(&dir);

        {
            let mut store = HistoryStore::open(&dir);
            assert!(store.entries().is_empty());

            store.record(
                "rust memory safety",
                "https://search.brave.com/search?q=rust+memory+safety",
                "Brave Search",
                true,
            );
            // Immediate duplicate within 4s window should not create a second entry.
            store.record(
                "rust memory safety",
                "https://search.brave.com/search?q=rust+memory+safety",
                "Brave Search",
                true,
            );
            store.record(
                "encrypted dns over https",
                "https://search.brave.com/search?q=encrypted+dns+over+https",
                "Brave Search",
                true,
            );
            assert_eq!(store.entries().len(), 2);
            assert_eq!(store.entries()[0].query, "encrypted dns over https");
            assert!(!store.entries()[0].moment.is_empty());
        }

        // Verify the file on disk is encrypted (does not contain plaintext query).
        let raw_bytes = fs::read(dir.join("history.enc")).unwrap();
        let raw_lossy = String::from_utf8_lossy(&raw_bytes);
        assert!(!raw_lossy.contains("rust memory safety"));
        assert!(!raw_lossy.contains("encrypted dns over https"));

        // Re-open from disk and verify decryption restores entries and moments.
        let mut reopened = HistoryStore::open(&dir);
        assert_eq!(reopened.entries().len(), 2);
        assert_eq!(reopened.entries()[0].query, "encrypted dns over https");
        assert_eq!(reopened.entries()[1].query, "rust memory safety");

        let first_id = reopened.entries()[0].id;
        assert!(reopened.remove(first_id));
        assert_eq!(reopened.entries().len(), 1);

        reopened.clear();
        assert!(reopened.entries().is_empty());

        let _ = fs::remove_dir_all(&dir);
    }
}
