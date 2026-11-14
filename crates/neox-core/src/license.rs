//! Premium licensing: offline, signed, bound to one PC.
//!
//! A license is `NEOX1.<payload>.<signature>` (base64url, no padding). The payload is JSON, the signature is
//! Ed25519 over the exact payload bytes. The private key never ships with the app; only the public key does.
//! The token names the hardware id (a hash of motherboard identifiers) it was issued for, so it only validates
//! on that PC. This is honest client-side protection: it stops casual sharing, not a determined cracker.

use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Raw Ed25519 public key (base64). The matching private key is a GitHub Actions secret of the owner.
pub const PUBLIC_KEY_B64: &str = "3Zg2ZZO2SYDIDL6kGFAqeNfSX+iNWtC55JgU9nD1iKQ=";
pub const PREFIX: &str = "NEOX1";
pub const BUY_URL: &str = "https://discord.gg/2h9FUFb99";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LicensePayload {
    pub id: String,
    pub name: String,
    pub hwid: String,
    #[serde(default)]
    pub plan: String,
    /// Unix seconds; 0 = lifetime.
    #[serde(default)]
    pub exp: i64,
    #[serde(default)]
    pub iat: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum LicenseStatus {
    Free,
    Active { id: String, name: String, exp: i64 },
    Invalid { reason: String },
}

impl LicenseStatus {
    pub fn is_premium(&self) -> bool { matches!(self, LicenseStatus::Active { .. }) }
}

/// Builds the hardware id shown to the user, e.g. `NEOX-1A2B-3C4D-5E6F-7A8B`.
/// `parts` are motherboard identifiers; empty or placeholder values must be removed by the caller.
pub fn hardware_id(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    h.update(b"neox|");
    for p in parts { h.update(p.trim().to_ascii_lowercase().as_bytes()); h.update(b"|"); }
    let d = h.finalize();
    let hex: String = d.iter().take(8).map(|b| format!("{b:02X}")).collect();
    format!("NEOX-{}-{}-{}-{}", &hex[0..4], &hex[4..8], &hex[8..12], &hex[12..16])
}

fn b64_decode(input: &str, url: bool) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in input.trim().bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A', b'a'..=b'z' => c - b'a' + 26, b'0'..=b'9' => c - b'0' + 52,
            b'+' if !url => 62, b'/' if !url => 63, b'-' if url => 62, b'_' if url => 63,
            b'=' => continue,
            _ => return None,
        } as u32;
        acc = (acc << 6) | v; bits += 6;
        if bits >= 8 { bits -= 8; out.push((acc >> bits) as u8); acc &= (1 << bits) - 1; }
    }
    Some(out)
}

fn invalid(r: &str) -> LicenseStatus { LicenseStatus::Invalid { reason: r.into() } }

/// Checks a token against `public_key`, this PC's hardware id and the current time.
pub fn verify_with(public_key: &[u8; 32], token: &str, my_hwid: &str, now: i64) -> LicenseStatus {
    let parts: Vec<&str> = token.trim().split('.').collect();
    if parts.len() != 3 || parts[0] != PREFIX { return invalid("This is not a NEOX license key."); }
    let (Some(payload), Some(sig)) = (b64_decode(parts[1], true), b64_decode(parts[2], true)) else { return invalid("The license key is damaged."); };
    let Ok(vk) = VerifyingKey::from_bytes(public_key) else { return invalid("Internal key error."); };
    let Ok(sig) = Signature::from_slice(&sig) else { return invalid("The license key is damaged."); };
    if vk.verify_strict(&payload, &sig).is_err() { return invalid("The signature does not match. The key was changed or is fake."); }
    let Ok(p) = serde_json::from_slice::<LicensePayload>(&payload) else { return invalid("The license key is damaged."); };
    if !p.hwid.eq_ignore_ascii_case(my_hwid.trim()) { return invalid("This license belongs to a different PC. Open a ticket to move it."); }
    if p.exp != 0 && now > p.exp { return invalid("This license has expired."); }
    LicenseStatus::Active { id: p.id, name: p.name, exp: p.exp }
}

pub fn verify(token: &str, my_hwid: &str, now: i64) -> LicenseStatus {
    match b64_decode(PUBLIC_KEY_B64, false).and_then(|v| <[u8; 32]>::try_from(v.as_slice()).ok()) {
        Some(k) => verify_with(&k, token, my_hwid, now),
        None => invalid("Internal key error."),
    }
}

// ---------------------------------------------------------------- what the free version includes

/// The optimizations available without a license (the free plan). Reverting is never restricted.
pub const FREE_IDS: &[&str] = &[
    "restore_point", "game_mode", "game_dvr", "gamebar_off", "mouse_accel", "sticky_keys", "bg_apps", "net_throttle",
    "qos_limit", "delivery_opt", "ads_off", "widgets_off", "temp_clean", "recycle", "dns_flush", "sysinfo",
    "open_startup", "open_taskmgr", "wpad_off", "llmnr_off",
    "mmcss_games", "fse_global", "kb_speed", "mouse_sens", "menu_delay", "startup_delay", "shutdown_fast", "no_auto_reboot",
    "lastaccess_off", "sysresp", "edge_boost_off", "chrome_bg_off", "dx_windowed", "prefetch", "shader_cache",
    "browser_cache", "crash_dumps", "trim_ssd", "open_resmon",
    // Keyboard and Mouse pages: the basics are free, the rest is Premium.
    "kb_numlock_on", "kb_printscreen_snip_off", "open_keyboard_settings", "kb_devices",
    "mouse_touch_feedback_off", "open_mouse", "mouse_devices", "mouse_accel_check",
    // More free tools.
    "usb3_lpm_off", "autoplay_off", "clipboard_history_off", "clean_vscode_cache", "clean_teams_cache", "net_defaults_reset",
];

