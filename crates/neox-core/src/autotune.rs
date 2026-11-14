//! Auto-Tune: hardware-aware planning and honest before/after comparison.

use crate::catalog::Catalog;
use crate::models::*;
use serde::Serialize;
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareProfile {
    pub cores: u32,
    pub threads: u32,
    pub ram_bytes: u64,
    pub is_laptop: bool,
    pub has_ssd: bool,
    pub has_discrete_gpu: bool,
    pub build: u32,
}

impl HardwareProfile {
    pub fn ram_gb(&self) -> f64 { self.ram_bytes as f64 / 1_073_741_824.0 }
    pub fn describe(&self) -> String {
        format!("{} cores / {} threads · RAM {:.0} GB · {} · {} · {}", self.cores, self.threads, self.ram_gb(),
                if self.is_laptop { "laptop" } else { "desktop" },
                if self.has_ssd { "SSD" } else { "no SSD detected" },
                if self.has_discrete_gpu { "dedicated GPU" } else { "integrated or unknown GPU" })
    }
}

/// Micro-benchmark result. This is not FPS: it only measures what is repeatable without a game.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkResult { pub cpu_ops_per_sec: f64, pub memory_mbps: f64, pub timer_avg_ms: f64, pub timer_p99_ms: f64 }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Verdict { Better, Worse, Noise }

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricDelta { pub name: String, pub unit: String, pub before: f64, pub after: f64, pub percent: f64, pub verdict: Verdict, pub higher_is_better: bool }

pub const THROUGHPUT_NOISE_PERCENT: f64 = 5.0;
pub const LATENCY_NOISE_PERCENT: f64 = 10.0;

pub fn compare(before: &BenchmarkResult, after: &BenchmarkResult) -> Vec<MetricDelta> {
    vec![
        make("CPU performance (synthetic test)", "Mops/s", before.cpu_ops_per_sec / 1e6, after.cpu_ops_per_sec / 1e6, true, THROUGHPUT_NOISE_PERCENT),
        make("Memory bandwidth", "MB/s", before.memory_mbps, after.memory_mbps, true, THROUGHPUT_NOISE_PERCENT),
        make("Timer latency (average)", "ms", before.timer_avg_ms, after.timer_avg_ms, false, LATENCY_NOISE_PERCENT),
        make("Timer latency (99th percentile)", "ms", before.timer_p99_ms, after.timer_p99_ms, false, LATENCY_NOISE_PERCENT),
    ]
}

