//! One-shot actions: cleaning folders, opening tools, diagnostic commands and native helpers.

use crate::cleaner::Cleaner;
use crate::runner::{argv, powershell, Runner};
use neox_core::engine::{ActionHandler, RestorePoints};
use neox_core::models::*;
use std::sync::Arc;

pub struct Actions {
    pub run: Arc<dyn Runner>,
    pub cleaner: Cleaner,
    pub restore: Arc<dyn RestorePoints>,
    pub system_info: Arc<dyn Fn() -> String + Send + Sync>,
}

impl Actions {
    fn open(&self, target: &str) -> Result<(), String> {
        let r = self.run.run(&argv(&["cmd", "/C", "start", "", target]), 15);
        if r.success() { Ok(()) } else { Err(format!("Could not open: {}", r.first_line())) }
    }

    fn native(&self, name: &str) -> Result<String, String> {
        match name {
            "RestorePoint" => {
                let r = self.restore.create("NEOX");
                if r.success { Ok(r.message) } else { Err("Failed: System Protection may be off (or the limit of 1 restore point per 24 h applies).".into()) }
            }
            "CleanTemp" => Ok(self.cleaner.clean(&["%TEMP%".to_string(), "%SystemRoot%\\Temp".to_string()])),
            "EmptyRecycleBin" => {
                self.run.run(&powershell("Clear-RecycleBin -Force -ErrorAction SilentlyContinue"), 60);
                Ok("Recycle Bin emptied (or it was already empty).".into())
            }
            "EnergyReport" => {
                let path = std::env::temp_dir().join("neox_energy.html");
                let _ = std::fs::remove_file(&path);
                let p = path.to_string_lossy().to_string();
                self.run.run(&argv(&["powercfg", "/energy", "/output", &p, "/duration", "10"]), 120);
                if !path.exists() { return Err("Could not generate the report.".into()); }
                self.open(&p)?;
                Ok(format!("Report saved: {p}"))
            }
            "RestartExplorer" => {
                self.run.run(&argv(&["taskkill", "/f", "/im", "explorer.exe"]), 15);
                self.open("explorer.exe")?;
                Ok("Explorer restarted.".into())
            }
            "StoreReset" => { self.open("wsreset.exe")?; Ok("Microsoft Store cache reset started (the Store window will open).".into()) }
            "RenewIp" => {
                let rel = self.run.run(&argv(&["ipconfig", "/release"]), 60);
                if !rel.success() { return Err(rel.first_line()); }
                let ren = self.run.run(&argv(&["ipconfig", "/renew"]), 90);
                if !ren.success() { return Err(ren.first_line()); }
                Ok("IP address renewed.".into())
            }
            "SystemInfo" => Ok((self.system_info)()),
            other => Err(format!("Unknown native action: {other}")),
        }
    }
}

impl ActionHandler for Actions {
    fn can_handle(&self, t: &TweakDefinition) -> bool { t.is_action() }

    fn run(&self, t: &TweakDefinition, _: &TweakContext) -> Result<String, String> {
        match t.action {
            Some(ActionKind::Clean) => Ok(self.cleaner.clean(t.patterns.as_deref().unwrap_or(&[]))),
            Some(ActionKind::Open) => { self.open(t.target.as_deref().unwrap_or(""))?; Ok("Opened.".into()) }
            Some(ActionKind::Info) => {
                let r = self.run.run(&powershell(t.target.as_deref().unwrap_or("")), 60);
                let text = r.output.trim().to_string();
                if !r.success() && text.is_empty() { return Err("The command failed.".into()); }
                Ok(if text.is_empty() { "No data.".into() } else { text.chars().take(4000).collect() })
            }
            Some(ActionKind::Command) => {
                let timeout = u64::from(t.timeout_seconds.unwrap_or(60));
                for a in t.commands.iter().flatten() {
                    let r = self.run.run(a, timeout);
                    if !r.success() { return Err(if r.first_line().is_empty() { "The command finished with an error.".into() } else { r.first_line() }); }
                }
                Ok(t.success_message.clone().unwrap_or_else(|| "Done.".into()))
            }
            Some(ActionKind::Native) => self.native(t.native.as_deref().unwrap_or("")),
            None => Err("Unknown action kind.".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::fake::Script;
    use crate::runner::RunResult;
    use neox_core::catalog::Catalog;
    use std::path::PathBuf;

    struct Rp(bool);
    impl RestorePoints for Rp { fn create(&self, _: &str) -> TweakResult { if self.0 { TweakResult::ok("created", RestartRequirement::None) } else { TweakResult::fail("off") } } }

    fn actions(run: Arc<Script>, rp: bool) -> Actions {
        Actions { run, cleaner: Cleaner::new(PathBuf::new()), restore: Arc::new(Rp(rp)), system_info: Arc::new(|| "info".into()) }
    }

    #[test]
    fn every_native_action_in_the_catalog_is_implemented() {
        let c = Catalog::load_embedded().unwrap();
        let a = actions(Arc::new(Script::new()), true);
        for t in c.all().iter().filter(|t| t.action == Some(ActionKind::Native)) {
            let name = t.native.as_deref().unwrap();
            let r = a.native(name);
            assert!(!matches!(&r, Err(e) if e.starts_with("Unknown native")), "{name}");
        }
    }

    #[test]
    fn command_actions_stop_at_the_first_failure() {
        let c = Catalog::load_embedded().unwrap();
        let t = c.all().iter().find(|t| t.action == Some(ActionKind::Command) && t.commands.as_ref().unwrap().len() > 1).cloned();
        let Some(mut t) = t else { return };
        t.commands = Some(vec![argv(&["a"]), argv(&["bad"]), argv(&["c"])]);
        let run = Arc::new(Script::new());
        run.on("bad", RunResult::err(1, "boom"));
        let a = actions(run.clone(), true);
        assert_eq!(a.run(&t, &TweakContext::default()).unwrap_err(), "boom");
        assert_eq!(run.calls(), vec!["a".to_string(), "bad".to_string()]);
    }

    #[test]
    fn restore_point_failure_is_reported() {
        let a = actions(Arc::new(Script::new()), false);
        assert!(a.native("RestorePoint").unwrap_err().contains("System Protection"));
    }

    #[test]
    fn info_actions_truncate_long_output() {
        let c = Catalog::load_embedded().unwrap();
        let mut t = c.all().iter().find(|t| t.action == Some(ActionKind::Info)).unwrap().clone();
        t.target = Some("Get-Thing".into());
        let run = Arc::new(Script::new());
        run.on("powershell", RunResult::ok(&"x".repeat(9000)));
        assert_eq!(actions(run, true).run(&t, &TweakContext::default()).unwrap().chars().count(), 4000);
    }
}
