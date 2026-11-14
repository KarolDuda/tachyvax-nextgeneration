//! Tiny hand-written parsers (no regex dependency) for powercfg / bcdedit / schtasks output.

/// First `8-4-4-4-12` hex GUID in the text, lowercased.
pub fn find_guid(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let pat = [8usize, 4, 4, 4, 12];
    let mut i = 0;
    while i + 36 <= b.len() {
        let mut p = i;
        let mut ok = true;
        for (gi, len) in pat.iter().enumerate() {
            if !b[p..p + len].iter().all(|c| c.is_ascii_hexdigit()) { ok = false; break; }
            p += len;
            if gi < 4 { if b.get(p) != Some(&b'-') { ok = false; break; } p += 1; }
        }
        if ok { return Some(s[i..i + 36].to_lowercase()); }
        i += 1;
    }
    None
}

/// All `0xXXXXXXXX` values (8 hex digits) in order.
pub fn hex_values(s: &str) -> Vec<u32> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(pos) = rest.find("0x") {
        let after = &rest[pos + 2..];
        if after.len() >= 8 && after[..8].chars().all(|c| c.is_ascii_hexdigit()) {
            if let Ok(v) = u32::from_str_radix(&after[..8], 16) { out.push(v); }
            rest = &after[8..];
        } else { rest = after; }
    }
    out
}

/// Value of `key` in `bcdedit /enum` output (`key<whitespace>value`).
pub fn bcd_value(output: &str, key: &str) -> Option<String> {
    for line in output.lines() {
        let l = line.trim_start();
        if l.len() > key.len() && l[..key.len()].eq_ignore_ascii_case(key) {
            let rest = &l[key.len()..];
            if rest.starts_with(char::is_whitespace) {
                if let Some(v) = rest.split_whitespace().next() { return Some(v.to_string()); }
            }
        }
    }
    None
}

/// `<Settings><Enabled>` from `schtasks /query /xml`; missing element means enabled.
pub fn task_enabled(xml: &str) -> bool {
    let low = xml.to_lowercase();
    let Some(s) = low.find("<settings>") else { return true };
    let Some(e) = low[s..].find("</settings>") else { return true };
    let block = &low[s..s + e];
    let Some(a) = block.find("<enabled>") else { return true };
    let Some(b) = block[a..].find("</enabled>") else { return true };
    block[a + 9..a + b].trim() == "true"
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finds_guid_in_localized_output() {
        let s = "GUID schematu zasilania: 8C5E7FDA-E8BF-4A96-9A85-A6E23A8C635C  (Wysoka wydajność) *";
        assert_eq!(find_guid(s).unwrap(), "8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c");
        assert!(find_guid("no guid here").is_none());
    }
    #[test]
    fn reads_ac_and_dc_hex_values() {
        let s = "Current AC Power Setting Index: 0x00000064\n Current DC Power Setting Index: 0x00000032";
        assert_eq!(hex_values(s), vec![100, 50]);
    }
    #[test]
    fn reads_bcd_values_case_insensitively() {
        let s = "Windows Boot Loader\n-------\nidentifier   {current}\ndisabledynamictick      Yes\nuseplatformtick   Yes";
        assert_eq!(bcd_value(s, "DisableDynamicTick").as_deref(), Some("Yes"));
        assert!(bcd_value(s, "missing").is_none());
    }
    #[test]
    fn reads_task_enabled_flag() {
        assert!(task_enabled("<Task><Settings><Enabled>true</Enabled></Settings></Task>"));
        assert!(!task_enabled("<Task><Settings><Enabled>false</Enabled></Settings></Task>"));
        assert!(task_enabled("<Task><Settings></Settings></Task>"));
        assert!(task_enabled("<Task></Task>"));
    }
}
