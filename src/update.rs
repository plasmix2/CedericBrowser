//! Automatic updates from this repository's GitHub Releases.
//!
//! A background thread looks at the latest releases a little after startup. When one has a higher
//! version than this build and carries a Windows `.exe`, it is downloaded, checked (size, `MZ`
//! header, and the SHA-256 digest GitHub publishes for the asset), and swapped in for the running
//! exe: Windows lets a running exe be renamed, so the current one becomes `<name>.old.exe` and the
//! new one takes its name. The next start (or "Restart to update") runs the new version, and
//! `cleanup` removes the old file.
//!
//! Only release assets of this repository are accepted, over HTTPS. Development builds (run from a
//! cargo `target` folder) never update themselves, and `PERSONA_NO_UPDATE=1` turns updates off.

use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use sha2::{Digest, Sha256};
use ureq::{
    Agent,
    tls::{TlsConfig, TlsProvider},
};

const REPO: &str = "plasmix2/PersonaBrowser";
/// Release downloads are only taken from this repository (also under its former name).
const DOWNLOAD_PREFIXES: [&str; 2] = [
    "https://github.com/plasmix2/PersonaBrowser/releases/download/",
    "https://github.com/plasmix2/CedericBrowser/releases/download/",
];
/// Release exes larger than this are refused.
const MAX_EXE: u64 = 200 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Version(pub u32, pub u32, pub u32);

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// This build's version, from `Cargo.toml`.
pub fn current() -> Version {
    parse_version(env!("CARGO_PKG_VERSION")).unwrap_or(Version(0, 0, 0))
}

/// The first `X.Y` or `X.Y.Z` in a tag or release name: `v0.3.0`, `Persona V0.3.0`, `0.3`.
pub fn parse_version(s: &str) -> Option<Version> {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() && (i == 0 || !b[i - 1].is_ascii_digit()) {
            let parts: Vec<&str> = s[i..]
                .split(|c: char| !(c.is_ascii_digit() || c == '.'))
                .next()
                .unwrap_or("")
                .split('.')
                .filter(|p| !p.is_empty())
                .take(3)
                .collect();
            if parts.len() >= 2 {
                let n = |k: usize| parts.get(k).and_then(|p| p.parse::<u32>().ok()).unwrap_or(0);
                return Some(Version(n(0), n(1), n(2)));
            }
        }
        i += 1;
    }
    None
}

/// A newer release's Windows exe.
#[derive(Debug, PartialEq, Eq)]
pub struct Candidate {
    pub version: Version,
    pub url: String,
    pub size: u64,
    /// Lowercase hex SHA-256, when GitHub provides it.
    pub sha256: Option<String>,
}

/// Picks the highest-versioned release newer than `than` that has a usable exe asset, from the
/// GitHub "list releases" JSON. Drafts are skipped; pre-releases count (test builds ship as those).
pub fn pick(releases_json: &str, than: Version) -> Option<Candidate> {
    let v: serde_json::Value = serde_json::from_str(releases_json).ok()?;
    let mut best: Option<Candidate> = None;
    for r in v.as_array()? {
        if r.get("draft").and_then(|d| d.as_bool()).unwrap_or(false) {
            continue;
        }
        let text = |k: &str| r.get(k).and_then(|x| x.as_str()).unwrap_or("");
        let Some(version) = parse_version(text("tag_name")).or_else(|| parse_version(text("name"))) else { continue };
        if version <= than || best.as_ref().is_some_and(|b| b.version >= version) {
            continue;
        }
        let assets = r.get("assets").and_then(|a| a.as_array()).cloned().unwrap_or_default();
        let exe = assets.iter().find(|a| {
            let name = a.get("name").and_then(|n| n.as_str()).unwrap_or("").to_ascii_lowercase();
            let url = a.get("browser_download_url").and_then(|u| u.as_str()).unwrap_or("");
            name.ends_with(".exe") && DOWNLOAD_PREFIXES.iter().any(|p| url.starts_with(p))
        });
        let Some(a) = exe else { continue };
        let size = a.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
        if size == 0 || size > MAX_EXE {
            continue;
        }
        let sha256 = a
            .get("digest")
            .and_then(|d| d.as_str())
            .and_then(|d| d.strip_prefix("sha256:"))
            .filter(|h| h.len() == 64 && h.bytes().all(|c| c.is_ascii_hexdigit()))
            .map(|h| h.to_ascii_lowercase());
        best = Some(Candidate {
            version,
            url: a.get("browser_download_url").and_then(|u| u.as_str()).unwrap_or("").to_owned(),
            size,
            sha256,
        });
    }
    best
}

