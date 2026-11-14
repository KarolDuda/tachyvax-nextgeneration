//! Built-in and custom profiles, JSON export/import (untrusted input), baselines and bulk revert.

use crate::catalog::Catalog;
use crate::engine::{new_id, Engine, RestorePoints};
use crate::models::*;
use crate::state::{Baseline, HistoryAction, Profile};
use crate::timefmt;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;

pub const FORMAT: &str = "neox-profile";
pub const MAX_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult { pub profile: Option<Profile>, pub dropped: usize, pub error: Option<String> }

fn fail(msg: &str, dropped: usize) -> ImportResult { ImportResult { profile: None, dropped, error: Some(msg.into()) } }

pub fn export(p: &Profile) -> String {
    serde_json::to_string_pretty(&json!({
        "format": FORMAT, "version": 1, "name": p.name, "description": p.description, "tweaks": p.tweak_ids
    })).unwrap_or_default()
}

/// Treats the file as untrusted: validates everything and drops risky entries.
pub fn import(catalog: &Catalog, text: &str) -> ImportResult {
    if text.trim().is_empty() { return fail("The file is empty.", 0); }
    if text.len() > MAX_BYTES { return fail("The file is too large to be a profile.", 0); }
    let Ok(root) = serde_json::from_str::<Value>(text) else { return fail("This is not a valid JSON file.", 0) };
    if root.get("format").and_then(Value::as_str) != Some(FORMAT) { return fail("This is not a NEOX profile.", 0); }
    let Some(arr) = root.get("tweaks").and_then(Value::as_array) else { return fail("The profile has no optimization list.", 0) };

    let mut name: String = root.get("name").and_then(Value::as_str).unwrap_or("").trim().chars().take(60).collect();
    if name.is_empty() { name = "Imported profile".into(); }
    let desc: String = root.get("description").and_then(Value::as_str).unwrap_or("").trim().chars().take(300).collect();

    let mut ids: Vec<String> = Vec::new();
    let mut dropped = 0usize;
    for n in arr {
        let allowed = n.as_str().and_then(|id| catalog.find(id)).filter(|t| is_profile_safe(t));
        match allowed {
            Some(t) if !ids.contains(&t.id) => ids.push(t.id.clone()),
            _ => dropped += 1,
        }
    }
    if ids.is_empty() { return fail("The profile contains no allowed optimizations.", dropped); }
    ImportResult {
        profile: Some(Profile { id: new_id(), name, description: if desc.is_empty() { "Imported profile.".into() } else { desc }, built_in: false, tweak_ids: ids }),
        dropped, error: None,
    }
}

fn is_profile_safe(t: &TweakDefinition) -> bool {
    !t.is_action() && t.is_simple() && matches!(t.risk, RiskLevel::Recommended | RiskLevel::Caution)
}

/// Built-in profiles are derived from the catalog by risk level, not from a hand-written list.
/// Left out of "Maximum performance", Esports and Network boost: conflicts with other items, only fixes specific symptoms,
/// or (platform_tick) forces the slow platform clock, which adds timer overhead and can raise latency on current Windows.
const MAX_EXCLUDED: &[&str] = &["ultimate_plan", "mpo_off", "hibernate_off", "fast_startup_off", "tcp_status", "kb_layout_hotkey", "platform_tick", "mouse_queue_small", "csrss_priority", "mouse_curve_flat"];

/// Raise power draw or disable power saving; kept out of the laptop profile.
const LAPTOP_UNSAFE: &[&str] = &["cpu_min", "core_parking", "boost_mode", "usb_suspend", "usb_reg", "pcie_aspm", "power_throttle", "cpu_epp", "cpu_idle_off", "wlan_perf", "ultimate_plan", "disk_idle", "usb3_lpm_off"];

