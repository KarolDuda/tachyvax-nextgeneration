//! Registry access with faithful snapshots: remembers whether a value existed, its type and its data
//! (DWORD, QWORD, String, ExpandString, MultiString, Binary), so undo restores exactly what was there.
//! Snapshot JSON is compatible with earlier NEOX versions.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq)]
pub enum RegData { Dword(u32), Qword(u64), Str(String), ExpandStr(String), Multi(Vec<String>), Binary(Vec<u8>) }

pub trait RegistryApi: Send + Sync {
    fn read(&self, hive: &str, path: &str, name: Option<&str>) -> Result<Option<RegData>, String>;
    fn write(&self, hive: &str, path: &str, name: Option<&str>, data: &RegData) -> Result<(), String>;
    fn delete(&self, hive: &str, path: &str, name: Option<&str>) -> Result<(), String>;
}

// ---- snapshot / restore ----

fn kind_name(d: &RegData) -> &'static str {
    match d { RegData::Dword(_) => "DWord", RegData::Qword(_) => "QWord", RegData::Str(_) => "String",
              RegData::ExpandStr(_) => "ExpandString", RegData::Multi(_) => "MultiString", RegData::Binary(_) => "Binary" }
}

pub fn snapshot(reg: &dyn RegistryApi, hive: &str, path: &str, name: Option<&str>) -> Result<Value, String> {
    let mut o = json!({ "hive": hive, "path": path, "name": name });
    match reg.read(hive, path, name)? {
        None => { o["exists"] = json!(false); }
        Some(d) => {
            o["exists"] = json!(true);
            o["kind"] = json!(kind_name(&d));
            o["data"] = match &d {
                RegData::Dword(v) => json!(*v as i64),
                RegData::Qword(v) => json!(*v as i64),
                RegData::Str(s) | RegData::ExpandStr(s) => json!(s),
                RegData::Multi(v) => json!(v),
                RegData::Binary(b) => json!(b64_encode(b)),
            };
        }
    }
    Ok(o)
}

pub fn restore(reg: &dyn RegistryApi, snap: &Value) -> Result<(), String> {
    let hive = snap["hive"].as_str().ok_or("Corrupt snapshot: hive")?;
    let path = snap["path"].as_str().ok_or("Corrupt snapshot: path")?;
    let name = snap["name"].as_str();
    if !snap["exists"].as_bool().unwrap_or(false) { return reg.delete(hive, path, name); }
    let data = &snap["data"];
    let d = match snap["kind"].as_str().unwrap_or("String") {
        "DWord" => RegData::Dword(data.as_i64().ok_or("Corrupt snapshot: dword")? as u32),
        "QWord" => RegData::Qword(data.as_i64().ok_or("Corrupt snapshot: qword")? as u64),
        "Binary" => RegData::Binary(b64_decode(data.as_str().ok_or("Corrupt snapshot: binary")?)?),
        "MultiString" => RegData::Multi(data.as_array().ok_or("Corrupt snapshot: multi")?.iter().filter_map(|v| v.as_str().map(String::from)).collect()),
        "ExpandString" => RegData::ExpandStr(data.as_str().unwrap_or("").into()),
        _ => RegData::Str(data.as_str().unwrap_or("").into()),
    };
    reg.write(hive, path, name, &d)
}

// ---- base64 (std only) ----

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

pub fn b64_decode(s: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0;
    for c in s.bytes().filter(|c| *c != b'=' && !c.is_ascii_whitespace()) {
        let v = B64.iter().position(|x| *x == c).ok_or("Invalid base64 data")? as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 { bits -= 8; out.push((buf >> bits) as u8); buf &= (1 << bits) - 1; }
    }
    Ok(out)
}

// ---- in-memory registry (tests and non-Windows builds) ----

#[derive(Default)]
pub struct MemRegistry { map: Mutex<HashMap<String, RegData>>, pub deny_write: Mutex<bool> }

impl MemRegistry {
    pub fn new() -> Self { Self::default() }
    fn key(hive: &str, path: &str, name: Option<&str>) -> String {
        format!("{}\\{}\\{}", hive.to_uppercase(), path.to_lowercase(), name.unwrap_or("").to_lowercase())
    }
}

impl RegistryApi for MemRegistry {
    fn read(&self, h: &str, p: &str, n: Option<&str>) -> Result<Option<RegData>, String> {
        Ok(self.map.lock().unwrap().get(&Self::key(h, p, n)).cloned())
    }
    fn write(&self, h: &str, p: &str, n: Option<&str>, d: &RegData) -> Result<(), String> {
        if *self.deny_write.lock().unwrap() { return Err("Access denied".into()); }
        self.map.lock().unwrap().insert(Self::key(h, p, n), d.clone());
        Ok(())
    }
    fn delete(&self, h: &str, p: &str, n: Option<&str>) -> Result<(), String> {
        self.map.lock().unwrap().remove(&Self::key(h, p, n));
        Ok(())
    }
}

// ---- real Windows registry ----

#[cfg(windows)]
pub use win::WinRegistry;

#[cfg(windows)]
mod win {
    use super::*;
    use winreg::enums::*;
    use winreg::{RegKey, RegValue};

    pub struct WinRegistry;

