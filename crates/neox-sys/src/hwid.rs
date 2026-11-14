//! Hardware id of this PC, derived from motherboard identifiers (not from the network or the IP address).

use crate::runner::{powershell, Runner};
use neox_core::license::hardware_id;
use serde_json::Value;

const SCRIPT: &str = "$u=(Get-CimInstance Win32_ComputerSystemProduct).UUID; $b=Get-CimInstance Win32_BaseBoard; \
$g=''; try{$g=(Get-ItemProperty 'HKLM:\\SOFTWARE\\Microsoft\\Cryptography').MachineGuid}catch{}; \
[pscustomobject]@{U=[string]$u;S=[string]$b.SerialNumber;M=[string]$b.Manufacturer;P=[string]$b.Product;G=[string]$g} | ConvertTo-Json -Compress";

/// Placeholder values that many boards report instead of a real serial. They must not feed the id.
pub fn is_junk(v: &str) -> bool {
    let t = v.trim().to_ascii_lowercase();
    if t.len() < 4 { return true; }
    if t.chars().all(|c| c == '0' || c == 'f' || c == '-') { return true; }
    ["to be filled", "default string", "o.e.m", "not applicable", "not specified", "none", "unknown", "system serial", "base board serial", "123456789", "0123456789"]
        .iter().any(|j| t.contains(j))
}

/// Combines the parts that are usable. Falls back to the Windows machine guid only when the board reports nothing usable.
pub fn from_parts(uuid: &str, serial: &str, maker: &str, product: &str, machine_guid: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for v in [uuid, serial] { if !is_junk(v) { parts.push(v); } }
    if parts.is_empty() { if !is_junk(machine_guid) { parts.push(machine_guid); } }
    else { for v in [maker, product] { if !is_junk(v) { parts.push(v); } } }
    hardware_id(&parts)
}

pub fn read(run: &dyn Runner) -> String {
    let r = run.run(&powershell(SCRIPT), 40);
    let v: Value = r.output.find('{').and_then(|a| r.output.rfind('}').map(|b| &r.output[a..=b]))
        .and_then(|j| serde_json::from_str(j).ok()).unwrap_or(Value::Null);
    let g = |k: &str| v[k].as_str().unwrap_or("").to_string();
    from_parts(&g("U"), &g("S"), &g("M"), &g("P"), &g("G"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn placeholder_serials_are_ignored() {
        for j in ["", "To be filled by O.E.M.", "Default string", "None", "FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF", "00000000-0000-0000-0000-000000000000", "123456789"] { assert!(is_junk(j), "{j}"); }
        assert!(!is_junk("M80-C4012300123"));
    }
    #[test]
    fn id_ignores_junk_and_is_stable() {
        let a = from_parts("4C4C4544-0042-3510-8053-B7C04F4D4B32", "To be filled by O.E.M.", "ASUS", "ROG STRIX", "g");
        let b = from_parts("4C4C4544-0042-3510-8053-B7C04F4D4B32", "Default string", "ASUS", "ROG STRIX", "other-guid");
        assert_eq!(a, b);
    }
    #[test]
    fn falls_back_to_machine_guid_when_board_reports_nothing() {
        let a = from_parts("", "None", "", "", "11111111-2222-3333-4444-555555555555");
        assert_ne!(a, from_parts("", "None", "", "", "99999999-2222-3333-4444-555555555555"));
    }
}
