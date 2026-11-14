//! Activation codes: a short code (NEOX-XXXX-XXXX-XXXX) that the license server binds to the first PC that uses it.
//! The app asks the server again from time to time; without a connection it keeps working for a grace period.

use crate::license::LicenseStatus;
use serde::{Deserialize, Serialize};

/// How long an activation stays valid without a successful online check.
pub const GRACE_SECONDS: i64 = 14 * 86400;
/// Re-check with the server when the last successful check is older than this.
pub const RECHECK_SECONDS: i64 = 6 * 3600;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct Activation {
    pub code: String,
    pub hwid: String,
    pub name: String,
    /// Unix seconds, 0 = lifetime.
    pub exp: i64,
    pub verified_at: i64,
}
impl Default for Activation { fn default() -> Self { Activation { code: String::new(), hwid: String::new(), name: String::new(), exp: 0, verified_at: 0 } } }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply { Ok { name: String, exp: i64 }, Denied(String), Unreachable }

/// Marker stored as the code of a trial activation.
pub const TRIAL_CODE: &str = "TRIAL";

pub fn is_trial(a: &Activation) -> bool { a.code == TRIAL_CODE }

/// `NEOX-XXXX-XXXX-XXXX` (three groups). Hardware ids have four groups and are not codes.
pub fn normalize_code(input: &str) -> Option<String> {
    let t = input.trim().to_ascii_uppercase();
    let g: Vec<&str> = t.split('-').collect();
    let ok = g.len() == 4 && g[0] == "NEOX" && g[1..].iter().all(|x| x.len() == 4 && x.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()));
    ok.then_some(t)
}

/// Parses the server's JSON answer. Anything that is not a clear answer counts as unreachable, never as a denial.
pub fn parse_reply(body: &str) -> Reply {
    let Some(a) = body.find('{') else { return Reply::Unreachable };
    let Some(b) = body.rfind('}') else { return Reply::Unreachable };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&body[a..=b]) else { return Reply::Unreachable };
    match v["ok"].as_bool() {
        Some(true) => Reply::Ok { name: v["name"].as_str().unwrap_or("").chars().take(60).collect(), exp: v["exp"].as_i64().unwrap_or(0).max(0) },
        Some(false) => Reply::Denied(v["reason"].as_str().unwrap_or("The code was refused.").chars().take(160).collect()),
        None => Reply::Unreachable,
    }
}

pub fn status(a: &Activation, my_hwid: &str, now: i64) -> LicenseStatus {
    let bad = |r: &str| LicenseStatus::Invalid { reason: r.into() };
    if !a.hwid.eq_ignore_ascii_case(my_hwid.trim()) { return bad("This activation belongs to a different PC."); }
    if a.exp != 0 && now > a.exp { return bad("This code has expired."); }
    if now - a.verified_at > GRACE_SECONDS { return bad("Could not reach the license server for 14 days. Connect to the internet and restart NEOX."); }
    LicenseStatus::Active { id: a.code.clone(), name: a.name.clone(), exp: a.exp }
}

pub fn needs_recheck(a: &Activation, now: i64) -> bool { now - a.verified_at >= RECHECK_SECONDS }

#[cfg(test)]
mod tests {
    use super::*;
    fn act(v: i64) -> Activation { Activation { code: "NEOX-AAAA-BBBB-CCCC".into(), hwid: "NEOX-1111-2222-3333-4444".into(), name: "T".into(), exp: 0, verified_at: v } }

    #[test]
    fn codes_have_three_groups_and_hardware_ids_are_rejected() {
        assert_eq!(normalize_code(" neox-ab12-cd34-ef56 ").as_deref(), Some("NEOX-AB12-CD34-EF56"));
        assert!(normalize_code("NEOX-1111-2222-3333-4444").is_none());
        assert!(normalize_code("NEOX1.abc.def").is_none());
        assert!(normalize_code("NEOX-AB12-CD34-EF5").is_none());
        assert!(normalize_code("NEOX-ab1!-cd34-ef56").is_none());
    }
    #[test]
    fn replies_are_parsed_and_junk_is_never_a_denial() {
        assert_eq!(parse_reply(r#"{"ok":true,"name":"Kuba","exp":0}"#), Reply::Ok { name: "Kuba".into(), exp: 0 });
        assert_eq!(parse_reply(r#"{"ok":false,"reason":"Unknown code"}"#), Reply::Denied("Unknown code".into()));
        for junk in ["", "NETERR", "<html>502</html>", "{}", r#"{"ok":"yes"}"#, "{broken"] { assert_eq!(parse_reply(junk), Reply::Unreachable, "{junk}"); }
    }
    #[test]
    fn grace_period_and_expiry_and_pc_binding() {
        let h = "NEOX-1111-2222-3333-4444";
        assert!(status(&act(1000), h, 1000 + GRACE_SECONDS).is_premium());
        assert!(!status(&act(1000), h, 1001 + GRACE_SECONDS).is_premium());
        assert!(!status(&act(1000), "NEOX-9999-9999-9999-9999", 1000).is_premium());
        let mut e = act(1000); e.exp = 1500;
        assert!(status(&e, h, 1400).is_premium()); assert!(!status(&e, h, 1501).is_premium());
    }
    #[test]
    fn trial_activation_is_marked_and_expires_like_any_other() {
        let t = Activation { code: TRIAL_CODE.into(), hwid: "H".into(), name: "Trial".into(), exp: 5000, verified_at: 1000 };
        assert!(is_trial(&t) && status(&t, "H", 4000).is_premium() && !status(&t, "H", 5001).is_premium());
    }
    #[test]
    fn recheck_is_due_after_six_hours() {
        assert!(!needs_recheck(&act(1000), 1000 + RECHECK_SECONDS - 1)); assert!(needs_recheck(&act(1000), 1000 + RECHECK_SECONDS));
    }
}
