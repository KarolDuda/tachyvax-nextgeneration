//! Startup entries (via StartupApproved, like Task Manager: nothing is deleted), processes, services and storage analysis.

use crate::registry::{RegData, RegistryApi};
use crate::runner::{argv, powershell, Runner};
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::Path;

// ---------------------------------------------------------------- startup

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StartupEntry { pub name: String, pub command: String, pub location: String, pub enabled: bool, pub hive: String, pub key_path: String }

const RUN: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const APPROVED: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run";

pub trait RegistryEnum: RegistryApi {
    fn value_names(&self, hive: &str, path: &str) -> Vec<String>;
}

pub fn list_startup(reg: &dyn RegistryEnum) -> Vec<StartupEntry> {
    let mut list = Vec::new();
    for (hive, label) in [("HKCU", "Current user"), ("HKLM", "All users")] {
        for name in reg.value_names(hive, RUN) {
            let enabled = match reg.read(hive, APPROVED, Some(&name)) {
                Ok(Some(RegData::Binary(b))) if !b.is_empty() => (b[0] & 1) == 0 || b[0] == 2,
                _ => true,
            };
            let command = match reg.read(hive, RUN, Some(&name)) {
                Ok(Some(RegData::Str(s))) | Ok(Some(RegData::ExpandStr(s))) => s,
                _ => String::new(),
            };
            list.push(StartupEntry { name, command, location: label.into(), enabled, hive: hive.into(), key_path: RUN.into() });
        }
    }
    list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    list
}

pub fn set_startup_enabled(reg: &dyn RegistryApi, hive: &str, name: &str, enabled: bool) -> Result<(), String> {
    if hive != "HKCU" && hive != "HKLM" { return Err("Unsupported hive.".into()); }
    let mut data = vec![0u8; 12];
    data[0] = if enabled { 2 } else { 3 }; // 02 = enabled, 03 = disabled
    reg.write(hive, APPROVED, Some(name), &RegData::Binary(data))
}

#[cfg(windows)]
impl RegistryEnum for crate::registry::WinRegistry {
    fn value_names(&self, hive: &str, path: &str) -> Vec<String> {
        use winreg::enums::*;
        use winreg::RegKey;
        let root = if hive == "HKCU" { RegKey::predef(HKEY_CURRENT_USER) } else { RegKey::predef(HKEY_LOCAL_MACHINE) };
        root.open_subkey_with_flags(path, KEY_READ)
            .map(|k| k.enum_values().filter_map(|r| r.ok().map(|(n, _)| n)).collect())
            .unwrap_or_default()
    }
}

// ---------------------------------------------------------------- processes

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessEntry { pub pid: u32, pub name: String, pub working_set_bytes: u64 }

/// Parses `tasklist /FO CSV /NH`; memory digits only, so any localized thousands separator works.
pub fn parse_tasklist(output: &str) -> Vec<ProcessEntry> {
    output.lines().filter_map(|l| {
        let f: Vec<&str> = l.trim().trim_matches('"').split("\",\"").collect();
        if f.len() < 5 { return None; }
        let pid = f[1].trim().parse().ok()?;
        let kb: u64 = f[4].chars().filter(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0);
        let name = f[0]; let name = name.strip_suffix(".exe").or_else(|| name.strip_suffix(".EXE")).unwrap_or(name);
        Some(ProcessEntry { pid, name: name.into(), working_set_bytes: kb * 1024 })
    }).collect()
}

pub fn processes(run: &dyn Runner) -> Vec<ProcessEntry> {
    let r = run.run(&argv(&["tasklist", "/FO", "CSV", "/NH"]), 30);
    if r.success() { parse_tasklist(&r.output) } else { vec![] }
}

pub fn top_processes(run: &dyn Runner, count: usize) -> Vec<ProcessEntry> {
    let mut p = processes(run);
    p.sort_by(|a, b| b.working_set_bytes.cmp(&a.working_set_bytes));
    p.truncate(count);
    p
}

// ---------------------------------------------------------------- services

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceEntry { pub name: String, pub display_name: String, pub status: String, pub start_type: String, pub is_critical: bool }

/// Critical services are marked and never offered for disabling.
pub const CRITICAL: &[&str] = &["wuauserv", "WinDefend", "Sense", "mpssvc", "SecurityHealthService", "wscsvc", "EventLog", "RpcSs", "RpcEptMapper",
    "DcomLaunch", "LSM", "SamSs", "BFE", "Dhcp", "Dnscache", "nsi", "CryptSvc", "TrustedInstaller", "gpsvc", "ProfSvc"];

pub fn parse_services(output: &str) -> Vec<ServiceEntry> {
    let (Some(a), Some(b)) = (output.find(|c| c == '[' || c == '{'), output.rfind(|c| c == ']' || c == '}')) else { return vec![] };
    let Ok(v) = serde_json::from_str::<Value>(&output[a..=b]) else { return vec![] };
    let items = match v { Value::Array(a) => a, o @ Value::Object(_) => vec![o], _ => vec![] };
    let mut list: Vec<ServiceEntry> = items.iter().filter_map(|i| {
        let name = i["Name"].as_str()?.to_string();
        Some(ServiceEntry {
            is_critical: CRITICAL.iter().any(|c| c.eq_ignore_ascii_case(&name)),
            display_name: i["DisplayName"].as_str().unwrap_or("").into(),
            status: i["State"].as_str().unwrap_or("").into(),
            start_type: i["StartMode"].as_str().unwrap_or("").into(),
            name,
        })
    }).collect();
    list.sort_by(|a, b| a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase()));
    list
}

