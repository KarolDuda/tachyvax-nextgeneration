//! Backend API exposed to the UI. Every command runs on a worker thread so the window never blocks.

use neox_core::autotune::{self, AutoTunePlan, BenchmarkResult, MetricDelta};
use neox_core::catalog::Catalog;
use neox_core::engine::{ActionHandler, Elevation, Engine, RestorePoints, TweakHandler};
use neox_core::health::{self, HealthInput, HealthReport};
use neox_core::license::{self, LicenseStatus};
use neox_core::nettune;
use neox_core::models::*;
use neox_core::profiles::{self, ImportResult, ProfileApplyReport, ProfileService};
use neox_core::state::{AppSettings, Baseline, GameProfile, HistoryEntry, Profile, StateStore};
use neox_sys::actions::Actions;
use neox_sys::adapters::{Adapters, PsAdapters};
use neox_sys::cleaner::Cleaner;
use neox_sys::games::{self, DetectedGame, GameBoost};
use neox_sys::handlers::*;
use neox_sys::managers::{self, FolderUsage, ProcessEntry, ServiceEntry, StartupEntry};
use neox_sys::network::{self, DnsBenchResult, PingStats};
use neox_sys::registry::RegistryApi;
use neox_sys::runner::{ProcRunner, Runner};
use neox_sys::sysinfo::{self, CpuSampler, LiveSample, SystemSnapshot};
use neox_sys::system::{self, PsRestorePoints, WinElevation};
use neox_sys::update::{self, UpdateInfo};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};

#[cfg(windows)]
type Reg = neox_sys::registry::WinRegistry;
#[cfg(not(windows))]
type Reg = neox_sys::registry::MemRegistry;

fn new_reg() -> Reg { Reg::default_instance() }

trait DefaultInstance { fn default_instance() -> Self; }
#[cfg(windows)]
impl DefaultInstance for neox_sys::registry::WinRegistry { fn default_instance() -> Self { neox_sys::registry::WinRegistry } }
#[cfg(not(windows))]
impl DefaultInstance for neox_sys::registry::MemRegistry { fn default_instance() -> Self { neox_sys::registry::MemRegistry::new() } }

pub struct Ctx {
    engine: Arc<Engine>,
    store: Arc<StateStore>,
    run: Arc<dyn Runner>,
    reg: Arc<Reg>,
    adapters: Arc<dyn Adapters>,
    restore: Arc<PsRestorePoints>,
    snapshot: Mutex<Option<(Instant, SystemSnapshot)>>,
    boost: Arc<GameBoost>,
    boost_on: AtomicBool,
    monitor_on: AtomicBool,
    pending_update: Mutex<Option<UpdateInfo>>,
    hwid: std::sync::OnceLock<String>,
}

type S<'a> = tauri::State<'a, Arc<Ctx>>;

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())
}

fn tctx(game_exe: Option<String>) -> TweakContext { TweakContext { game_exe_path: game_exe } }

impl Ctx {
    fn hwid(&self) -> &str { self.hwid.get_or_init(|| neox_sys::hwid::read(self.run.as_ref())) }
    /// Offline signed key first, then a server-bound activation code. Never touches the network.
    fn license(&self) -> LicenseStatus {
        let now = neox_core::timefmt::now_unix();
        let mut last_bad = None;
        if let Some(k) = self.store.read(|s| s.license_key.clone()) {
            let st = license::verify(&k, self.hwid(), now);
            if st.is_premium() { return st; }
            last_bad = Some(st);
        }
        if let Some(a) = self.store.read(|s| s.activation.clone()) {
            let st = neox_core::activation::status(&a, self.hwid(), now);
            if st.is_premium() { return st; }
            last_bad = Some(st);
        }
        last_bad.unwrap_or(LicenseStatus::Free)
    }
    /// Asks the license server again (at start and every few hours). A clear refusal removes the activation;
    /// a network failure changes nothing, so offline use keeps working for the grace period.
    fn recheck_activation(&self) {
        use neox_core::activation::{needs_recheck, Reply};
        let now = neox_core::timefmt::now_unix();
        let Some(a) = self.store.read(|s| s.activation.clone()) else { return };
        if !needs_recheck(&a, now) { return; }
        let reply = if neox_core::activation::is_trial(&a) { neox_sys::activation::trial(self.run.as_ref(), self.hwid()) } else { neox_sys::activation::verify(self.run.as_ref(), &a.code, self.hwid()) };
        match reply {
            Reply::Ok { name, exp } => { self.store.write(|s| if let Some(x) = s.activation.as_mut() { x.verified_at = now; x.name = name; x.exp = exp; }); let _ = self.store.save(); }
            Reply::Denied(_) => { self.store.write(|s| s.activation = None); let _ = self.store.save(); }
            Reply::Unreachable => {}
        }
    }
    fn premium(&self) -> bool { self.license().is_premium() }
    /// When Premium (or the trial) has ended, everything that was applied with it is rolled back to how it was before.
    /// Free items stay. Returns how many changes were reverted. Never runs for a user who never had Premium.
    fn enforce_free_plan(&self) -> usize {
        if !self.store.read(|s| s.premium_ever) || self.premium() { return 0; }
        let ids: Vec<String> = self.store.read(|s| s.snapshots.keys().filter(|id| !license::is_free(id)).cloned().collect());
        let n = ids.iter().filter(|id| self.engine.revert(id, &tctx(None)).success).count();
        if self.store.read(|s| s.settings.game_boost) { self.store.write(|s| s.settings.game_boost = false); }
        if n > 0 { self.store.write(|s| s.downgrade_notice += n); }
        let _ = self.store.save();
        n
    }
    /// Err("PREMIUM: …") when the feature needs a license.
    fn need(&self, feature: &str) -> Result<(), String> { license::check_feature(self.premium(), feature) }
    fn need_item(&self, id: &str) -> Result<(), String> { if license::is_free(id) { Ok(()) } else { license::check_item(self.premium(), id) } }

