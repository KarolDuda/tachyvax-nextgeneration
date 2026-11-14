//! Deletes the CONTENTS of the given folders (or files matching a pattern). Safeguards: paths shallower than
//! 3 segments are refused, links/junctions are never followed, and the app's own directory is never touched.

use std::fs;
use std::path::{Component, Path, PathBuf};

pub struct Cleaner { app_dir: PathBuf }

fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(a) = rest.find('%') {
        out.push_str(&rest[..a]);
        let after = &rest[a + 1..];
        match after.find('%') {
            Some(b) => {
                let name = &after[..b];
                match std::env::var(name) {
                    Ok(v) if !name.is_empty() => out.push_str(&v),
                    _ => { out.push('%'); out.push_str(name); out.push('%'); } // unresolved: left in place, caller skips it
                }
                rest = &after[b + 1..];
            }
            None => { out.push('%'); rest = after; }
        }
    }
    out.push_str(rest);
    out
}

/// `*` and `?` wildcard match, case-insensitive.
pub fn wildcard(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let (mut pi, mut ti, mut star, mut mark) = (0usize, 0usize, None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) { pi += 1; ti += 1; }
        else if pi < p.len() && p[pi] == '*' { star = Some(pi); mark = ti; pi += 1; }
        else if let Some(s) = star { pi = s + 1; mark += 1; ti = mark; }
        else { return false; }
    }
    while pi < p.len() && p[pi] == '*' { pi += 1; }
    pi == p.len()
}

fn is_reparse(p: &Path) -> bool {
    let Ok(m) = fs::symlink_metadata(p) else { return true };
    if m.file_type().is_symlink() { return true; }
    #[cfg(windows)]
    { use std::os::windows::fs::MetadataExt; if m.file_attributes() & 0x400 != 0 { return true; } }
    false
}

/// Expands `*` / `?` in any path segment.
pub fn expand(pattern: &str) -> Vec<PathBuf> {
    let norm = pattern.replace('\\', "/");
    let (root, rest): (String, &str) = if norm.starts_with('/') { ("/".into(), &norm[1..]) }
        else if norm.len() >= 2 && norm.as_bytes()[1] == b':' { (format!("{}/", &norm[..2]), norm[2..].trim_start_matches('/')) }
        else { return vec![] };
    let mut current = vec![PathBuf::from(root)];
    for seg in rest.split('/').filter(|s| !s.is_empty()) {
        if !seg.contains('*') && !seg.contains('?') { current = current.into_iter().map(|c| c.join(seg)).collect(); continue; }
        let mut next = Vec::new();
        for c in &current {
            if let Ok(rd) = fs::read_dir(c) {
                for e in rd.flatten() { if wildcard(seg, &e.file_name().to_string_lossy()) { next.push(e.path()); } }
            }
        }
        current = next;
    }
    current.into_iter().filter(|p| p.exists()).collect()
}

impl Cleaner {
    pub fn new(app_dir: PathBuf) -> Self { Self { app_dir } }

    fn is_safe(&self, p: &Path) -> bool {
        let depth = p.components().filter(|c| matches!(c, Component::Normal(_) | Component::Prefix(_))).count();
        if depth < 3 { return false; }
        if !self.app_dir.as_os_str().is_empty() && p.starts_with(&self.app_dir) { return false; }
        !p.components().any(|c| c.as_os_str().to_string_lossy().starts_with("_MEI"))
    }

    pub fn clean(&self, patterns: &[String]) -> String {
        let (mut bytes, mut files) = (0u64, 0u32);
        for raw in patterns {
            let expanded = expand_env(raw);
            if expanded.contains('%') || expanded.contains('$') { continue; } // unresolved variable: never guess
            for m in expand(&expanded) {
                if !self.is_safe(&m) { continue; }
                if m.is_dir() && !is_reparse(&m) {
                    if let Ok(rd) = fs::read_dir(&m) { for e in rd.flatten() { delete(&e.path(), &mut bytes, &mut files); } }
                } else if m.is_file() { delete(&m, &mut bytes, &mut files); }
            }
        }
        format!("Removed {files} files ({:.0} MB).", bytes as f64 / 1_048_576.0)
    }