pub fn built_in(catalog: &Catalog) -> Vec<Profile> {
    let net_ok = |t: &TweakDefinition| !crate::nettune::NET_UNSAFE.contains(&t.id.as_str());
    let game_area = |t: &TweakDefinition| matches!(t.category, TweakCategory::Performance | TweakCategory::Input | TweakCategory::Network);
    let ids = |f: &dyn Fn(&TweakDefinition) -> bool| catalog.all().iter().filter(|t| f(t)).map(|t| t.id.clone()).collect::<Vec<_>>();
    vec![
        Profile { id: "builtin.safe".into(), built_in: true, name: "Safe start".into(),
            description: "Only items marked as recommended for performance, input delay and network. No services or tasks.".into(),
            tweak_ids: ids(&|t| t.is_simple() && game_area(t) && net_ok(t) && t.risk == RiskLevel::Recommended) },
        Profile { id: "builtin.gaming".into(), built_in: true, name: "Gaming".into(),
            description: "Recommended and cautious changes for performance, input and network. No services, experimental or high-risk items.".into(),
            tweak_ids: ids(&|t| t.is_simple() && game_area(t) && net_ok(t) && matches!(t.risk, RiskLevel::Recommended | RiskLevel::Caution)) },
        Profile { id: "builtin.max".into(), built_in: true, name: "Maximum performance".into(),
            description: "Everything that can raise performance or cut latency: every performance, input and network tweak including high-risk ones (CPU mitigations, VBS/HVCI, hypervisor). Back up first; some items lower security and need a restart.".into(),
            tweak_ids: ids(&|t| (t.is_simple() || matches!(t.kind, TweakKind::Bcd | TweakKind::Cmd)) && game_area(t) && net_ok(t) && !MAX_EXCLUDED.contains(&t.id.as_str())) },
        Profile { id: "builtin.esports".into(), built_in: true, name: "Esports (lowest input latency)".into(),
            description: "Everything that shortens the path from mouse and keyboard to the screen: input, timers, power, MMCSS. Includes experimental items and boot settings.".into(),
            tweak_ids: ids(&|t| (t.is_simple() || matches!(t.kind, TweakKind::Bcd)) && matches!(t.category, TweakCategory::Input | TweakCategory::Performance)
                && !matches!(t.risk, RiskLevel::HighRisk) && !MAX_EXCLUDED.contains(&t.id.as_str())) },
        Profile { id: "builtin.network".into(), built_in: true, name: "Network boost".into(),
            description: "Lower ping and jitter: every network item including TCP, DNS and adapter settings. Some adapter changes briefly reconnect the network.".into(),
            tweak_ids: ids(&|t| !t.is_action() && t.category == TweakCategory::Network && net_ok(t) && !MAX_EXCLUDED.contains(&t.id.as_str())) },
        Profile { id: "builtin.streaming".into(), built_in: true, name: "Gaming + streaming".into(),
            description: "Gaming tweaks that keep OBS, Discord and the browser smooth: no services touched, no boot changes, nothing that limits background apps you stream with.".into(),
            tweak_ids: ids(&|t| t.is_simple() && game_area(t) && net_ok(t) && matches!(t.risk, RiskLevel::Recommended | RiskLevel::Caution)
                && !matches!(t.id.as_str(), "bg_apps" | "bg_access_off" | "cpu_min" | "core_parking" | "ultimate_plan")) },
        Profile { id: "builtin.laptop".into(), built_in: true, name: "Laptop (battery friendly)".into(),
            description: "Only low-risk items that do not raise power draw: no forced maximum CPU state, no disabled power saving.".into(),
            tweak_ids: ids(&|t| t.is_simple() && game_area(t) && t.risk == RiskLevel::Recommended && !LAPTOP_UNSAFE.contains(&t.id.as_str())) },
        Profile { id: "builtin.privacy".into(), built_in: true, name: "Privacy".into(),
            description: "Recommended privacy settings (telemetry, ads, suggestions).".into(),
            tweak_ids: ids(&|t| !t.is_action() && t.category == TweakCategory::Privacy && t.risk == RiskLevel::Recommended) },
    ]
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileApplyReport { pub applied: usize, pub skipped: usize, pub failed: usize, pub restart: RestartRequirement, pub lines: Vec<String> }

pub struct ProfileService<'a> { engine: &'a Engine, restore: &'a dyn RestorePoints }

impl<'a> ProfileService<'a> {
    pub fn new(engine: &'a Engine, restore: &'a dyn RestorePoints) -> Self { Self { engine, restore } }

    pub fn all(&self) -> Vec<Profile> {
        let mut v = built_in(self.engine.catalog());
        v.extend(self.engine.store().read(|s| s.custom_profiles.clone()));
        v
    }

    pub fn save_custom(&self, name: &str, ids: &[String], description: &str) -> Profile {
        let cat = self.engine.catalog();
        let mut clean: Vec<String> = Vec::new();
        for i in ids { if let Some(t) = cat.find(i) { if !t.is_action() && !clean.contains(&t.id) { clean.push(t.id.clone()); } } }
        let p = Profile { id: new_id(), name: name.trim().into(), description: description.into(), built_in: false, tweak_ids: clean };
        self.engine.store().write(|s| s.custom_profiles.push(p.clone()));
        let _ = self.engine.store().save();
        p
    }

    pub fn delete_custom(&self, id: &str) {
        self.engine.store().write(|s| s.custom_profiles.retain(|p| p.id != id));
        let _ = self.engine.store().save();
    }

    pub fn apply(&self, profile: &Profile, ctx: &TweakContext, progress: &dyn Fn(&str)) -> ProfileApplyReport {
        let mut lines = Vec::new();
        let (mut ok, mut skip, mut fail) = (0, 0, 0);
        let mut restart = RestartRequirement::None;
        let want_rp = self.engine.store().read(|s| s.settings.create_restore_point_before_profiles);
        if want_rp && self.engine.is_elevated() {
            progress("Creating a restore point…");
            let rp = self.restore.create(&format!("NEOX: {}", profile.name));
            lines.push(if rp.success { "Restore point created.".to_string() } else { format!("Restore point skipped: {}", rp.message) });
        }
        for id in &profile.tweak_ids {
            let Some(t) = self.engine.catalog().find(id) else { skip += 1; lines.push(format!("• {id}: not in the catalog")); continue };
            if t.requires_admin && !self.engine.is_elevated() { skip += 1; lines.push(format!("• {}: requires administrator", t.title)); continue; }
            progress(&t.title);
            let r = self.engine.apply(id, ctx);
            if r.success { ok += 1; if r.restart > restart { restart = r.restart; } lines.push(format!("✔ {}", t.title)); }
            else { fail += 1; lines.push(format!("✖ {}: {}", t.title, r.message)); }
        }
        self.engine.record_raw(&format!("profile:{}", profile.id), &format!("Profile: {}", profile.name), HistoryAction::ProfileApply,
                               fail == 0, &format!("{ok} applied, {skip} skipped, {fail} errors"));
        ProfileApplyReport { applied: ok, skipped: skip, failed: fail, restart, lines }
    }

    /// Reverts every change made by NEOX; returns how many were reverted.
    pub fn revert_all(&self, ctx: &TweakContext) -> usize {
        let ids: Vec<String> = self.engine.store().read(|s| s.snapshots.keys().cloned().collect());
        ids.iter().filter(|id| self.engine.revert(id, ctx).success).count()
    }

    pub fn capture_baseline(&self, name: &str, ctx: &TweakContext) -> Baseline {
        let b = Baseline { name: name.into(), time: timefmt::now_rfc3339(), statuses: self.engine.all_statuses(ctx) };
        self.engine.store().write(|s| { s.baselines.push(b.clone()); if s.baselines.len() > 20 { s.baselines.remove(0); } });
        let _ = self.engine.store().save();
        b
    }

    pub fn compare(&self, baseline: &Baseline, ctx: &TweakContext) -> Vec<(String, TweakStatus, TweakStatus)> {
        let now: HashMap<String, TweakStatus> = self.engine.all_statuses(ctx);
        baseline.statuses.iter().filter_map(|(id, before)| {
            let cur = now.get(id)?;
            (cur != before).then(|| (id.clone(), *before, *cur))
        }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::testing::*;

    struct NoRp;
    impl RestorePoints for NoRp { fn create(&self, _: &str) -> TweakResult { TweakResult::ok("ok", RestartRequirement::None) } }

    fn cat() -> Catalog { Catalog::load_embedded().unwrap() }

    #[test]
    fn builtin_profiles_never_contain_risky_or_system_items() {
        let c = cat();
        for p in built_in(&c) {
            assert!(!p.tweak_ids.is_empty(), "{}", p.name);
            for id in &p.tweak_ids {
                let t = c.find(id).unwrap();
                // "Maximum performance", "Esports" and "Network boost" deliberately include experimental or boot items.
                if !matches!(p.id.as_str(), "builtin.max" | "builtin.esports" | "builtin.network") {
                    assert!(!matches!(t.risk, RiskLevel::Experimental | RiskLevel::HighRisk), "{id}");
                    assert!(!matches!(t.kind, TweakKind::Bcd), "{id}");
                }
                assert!(!matches!(t.kind, TweakKind::Service | TweakKind::Task), "{id}");
            }
        }
    }

    #[test]
    fn no_profile_contains_items_that_can_slow_the_internet() {
        let c = cat();
        for p in built_in(&c) {
            for id in &p.tweak_ids { assert!(!crate::nettune::NET_UNSAFE.contains(&id.as_str()), "{} in {}", id, p.name); }
        }
    }

    #[test]
    fn keyboard_and_mouse_items_never_end_up_in_a_profile_by_accident() {
        let c = cat();
        for p in built_in(&c) {
            for id in &p.tweak_ids {
                let t = c.find(id).unwrap();
                if t.section.is_some() { assert!(matches!(t.risk, RiskLevel::Recommended | RiskLevel::Caution) || p.id == "builtin.max" || p.id == "builtin.esports", "{id} in {}", p.name); }
                assert!(!matches!(id.as_str(), "kb_scancode_gaming" | "kb_winkey_shortcuts_off"), "{id} in {}", p.name);
            }
        }
    }

    #[test]
    fn export_import_round_trip() {
        let c = cat();
        let p = built_in(&c).remove(0);
        let r = import(&c, &export(&p));
        let got = r.profile.expect("import should succeed");
        assert_eq!(got.tweak_ids, p.tweak_ids);
        assert_eq!(r.dropped, 0);
    }

    #[test]
    fn import_rejects_garbage_and_oversized_files() {
        let c = cat();
        assert!(import(&c, "").error.is_some());
        assert!(import(&c, "not json").error.is_some());
        assert!(import(&c, "{\"format\":\"other\",\"tweaks\":[]}").error.is_some());
        assert!(import(&c, &"x".repeat(MAX_BYTES + 1)).error.is_some());
    }

    #[test]
    fn import_drops_services_unknown_and_high_risk_entries() {
        let c = cat();
        let service = c.all().iter().find(|t| t.kind == TweakKind::Service).unwrap().id.clone();
        let high = c.all().iter().find(|t| t.risk == RiskLevel::HighRisk).unwrap().id.clone();
        let good = built_in(&c)[0].tweak_ids[0].clone();
        let text = json!({"format": FORMAT, "version": 1, "name": "x", "tweaks": [good, service, high, "nope", 5, good]}).to_string();
        let r = import(&c, &text);
        assert_eq!(r.profile.unwrap().tweak_ids, vec![good]);
        assert_eq!(r.dropped, 5);
    }

    #[test]
    fn applying_a_profile_reports_counts_and_revert_all_undoes_it() {
        let (e, store, _) = engine(true, Fake::new());
        store.write(|s| s.settings.create_restore_point_before_profiles = false);
        let rp = NoRp;
        let svc = ProfileService::new(&e, &rp);
        let p = Profile { id: "t".into(), name: "T".into(), description: "".into(), built_in: false,
                          tweak_ids: vec!["game_mode".into(), "missing".into()] };
        let rep = svc.apply(&p, &TweakContext::default(), &|_| {});
        assert_eq!((rep.applied, rep.skipped, rep.failed), (1, 1, 0));
        assert_eq!(svc.revert_all(&TweakContext::default()), 1);
        assert!(store.read(|s| s.snapshots.is_empty()));
    }

    #[test]
    fn baseline_compare_shows_only_changes() {
        let (e, _, _) = engine(true, Fake::new());
        let rp = NoRp;
        let svc = ProfileService::new(&e, &rp);
        let ctx = TweakContext::default();
        let b = svc.capture_baseline("before", &ctx);
        assert!(svc.compare(&b, &ctx).is_empty());
        assert!(e.apply("game_mode", &ctx).success);
        let diff = svc.compare(&b, &ctx);
        assert_eq!(diff.len(), 1);
        assert_eq!(diff[0].0, "game_mode");
    }
}
