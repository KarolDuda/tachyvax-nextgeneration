//! Game discovery (Epic manifests), Fortnite config editing and Game Boost.

use crate::managers::processes;
use crate::runner::{argv, powershell, Runner};
use neox_core::state::StateStore;
use serde::Serialize;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DetectedGame { pub name: String, pub exe_path: String }

/// Reads Epic Games launcher manifests; never guesses drive letters.
pub fn epic_games(manifests_dir: &Path) -> Vec<DetectedGame> {
    let mut list = Vec::new();
    let Ok(rd) = std::fs::read_dir(manifests_dir) else { return list };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("item") { continue; }
        let Ok(text) = std::fs::read_to_string(&p) else { continue };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        let install = v["InstallLocation"].as_str().unwrap_or("");
        let launch = v["LaunchExecutable"].as_str().unwrap_or("");
        let name = v["DisplayName"].as_str().unwrap_or("").trim();
        if install.is_empty() || launch.is_empty() { continue; }
        let exe = Path::new(install).join(launch.replace('\\', "/"));
        if exe.extension().and_then(|x| x.to_str()).is_some_and(|x| x.eq_ignore_ascii_case("exe")) && exe.is_file() {
            let n = if name.is_empty() { exe.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default() } else { name.to_string() };
            list.push(DetectedGame { name: n, exe_path: exe.to_string_lossy().into() });
        }
    }
    list.sort_by(|a, b| a.name.cmp(&b.name));
    list
}

pub fn epic_manifests_dir() -> std::path::PathBuf {
    let base = std::env::var_os("ProgramData").map(std::path::PathBuf::from).unwrap_or_else(|| "C:\\ProgramData".into());
    base.join("Epic").join("EpicGamesLauncher").join("Data").join("Manifests")
}

/// Lowercase file name without extension; accepts both `\\` and `/` separators regardless of the host OS.
pub fn exe_stem(path: &str) -> String {
    let p = path.trim().trim_matches('"');
    let file = p.rsplit(['\\', '/']).next().unwrap_or("");
    let stem = match file.rfind('.') { Some(i) if i > 0 => &file[..i], _ => file };
    stem.to_lowercase()
}

pub fn is_running(run: &dyn Runner, exe_path_or_name: &str) -> bool {
    let stem = exe_stem(exe_path_or_name);
    !stem.is_empty() && processes(run).iter().any(|p| p.name.to_lowercase() == stem)
}

// ---------------------------------------------------------------- Fortnite

const RULES: &[(&str, &str, &str)] = &[("bUseVSync", "True", "False"), ("bUseHDRDisplayOutput", "True", "False")];
pub const FORTNITE_PROCESS: &str = "FortniteClient-Win64-Shipping";

pub fn fortnite_ini_path() -> std::path::PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from).unwrap_or_default();
    base.join("FortniteGame").join("Saved").join("Config").join("WindowsClient").join("GameUserSettings.ini")
}

pub fn fortnite_pending(text: &str) -> Vec<String> {
    RULES.iter().filter(|(k, bad, _)| text.lines().any(|l| l.trim_end() == format!("{k}={bad}"))).map(|(k, b, g)| format!("{k}: {b} → {g}")).collect()
}

/// Returns the new text and the number of changed settings; line endings are preserved.
pub fn fortnite_apply(text: &str) -> (String, usize) {
    let mut changed = 0;
    let out: String = text.split_inclusive('\n').map(|line| {
        let body = line.trim_end_matches(['\r', '\n']);
        let eol = &line[body.len()..];
        for (k, bad, good) in RULES {
            if body.trim_end() == format!("{k}={bad}") { changed += 1; return format!("{k}={good}{eol}"); }
        }
        line.to_string()
    }).collect();
    (out, changed)
}

pub fn fortnite_apply_file(run: &dyn Runner, ini: &Path) -> Result<String, String> {
    if !ini.is_file() { return Err("Fortnite settings file not found.".into()); }
    if is_running(run, FORTNITE_PROCESS) { return Err("Close Fortnite before changing settings (the game overwrites the file on exit).".into()); }
    let text = std::fs::read_to_string(ini).map_err(|e| e.to_string())?;
    let (next, n) = fortnite_apply(&text);
    if n == 0 { return Ok("Nothing to change: V-Sync and HDR are already off.".into()); }
    let bak = ini.with_extension("ini.neox.bak");
    std::fs::copy(ini, &bak).map_err(|e| e.to_string())?;
    std::fs::write(ini, next).map_err(|e| e.to_string())?;
    Ok(format!("Changed {n} settings. Backup: {}", bak.to_string_lossy()))
}