    fn snapshot(&self) -> SystemSnapshot {
        let mut g = self.snapshot.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((t, s)) = g.as_ref() { if t.elapsed() < Duration::from_secs(30) { return s.clone(); } }
        let s = sysinfo::scan(self.run.as_ref());
        *g = Some((Instant::now(), s.clone()));
        s
    }

    fn profiles(&self) -> ProfileService<'_> { ProfileService::new(&self.engine, self.restore.as_ref()) }

    fn start_boost(self: &Arc<Self>) {
        if self.boost_on.swap(true, Ordering::SeqCst) { return; }
        let me = self.clone();
        std::thread::spawn(move || {
            me.boost.mark_waiting();
            while me.boost_on.load(Ordering::SeqCst) {
                me.boost.tick();
                for _ in 0..8 { if !me.boost_on.load(Ordering::SeqCst) { break; } std::thread::sleep(Duration::from_millis(500)); }
            }
            me.boost.restore();
            me.boost.mark_off();
        });
    }
}

// ------------------------------------------------------------------ app

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo { version: String, elevated: bool, first_run_done: bool, settings: AppSettings, pending_restart: Vec<String>, applied_count: usize, downgrade_notice: usize }

#[tauri::command]
async fn app_info(c: S<'_>) -> Result<AppInfo, String> {
    let c = c.inner().clone();
    blocking(move || {
        // A restart that already happened must not keep nagging: drop the list when Windows booted after the change.
        let now = neox_core::timefmt::now_unix();
        let boot = neox_sys::memory::boot_unix(now);
        if boot > 0 { if c.engine.clear_pending_if_rebooted(boot) { let _ = c.store.save(); } }
        c.recheck_activation();
        c.enforce_free_plan();
        AppInfo {
        version: update::current_version().to_string(), elevated: c.engine.is_elevated(),
        first_run_done: c.store.read(|s| s.settings.first_run_done), settings: c.store.read(|s| s.settings.clone()),
        pending_restart: c.store.read(|s| s.pending_restart.clone()), applied_count: c.store.read(|s| s.snapshots.len()), downgrade_notice: c.store.read(|s| s.downgrade_notice),
    } }).await
}

#[tauri::command]
async fn settings_set(c: S<'_>, patch: serde_json::Value) -> Result<AppSettings, String> {
    let c = c.inner().clone();
    blocking(move || {
        c.store.write(|s| {
            if let Some(v) = patch["autoUpdate"].as_bool() { s.settings.auto_update = v; }
            if let Some(v) = patch["createRestorePointBeforeProfiles"].as_bool() { s.settings.create_restore_point_before_profiles = v; }
            if let Some(v) = patch["firstRunDone"].as_bool() { s.settings.first_run_done = v; }
            if patch.get("gameExePath").is_some() { s.settings.game_exe_path = patch["gameExePath"].as_str().map(|x| x.to_string()).filter(|x| !x.is_empty()); }
        });
        let _ = c.store.save();
        c.store.read(|s| s.settings.clone())
    }).await
}

#[tauri::command]
async fn restart_as_admin(c: S<'_>, app: tauri::AppHandle) -> Result<bool, String> {
    let c = c.inner().clone();
    let ok = blocking(move || system::restart_elevated(c.run.as_ref())).await?;
    if ok { app.exit(0); }
    Ok(ok)
}

#[tauri::command]
async fn pick_file(c: S<'_>, filter: String) -> Result<Option<String>, String> {
    let c = c.inner().clone();
    blocking(move || {
        let f = filter.replace('\'', "''");
        let script = format!("Add-Type -AssemblyName System.Windows.Forms; $d=New-Object System.Windows.Forms.OpenFileDialog; $d.Filter='{f}'; \
if($d.ShowDialog() -eq 'OK'){{ $d.FileName }}");
        let argv = vec!["powershell".to_string(), "-NoProfile".into(), "-STA".into(), "-Command".into(), script];
        let r = c.run.run(&argv, 600);
        let p = r.output.trim().to_string();
        if r.success() && !p.is_empty() { Some(p) } else { None }
    }).await
}

// ------------------------------------------------------------------ catalog / engine

