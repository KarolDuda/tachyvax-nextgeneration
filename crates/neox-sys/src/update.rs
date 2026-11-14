//! Updates from GitHub Releases (REST, no token; the repository must be public).
//! The installer is downloaded over HTTPS from github.com only and started silently.

use crate::runner::{powershell, Runner};
use serde::Serialize;
use serde_json::Value;
use std::cmp::Ordering;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

pub const OWNER: &str = "KarolDuda";
pub const REPO: &str = "NEOX";
pub const ASSET_NAME: &str = "Neox_Setup.exe";

/// Build-time version: CI sets NEOX_VERSION (e.g. 3.1.57); local builds fall back to the Cargo version.
pub fn current_version() -> &'static str { option_env!("NEOX_VERSION").unwrap_or(env!("CARGO_PKG_VERSION")) }

pub fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let mut it = s.trim().trim_start_matches(['v', 'V']).split('.');
    let a = it.next()?.parse().ok()?;
    let b = it.next().unwrap_or("0").parse().ok()?;
    let c = it.next().unwrap_or("0").parse().ok()?;
    if it.next().is_some() { return None; }
    Some((a, b, c))
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo { pub version: String, pub tag: String, pub download_url: String, pub size: u64, pub notes: String }

/// Only HTTPS URLs on github.com or *.githubusercontent.com are trusted.
pub fn is_trusted_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let host = rest.split('/').next().unwrap_or("");
    if host.contains('@') || host.contains(':') || host.is_empty() { return false; }
    host == "github.com" || host.ends_with(".githubusercontent.com")
}