pub fn services(run: &dyn Runner) -> Vec<ServiceEntry> {
    let r = run.run(&powershell("Get-CimInstance Win32_Service | Select-Object Name,DisplayName,State,StartMode | ConvertTo-Json -Compress"), 60);
    if r.success() { parse_services(&r.output) } else { vec![] }
}

// ---------------------------------------------------------------- storage

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FolderUsage { pub path: String, pub bytes: u64 }

fn dir_size(p: &Path, cancelled: &dyn Fn() -> bool) -> u64 {
    let Ok(m) = fs::symlink_metadata(p) else { return 0 };
    if m.file_type().is_symlink() { return 0; }
    #[cfg(windows)]
    { use std::os::windows::fs::MetadataExt; if m.file_attributes() & 0x400 != 0 { return 0; } }
    if m.is_file() { return m.len(); }
    if cancelled() { return 0; }
    fs::read_dir(p).map(|rd| rd.flatten().map(|e| dir_size(&e.path(), cancelled)).sum()).unwrap_or(0)
}

/// Sizes of first-level folders of `root` (links skipped), biggest first.
pub fn analyze_storage(root: &Path, top: usize, progress: &dyn Fn(&str), cancelled: &dyn Fn() -> bool) -> Vec<FolderUsage> {
    let mut res = Vec::new();
    let Ok(rd) = fs::read_dir(root) else { return res };
    for e in rd.flatten() {
        if cancelled() { break; }
        let p = e.path();
        if !p.is_dir() { continue; }
        progress(&p.to_string_lossy());
        res.push(FolderUsage { path: p.to_string_lossy().into(), bytes: dir_size(&p, cancelled) });
    }
    res.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    res.truncate(top);
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::MemRegistry;

    struct EnumReg { inner: MemRegistry, names: Vec<(String, String)> }
    impl RegistryApi for EnumReg {
        fn read(&self, h: &str, p: &str, n: Option<&str>) -> Result<Option<RegData>, String> { self.inner.read(h, p, n) }
        fn write(&self, h: &str, p: &str, n: Option<&str>, d: &RegData) -> Result<(), String> { self.inner.write(h, p, n, d) }
        fn delete(&self, h: &str, p: &str, n: Option<&str>) -> Result<(), String> { self.inner.delete(h, p, n) }
    }
    impl RegistryEnum for EnumReg {
        fn value_names(&self, hive: &str, path: &str) -> Vec<String> {
            if path != RUN { return vec![]; }
            self.names.iter().filter(|(h, _)| h == hive).map(|(_, n)| n.clone()).collect()
        }
    }

    #[test]
    fn startup_enable_disable_uses_startup_approved_and_never_deletes() {
        let reg = EnumReg { inner: MemRegistry::new(), names: vec![("HKCU".into(), "Discord".into()), ("HKLM".into(), "Zed".into())] };
        reg.write("HKCU", RUN, Some("Discord"), &RegData::Str("C:\\d.exe".into())).unwrap();
        assert!(list_startup(&reg).iter().all(|e| e.enabled));
        set_startup_enabled(&reg, "HKCU", "Discord", false).unwrap();
        let l = list_startup(&reg);
        assert_eq!(l[0].name, "Discord");
        assert!(!l[0].enabled);
        assert_eq!(l[0].command, "C:\\d.exe");
        assert!(reg.read("HKCU", RUN, Some("Discord")).unwrap().is_some());
        set_startup_enabled(&reg, "HKCU", "Discord", true).unwrap();
        assert!(list_startup(&reg)[0].enabled);
        assert!(set_startup_enabled(&reg, "HKCR", "x", true).is_err());
    }

    #[test]
    fn parses_tasklist_with_localized_memory_separators() {
        let out = "\"chrome.exe\",\"1234\",\"Console\",\"1\",\"123 456 K\"\n\"System\",\"4\",\"Services\",\"0\",\"1,024 K\"\njunk";
        let p = parse_tasklist(out);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0], ProcessEntry { pid: 1234, name: "chrome".into(), working_set_bytes: 123_456 * 1024 });
        assert_eq!(p[1].working_set_bytes, 1024 * 1024);
    }

    #[test]
    fn services_mark_critical_ones() {
        let out = r#"[{"Name":"wuauserv","DisplayName":"Windows Update","State":"Running","StartMode":"Manual"},{"Name":"SysMain","DisplayName":"Superfetch","State":"Running","StartMode":"Auto"}]"#;
        let s = parse_services(out);
        assert_eq!(s.len(), 2);
        assert!(s.iter().find(|x| x.name == "wuauserv").unwrap().is_critical);
        assert!(!s.iter().find(|x| x.name == "SysMain").unwrap().is_critical);
    }

    #[test]
    fn storage_analysis_sorts_biggest_first() {
        let root = std::env::temp_dir().join(format!("neox-stor-{}", neox_core::timefmt::now_unix()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("small")).unwrap();
        fs::create_dir_all(root.join("big/inner")).unwrap();
        fs::write(root.join("small/a"), vec![0; 10]).unwrap();
        fs::write(root.join("big/inner/b"), vec![0; 5000]).unwrap();
        let r = analyze_storage(&root, 10, &|_| {}, &|| false);
        assert_eq!(r.len(), 2);
        assert!(r[0].path.ends_with("big") && r[0].bytes == 5000);
    }
}