pub fn fortnite_restore_file(run: &dyn Runner, ini: &Path) -> Result<String, String> {
    let bak = ini.with_extension("ini.neox.bak");
    if !bak.is_file() { return Err("No backup available.".into()); }
    if is_running(run, FORTNITE_PROCESS) { return Err("Close Fortnite before restoring settings.".into()); }
    std::fs::copy(&bak, ini).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&bak);
    Ok("Settings restored to their state before the change.".into())
}

// ---------------------------------------------------------------- Game Boost

const HIGH_PERFORMANCE: &str = "8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c";

/// While a game from your list runs: high-performance power plan + higher process priority; afterwards the previous
/// plan is restored. The previous plan is stored in `state.json`, so it is recovered even after a crash.
pub struct GameBoost { store: Arc<StateStore>, run: Arc<dyn Runner>, boosted_pid: Mutex<u32>, status: Mutex<String> }

impl GameBoost {
    pub fn new(store: Arc<StateStore>, run: Arc<dyn Runner>) -> Self {
        Self { store, run, boosted_pid: Mutex::new(0), status: Mutex::new("Off".into()) }
    }
    pub fn status(&self) -> String { self.status.lock().unwrap_or_else(|e| e.into_inner()).clone() }
    fn set_status(&self, s: &str) { *self.status.lock().unwrap_or_else(|e| e.into_inner()) = s.into(); }
    pub fn mark_waiting(&self) { self.set_status("Waiting for a game to start"); }
    pub fn mark_off(&self) { self.set_status("Off"); }

    fn watched(&self) -> Vec<String> {
        let mut v: Vec<String> = self.store.read(|s| s.games.iter().map(|g| g.exe_path.clone()).chain(s.settings.game_exe_path.clone()).collect());
        v = v.into_iter().map(|p| exe_stem(&p)).filter(|s| !s.is_empty()).collect();
        v.sort();
        v.dedup();
        v
    }

    /// One polling cycle (call every few seconds while Game Boost is enabled).
    pub fn tick(&self) {
        let watched = self.watched();
        let game = processes(self.run.as_ref()).into_iter().find(|p| watched.contains(&p.name.to_lowercase()));
        let boosted = *self.boosted_pid.lock().unwrap_or_else(|e| e.into_inner());
        match game {
            Some(g) => if boosted != g.pid { self.boost(g.pid, &g.name); },
            None => if boosted != 0 || self.store.read(|s| s.settings.game_boost_restore_scheme.is_some()) {
                self.restore();
                self.set_status("Waiting for a game to start");
            },
        }
    }

    fn active_scheme(&self) -> Option<String> {
        let r = self.run.run(&argv(&["powercfg", "/getactivescheme"]), 30);
        if r.success() { crate::parse::find_guid(&r.output) } else { None }
    }

    fn boost(&self, pid: u32, name: &str) {
        let active = self.active_scheme();
        let none_saved = self.store.read(|s| s.settings.game_boost_restore_scheme.is_none());
        if let Some(a) = active.filter(|a| a != HIGH_PERFORMANCE && none_saved) {
            self.store.write(|s| s.settings.game_boost_restore_scheme = Some(a));
            let _ = self.store.save(); // persisted BEFORE switching, so a crash cannot lose the original plan
            if !self.run.run(&argv(&["powercfg", "/setactive", HIGH_PERFORMANCE]), 30).success() {
                self.store.write(|s| s.settings.game_boost_restore_scheme = None);
                let _ = self.store.save();
            }
        }
        self.run.run(&powershell(&format!("(Get-Process -Id {pid} -ErrorAction Stop).PriorityClass = 'High'")), 30);
        *self.boosted_pid.lock().unwrap_or_else(|e| e.into_inner()) = pid;
        self.set_status(&format!("Active: {name}"));
    }