/// Changes below the noise threshold are measurement noise, never an improvement.
pub fn make(name: &str, unit: &str, before: f64, after: f64, higher_is_better: bool, noise_percent: f64) -> MetricDelta {
    let pct = if before <= 0.0 { 0.0 } else { (after - before) / before * 100.0 };
    let gain = if higher_is_better { pct } else { -pct };
    let verdict = if pct.abs() < noise_percent { Verdict::Noise } else if gain > 0.0 { Verdict::Better } else { Verdict::Worse };
    MetricDelta { name: name.into(), unit: unit.into(), before, after, percent: pct, verdict, higher_is_better }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedTweak { pub id: String, pub title: String, pub reason: String }

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoTunePlan { pub hardware: HardwareProfile, pub planned: Vec<PlannedTweak>, pub skipped: Vec<PlannedTweak> }

pub const ALREADY_APPLIED: &str = "already applied";

const LAPTOP_EXCLUDED: &[&str] = &["cpu_min", "core_parking", "cpu_epp", "pcie_aspm", "usb_suspend", "usb_reg", "power_throttle", "powerPlan", "wlan_perf", "disk_idle", "usb3_lpm_off"];

/// Items that change comfort or system behavior: Auto-Tune never touches them (you can enable them manually).
const ALWAYS_EXCLUDED: &[(&str, &str)] = &[
    ("ultimate_plan", "conflicts with the high-performance plan; pick it manually if you want it"),
    ("fast_startup_off", "affects boot time, not gaming"),
    ("toasts_off", "turns off notifications, so it is your call"),
    ("onedrive_off", "turns off syncing, so it is your call"),
    ("store_autodl", "changes Store behavior, so it is your call"),
    ("hover_time", "cosmetic UI change"),
    ("ink_workspace_off", "turns off the handwriting feature"),
    ("net_connectivity_probe_off", "may worsen network detection"),
    ("netbios_off", "may break network shares at home or at work"),
    ("prio_sep", "small effect that is hard to measure"),
    ("ui_priority_boost", "small effect that is hard to measure"),
    ("bg_access_off", "limits background UWP apps"),
];

/// Only Reg/PowerIdx/PowerPlan changes of recommended or caution risk in performance/input/network;
/// no services, tasks or bcdedit.
pub fn plan(catalog: &Catalog, hw: &HardwareProfile, elevated: bool, already_applied: &dyn Fn(&str) -> bool) -> AutoTunePlan {
    let laptop_ex: HashSet<&str> = LAPTOP_EXCLUDED.iter().copied().collect();
    let mut planned = Vec::new();
    let mut skipped = Vec::new();
    for t in catalog.all() {
        if t.is_action() { continue; }
        if !t.is_simple() { continue; }
        if !matches!(t.category, TweakCategory::Performance | TweakCategory::Input | TweakCategory::Network) { continue; }
        if !matches!(t.risk, RiskLevel::Recommended | RiskLevel::Caution) { continue; }
        match exclusion(t, hw, elevated, already_applied, &laptop_ex) {
            None => planned.push(PlannedTweak { id: t.id.clone(), title: t.title.clone(), reason: why(t, hw) }),
            Some(r) => skipped.push(PlannedTweak { id: t.id.clone(), title: t.title.clone(), reason: r }),
        }
    }
    AutoTunePlan { hardware: hw.clone(), planned, skipped }
}

fn exclusion(t: &TweakDefinition, hw: &HardwareProfile, elevated: bool, applied: &dyn Fn(&str) -> bool, laptop_ex: &HashSet<&str>) -> Option<String> {
    if let Some((_, why)) = ALWAYS_EXCLUDED.iter().find(|(id, _)| *id == t.id) { return Some((*why).into()); }
    if applied(&t.id) { return Some(ALREADY_APPLIED.into()); }
    if t.requires_admin && !elevated { return Some("requires administrator".into()); }
    if hw.is_laptop && laptop_ex.contains(t.id.as_str()) { return Some("laptop: we keep battery saving and quiet operation".into()); }
    if t.id == "prefetch_off" && !hw.has_ssd { return Some("Prefetch helps on drives without an SSD".into()); }
    if t.id == "nopaging_exec" && hw.ram_gb() < 16.0 { return Some("needs at least 16 GB of RAM".into()); }
    if t.id == "core_parking" && hw.threads < 6 { return Some("too few threads for this to matter".into()); }
    None
}

fn why(t: &TweakDefinition, hw: &HardwareProfile) -> String {
    match t.category {
        TweakCategory::Input => "lower latency and more consistent input".into(),
        TweakCategory::Network => "steadier network in games".into(),
        _ => if hw.is_laptop { "lower system load with no battery impact".into() } else { "less background work while gaming".into() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hw(laptop: bool, ssd: bool, ram_gb: u64, threads: u32) -> HardwareProfile {
        HardwareProfile { cores: threads / 2, threads, ram_bytes: ram_gb * 1_073_741_824, is_laptop: laptop, has_ssd: ssd, has_discrete_gpu: true, build: 22631 }
    }
    fn cat() -> Catalog { Catalog::load_embedded().unwrap() }

    #[test]
    fn plan_only_contains_safe_simple_tweaks() {
        let c = cat();
        let p = plan(&c, &hw(false, true, 32, 16), true, &|_| false);
        assert!(!p.planned.is_empty());
        for t in &p.planned {
            let d = c.find(&t.id).unwrap();
            assert!(d.is_simple(), "{}", t.id);
            assert!(matches!(d.risk, RiskLevel::Recommended | RiskLevel::Caution), "{}", t.id);
            assert!(!t.reason.is_empty());
        }
    }

    #[test]
    fn laptop_keeps_battery_features() {
        let c = cat();
        let p = plan(&c, &hw(true, true, 16, 8), true, &|_| false);
        for id in ["cpu_min", "core_parking", "pcie_aspm"] { assert!(!p.planned.iter().any(|t| t.id == id), "{id}"); }
    }

    #[test]
    fn already_applied_and_admin_are_skipped_with_reasons() {
        let c = cat();
        let first = plan(&c, &hw(false, true, 32, 16), true, &|_| false);
        let id = first.planned[0].id.clone();
        let p = plan(&c, &hw(false, true, 32, 16), true, &|x| x == id);
        assert!(p.skipped.iter().any(|s| s.id == id && s.reason == ALREADY_APPLIED));
        let no_admin = plan(&c, &hw(false, true, 32, 16), false, &|_| false);
        assert!(no_admin.skipped.iter().any(|s| s.reason == "requires administrator"));
    }

    #[test]
    fn small_ram_excludes_nopaging_exec() {
        let c = cat();
        let p = plan(&c, &hw(false, true, 8, 16), true, &|_| false);
        assert!(!p.planned.iter().any(|t| t.id == "nopaging_exec"));
    }

    #[test]
    fn noise_is_never_reported_as_improvement() {
        let m = make("x", "u", 100.0, 103.0, true, THROUGHPUT_NOISE_PERCENT);
        assert_eq!(m.verdict, Verdict::Noise);
    }

    #[test]
    fn higher_is_better_and_lower_is_better_are_handled() {
        assert_eq!(make("x", "u", 100.0, 120.0, true, 5.0).verdict, Verdict::Better);
        assert_eq!(make("x", "u", 100.0, 80.0, true, 5.0).verdict, Verdict::Worse);
        assert_eq!(make("x", "ms", 1.0, 0.7, false, 10.0).verdict, Verdict::Better);
        assert_eq!(make("x", "ms", 1.0, 1.5, false, 10.0).verdict, Verdict::Worse);
        assert_eq!(make("x", "ms", 0.0, 5.0, false, 10.0).verdict, Verdict::Noise);
    }

    #[test]
    fn compare_returns_four_metrics() {
        let r = BenchmarkResult { cpu_ops_per_sec: 1e8, memory_mbps: 9000.0, timer_avg_ms: 1.0, timer_p99_ms: 2.0 };
        assert_eq!(compare(&r, &r).len(), 4);
    }
}