/// Pure part of the check: decides from the release JSON.
pub fn evaluate(json: &str, current: &str) -> (Option<UpdateInfo>, String) {
    let Ok(v) = serde_json::from_str::<Value>(json) else { return (None, "Cannot read the update information.".into()) };
    let tag = v["tag_name"].as_str().unwrap_or("").to_string();
    let Some(latest) = parse_version(&tag) else { return (None, format!("Unknown release version format: {tag}")) };
    let cur = parse_version(current).unwrap_or((0, 0, 0));
    let asset = v["assets"].as_array().and_then(|a| a.iter().find(|x| x["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(ASSET_NAME))));
    let Some(asset) = asset else { return (None, "The release does not contain an installer file.".into()) };
    let url = asset["browser_download_url"].as_str().unwrap_or("").to_string();
    if !is_trusted_url(&url) { return (None, "The download URL is not from github.com: skipped.".into()); }
    let fmt = |t: (u32, u32, u32)| format!("{}.{}.{}", t.0, t.1, t.2);
    if latest.cmp(&cur) == Ordering::Greater {
        (Some(UpdateInfo { version: fmt(latest), tag, download_url: url, size: asset["size"].as_u64().unwrap_or(0), notes: v["name"].as_str().unwrap_or("").into() }),
         format!("Version {} is available (you have {}).", fmt(latest), fmt(cur)))
    } else { (None, format!("You have the latest version ({}).", fmt(cur))) }
}

/// Picks the newest published (not draft, not pre-release) release that has the installer from a list of releases.
/// GitHub's "latest" endpoint is not used: it can point at an unrelated, older release line.
pub fn evaluate_list(json: &str, current: &str) -> (Option<UpdateInfo>, String) {
    let Ok(Value::Array(list)) = serde_json::from_str::<Value>(json) else { return evaluate(json, current) };
    let has_installer = |r: &Value| r["assets"].as_array().is_some_and(|a| a.iter().any(|x| x["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(ASSET_NAME))));
    let best = list.iter()
        .filter(|r| !r["draft"].as_bool().unwrap_or(false) && !r["prerelease"].as_bool().unwrap_or(false) && has_installer(r))
        .filter_map(|r| parse_version(r["tag_name"].as_str()?).map(|v| (v, r)))
        .max_by_key(|(v, _)| *v);
    match best {
        Some((_, r)) => evaluate(&r.to_string(), current),
        None => (None, "No published release was found.".into()),
    }
}

pub fn check(run: &dyn Runner) -> (Option<UpdateInfo>, String) {
    let script = format!("[Net.ServicePointManager]::SecurityProtocol=[Net.SecurityProtocolType]::Tls12; \
try {{ (Invoke-WebRequest -UseBasicParsing -Uri 'https://api.github.com/repos/{OWNER}/{REPO}/releases?per_page=30' \
-Headers @{{'User-Agent'='Neox-Updater';'Accept'='application/vnd.github+json'}} -TimeoutSec 20).Content }} \
catch {{ if($_.Exception.Response -and [int]$_.Exception.Response.StatusCode -eq 404){{'NOTFOUND'}} else {{ exit 3 }} }}");
    let r = run.run(&powershell(&script), 40);
    if r.output.trim() == "NOTFOUND" { return (None, "Cannot check for updates: the repository is private or has no release yet.".into()); }
    if !r.success() { return (None, "Cannot reach the update server.".into()); }
    evaluate_list(&r.output, current_version())
}

/// Downloads the installer to a temp file; `progress` receives 0.0..=1.0 (polled from the file size). Verifies the size.
pub fn download(run: &dyn Runner, info: &UpdateInfo, progress: &(dyn Fn(f64) + Sync)) -> Result<PathBuf, String> {
    if !is_trusted_url(&info.download_url) { return Err("The download URL is not from github.com.".into()); }
    let path = std::env::temp_dir().join(format!("Neox_Setup_{}.exe", info.version));
    let _ = std::fs::remove_file(&path);
    let p = path.to_string_lossy().replace('\'', "''");
    let script = format!("[Net.ServicePointManager]::SecurityProtocol=[Net.SecurityProtocolType]::Tls12; $ProgressPreference='SilentlyContinue'; \
Invoke-WebRequest -UseBasicParsing -Uri '{}' -OutFile '{p}'", info.download_url.replace('\'', "''"));
    let done = AtomicBool::new(false);
    let result = std::thread::scope(|s| {
        s.spawn(|| {
            while !done.load(AtomicOrdering::SeqCst) {
                if info.size > 0 { if let Ok(m) = std::fs::metadata(&path) { progress((m.len() as f64 / info.size as f64).min(1.0)); } }
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        });
        let r = run.run(&powershell(&script), 900);
        done.store(true, AtomicOrdering::SeqCst);
        r
    });
    if !result.success() { return Err(format!("Download failed: {}", result.first_line())); }
    let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    if info.size > 0 && len != info.size {
        let _ = std::fs::remove_file(&path);
        return Err("The downloaded file size differs from the release; download aborted.".into());
    }
    progress(1.0);
    Ok(path)
}

/// Starts the installer silently (a UAC prompt appears). False if the user declines.
pub fn launch_installer(run: &dyn Runner, path: &std::path::Path) -> bool {
    let p = path.to_string_lossy().replace('\'', "''");
    run.run(&powershell(&format!("Start-Process -FilePath '{p}' -ArgumentList '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /CLOSEAPPLICATIONS' -ErrorAction Stop")), 30).success()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, url: &str) -> String {
        format!(r#"{{"tag_name":"{tag}","name":"NEOX {tag}","assets":[{{"name":"Neox_Setup.exe","size":1234,"browser_download_url":"{url}"}}]}}"#)
    }

    #[test]
    fn versions_parse_and_compare() {
        assert_eq!(parse_version("v3.1.57"), Some((3, 1, 57)));
        assert_eq!(parse_version("3.1"), Some((3, 1, 0)));
        assert_eq!(parse_version("abc"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
    }

    #[test]
    fn offers_only_newer_versions() {
        let url = "https://github.com/KarolDuda/NEOX/releases/download/v3.1.5/Neox_Setup.exe";
        let (u, msg) = evaluate(&release("v3.1.5", url), "3.1.4");
        assert_eq!(u.unwrap().version, "3.1.5");
        assert!(msg.contains("available"));
        assert!(evaluate(&release("v3.1.5", url), "3.1.5").0.is_none());
        assert!(evaluate(&release("v3.1.5", url), "3.2.0").0.is_none());
        assert!(evaluate(&release("v3.1.5", url), "3.0.99").0.is_some());
    }

    #[test]
    fn untrusted_download_hosts_are_rejected() {
        for bad in ["http://github.com/x", "https://evil.com/github.com/x", "https://github.com@evil.com/x", "https://github.com:444/x", "https://notgithub.com/x", "ftp://github.com/x"] {
            assert!(!is_trusted_url(bad), "{bad}");
            assert!(evaluate(&release("v9.0.0", bad), "1.0.0").0.is_none(), "{bad}");
        }
        assert!(is_trusted_url("https://objects.githubusercontent.com/a/b"));
        assert!(is_trusted_url("https://github.com/a/b"));
    }

    #[test]
    fn missing_asset_and_bad_json_are_reported() {
        assert!(evaluate(r#"{"tag_name":"v9.0.0","assets":[]}"#, "1.0.0").1.contains("installer"));
        assert!(evaluate("{nope", "1.0.0").0.is_none());
        assert!(evaluate(r#"{"tag_name":"nightly","assets":[]}"#, "1.0.0").1.contains("Unknown"));
    }

    #[test]
    fn picks_the_newest_published_release_and_skips_prereleases_and_drafts() {
        let rel = |t: &str, pre: bool, draft: bool| format!(r#"{{"tag_name":"{t}","name":"NEOX {t}","prerelease":{pre},"draft":{draft},"assets":[{{"name":"Neox_Setup.exe","size":1,"browser_download_url":"https://github.com/KarolDuda/NEOX/releases/download/{t}/Neox_Setup.exe"}}]}}"#);
        let list = format!("[{},{},{},{},{}]", rel("v3.0.18", false, false), rel("v3.1.16", false, false), rel("v3.1.99", true, false), rel("v3.2.0", false, true), rel("recode-15", true, false));
        let (u, _) = evaluate_list(&list, "3.1.15");
        assert_eq!(u.unwrap().version, "3.1.16");
        assert!(evaluate_list(&list, "3.1.16").0.is_none());
        assert!(evaluate_list("[]", "1.0.0").0.is_none());
        // a single release object still works
        assert!(evaluate_list(&rel("v3.1.16", false, false), "3.1.15").0.is_some());
    }

    #[test]
    fn current_version_is_a_valid_version() { assert!(parse_version(current_version()).is_some()); }
}