#[tauri::command]
fn catalog_list(c: S<'_>) -> Vec<TweakDefinition> { c.engine.catalog().all().to_vec() }

#[tauri::command]
async fn statuses(c: S<'_>, game_exe: Option<String>) -> Result<std::collections::HashMap<String, TweakStatus>, String> {
    let c = c.inner().clone();
    blocking(move || c.engine.all_statuses(&tctx(game_exe))).await
}

#[tauri::command]
async fn applied_ids(c: S<'_>) -> Result<Vec<String>, String> {
    let c = c.inner().clone();
    blocking(move || c.store.read(|s| s.snapshots.keys().cloned().collect())).await
}

#[tauri::command]
async fn tweak_apply(c: S<'_>, id: String, game_exe: Option<String>) -> Result<TweakResult, String> {
    let c = c.inner().clone();
    blocking(move || { c.need_item(&id)?; Ok(c.engine.apply(&id, &tctx(game_exe))) }).await?
}

#[tauri::command]
async fn tweak_revert(c: S<'_>, id: String, game_exe: Option<String>) -> Result<TweakResult, String> {
    let c = c.inner().clone();
    blocking(move || c.engine.revert(&id, &tctx(game_exe))).await
}

#[tauri::command]
async fn action_run(c: S<'_>, id: String) -> Result<TweakResult, String> {
    let c = c.inner().clone();
    blocking(move || { c.need_item(&id)?; Ok(c.engine.run_action(&id, &tctx(None))) }).await?
}

#[tauri::command]
async fn clear_pending_restart(c: S<'_>) -> Result<(), String> {
    let c = c.inner().clone();
    blocking(move || c.engine.clear_pending_restart()).await
}

// ------------------------------------------------------------------ profiles

#[tauri::command]
async fn profiles_list(c: S<'_>) -> Result<Vec<Profile>, String> {
    let c = c.inner().clone();
    blocking(move || c.profiles().all()).await
}

#[tauri::command]
async fn profile_apply(c: S<'_>, app: tauri::AppHandle, id: String, game_exe: Option<String>) -> Result<ProfileApplyReport, String> {
    let c = c.inner().clone();
    blocking(move || {
        c.need("Profiles")?;
        let svc = c.profiles();
        let Some(p) = svc.all().into_iter().find(|p| p.id == id) else {
            return Ok(ProfileApplyReport { applied: 0, skipped: 0, failed: 1, restart: RestartRequirement::None, lines: vec!["Unknown profile.".into()] });
        };
        Ok(svc.apply(&p, &tctx(game_exe), &|m| { let _ = app.emit("progress", m); }))
    }).await?
}

#[tauri::command]
async fn profile_save(c: S<'_>, name: String, description: Option<String>) -> Result<Option<Profile>, String> {
    let c = c.inner().clone();
    blocking(move || {
        c.need("Custom profiles")?;
        let ids: Vec<String> = c.store.read(|s| s.snapshots.keys().cloned().collect());
        if ids.is_empty() { return Ok(None); }
        Ok(Some(c.profiles().save_custom(&name, &ids, &description.unwrap_or_else(|| "Custom profile from applied changes.".into()))))
    }).await?
}

#[tauri::command]
async fn profile_delete(c: S<'_>, id: String) -> Result<(), String> {
    let c = c.inner().clone();
    blocking(move || c.profiles().delete_custom(&id)).await
}

#[tauri::command]
async fn profile_export(c: S<'_>, id: String) -> Result<Option<String>, String> {
    let c = c.inner().clone();
    blocking(move || { c.need("Profile export")?; Ok(c.profiles().all().into_iter().find(|p| p.id == id).map(|p| profiles::export(&p))) }).await?
}

#[tauri::command]
async fn profile_import(c: S<'_>, text: String) -> Result<ImportResult, String> {
    let c = c.inner().clone();
    blocking(move || {
        c.need("Profile import")?;
        let r = profiles::import(c.engine.catalog(), &text);
        if let Some(p) = &r.profile { c.profiles().save_custom(&p.name, &p.tweak_ids, &p.description); }
        Ok(r)
    }).await?
}

#[tauri::command]
async fn revert_all(c: S<'_>) -> Result<usize, String> {
    let c = c.inner().clone();
    blocking(move || c.profiles().revert_all(&tctx(None))).await
}

#[tauri::command]
fn history_list(c: S<'_>) -> Vec<HistoryEntry> {
    let mut h = c.store.read(|s| s.history.clone());
    h.reverse();
    h.truncate(300);
    h
}

#[tauri::command]
async fn baseline_capture(c: S<'_>, name: String) -> Result<Baseline, String> {
    let c = c.inner().clone();
    blocking(move || { c.need("Baselines")?; Ok(c.profiles().capture_baseline(&name, &tctx(None))) }).await?
}

#[tauri::command]
fn baselines_list(c: S<'_>) -> Vec<Baseline> { c.store.read(|s| s.baselines.clone()) }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiffRow { id: String, title: String, before: TweakStatus, now: TweakStatus }

#[tauri::command]
async fn baseline_compare(c: S<'_>, index: usize) -> Result<Vec<DiffRow>, String> {
    let c = c.inner().clone();
    blocking(move || {
        c.need("Baselines")?;
        let Some(b) = c.store.read(|s| s.baselines.get(index).cloned()) else { return Ok(vec![]) };
        Ok(c.profiles().compare(&b, &tctx(None)).into_iter()
            .map(|(id, before, now)| DiffRow { title: c.engine.catalog().find(&id).map(|t| t.title.clone()).unwrap_or_else(|| id.clone()), id, before, now }).collect())
    }).await?
}

// ------------------------------------------------------------------ Auto-Tune + health

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanView { plan: AutoTunePlan, description: String, elevated: bool }

#[tauri::command]
async fn autotune_plan(c: S<'_>, app: tauri::AppHandle) -> Result<PlanView, String> {
    let c = c.inner().clone();
    blocking(move || {
        let _ = app.emit("progress", "Scanning hardware…");
        let hw = c.snapshot().hardware_profile(std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(4));
        let plan = autotune::plan(c.engine.catalog(), &hw, c.engine.is_elevated(), &|id| c.engine.is_applied(id));
        PlanView { description: hw.describe(), plan, elevated: c.engine.is_elevated() }
    }).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AutoTuneReport { apply: ProfileApplyReport, applied_ids: Vec<String>, before: Option<BenchmarkResult>, after: Option<BenchmarkResult>, deltas: Vec<MetricDelta> }

#[tauri::command]
async fn autotune_run(c: S<'_>, app: tauri::AppHandle, ids: Vec<String>, benchmark: bool) -> Result<AutoTuneReport, String> {
    let c = c.inner().clone();
    blocking(move || {
        c.need("Auto-Tune")?;
        let say = |m: &str| { let _ = app.emit("progress", m); };
        let never = || false;
        let before = if benchmark { say("Measuring your PC before the changes…"); neox_sys::bench::run(&say, &never) } else { None };
        let profile = Profile { id: "autotune".into(), name: "Auto-Tune".into(), description: String::new(), built_in: false, tweak_ids: ids };
        let apply = c.profiles().apply(&profile, &tctx(None), &say);
        let applied_ids: Vec<String> = profile.tweak_ids.iter().filter(|id| c.engine.is_applied(id)).cloned().collect();
        c.store.write(|s| { s.last_autotune_ids = applied_ids.clone(); s.settings.last_auto_tune = Some(neox_core::timefmt::now_rfc3339()); });
        let _ = c.store.save();
        let after = if benchmark { say("Measuring again after the changes…"); neox_sys::bench::run(&say, &never) } else { None };
        let deltas = match (&before, &after) { (Some(b), Some(a)) => autotune::compare(b, a), _ => vec![] };
        Ok(AutoTuneReport { apply, applied_ids, before, after, deltas })
    }).await?
}

#[tauri::command]
async fn autotune_undo(c: S<'_>) -> Result<usize, String> {
    let c = c.inner().clone();
    blocking(move || {
        let ids: Vec<String> = c.store.read(|s| s.last_autotune_ids.clone());
        let n = ids.iter().filter(|id| c.engine.revert(id, &tctx(None)).success).count();
        c.store.write(|s| s.last_autotune_ids.clear());
        let _ = c.store.save();
        n
    }).await
}

#[tauri::command]
async fn health_scan(c: S<'_>) -> Result<HealthReport, String> {
    let c = c.inner().clone();
    blocking(move || {
        let snap = c.snapshot();
        let reg = new_reg();
        let startup = managers::list_startup(&reg).iter().filter(|e| e.enabled).count() as i32;
        let safe: Vec<String> = profiles::built_in(c.engine.catalog()).remove(0).tweak_ids.into_iter()
            .filter(|id| c.engine.catalog().find(id).is_some_and(|t| c.engine.is_elevated() || !t.requires_admin)).collect();
        let applied = safe.iter().filter(|id| c.engine.is_applied(id)).count() as i32;
        health::score(&HealthInput {
            worst_disk_free_percent: snap.worst_disk_free_percent(), ram_load_percent: snap.ram_load_percent(),
            uptime_days: snap.uptime_days(neox_core::timefmt::now_unix()), startup_enabled_count: startup,
            attainable_tweaks: safe.len() as i32, applied_tweaks: applied, pending_restart: c.store.read(|s| !s.pending_restart.is_empty()),
        })
    }).await
}

#[tauri::command]
async fn system_snapshot(c: S<'_>) -> Result<SystemSnapshot, String> {
    let c = c.inner().clone();
    blocking(move || c.snapshot()).await
}

// ------------------------------------------------------------------ monitor

#[tauri::command]
fn monitor_start(c: S<'_>, app: tauri::AppHandle) {
    let c = c.inner().clone();
    if c.monitor_on.swap(true, Ordering::SeqCst) { return; }
    std::thread::spawn(move || {
        let mut sampler = CpuSampler::new();
        while c.monitor_on.load(Ordering::SeqCst) {
            let s: LiveSample = sampler.sample();
            let _ = app.emit("monitor", s);
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}

#[tauri::command]
fn monitor_stop(c: S<'_>) { c.monitor_on.store(false, Ordering::SeqCst); }

// ------------------------------------------------------------------ managers

#[tauri::command]
async fn startup_list() -> Result<Vec<StartupEntry>, String> { blocking(|| managers::list_startup(&new_reg())).await }

#[tauri::command]
async fn startup_set(hive: String, name: String, enabled: bool) -> Result<Option<String>, String> {
    blocking(move || managers::set_startup_enabled(&new_reg(), &hive, &name, enabled).err()).await
}

#[tauri::command]
async fn processes_top(c: S<'_>) -> Result<Vec<ProcessEntry>, String> {
    let c = c.inner().clone();
    blocking(move || managers::top_processes(c.run.as_ref(), 25)).await
}

#[tauri::command]
async fn services_list(c: S<'_>) -> Result<Vec<ServiceEntry>, String> {
    let c = c.inner().clone();
    blocking(move || managers::services(c.run.as_ref())).await
}

#[tauri::command]
async fn storage_analyze(app: tauri::AppHandle, root: String) -> Result<Vec<FolderUsage>, String> {
    blocking(move || managers::analyze_storage(std::path::Path::new(&root), 20, &|p| { let _ = app.emit("progress", format!("Scanning: {p}")); }, &|| false)).await
}

// ------------------------------------------------------------------ network

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PingRow { name: String, stats: PingStats }

#[tauri::command]
async fn net_ping_all(c: S<'_>, app: tauri::AppHandle) -> Result<Vec<PingRow>, String> {
    let c = c.inner().clone();
    blocking(move || {
        let mut rows = Vec::new();
        for (name, host) in network::PING_TARGETS {
            let _ = app.emit("progress", format!("Ping: {name}…"));
            if let Ok(stats) = network::ping(c.run.as_ref(), host, 8) {
                let _ = app.emit("ping-row", &PingRow { name: (*name).into(), stats: stats.clone() });
                rows.push(PingRow { name: (*name).into(), stats });
            }
        }
        rows
    }).await
}

#[tauri::command]
async fn net_mtu(c: S<'_>) -> Result<Option<u32>, String> {
    let c = c.inner().clone();
    blocking(move || network::find_mtu(c.run.as_ref(), "1.1.1.1")).await
}

#[tauri::command]
async fn net_dns_bench(app: tauri::AppHandle) -> Result<Vec<DnsBenchResult>, String> {
    blocking(move || {
        let mut res: Vec<DnsBenchResult> = network::DNS_PROVIDERS.iter().map(|p| {
            let r = network::bench_dns(*p, 2, &|| false);
            let _ = app.emit("dns-row", &r);
            r
        }).collect();
        res.sort_by(|a, b| a.median_ms.unwrap_or(f64::MAX).partial_cmp(&b.median_ms.unwrap_or(f64::MAX)).unwrap_or(std::cmp::Ordering::Equal));
        res
    }).await
}

#[tauri::command]
async fn net_dns_set(c: S<'_>, servers: Vec<String>) -> Result<TweakResult, String> {
    let c = c.inner().clone();
    blocking(move || { Ok(network::set_dns(c.run.as_ref(), c.adapters.as_ref(), c.reg.as_ref() as &dyn RegistryApi, &c.store, &servers)) }).await?
}

#[tauri::command]
async fn net_dns_restore(c: S<'_>) -> Result<TweakResult, String> {
    let c = c.inner().clone();
    blocking(move || network::restore_dns(c.run.as_ref(), c.adapters.as_ref(), &c.store)).await
}

#[tauri::command]
async fn net_adapters(c: S<'_>) -> Result<Vec<neox_sys::adapters::AdapterInfo>, String> {
    let c = c.inner().clone();
    blocking(move || c.adapters.active()).await
}

// ------------------------------------------------------------------ games

#[tauri::command]
fn games_list(c: S<'_>) -> Vec<GameProfile> { c.store.read(|s| s.games.clone()) }

#[tauri::command]
async fn games_epic() -> Result<Vec<DetectedGame>, String> { blocking(|| games::epic_games(&games::epic_manifests_dir())).await }

#[tauri::command]
fn game_add(c: S<'_>, name: String, exe_path: String) {
    c.store.write(|s| { if !s.games.iter().any(|g| g.exe_path.eq_ignore_ascii_case(&exe_path)) { s.games.push(GameProfile { name, exe_path, profile_id: None }); } });
    let _ = c.store.save();
}

#[tauri::command]
fn game_remove(c: S<'_>, exe_path: String) {
    c.store.write(|s| s.games.retain(|g| !g.exe_path.eq_ignore_ascii_case(&exe_path)));
    let _ = c.store.save();
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BoostView { enabled: bool, status: String }

#[tauri::command]
fn gameboost_get(c: S<'_>) -> BoostView { BoostView { enabled: c.boost_on.load(Ordering::SeqCst), status: c.boost.status() } }

#[tauri::command]
fn gameboost_set(c: S<'_>, enabled: bool) -> Result<BoostView, String> {
    let c = c.inner().clone();
    if enabled { c.need("Game Boost")?; }
    c.store.write(|s| s.settings.game_boost = enabled);
    let _ = c.store.save();
    if enabled { c.start_boost(); } else { c.boost_on.store(false, Ordering::SeqCst); }
    Ok(BoostView { enabled, status: if enabled { "Waiting for a game to start".into() } else { "Off".into() } })
}

#[tauri::command]
async fn fortnite_info(c: S<'_>) -> Result<(bool, Vec<String>, bool), String> {
    let _ = c;
    blocking(|| {
        let ini = games::fortnite_ini_path();
        let exists = ini.is_file();
        let pending = if exists { std::fs::read_to_string(&ini).map(|t| games::fortnite_pending(&t)).unwrap_or_default() } else { vec![] };
        (exists, pending, ini.with_extension("ini.neox.bak").is_file())
    }).await
}

#[tauri::command]
async fn fortnite_apply(c: S<'_>) -> Result<TweakResult, String> {
    let c = c.inner().clone();
    blocking(move || { c.need("Fortnite optimizer")?; Ok(match games::fortnite_apply_file(c.run.as_ref(), &games::fortnite_ini_path()) {
        Ok(m) => TweakResult::ok(m, RestartRequirement::None), Err(e) => TweakResult::fail(e) }) }).await?
}

#[tauri::command]
async fn fortnite_restore(c: S<'_>) -> Result<TweakResult, String> {
    let c = c.inner().clone();
    blocking(move || match games::fortnite_restore_file(c.run.as_ref(), &games::fortnite_ini_path()) {
        Ok(m) => TweakResult::ok(m, RestartRequirement::None), Err(e) => TweakResult::fail(e) }).await
}

// ------------------------------------------------------------------ updates

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateCheck { update: Option<UpdateInfo>, message: String }

#[tauri::command]
async fn update_check(c: S<'_>) -> Result<UpdateCheck, String> {
    let c = c.inner().clone();
    blocking(move || {
        let (u, message) = update::check(c.run.as_ref());
        *c.pending_update.lock().unwrap_or_else(|e| e.into_inner()) = u.clone();
        UpdateCheck { update: u, message }
    }).await
}

#[tauri::command]
async fn update_install(c: S<'_>, app: tauri::AppHandle) -> Result<TweakResult, String> {
    let c = c.inner().clone();
    let handle = app.clone();
    let res = blocking(move || {
        let Some(info) = c.pending_update.lock().unwrap_or_else(|e| e.into_inner()).clone() else { return (TweakResult::fail("No update has been checked."), false) };
        let progress = |p: f64| { let _ = handle.emit("update-progress", p); };
        match update::download(c.run.as_ref(), &info, &progress) {
            Err(e) => (TweakResult::fail(e), false),
            Ok(path) => {
                if update::launch_installer(c.run.as_ref(), &path) { (TweakResult::ok("Installing. NEOX will restart.", RestartRequirement::None), true) }
                else { (TweakResult::fail("Installation cancelled. It will be retried on the next launch, or you can update from Settings."), false) }
            }
        }
    }).await?;
    if res.1 { std::thread::sleep(Duration::from_millis(1500)); app.exit(0); }
    Ok(res.0)
}

// ------------------------------------------------------------------ premium

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LicenseView { hwid: String, status: LicenseStatus, premium: bool, free_ids: Vec<&'static str>, buy_url: &'static str }

fn license_view(c: &Ctx) -> LicenseView {
    let status = c.license();
    LicenseView { hwid: c.hwid().to_string(), premium: status.is_premium(), status, free_ids: license::FREE_IDS.to_vec(), buy_url: license::BUY_URL }
}

#[tauri::command]
async fn license_info(c: S<'_>) -> Result<LicenseView, String> {
    let c = c.inner().clone();
    blocking(move || { c.recheck_activation(); license_view(&c) }).await
}

/// Saves the key only when it is valid for this PC; otherwise reports why without touching the stored key.
#[tauri::command]
async fn license_activate(c: S<'_>, token: String) -> Result<LicenseStatus, String> {
    let c = c.inner().clone();
    blocking(move || {
        let now = neox_core::timefmt::now_unix();
        // Activation code: the server binds it to this PC's hardware id on first use.
        if let Some(code) = neox_core::activation::normalize_code(&token) {
            if neox_sys::activation::server_url().is_none() { return LicenseStatus::Invalid { reason: "Activation codes are not available in this build. Use a license key.".into() }; }
            return match neox_sys::activation::verify(c.run.as_ref(), &code, c.hwid()) {
                neox_core::activation::Reply::Ok { name, exp } => {
                    let a = neox_core::activation::Activation { code, hwid: c.hwid().to_string(), name, exp, verified_at: now };
                    let st = neox_core::activation::status(&a, c.hwid(), now);
                    c.store.write(|s| { s.activation = Some(a); s.premium_ever = true; }); let _ = c.store.save();
                    st
                }
                neox_core::activation::Reply::Denied(r) => LicenseStatus::Invalid { reason: r },
                neox_core::activation::Reply::Unreachable => LicenseStatus::Invalid { reason: "Could not reach the license server. Check your internet connection and try again.".into() },
            };
        }
        let st = license::verify(&token, c.hwid(), now);
        if st.is_premium() { c.store.write(|s| { s.license_key = Some(token.trim().to_string()); s.premium_ever = true; }); let _ = c.store.save(); }
        st
    }).await
}

#[tauri::command]
async fn license_remove(c: S<'_>) -> Result<(), String> {
    let c = c.inner().clone();
    blocking(move || { c.store.write(|s| { s.license_key = None; s.activation = None; }); let _ = c.store.save(); }).await
}


/// Starts the 7-day trial for this PC. The server allows one trial per hardware id.
#[tauri::command]
async fn license_trial(c: S<'_>) -> Result<LicenseStatus, String> {
    let c = c.inner().clone();
    blocking(move || {
        let now = neox_core::timefmt::now_unix();
        if neox_sys::activation::server_url().is_none() { return LicenseStatus::Invalid { reason: "The trial is not available in this build.".into() }; }
        if c.premium() { return LicenseStatus::Invalid { reason: "Premium is already active on this PC.".into() }; }
        match neox_sys::activation::trial(c.run.as_ref(), c.hwid()) {
            neox_core::activation::Reply::Ok { name, exp } => {
                let a = neox_core::activation::Activation { code: neox_core::activation::TRIAL_CODE.into(), hwid: c.hwid().to_string(), name: if name.is_empty() { "Trial".into() } else { name }, exp, verified_at: now };
                let st = neox_core::activation::status(&a, c.hwid(), now);
                c.store.write(|s| { s.activation = Some(a); s.premium_ever = true; }); let _ = c.store.save();
                st
            }
            neox_core::activation::Reply::Denied(r) => LicenseStatus::Invalid { reason: r },
            neox_core::activation::Reply::Unreachable => LicenseStatus::Invalid { reason: "Could not reach the license server. Check your internet connection and try again.".into() },
        }
    }).await
}

/// The user has been told that premium changes were rolled back.
#[tauri::command]
async fn ack_downgrade(c: S<'_>) -> Result<(), String> {
    let c = c.inner().clone();
    blocking(move || { c.store.write(|s| s.downgrade_notice = 0); let _ = c.store.save(); }).await
}

/// Opens the purchase invite. Only the fixed URL is ever opened; the UI cannot pass one.
#[tauri::command]
async fn open_buy(c: S<'_>) -> Result<(), String> {
    let c = c.inner().clone();
    blocking(move || { let _ = c.run.run(&["explorer.exe".to_string(), license::BUY_URL.to_string()], 10); }).await
}

// ------------------------------------------------------------------ internet auto-tune (premium)

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NetPlanView { plan: nettune::NetPlan, adapters: Vec<neox_sys::adapters::AdapterInfo>, elevated: bool }

#[tauri::command]
async fn nettune_plan(c: S<'_>) -> Result<NetPlanView, String> {
    let c = c.inner().clone();
    blocking(move || {
        let adapters = c.adapters.active();
        let eth = adapters.iter().any(|a| a.kind == "ethernet");
        let wifi = adapters.iter().any(|a| a.kind == "wifi");
        // Unknown adapter type: treat as Ethernet-like so the user still gets the common items.
        let plan = nettune::plan(c.engine.catalog(), eth, wifi, c.engine.is_elevated(), &|id| c.engine.is_applied(id));
        NetPlanView { plan, adapters, elevated: c.engine.is_elevated() }
    }).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NetTuneReport { apply: ProfileApplyReport, applied_ids: Vec<String>, deltas: Vec<MetricDelta>, reverted: bool }

fn ping_summary(c: &Ctx, say: &dyn Fn(&str)) -> (f64, f64, f64) {
    let (mut avg, mut jit, mut loss, mut n) = (0.0, 0.0, 0.0, 0.0);
    for (name, host) in network::PING_TARGETS.iter().take(3) {
        say(&format!("Measuring latency: {name}…"));
        if let Ok(s) = network::ping(c.run.as_ref(), host, 10) { if s.received > 0 { avg += s.avg_ms; jit += s.jitter_ms; loss += s.loss_percent; n += 1.0; } }
    }
    if n == 0.0 { (0.0, 0.0, 0.0) } else { (avg / n, jit / n, loss / n) }
}

#[tauri::command]
async fn nettune_run(c: S<'_>, app: tauri::AppHandle, ids: Vec<String>, measure: bool) -> Result<NetTuneReport, String> {
    let c = c.inner().clone();
    blocking(move || {
        c.need("Internet Auto-Tune")?;
        let say = |m: &str| { let _ = app.emit("progress", m); };
        let before = if measure { Some(ping_summary(&c, &say)) } else { None };
        let profile = Profile { id: "nettune".into(), name: "Internet Auto-Tune".into(), description: String::new(), built_in: false, tweak_ids: ids };
        let apply = c.profiles().apply(&profile, &tctx(None), &say);
        let applied_ids: Vec<String> = profile.tweak_ids.iter().filter(|id| c.engine.is_applied(id)).cloned().collect();
        c.store.write(|s| s.last_nettune_ids = applied_ids.clone());
        let _ = c.store.save();
        let mut deltas = vec![];
        if let Some(b) = before {
            say("Waiting for the network to settle…");
            std::thread::sleep(Duration::from_secs(4));
            let a = ping_summary(&c, &say);
            if b.0 > 0.0 && a.0 > 0.0 {
                deltas.push(autotune::make("Average latency", "ms", b.0, a.0, false, autotune::LATENCY_NOISE_PERCENT));
                deltas.push(autotune::make("Jitter", "ms", b.1, a.1, false, autotune::LATENCY_NOISE_PERCENT));
                deltas.push(autotune::make("Packet loss", "%", b.2, a.2, false, autotune::LATENCY_NOISE_PERCENT));
            }
        }
        // Safety net: if the measured latency got clearly worse, undo everything instead of leaving a slower connection behind.
        let worse = deltas.first().is_some_and(|d| d.verdict == autotune::Verdict::Worse);
        let mut applied_ids = applied_ids;
        if worse {
            say("Latency got worse, reverting the changes…");
            for id in &applied_ids { let _ = c.engine.revert(id, &tctx(None)); }
            applied_ids.clear();
            c.store.write(|s| s.last_nettune_ids.clear());
            let _ = c.store.save();
        }
        Ok(NetTuneReport { apply, applied_ids, deltas, reverted: worse })
    }).await?
}

#[tauri::command]
async fn nettune_undo(c: S<'_>) -> Result<usize, String> {
    let c = c.inner().clone();
    blocking(move || {
        let ids: Vec<String> = c.store.read(|s| s.last_nettune_ids.clone());
        let n = ids.iter().filter(|id| c.engine.revert(id, &tctx(None)).success).count();
        c.store.write(|s| s.last_nettune_ids.clear());
        let _ = c.store.save();
        n
    }).await
}

// ------------------------------------------------------------------ restart, Memory Cleaner, Performance Score

/// Restarts Windows after a short delay so the user can still cancel with `shutdown /a`.
#[tauri::command]
async fn restart_windows(c: S<'_>) -> Result<bool, String> {
    let c = c.inner().clone();
    blocking(move || c.run.run(&neox_sys::runner::argv(&["shutdown", "/r", "/t", "10", "/c", "NEOX: restarting to apply your changes. Run shutdown /a to cancel."]), 15).code == 0).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MemoryCleanReport { before_free: u64, after_free: u64, freed: u64, total: u64 }

#[tauri::command]
async fn memory_clean(c: S<'_>) -> Result<MemoryCleanReport, String> {
    let c = c.inner().clone();
    blocking(move || {
        c.need("Memory Cleaner")?;
        if !c.engine.is_elevated() { return Err("Memory Cleaner needs administrator rights. Restart NEOX as administrator.".to_string()); }
        let (total, before, _) = neox_sys::sysinfo::memory();
        neox_sys::memory::clean()?;
        std::thread::sleep(Duration::from_millis(600));
        let (_, after, _) = neox_sys::sysinfo::memory();
        Ok(MemoryCleanReport { before_free: before, after_free: after, freed: after.saturating_sub(before), total })
    }).await?
}

#[tauri::command]
async fn score_list(c: S<'_>) -> Result<Vec<neox_core::state::ScoreEntry>, String> {
    let c = c.inner().clone();
    blocking(move || c.store.read(|s| s.score_history.clone())).await
}

/// Runs the built-in benchmark and stores the result so the user can see the trend after each change.
#[tauri::command]
async fn score_run(c: S<'_>, app: tauri::AppHandle, label: String) -> Result<Vec<neox_core::state::ScoreEntry>, String> {
    let c = c.inner().clone();
    blocking(move || {
        c.need("Performance Score")?;
        let say = |m: &str| { let _ = app.emit("progress", m); };
        let never = || false;
        let Some(b) = neox_sys::bench::run(&say, &never) else { return Err("The measurement was cancelled.".to_string()) };
        let label: String = label.trim().chars().take(40).collect();
        c.store.write(|s| {
            s.score_history.push(neox_core::state::ScoreEntry { time: neox_core::timefmt::now_unix(), label: if label.is_empty() { "Measurement".into() } else { label },
                cpu_ops_per_sec: b.cpu_ops_per_sec, memory_mbps: b.memory_mbps, timer_avg_ms: b.timer_avg_ms, timer_p99_ms: b.timer_p99_ms });
            let n = s.score_history.len(); if n > 50 { s.score_history.drain(0..n - 50); }
        });
        let _ = c.store.save();
        Ok(c.store.read(|s| s.score_history.clone()))
    }).await?
}

#[tauri::command]
async fn score_clear(c: S<'_>) -> Result<(), String> {
    let c = c.inner().clone();
    blocking(move || { c.store.write(|s| s.score_history.clear()); let _ = c.store.save(); }).await
}

// ------------------------------------------------------------------ bootstrap

pub fn run() {
    let run: Arc<dyn Runner> = Arc::new(ProcRunner);
    let reg = Arc::new(new_reg());
    let store = Arc::new(StateStore::at(StateStore::default_path()));
    let _ = store.load();
    let catalog = Arc::new(Catalog::load_embedded().expect("embedded catalog is valid"));
    let elevation: Arc<dyn Elevation> = Arc::new(WinElevation::detect(run.as_ref()));
    let adapters: Arc<dyn Adapters> = Arc::new(PsAdapters { runner: run.clone() });
    let restore = Arc::new(PsRestorePoints { run: run.clone() });
    let reg_dyn: Arc<dyn RegistryApi> = reg.clone();
    let mouse_reg = reg.clone();
    let post: Arc<dyn Fn() + Send + Sync> = Arc::new(move || system::refresh_mouse(mouse_reg.as_ref()));

    let handlers: Vec<Arc<dyn TweakHandler>> = vec![
        Arc::new(RegistryHandler::new(reg_dyn.clone(), adapters.clone(), post)),
        Arc::new(PowerHandler::new(run.clone())),
        Arc::new(ServiceHandler::new(run.clone(), reg_dyn.clone())),
        Arc::new(TaskHandler::new(run.clone())),
        Arc::new(BcdHandler::new(run.clone())),
        Arc::new(NicHandler::new(run.clone())),
        Arc::new(CmdHandler::new(run.clone(), reg_dyn.clone())),
    ];
    let info_run = run.clone();
    let actions: Vec<Arc<dyn ActionHandler>> = vec![Arc::new(Actions {
        run: run.clone(),
        cleaner: Cleaner::new(std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default()),
        restore: restore.clone() as Arc<dyn RestorePoints>,
        system_info: Arc::new(move || sysinfo::scan(info_run.as_ref()).describe(neox_core::timefmt::now_unix())),
    })];
    let engine = Arc::new(Engine::new(catalog, store.clone(), elevation, handlers, actions));
    let boost = Arc::new(GameBoost::new(store.clone(), run.clone()));

    let ctx = Arc::new(Ctx {
        engine, store: store.clone(), run, reg, adapters, restore, snapshot: Mutex::new(None), boost, boost_on: AtomicBool::new(false),
        monitor_on: AtomicBool::new(false), pending_update: Mutex::new(None), hwid: std::sync::OnceLock::new(),
    });

    // Crash recovery: a power plan left behind by an interrupted Game Boost is restored right away.
    if store.read(|s| s.settings.game_boost) && ctx.premium() { ctx.start_boost(); } else { ctx.boost.restore(); }

    tauri::Builder::default()
        .manage(ctx)
        .setup(|app| {
            if let Some(w) = app.get_webview_window("main") { let _ = w.set_title("NEOX"); }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info, settings_set, restart_as_admin, pick_file,
            catalog_list, statuses, applied_ids, tweak_apply, tweak_revert, action_run, clear_pending_restart,
            profiles_list, profile_apply, profile_save, profile_delete, profile_export, profile_import, revert_all,
            history_list, baseline_capture, baselines_list, baseline_compare,
            autotune_plan, autotune_run, autotune_undo, health_scan, system_snapshot,
            monitor_start, monitor_stop,
            startup_list, startup_set, processes_top, services_list, storage_analyze,
            net_ping_all, net_mtu, net_dns_bench, net_dns_set, net_dns_restore, net_adapters,
            games_list, games_epic, game_add, game_remove, gameboost_get, gameboost_set, fortnite_info, fortnite_apply, fortnite_restore,
            update_check, update_install,
            license_info, license_activate, license_remove, license_trial, ack_downgrade, open_buy, nettune_plan, nettune_run, nettune_undo,
            restart_windows, memory_clean, score_list, score_run, score_clear,
        ])
        .run(tauri::generate_context!())
        .expect("error while running NEOX");
}