    fn root(hive: &str) -> Result<RegKey, String> {
        match hive.to_uppercase().as_str() {
            "HKCU" => Ok(RegKey::predef(HKEY_CURRENT_USER)),
            "HKLM" => Ok(RegKey::predef(HKEY_LOCAL_MACHINE)),
            other => Err(format!("Unsupported hive: {other}")),
        }
    }

    fn utf16(bytes: &[u8]) -> Vec<u16> { bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect() }
    fn to_utf16_bytes(s: &str) -> Vec<u8> { s.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_le_bytes).collect() }

    fn decode(v: &RegValue) -> RegData {
        let b: &[u8] = &v.bytes;
        match v.vtype {
            REG_DWORD if b.len() >= 4 => RegData::Dword(u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
            REG_QWORD if b.len() >= 8 => RegData::Qword(u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])),
            REG_SZ => RegData::Str(String::from_utf16_lossy(&utf16(b)).trim_end_matches('\0').to_string()),
            REG_EXPAND_SZ => RegData::ExpandStr(String::from_utf16_lossy(&utf16(b)).trim_end_matches('\0').to_string()),
            REG_MULTI_SZ => RegData::Multi(String::from_utf16_lossy(&utf16(b)).split('\0').filter(|s| !s.is_empty()).map(String::from).collect()),
            _ => RegData::Binary(b.to_vec()),
        }
    }

    fn encode(d: &RegData) -> RegValue {
        let (bytes, vtype) = match d {
            RegData::Dword(v) => (v.to_le_bytes().to_vec(), REG_DWORD),
            RegData::Qword(v) => (v.to_le_bytes().to_vec(), REG_QWORD),
            RegData::Str(s) => (to_utf16_bytes(s), REG_SZ),
            RegData::ExpandStr(s) => (to_utf16_bytes(s), REG_EXPAND_SZ),
            RegData::Multi(list) => {
                let mut v: Vec<u8> = list.iter().flat_map(|s| s.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_le_bytes)).collect();
                v.extend_from_slice(&[0, 0]);
                (v, REG_MULTI_SZ)
            }
            RegData::Binary(b) => (b.clone(), REG_BINARY),
        };
        RegValue { bytes, vtype }
    }

    impl RegistryApi for WinRegistry {
        fn read(&self, hive: &str, path: &str, name: Option<&str>) -> Result<Option<RegData>, String> {
            let key = match root(hive)?.open_subkey_with_flags(path, KEY_READ) { Ok(k) => k, Err(_) => return Ok(None) };
            Ok(key.get_raw_value(name.unwrap_or("")).ok().map(|v| decode(&v)))
        }
        fn write(&self, hive: &str, path: &str, name: Option<&str>, data: &RegData) -> Result<(), String> {
            let (key, _) = root(hive)?.create_subkey_with_flags(path, KEY_WRITE).map_err(|e| format!("Cannot open {hive}\\{path}: {e}"))?;
            key.set_raw_value(name.unwrap_or(""), &encode(data)).map_err(|e| e.to_string())
        }
        fn delete(&self, hive: &str, path: &str, name: Option<&str>) -> Result<(), String> {
            let Ok(key) = root(hive)?.open_subkey_with_flags(path, KEY_WRITE) else { return Ok(()) };
            match key.delete_value(name.unwrap_or("")) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e.to_string()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips() {
        for data in [&b""[..], b"a", b"ab", b"abc", b"abcd", &[0, 255, 7, 8, 9, 10]] {
            assert_eq!(b64_decode(&b64_encode(data)).unwrap(), data);
        }
        assert_eq!(b64_encode(b"Man"), "TWFu");
    }

    #[test]
    fn snapshot_restores_every_value_kind_exactly() {
        let reg = MemRegistry::new();
        let all = [RegData::Dword(0xFFFF_FFFF), RegData::Qword(1 << 40), RegData::Str("a".into()), RegData::ExpandStr("%x%".into()),
                   RegData::Multi(vec!["a".into(), "b".into()]), RegData::Binary(vec![1, 2, 3])];
        for (i, d) in all.iter().enumerate() {
            let name = format!("v{i}");
            reg.write("HKCU", "Software\\T", Some(&name), d).unwrap();
            let snap = snapshot(&reg, "HKCU", "Software\\T", Some(&name)).unwrap();
            reg.write("HKCU", "Software\\T", Some(&name), &RegData::Dword(7)).unwrap();
            restore(&reg, &snap).unwrap();
            assert_eq!(reg.read("HKCU", "Software\\T", Some(&name)).unwrap().unwrap(), *d, "{name}");
        }
    }

    #[test]
    fn restoring_a_missing_value_deletes_it() {
        let reg = MemRegistry::new();
        let snap = snapshot(&reg, "HKLM", "Software\\X", Some("n")).unwrap();
        reg.write("HKLM", "Software\\X", Some("n"), &RegData::Dword(1)).unwrap();
        restore(&reg, &snap).unwrap();
        assert!(reg.read("HKLM", "Software\\X", Some("n")).unwrap().is_none());
    }

    #[test]
    fn reads_snapshots_written_by_the_dotnet_version() {
        let reg = MemRegistry::new();
        let snap = json!({"hive":"HKCU","path":"Software\\T","name":"n","exists":true,"kind":"DWord","data":4294967295i64});
        restore(&reg, &snap).unwrap();
        assert_eq!(reg.read("HKCU", "Software\\T", Some("n")).unwrap(), Some(RegData::Dword(u32::MAX)));
    }
}
