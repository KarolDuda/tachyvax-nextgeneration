//! Persistent application state (`%LOCALAPPDATA%\Neox\state.json`), written atomically with a `.bak` copy.

use crate::models::TweakStatus;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HistoryAction { Apply, Revert, Run, ProfileApply }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ScoreEntry { pub time: i64, pub label: String, pub cpu_ops_per_sec: f64, pub memory_mbps: f64, pub timer_avg_ms: f64, pub timer_p99_ms: f64 }
impl Default for ScoreEntry { fn default() -> Self { ScoreEntry { time: 0, label: String::new(), cpu_ops_per_sec: 0.0, memory_mbps: 0.0, timer_avg_ms: 0.0, timer_p99_ms: 0.0 } } }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HistoryEntry {
    pub id: String,
    pub time: String,
    pub tweak_id: String,
    pub title: String,
    pub action: HistoryAction,
    pub success: bool,
    pub message: String,
}
impl Default for HistoryEntry {
    fn default() -> Self {
        Self { id: String::new(), time: String::new(), tweak_id: String::new(), title: String::new(),
               action: HistoryAction::Apply, success: true, message: String::new() }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub description: String,
    pub built_in: bool,
    pub tweak_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GameProfile {
    pub name: String,
    pub exe_path: String,
    pub profile_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Baseline {
    pub name: String,
    pub time: String,
    pub statuses: HashMap<String, TweakStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    pub create_restore_point_before_profiles: bool,
    pub auto_update: bool,
    pub game_exe_path: Option<String>,
    pub first_run_done: bool,
    pub game_boost: bool,
    /// Power scheme GUID to restore after a game (persisted so it survives a crash).
    pub game_boost_restore_scheme: Option<String>,
    pub last_auto_tune: Option<String>,
}
impl Default for AppSettings {
    fn default() -> Self {
        Self { create_restore_point_before_profiles: true, auto_update: true, game_exe_path: None, first_run_done: false,
               game_boost: false, game_boost_restore_scheme: None, last_auto_tune: None }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppState {
    pub schema_version: i32,
    /// Undo snapshots keyed by tweak id.
    pub snapshots: HashMap<String, Value>,
    pub history: Vec<HistoryEntry>,
    pub custom_profiles: Vec<Profile>,
    pub games: Vec<GameProfile>,
    pub baselines: Vec<Baseline>,
    pub pending_restart: Vec<String>,
    /// Unix seconds when the oldest pending item was added; cleared items are dropped after a real reboot.
    pub pending_restart_at: i64,
    /// Performance Score runs (newest last).
    pub score_history: Vec<ScoreEntry>,
    /// DNS backups: adapter GUID -> manual servers (empty list = DHCP).
    pub dns_backups: HashMap<String, Vec<String>>,
    /// Ids applied by the last Auto-Tune run (so it can be undone in one click).
    pub last_autotune_ids: Vec<String>,
    /// Premium license key (`NEOX1.…`), checked against this PC on every start.
    pub license_key: Option<String>,
    /// Activation bound to this PC by the license server.
    pub activation: Option<crate::activation::Activation>,
    /// True once a license, code or trial was activated on this PC. Only then are premium changes rolled back at the end.
    pub premium_ever: bool,
    /// How many premium changes were rolled back and not yet shown to the user.
    pub downgrade_notice: usize,
    /// Ids applied by the last Internet Auto-Tune run.
    pub last_nettune_ids: Vec<String>,
    pub settings: AppSettings,
}
impl Default for AppState {
    fn default() -> Self {
        Self { schema_version: 1, snapshots: HashMap::new(), history: Vec::new(), custom_profiles: Vec::new(), games: Vec::new(),
               baselines: Vec::new(), pending_restart: Vec::new(), pending_restart_at: 0, score_history: Vec::new(), dns_backups: HashMap::new(), last_autotune_ids: Vec::new(), license_key: None, activation: None, premium_ever: false, downgrade_notice: 0, last_nettune_ids: Vec::new(), settings: AppSettings::default() }
    }
}

pub struct StateStore {
    path: Option<PathBuf>,
    state: Mutex<AppState>,
    saves: AtomicUsize,
}

impl StateStore {
    pub fn in_memory() -> Self { Self { path: None, state: Mutex::new(AppState::default()), saves: AtomicUsize::new(0) } }
    pub fn at(path: PathBuf) -> Self { Self { path: Some(path), state: Mutex::new(AppState::default()), saves: AtomicUsize::new(0) } }

    pub fn default_path() -> PathBuf {
        let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        base.join("Neox").join("state.json")
    }

    fn lock(&self) -> MutexGuard<'_, AppState> { self.state.lock().unwrap_or_else(|e| e.into_inner()) }

    pub fn read<R>(&self, f: impl FnOnce(&AppState) -> R) -> R { f(&self.lock()) }
    pub fn write<R>(&self, f: impl FnOnce(&mut AppState) -> R) -> R { f(&mut self.lock()) }
    pub fn saves(&self) -> usize { self.saves.load(Ordering::SeqCst) }

    pub fn load(&self) -> Result<(), String> {
        let Some(path) = &self.path else { return Ok(()) };
        if !path.exists() { *self.lock() = AppState::default(); return Ok(()); }
        match std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<AppState>(&t).map_err(|e| e.to_string())) {
            Ok(s) => { *self.lock() = s; Ok(()) }
            Err(_) => {
                // Corrupted file: keep it aside for analysis and fall back to the previous copy.
                let broken = path.with_extension(format!("json.broken-{}", crate::timefmt::now_unix()));
                let _ = std::fs::rename(path, broken);
                let bak = path.with_extension("json.bak");
                let restored = std::fs::read_to_string(&bak).ok().and_then(|t| serde_json::from_str::<AppState>(&t).ok());
                *self.lock() = restored.unwrap_or_default();
                Ok(())
            }
        }
    }

    pub fn save(&self) -> Result<(), String> {
        self.saves.fetch_add(1, Ordering::SeqCst);
        let Some(path) = &self.path else { return Ok(()) };
        if let Some(dir) = path.parent() { std::fs::create_dir_all(dir).map_err(|e| e.to_string())?; }
        let text = serde_json::to_string_pretty(&*self.lock()).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        if path.exists() { let _ = std::fs::copy(path, path.with_extension("json.bak")); }
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("neox-test-{}-{}", name, crate::timefmt::now_unix()));
        let _ = std::fs::remove_dir_all(&d);
        d.join("state.json")
    }

    #[test]
    fn round_trips_and_keeps_backup() {
        let p = temp("rt");
        let s = StateStore::at(p.clone());
        s.write(|st| { st.snapshots.insert("a".into(), serde_json::json!({"x": 1})); });
        s.save().unwrap();
        s.write(|st| st.pending_restart.push("a".into()));
        s.save().unwrap();
        assert!(p.with_extension("json.bak").exists());
        let s2 = StateStore::at(p);
        s2.load().unwrap();
        assert!(s2.read(|st| st.snapshots.contains_key("a")));
        assert_eq!(s2.read(|st| st.pending_restart.clone()), vec!["a".to_string()]);
    }

    #[test]
    fn corrupted_file_falls_back_to_backup() {
        let p = temp("bad");
        let s = StateStore::at(p.clone());
        s.write(|st| { st.snapshots.insert("keep".into(), Value::Null); });
        s.save().unwrap();
        s.save().unwrap(); // second save creates the .bak
        std::fs::write(&p, "{ not json").unwrap();
        let s2 = StateStore::at(p);
        s2.load().unwrap();
        assert!(s2.read(|st| st.snapshots.contains_key("keep")));
    }

    #[test]
    fn reads_state_written_by_earlier_versions() {
        let json = r#"{"schemaVersion":1,"snapshots":{"game_mode":{"k":1}},"history":[{"id":"1","time":"2026-11-08T10:00:00+01:00","tweakId":"game_mode","title":"t","action":"apply","success":true,"message":"Applied"}],"settings":{"autoUpdate":false}}"#;
        let st: AppState = serde_json::from_str(json).unwrap();
        assert!(st.snapshots.contains_key("game_mode"));
        assert_eq!(st.history[0].action, HistoryAction::Apply);
        assert!(!st.settings.auto_update);
        assert!(st.settings.create_restore_point_before_profiles);
    }
}
