//! Pure function: 0-100 score from explicit, explainable rules. No magic and no invented numbers.

use serde::Serialize;

#[derive(Debug, Clone)]
pub struct HealthInput {
    pub worst_disk_free_percent: f64,
    pub ram_load_percent: i32,
    pub uptime_days: f64,
    pub startup_enabled_count: i32,
    pub attainable_tweaks: i32,
    pub applied_tweaks: i32,
    pub pending_restart: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FixKind { None, ApplySafeProfile, OpenStartup, OpenDisk, Restart }

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthCheck { pub name: String, pub detail: String, pub score: f64, pub weight: f64, pub fix: FixKind, pub ok: bool }

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthReport { pub score: i32, pub label: String, pub checks: Vec<HealthCheck> }

fn check(name: &str, detail: String, score: f64, weight: f64, fix: FixKind) -> HealthCheck {
    HealthCheck { name: name.into(), detail, score, weight, fix, ok: score >= 0.99 }
}

pub fn score(i: &HealthInput) -> HealthReport {
    let mut checks = Vec::new();

    let d = i.worst_disk_free_percent;
    checks.push(if d < 10.0 { check("Free disk space", format!("Only {d:.0}% free space left"), 0.0, 25.0, FixKind::OpenDisk) }
        else if d < 20.0 { check("Free disk space", format!("Free space: {d:.0}% (20%+ recommended)"), 0.5, 25.0, FixKind::OpenDisk) }
        else { check("Free disk space", format!("Free space: {d:.0}%"), 1.0, 25.0, FixKind::None) });

    let u = i.uptime_days;
    checks.push(if u > 14.0 { check("Time since last restart", format!("Up for {u:.0} days without a restart"), 0.0, 15.0, FixKind::Restart) }
        else if u > 7.0 { check("Time since last restart", format!("Up for {u:.0} days without a restart"), 0.5, 15.0, FixKind::Restart) }
        else { check("Time since last restart", "Restarted recently".into(), 1.0, 15.0, FixKind::None) });

    let s = i.startup_enabled_count;
    checks.push(if s > 12 { check("Startup programs", format!("{s} programs start with Windows"), 0.0, 15.0, FixKind::OpenStartup) }
        else if s > 8 { check("Startup programs", format!("{s} programs start with Windows"), 0.5, 15.0, FixKind::OpenStartup) }
        else { check("Startup programs", format!("{s} startup programs"), 1.0, 15.0, FixKind::None) });

    checks.push(if i.attainable_tweaks == 0 {
        check("Applied optimizations", "No optimizations available".into(), 1.0, 30.0, FixKind::None)
    } else {
        let ratio = (i.applied_tweaks as f64 / i.attainable_tweaks as f64).clamp(0.0, 1.0);
        check("Applied optimizations", format!("Applied {} of {} safe optimizations", i.applied_tweaks, i.attainable_tweaks), ratio, 30.0,
              if ratio >= 0.99 { FixKind::None } else { FixKind::ApplySafeProfile })
    });

    checks.push(if i.pending_restart {
        check("Changes waiting for a restart", "Some optimizations only take effect after a restart".into(), 0.0, 15.0, FixKind::Restart)
    } else { check("Changes waiting for a restart", "No pending restarts".into(), 1.0, 15.0, FixKind::None) });

    let r = i.ram_load_percent;
    checks.push(if r > 85 { check("Memory usage", format!("RAM is {r}% in use"), 0.0, 10.0, FixKind::OpenStartup) }
        else if r > 70 { check("Memory usage", format!("RAM is {r}% in use"), 0.5, 10.0, FixKind::OpenStartup) }
        else { check("Memory usage", format!("RAM is {r}% in use"), 1.0, 10.0, FixKind::None) });

    let total: f64 = checks.iter().map(|c| c.weight).sum();
    let sc = (checks.iter().map(|c| c.score * c.weight).sum::<f64>() / total * 100.0).round() as i32;
    let label = match sc { 90..=100 => "Excellent", 75..=89 => "Good", 55..=74 => "Could be better", _ => "Needs attention" };
    HealthReport { score: sc, label: label.into(), checks }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn base() -> HealthInput {
        HealthInput { worst_disk_free_percent: 50.0, ram_load_percent: 40, uptime_days: 1.0, startup_enabled_count: 3,
                      attainable_tweaks: 20, applied_tweaks: 20, pending_restart: false }
    }
    #[test] fn perfect_system_scores_100() { let r = score(&base()); assert_eq!(r.score, 100); assert_eq!(r.label, "Excellent"); }
    #[test] fn low_disk_space_lowers_score_and_offers_fix() {
        let mut i = base(); i.worst_disk_free_percent = 5.0;
        let r = score(&i);
        assert_eq!(r.score, 77);
        assert!(r.checks.iter().any(|c| c.fix == FixKind::OpenDisk));
    }
    #[test] fn coverage_scales_with_applied_tweaks() {
        let mut i = base(); i.applied_tweaks = 10;
        let r = score(&i);
        assert_eq!(r.score, 86);
        assert!(r.checks.iter().any(|c| c.fix == FixKind::ApplySafeProfile));
    }
    #[test] fn no_attainable_tweaks_does_not_divide_by_zero() { let mut i = base(); i.attainable_tweaks = 0; i.applied_tweaks = 0; assert_eq!(score(&i).score, 100); }
    #[test] fn everything_bad_needs_attention() {
        let i = HealthInput { worst_disk_free_percent: 3.0, ram_load_percent: 95, uptime_days: 30.0, startup_enabled_count: 20,
                              attainable_tweaks: 20, applied_tweaks: 0, pending_restart: true };
        let r = score(&i); assert_eq!(r.score, 0); assert_eq!(r.label, "Needs attention");
    }
}