fn agent() -> Agent {
    Agent::config_builder()
        .tls_config(TlsConfig::builder().provider(TlsProvider::NativeTls).build())
        .timeout_global(Some(Duration::from_secs(300)))
        .user_agent(concat!("Persona-updater/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// Whether this exe may replace itself: not a cargo build, and not turned off.
fn enabled(exe: &Path) -> bool {
    if std::env::var_os("PERSONA_NO_UPDATE").is_some_and(|v| !v.is_empty() && v != "0") {
        return false;
    }
    !exe.components().any(|c| c.as_os_str().eq_ignore_ascii_case("target"))
}

fn old_path(exe: &Path) -> PathBuf {
    exe.with_extension("old.exe")
}

/// Removes the exe a previous update replaced. Call at startup.
pub fn cleanup() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(old_path(&exe));
    }
}

/// Checks for a newer release a little after startup and installs it. `on_ready` gets the new
/// version once it is in place (it runs from the next start, or after a restart).
pub fn spawn(on_ready: impl FnOnce(Version) + Send + 'static) {
    std::thread::spawn(move || {
        // Stay out of the way while the first page loads.
        std::thread::sleep(Duration::from_secs(15));
        if let Some(v) = check_and_install() {
            on_ready(v);
        }
    });
}

fn check_and_install() -> Option<Version> {
    let exe = std::env::current_exe().ok()?;
    if !enabled(&exe) {
        return None;
    }
    let agent = agent();
    let list = agent
        .get(&format!("https://api.github.com/repos/{REPO}/releases?per_page=15"))
        .header("Accept", "application/vnd.github+json")
        .call()
        .ok()?
        .body_mut()
        .with_config()
        .limit(4 * 1024 * 1024)
        .read_to_string()
        .ok()?;
    let c = pick(&list, current())?;
    let bytes = download(&agent, &c)?;
    install(&exe, &bytes).ok()?;
    Some(c.version)
}

fn download(agent: &Agent, c: &Candidate) -> Option<Vec<u8>> {
    let mut resp = agent.get(&c.url).call().ok()?;
    let mut bytes = Vec::with_capacity(c.size as usize);
    resp.body_mut().with_config().limit(MAX_EXE).reader().read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 != c.size || !bytes.starts_with(b"MZ") {
        return None;
    }
    if let Some(want) = &c.sha256 {
        let got: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        if &got != want {
            return None;
        }
    }
    Some(bytes)
}

/// Puts `bytes` in place of the running `exe`, keeping the old one until the next start.
fn install(exe: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let new = exe.with_extension("new.exe");
    {
        let mut f = std::fs::File::create(&new)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    let old = old_path(exe);
    let _ = std::fs::remove_file(&old);
    if let Err(e) = std::fs::rename(exe, &old) {
        let _ = std::fs::remove_file(&new);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&new, exe) {
        // Put the running version back so the next start still works.
        let _ = std::fs::rename(&old, exe);
        let _ = std::fs::remove_file(&new);
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_versions() {
        assert_eq!(parse_version("v0.3.0"), Some(Version(0, 3, 0)));
        assert_eq!(parse_version("Persona V0.1.0"), Some(Version(0, 1, 0)));
        assert_eq!(parse_version("v1.2"), Some(Version(1, 2, 0)));
        assert_eq!(parse_version("v0.3.0-beta.2"), Some(Version(0, 3, 0)));
        assert_eq!(parse_version("P1"), None);
        assert_eq!(parse_version("Releases"), None);
        assert!(Version(0, 10, 0) > Version(0, 9, 9));
    }

    #[test]
    fn picks_newest_release_with_exe() {
        let json = r#"[
          {"tag_name":"v0.4.0","name":"Draft","draft":true,"assets":[{"name":"Persona.exe","size":10,"browser_download_url":"https://github.com/plasmix2/CedericBrowser/releases/download/v0.4.0/Persona.exe"}]},
          {"tag_name":"v0.3.1","name":"No exe","draft":false,"assets":[{"name":"notes.txt","size":10,"browser_download_url":"https://github.com/plasmix2/CedericBrowser/releases/download/v0.3.1/notes.txt"}]},
          {"tag_name":"v0.3.0","draft":false,"prerelease":true,"assets":[{"name":"Persona-v0.3.0.exe","size":1234,"digest":"sha256:ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef0123456789","browser_download_url":"https://github.com/plasmix2/CedericBrowser/releases/download/v0.3.0/Persona-v0.3.0.exe"}]},
          {"tag_name":"v0.2.5","draft":false,"assets":[{"name":"Persona.exe","size":99,"browser_download_url":"https://github.com/plasmix2/CedericBrowser/releases/download/v0.2.5/Persona.exe"}]},
          {"tag_name":"P1","name":"Persona V0.1.0","draft":false,"assets":[]}
        ]"#;
        let c = pick(json, Version(0, 2, 0)).expect("v0.3.0");
        assert_eq!(c.version, Version(0, 3, 0));
        assert_eq!(c.size, 1234);
        assert_eq!(c.sha256.as_deref(), Some("abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"));
        // Nothing newer than what is running.
        assert_eq!(pick(json, Version(0, 3, 0)), None);
        // Assets from anywhere but this repository's releases are ignored.
        let foreign = r#"[{"tag_name":"v9.0.0","draft":false,"assets":[{"name":"Persona.exe","size":5,"browser_download_url":"https://evil.example/Persona.exe"}]}]"#;
        assert_eq!(pick(foreign, Version(0, 2, 0)), None);
    }

    #[test]
    fn install_swaps_the_exe_and_keeps_the_old_one() {
        let dir = std::env::temp_dir().join(format!("persona_update_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("Persona.exe");
        std::fs::write(&exe, b"MZ old").unwrap();
        install(&exe, b"MZ new").unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"MZ new");
        assert_eq!(std::fs::read(dir.join("Persona.old.exe")).unwrap(), b"MZ old");
        assert!(!dir.join("Persona.new.exe").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dev_builds_do_not_update() {
        assert!(!enabled(Path::new("C:\\src\\CedericBrowser\\target\\release\\Persona.exe")));
    }
}