pub fn is_free(id: &str) -> bool { FREE_IDS.contains(&id) }

/// Returns an error string starting with `PREMIUM:` when the caller may not use `id` on the free plan.
pub fn check_item(premium: bool, id: &str) -> Result<(), String> {
    if premium || is_free(id) { Ok(()) } else { Err("PREMIUM: This optimization is part of NEOX Premium.".into()) }
}
pub fn check_feature(premium: bool, name: &str) -> Result<(), String> {
    if premium { Ok(()) } else { Err(format!("PREMIUM: {name} is part of NEOX Premium.")) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn sk() -> SigningKey { SigningKey::from_bytes(&[7u8; 32]) }
    fn b64u(b: &[u8]) -> String {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut s = String::new();
        for c in b.chunks(3) {
            let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
            for i in 0..=c.len() { s.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char); }
        }
        s
    }
    fn make(p: &LicensePayload) -> String {
        let payload = serde_json::to_vec(p).unwrap();
        format!("{PREFIX}.{}.{}", b64u(&payload), b64u(&sk().sign(&payload).to_bytes()))
    }
    fn pubkey() -> [u8; 32] { sk().verifying_key().to_bytes() }
    fn payload(hwid: &str, exp: i64) -> LicensePayload { LicensePayload { id: "L1".into(), name: "Tester".into(), hwid: hwid.into(), plan: "premium".into(), exp, iat: 1 } }

    #[test]
    fn valid_license_on_the_right_pc_is_active() {
        let t = make(&payload("NEOX-AAAA-BBBB-CCCC-DDDD", 0));
        assert_eq!(verify_with(&pubkey(), &t, "neox-aaaa-bbbb-cccc-dddd", 100), LicenseStatus::Active { id: "L1".into(), name: "Tester".into(), exp: 0 });
    }
    #[test]
    fn another_pc_is_rejected() {
        let t = make(&payload("NEOX-AAAA-BBBB-CCCC-DDDD", 0));
        assert!(matches!(verify_with(&pubkey(), &t, "NEOX-1111-2222-3333-4444", 100), LicenseStatus::Invalid { .. }));
    }
    #[test]
    fn expired_license_is_rejected_and_future_one_is_fine() {
        let t = make(&payload("H", 1000));
        assert!(matches!(verify_with(&pubkey(), &t, "H", 1001), LicenseStatus::Invalid { .. }));
        assert!(verify_with(&pubkey(), &t, "H", 999).is_premium());
    }
    #[test]
    fn tampering_with_the_payload_breaks_the_signature() {
        let t = make(&payload("H", 1000));
        let mut parts: Vec<String> = t.split('.').map(String::from).collect();
        parts[1] = b64u(&serde_json::to_vec(&payload("H", 0)).unwrap());
        assert!(matches!(verify_with(&pubkey(), &parts.join("."), "H", 5), LicenseStatus::Invalid { .. }));
    }
    #[test]
    fn a_key_signed_by_someone_else_is_rejected() {
        let other = SigningKey::from_bytes(&[9u8; 32]);
        let pl = serde_json::to_vec(&payload("H", 0)).unwrap();
        let t = format!("{PREFIX}.{}.{}", b64u(&pl), b64u(&other.sign(&pl).to_bytes()));
        assert!(matches!(verify_with(&pubkey(), &t, "H", 5), LicenseStatus::Invalid { .. }));
    }
    #[test]
    fn garbage_is_rejected_without_panicking() {
        for g in ["", "x", "NEOX1.a.b", "NEOX1..", "NEOX1.!!!.???", "A.B.C"] {
            assert!(matches!(verify_with(&pubkey(), g, "H", 5), LicenseStatus::Invalid { .. }), "{g}");
        }
    }
    #[test]
    fn embedded_public_key_decodes_to_32_bytes() {
        assert_eq!(b64_decode(PUBLIC_KEY_B64, false).unwrap().len(), 32);
    }
    #[test]
    fn hardware_id_is_stable_and_formatted() {
        let a = hardware_id(&["uuid-1", "SN123", "Board X"]);
        assert_eq!(a, hardware_id(&["UUID-1 ", "sn123", "board x"]));
        assert_ne!(a, hardware_id(&["uuid-2", "SN123", "Board X"]));
        assert!(a.starts_with("NEOX-") && a.len() == 24);
    }
    #[test]
    fn free_plan_has_the_expected_items_and_gates_the_rest() {
        assert_eq!(FREE_IDS.len(), 53);
        let mut seen = std::collections::HashSet::new();
        assert!(FREE_IDS.iter().all(|i| seen.insert(*i)), "duplicate id in FREE_IDS");
        assert!(check_item(false, "game_mode").is_ok());
        assert!(check_item(false, "hvci_off").unwrap_err().starts_with("PREMIUM:"));
        assert!(check_item(true, "hvci_off").is_ok());
        assert!(check_feature(false, "Auto-Tune").is_err());
    }
}