    pub fn restore(&self) {
        *self.boosted_pid.lock().unwrap_or_else(|e| e.into_inner()) = 0;
        let Some(prev) = self.store.read(|s| s.settings.game_boost_restore_scheme.clone()) else { return };
        self.run.run(&argv(&["powercfg", "/setactive", &prev]), 30);
        self.store.write(|s| s.settings.game_boost_restore_scheme = None);
        let _ = self.store.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::fake::Script;
    use crate::runner::RunResult;

    const BALANCED: &str = "Power Scheme GUID: 381b4222-f694-41f0-9685-ff5bb260df2e  (Balanced)";

    fn boost(running: &str) -> (GameBoost, Arc<Script>, Arc<StateStore>) {
        let run = Arc::new(Script::new());
        run.on("powercfg /getactivescheme", RunResult::ok(BALANCED));
        run.on("tasklist", RunResult::ok(running));
        let store = Arc::new(StateStore::in_memory());
        store.write(|s| s.settings.game_exe_path = Some("C:\\Games\\Game.exe".into()));
        (GameBoost::new(store.clone(), run.clone()), run, store)
    }

    #[test]
    fn boosts_when_the_game_starts_and_restores_when_it_exits() {
        let (g, run, store) = boost("\"Game.exe\",\"4242\",\"Console\",\"1\",\"500 000 K\"");
        g.tick();
        assert_eq!(g.status(), "Active: Game");
        assert_eq!(store.read(|s| s.settings.game_boost_restore_scheme.clone()).as_deref(), Some("381b4222-f694-41f0-9685-ff5bb260df2e"));
        assert!(run.calls().iter().any(|c| c == &format!("powercfg /setactive {HIGH_PERFORMANCE}")));
        assert!(run.calls().iter().any(|c| c.contains("Get-Process -Id 4242")));

        run.rules.lock().unwrap().retain(|(p, _)| p != "tasklist");
        run.on("tasklist", RunResult::ok(""));
        g.tick();
        assert_eq!(g.status(), "Waiting for a game to start");
        assert!(store.read(|s| s.settings.game_boost_restore_scheme.is_none()));
        assert!(run.calls().iter().any(|c| c == "powercfg /setactive 381b4222-f694-41f0-9685-ff5bb260df2e"));
    }

    #[test]
    fn restores_a_plan_left_behind_by_a_crash() {
        let (g, run, store) = boost("");
        store.write(|s| s.settings.game_boost_restore_scheme = Some("11111111-2222-3333-4444-555555555555".into()));
        g.tick();
        assert!(run.calls().iter().any(|c| c == "powercfg /setactive 11111111-2222-3333-4444-555555555555"));
        assert!(store.read(|s| s.settings.game_boost_restore_scheme.is_none()));
    }

    #[test]
    fn does_nothing_for_unlisted_processes() {
        let (g, run, _) = boost("\"notepad.exe\",\"1\",\"Console\",\"1\",\"10 K\"");
        g.tick();
        assert!(!run.calls().iter().any(|c| c.contains("/setactive")));
    }

    #[test]
    fn fortnite_edit_changes_only_the_two_settings_and_keeps_line_endings() {
        let text = "[Sec]\r\nbUseVSync=True\r\nbUseHDRDisplayOutput=True \r\nOther=True\r\n";
        assert_eq!(fortnite_pending(text).len(), 2);
        let (out, n) = fortnite_apply(text);
        assert_eq!(n, 2);
        assert_eq!(out, "[Sec]\r\nbUseVSync=False\r\nbUseHDRDisplayOutput=False\r\nOther=True\r\n");
        assert_eq!(fortnite_apply(&out).1, 0);
    }

    #[test]
    fn fortnite_apply_makes_a_backup_and_restore_brings_it_back() {
        let dir = std::env::temp_dir().join(format!("neox-fn-{}", neox_core::timefmt::now_unix()));
        std::fs::create_dir_all(&dir).unwrap();
        let ini = dir.join("GameUserSettings.ini");
        std::fs::write(&ini, "bUseVSync=True\n").unwrap();
        let run = Script::new();
        run.on("tasklist", RunResult::ok(""));
        assert!(fortnite_apply_file(&run, &ini).unwrap().starts_with("Changed 1"));
        assert_eq!(std::fs::read_to_string(&ini).unwrap(), "bUseVSync=False\n");
        fortnite_restore_file(&run, &ini).unwrap();
        assert_eq!(std::fs::read_to_string(&ini).unwrap(), "bUseVSync=True\n");
        assert!(fortnite_restore_file(&run, &ini).is_err());
    }

    #[test]
    fn fortnite_is_not_touched_while_the_game_runs() {
        let dir = std::env::temp_dir().join(format!("neox-fn2-{}", neox_core::timefmt::now_unix()));
        std::fs::create_dir_all(&dir).unwrap();
        let ini = dir.join("GameUserSettings.ini");
        std::fs::write(&ini, "bUseVSync=True\n").unwrap();
        let run = Script::new();
        run.on("tasklist", RunResult::ok("\"FortniteClient-Win64-Shipping.exe\",\"9\",\"Console\",\"1\",\"1 K\""));
        assert!(fortnite_apply_file(&run, &ini).unwrap_err().contains("Close Fortnite"));
    }

    #[test]
    fn epic_manifests_yield_only_existing_exes() {
        let dir = std::env::temp_dir().join(format!("neox-epic-{}", neox_core::timefmt::now_unix()));
        let game = dir.join("game");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("g.exe"), b"x").unwrap();
        std::fs::write(dir.join("a.item"), format!(r#"{{"InstallLocation":"{}","LaunchExecutable":"g.exe","DisplayName":"My Game"}}"#, game.to_string_lossy().replace('\\', "/"))).unwrap();
        std::fs::write(dir.join("b.item"), r#"{"InstallLocation":"/nope","LaunchExecutable":"x.exe","DisplayName":"Gone"}"#).unwrap();
        std::fs::write(dir.join("c.item"), "garbage").unwrap();
        let g = epic_games(&dir);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].name, "My Game");
    }
}
