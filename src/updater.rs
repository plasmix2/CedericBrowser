//! Checks GitHub releases on startup and applies updates when confirmed by user.

use std::time::Duration;
use serde::Deserialize;
use ureq::{
    Agent,
    tls::{TlsConfig, TlsProvider},
};

pub const CURRENT_VERSION: &str = "0.1.1";
pub const CURRENT_TAG: &str = "P1";

#[derive(Debug, Clone, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    pub browser_download_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReleaseInfo {
    pub tag_name: String,
    pub name: Option<String>,
    #[serde(default)]
    pub assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Clone)]
pub struct UpdateInfo {
    pub version: String,
    pub tag: String,
    pub download_url: String,
}

/// Spawns the startup check in a background thread.
pub fn check_on_startup(on_available: impl Fn(UpdateInfo) + Send + 'static) {
    std::thread::spawn(move || {
        // Wait 1.5 seconds so UI finishes loading first
        std::thread::sleep(Duration::from_millis(1500));
        if let Some(info) = check_latest_release() {
            on_available(info);
        }
    });
}

fn make_agent() -> Option<Agent> {
    Some(
        Agent::config_builder()
            .tls_config(TlsConfig::builder().provider(TlsProvider::NativeTls).build())
            .timeout_global(Some(Duration::from_secs(10)))
            .user_agent("PersonaBrowser-Updater")
            .build()
            .into(),
    )
}

pub fn check_latest_release() -> Option<UpdateInfo> {
    let agent = make_agent()?;
    let urls = [
        "https://api.github.com/repos/plasmix2/PersonaBrowser/releases/latest",
        "https://api.github.com/repos/plasmix2/CedericBrowser/releases/latest",
    ];
    for url in urls {
        if let Ok(mut resp) = agent.get(url).call() {
            if let Ok(body) = resp.body_mut().read_to_string() {
                if let Ok(rel) = serde_json::from_str::<ReleaseInfo>(&body) {
                    if is_newer_version(&rel.tag_name, rel.name.as_deref()) {
                        if let Some(asset) = rel.assets.iter().find(|a| {
                            a.name.eq_ignore_ascii_case("Persona.exe") || a.name.to_ascii_lowercase().ends_with(".exe")
                        }) {
                            let display_version = rel.name.unwrap_or_else(|| rel.tag_name.clone());
                            return Some(UpdateInfo {
                                version: display_version,
                                tag: rel.tag_name,
                                download_url: asset.browser_download_url.clone(),
                            });
                        }
                    }
                }
            }
        }
    }
    None
}

/// Compare tag / release name against CURRENT_VERSION and CURRENT_TAG.
pub fn is_newer_version(tag: &str, name: Option<&str>) -> bool {
    let tag = tag.trim();
    if tag.eq_ignore_ascii_case(CURRENT_TAG) {
        return false;
    }
    let parse_nums = |s: &str| -> Option<(u32, u32, u32)> {
        let clean: String = s
            .chars()
            .map(|c| if c.is_ascii_digit() || c == '.' { c } else { ' ' })
            .collect();
        let parts: Vec<&str> = clean.split_whitespace().collect();
        for part in parts {
            let nums: Vec<u32> = part.split('.').filter_map(|n| n.parse::<u32>().ok()).collect();
            if nums.len() >= 2 {
                return Some((nums[0], nums[1], nums.get(2).copied().unwrap_or(0)));
            }
        }
        None
    };

    let cur = parse_nums(CURRENT_VERSION).unwrap_or((0, 1, 1));
    if let Some(remote) = name.and_then(parse_nums).or_else(|| parse_nums(tag)) {
        return remote > cur;
    }
    false
}

/// Downloads and replaces the currently running executable, then spawns the new version and exits.
pub fn apply_update_and_restart(download_url: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let agent = make_agent().ok_or("failed to create http agent")?;
    let mut resp = agent.get(download_url).call()?;
    let bytes = resp.body_mut().read_to_vec()?;
    if bytes.len() < 100_000 {
        return Err("downloaded binary is too small or invalid".into());
    }

    let current_exe = std::env::current_exe()?;
    let exe_dir = current_exe.parent().ok_or("no parent dir")?;
    let old_exe = exe_dir.join("persona_prev.old");
    let new_tmp = exe_dir.join("persona_update.tmp");

    let _ = std::fs::remove_file(&old_exe);
    let _ = std::fs::remove_file(&new_tmp);

    std::fs::write(&new_tmp, &bytes)?;
    std::fs::rename(&current_exe, &old_exe)?;

    if let Err(e) = std::fs::rename(&new_tmp, &current_exe) {
        let _ = std::fs::rename(&old_exe, &current_exe);
        return Err(Box::new(e));
    }

    std::process::Command::new(&current_exe)
        .current_dir(exe_dir)
        .spawn()?;

    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_comparison() {
        assert!(!is_newer_version("P1", Some("Persona V0.1.1")));
        assert!(!is_newer_version("v0.1.0", Some("Persona V0.1.0")));
        assert!(is_newer_version("v0.1.2", Some("Persona V0.1.2")));
        assert!(is_newer_version("v0.2.0", Some("Persona V0.2.0")));
        assert!(is_newer_version("v1.0.0", Some("Persona V1.0.0")));
    }
}