    pub fn measure(&self, patterns: &[String]) -> u64 {
        let mut total = 0;
        for raw in patterns {
            let expanded = expand_env(raw);
            if expanded.contains('%') { continue; }
            for m in expand(&expanded) {
                if !self.is_safe(&m) { continue; }
                total += size_of(&m);
            }
        }
        total
    }
}

fn size_of(p: &Path) -> u64 {
    if is_reparse(p) { return 0; }
    if p.is_file() { return fs::metadata(p).map(|m| m.len()).unwrap_or(0); }
    fs::read_dir(p).map(|rd| rd.flatten().map(|e| size_of(&e.path())).sum()).unwrap_or(0)
}

fn delete(p: &Path, bytes: &mut u64, files: &mut u32) {
    if is_reparse(p) {
        // remove only the link itself, never what it points to
        if fs::remove_file(p).is_err() { let _ = fs::remove_dir(p); }
        return;
    }
    if p.is_dir() {
        if let Ok(rd) = fs::read_dir(p) { for e in rd.flatten() { delete(&e.path(), bytes, files); } }
        let _ = fs::remove_dir(p); // in use or not empty: skip
    } else {
        let len = fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        if fs::remove_file(p).is_ok() { *bytes += len; *files += 1; } // file in use: skip
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("neox-clean-{name}-{}", neox_core::timefmt::now_unix())).join("work").join("dir");
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn wildcards_match_like_windows_patterns() {
        assert!(wildcard("*.log", "a.LOG"));
        assert!(wildcard("a?c", "abc"));
        assert!(!wildcard("a?c", "ac"));
        assert!(wildcard("*", ""));
        assert!(!wildcard("*.log", "a.txt"));
    }

    #[test]
    fn cleans_contents_but_keeps_the_folder() {
        let d = tmp("contents");
        fs::write(d.join("a.tmp"), vec![0u8; 2048]).unwrap();
        fs::create_dir_all(d.join("sub/deep")).unwrap();
        fs::write(d.join("sub/deep/b.tmp"), b"x").unwrap();
        let msg = Cleaner::new(PathBuf::new()).clean(&[d.to_string_lossy().to_string()]);
        assert!(msg.starts_with("Removed 2 files"), "{msg}");
        assert!(d.exists());
        assert_eq!(fs::read_dir(&d).unwrap().count(), 0);
    }

    #[test]
    fn wildcard_patterns_only_touch_matching_files() {
        let d = tmp("glob");
        fs::write(d.join("x.log"), b"1").unwrap();
        fs::write(d.join("keep.txt"), b"1").unwrap();
        Cleaner::new(PathBuf::new()).clean(&[format!("{}/*.log", d.to_string_lossy())]);
        assert!(!d.join("x.log").exists());
        assert!(d.join("keep.txt").exists());
    }

    #[test]
    fn refuses_shallow_paths_and_the_apps_own_directory() {
        let c = Cleaner::new(PathBuf::from("/opt/neox/app"));
        assert!(!c.is_safe(Path::new("/")));
        assert!(!c.is_safe(Path::new("/tmp")));
        assert!(!c.is_safe(Path::new("/opt/neox/app/data")));
        assert!(c.is_safe(Path::new("/var/tmp/x")));
    }

    #[test]
    fn unresolved_variables_are_never_guessed() {
        let c = Cleaner::new(PathBuf::new());
        assert_eq!(c.clean(&["%NEOX_DOES_NOT_EXIST%/a/b".to_string()]), "Removed 0 files (0 MB).");
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_removed_but_never_followed() {
        let d = tmp("link");
        let outside = tmp("outside");
        fs::write(outside.join("precious.txt"), b"keep me").unwrap();
        std::os::unix::fs::symlink(&outside, d.join("link")).unwrap();
        Cleaner::new(PathBuf::new()).clean(&[d.to_string_lossy().to_string()]);
        assert!(outside.join("precious.txt").exists());
        assert!(!d.join("link").exists());
    }

    #[test]
    fn measure_matches_what_clean_removes() {
        let d = tmp("measure");
        fs::write(d.join("a"), vec![0u8; 1000]).unwrap();
        fs::write(d.join("b"), vec![0u8; 500]).unwrap();
        assert_eq!(Cleaner::new(PathBuf::new()).measure(&[d.to_string_lossy().to_string()]), 1500);
    }
}
