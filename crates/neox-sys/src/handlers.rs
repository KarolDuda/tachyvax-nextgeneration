//! Reversible-change handlers. Each one returns an undo snapshot from `apply` and restores it in `revert`.

use crate::adapters::Adapters;
use crate::parse::{bcd_value, find_guid, hex_values, task_enabled};
use crate::registry::{restore, snapshot, RegData, RegistryApi};
use crate::runner::{argv, powershell, Runner};
use neox_core::engine::TweakHandler;
use neox_core::models::*;
use serde_json::{json, Value};
use std::sync::Arc;

fn dword(d: &RegData) -> i64 { match d { RegData::Dword(v) => i64::from(*v), RegData::Qword(v) => *v as i64, _ => -1 } }
fn q(s: &str) -> String { s.replace('\'', "''") }

/// Parses a hex string such as "0A0b" into bytes; anything that is not a hex digit (spaces, commas) is ignored.
pub(crate) fn hex_bytes(s: &str) -> Vec<u8> {
    let digits: Vec<u8> = s.bytes().filter(u8::is_ascii_hexdigit).collect();
    digits.chunks_exact(2).filter_map(|p| u8::from_str_radix(std::str::from_utf8(p).ok()?, 16).ok()).collect()
}

// ---------------------------------------------------------------- registry

pub struct RegistryHandler { reg: Arc<dyn RegistryApi>, adapters: Arc<dyn Adapters>, post: Arc<dyn Fn() + Send + Sync> }

impl RegistryHandler {
    pub fn new(reg: Arc<dyn RegistryApi>, adapters: Arc<dyn Adapters>, post: Arc<dyn Fn() + Send + Sync>) -> Self { Self { reg, adapters, post } }

    fn resolve(&self, t: &TweakDefinition, c: &TweakContext) -> Result<Vec<RegValueSpec>, String> {
        match t.kind {
            TweakKind::Reg => Ok(t.values.clone().unwrap_or_default()),
            TweakKind::RegPerAdapter => {
                let adapters = self.adapters.active();
                if adapters.is_empty() { return Err("No active network adapter found.".into()); }
                let spec = t.per_adapter.as_ref().ok_or("Missing per-adapter data")?;
                Ok(adapters.iter().flat_map(|a| spec.values.iter().map(move |v| RegValueSpec {
                    hive: "HKLM".into(), path: spec.path_template.replace("{guid}", &format!("{{{}}}", a.guid)),
                    name: v.name.clone(), ty: v.ty.clone(), value: v.value.clone() })).collect())
            }
            TweakKind::RegPerGame => {
                let exe = c.game_exe_path.as_deref().map(|s| s.trim().trim_matches('"').to_string()).unwrap_or_default();
                if exe.is_empty() || !std::path::Path::new(&exe).exists() { return Err("Choose a valid path to the game's .exe file.".into()); }
                Ok(t.per_game.clone().unwrap_or_default().into_iter().map(|v| RegValueSpec { name: Some(exe.clone()), ..v }).collect())
            }
            _ => Err("Unsupported kind".into()),
        }
    }

    fn wanted(s: &RegValueSpec) -> RegData {
        match s.ty.as_str() {
            "dword" => RegData::Dword(s.as_i64() as u32),
            "binary" => RegData::Binary(hex_bytes(&s.as_string())),
            _ => RegData::Str(s.as_string()),
        }
    }
}

impl TweakHandler for RegistryHandler {
    fn kinds(&self) -> &'static [TweakKind] { &[TweakKind::Reg, TweakKind::RegPerAdapter, TweakKind::RegPerGame] }

    fn status(&self, t: &TweakDefinition, c: &TweakContext) -> Result<TweakStatus, String> {
        let Ok(specs) = self.resolve(t, c) else { return Ok(TweakStatus::Unknown) };
        for s in &specs {
            match self.reg.read(&s.hive, &s.path, s.name.as_deref())? {
                None => return Ok(TweakStatus::NotApplied),
                Some(cur) => {
                    let same = match s.ty.as_str() {
                        "dword" => dword(&cur) == i64::from(s.as_i64() as u32),
                        "binary" => matches!(&cur, RegData::Binary(x) if *x == hex_bytes(&s.as_string())),
                        _ => matches!(&cur, RegData::Str(x) | RegData::ExpandStr(x) if *x == s.as_string()),
                    };
                    if !same { return Ok(TweakStatus::NotApplied); }
                }
            }
        }
        Ok(TweakStatus::Applied)
    }

    fn apply(&self, t: &TweakDefinition, c: &TweakContext) -> Result<Value, String> {
        let specs = self.resolve(t, c)?;
        let mut snaps = Vec::new();
        for s in &specs { snaps.push(snapshot(self.reg.as_ref(), &s.hive, &s.path, s.name.as_deref())?); }
        for (i, s) in specs.iter().enumerate() {
            if let Err(e) = self.reg.write(&s.hive, &s.path, s.name.as_deref(), &Self::wanted(s)) {
                // Roll back what was already written so we never leave a half-applied change.
                for sn in &snaps[..i] { let _ = restore(self.reg.as_ref(), sn); }
                return Err(format!("Registry access denied (policy or missing permissions): {e}"));
            }
        }
        if t.post.as_deref() == Some("refreshMouse") { (self.post)(); }
        Ok(Value::Array(snaps))
    }

    fn revert(&self, t: &TweakDefinition, snap: &Value, _: &TweakContext) -> Result<(), String> {
        for s in snap.as_array().ok_or("Corrupt snapshot")? { restore(self.reg.as_ref(), s)?; }
        if t.post.as_deref() == Some("refreshMouse") { (self.post)(); }
        Ok(())
    }
}

// ---------------------------------------------------------------- power

pub const HIGH_PERFORMANCE: &str = "8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c";
pub const ULTIMATE: &str = "e9a42b02-d5df-448d-aa00-03f14749eb61";
pub const BALANCED: &str = "381b4222-f694-41f0-9685-ff5bb260df2e";

/// Settings of the custom plan created when Windows (Modern Standby, OEM editions) does not offer a high-performance plan.
const CUSTOM_PLAN: &[(&str, &str, i32)] = &[
    ("54533251-82be-4824-96c1-47b60b740d00", "893dee8e-2bef-41e0-89c6-b55d0929964c", 100), // min CPU state
    ("54533251-82be-4824-96c1-47b60b740d00", "bc5038f7-23e0-4960-96da-33abaf5935ec", 100), // max CPU state
    ("54533251-82be-4824-96c1-47b60b740d00", "36687f9e-e3a5-4dbf-b1dc-15eb381c6863", 0),   // EPP
    ("54533251-82be-4824-96c1-47b60b740d00", "be337238-0d82-4146-a960-4f3749d470c7", 2),   // boost
    ("54533251-82be-4824-96c1-47b60b740d00", "0cc5b647-c1df-4637-891a-dec35c318583", 100), // core parking
    ("0012ee47-9041-4b5d-9b77-535fba8b1442", "6738e2c4-e8a5-4a42-b16a-e040e769756e", 0),   // disk idle
    ("2a737441-1930-4402-8d77-b2bebba308a3", "48e6b7a6-50f5-4782-a5d4-53bb8f07e226", 0),   // USB selective suspend
    ("501a4d13-42af-4429-9fd1-a8218c268e20", "ee12f906-d277-404b-b6da-e5fa1a576df5", 0),   // PCIe link state
    ("19cbb8fa-5279-450e-9fac-8a3d5fedd0c1", "12bbebe6-58d6-4636-95bb-3217ef867c1a", 0),   // Wi-Fi power saving
];

pub struct PowerHandler { run: Arc<dyn Runner> }

impl PowerHandler {
    pub fn new(run: Arc<dyn Runner>) -> Self { Self { run } }

    fn cmd(&self, parts: &[&str]) -> crate::runner::RunResult { self.run.run(&argv(parts), 60) }

    pub fn active_scheme(&self) -> Option<String> {
        let r = self.cmd(&["powercfg", "/getactivescheme"]);
        if r.success() { find_guid(&r.output) } else { None }
    }

    fn try_query(&self, sub: &str, setting: &str) -> Option<i64> {
        let r = self.cmd(&["powercfg", "/query", "SCHEME_CURRENT", sub, setting]);
        if !r.success() { return None; }
        let v = hex_values(&r.output);
        if v.len() < 2 { return None; }
        Some(i64::from(v[v.len() - 2])) // second to last = AC, last = DC
    }

    fn ac_value(&self, sub: &str, setting: &str) -> Option<i64> {
        self.try_query(sub, setting).or_else(|| {
            // The setting may be hidden in the plan; reveal the attribute and retry.
            self.cmd(&["powercfg", "-attributes", sub, setting, "-ATTRIB_HIDE"]);
            self.try_query(sub, setting)
        })
    }

    fn set_ac(&self, scheme: &str, sub: &str, setting: &str, value: i64) -> Result<(), String> {
        let r = self.cmd(&["powercfg", "/setacvalueindex", scheme, sub, setting, &value.to_string()]);
        if r.success() { Ok(()) } else { Err(if r.first_line().is_empty() { "powercfg: setting failed".into() } else { r.first_line() }) }
    }

    fn target(t: &TweakDefinition) -> &'static str { if t.plan.as_deref() == Some("ultimate") { ULTIMATE } else { HIGH_PERFORMANCE } }

    fn create_custom_plan(&self) -> Option<String> {
        let dup = self.cmd(&["powercfg", "/duplicatescheme", BALANCED]);
        let g = if dup.success() { find_guid(&dup.output)? } else { return None };
        self.cmd(&["powercfg", "/changename", &g, "NEOX Performance", "Performance plan created by NEOX"]);
        for (sub, setting, v) in CUSTOM_PLAN { self.cmd(&["powercfg", "/setacvalueindex", &g, sub, setting, &v.to_string()]); }
        Some(g)
    }
}

impl TweakHandler for PowerHandler {
    fn kinds(&self) -> &'static [TweakKind] { &[TweakKind::PowerIdx, TweakKind::PowerPlan] }

    fn status(&self, t: &TweakDefinition, _: &TweakContext) -> Result<TweakStatus, String> {
        if t.kind == TweakKind::PowerPlan {
            return Ok(match self.active_scheme() { None => TweakStatus::Unknown,
                Some(a) if a == Self::target(t) => TweakStatus::Applied, Some(_) => TweakStatus::NotApplied });
        }
        let p = t.power.as_ref().ok_or("Missing power data")?;
        Ok(match self.ac_value(&p.subgroup, &p.setting) { None => TweakStatus::Unknown,
            Some(v) if v == p.value => TweakStatus::Applied, Some(_) => TweakStatus::NotApplied })
    }

    fn apply(&self, t: &TweakDefinition, _: &TweakContext) -> Result<Value, String> {
        let scheme = self.active_scheme().ok_or("Could not read the active power plan.")?;
        if t.kind == TweakKind::PowerIdx {
            let p = t.power.as_ref().ok_or("Missing power data")?;
            let old = self.ac_value(&p.subgroup, &p.setting).ok_or("This power plan does not expose this setting.")?;
            self.set_ac(&scheme, &p.subgroup, &p.setting, p.value)?;
            self.cmd(&["powercfg", "/setactive", &scheme]);
            return Ok(json!({"scheme": scheme, "sub": p.subgroup, "setting": p.setting, "old": old}));
        }
        let target = Self::target(t);
        let mut created: Option<String> = None;
        if scheme != target {
            let r = self.cmd(&["powercfg", "/setactive", target]);
            if !r.success() {
                // Hidden plan: duplicate from the GUID template; if that fails, build a custom one from Balanced.
                let dup = self.cmd(&["powercfg", "/duplicatescheme", target]);
                let g = match (dup.success(), find_guid(&dup.output)) {
                    (true, Some(g)) => g,
                    _ => self.create_custom_plan().ok_or("Windows does not provide this power plan and a custom one could not be created.")?,
                };
                let act = self.cmd(&["powercfg", "/setactive", &g]);
                if !act.success() {
                    self.cmd(&["powercfg", "/delete", &g]);
                    return Err(format!("Could not activate the power plan: {}", act.first_line()));
                }
                created = Some(g);
            }
        }
        Ok(json!({"previous": scheme, "created": created}))
    }

    fn revert(&self, t: &TweakDefinition, snap: &Value, _: &TweakContext) -> Result<(), String> {
        if t.kind == TweakKind::PowerIdx {
            let scheme = snap["scheme"].as_str().ok_or("Corrupt snapshot")?;
            self.set_ac(scheme, snap["sub"].as_str().unwrap_or(""), snap["setting"].as_str().unwrap_or(""), snap["old"].as_i64().unwrap_or(0))?;
            if self.active_scheme().as_deref() == Some(scheme) { self.cmd(&["powercfg", "/setactive", scheme]); }
            return Ok(());
        }
        let prev = snap["previous"].as_str().ok_or("Corrupt snapshot")?;
        let r = self.cmd(&["powercfg", "/setactive", prev]);
        if !r.success() { return Err(format!("Could not restore the previous power plan: {}", r.first_line())); }
        if let Some(c) = snap["created"].as_str().filter(|c| !c.is_empty()) { self.cmd(&["powercfg", "/delete", c]); }
        Ok(())
    }
}

// ---------------------------------------------------------------- services

pub struct ServiceHandler { run: Arc<dyn Runner>, reg: Arc<dyn RegistryApi> }

impl ServiceHandler {
    pub fn new(run: Arc<dyn Runner>, reg: Arc<dyn RegistryApi>) -> Self { Self { run, reg } }
    fn key(svc: &str) -> String { format!("SYSTEM\\CurrentControlSet\\Services\\{svc}") }
    /// (start type, delayed-autostart flag); start type is read from the registry, never parsed from localized `sc` output.
    fn read(&self, svc: &str) -> Option<(i64, i64)> {
        let s = self.reg.read("HKLM", &Self::key(svc), Some("Start")).ok()??;
        let d = self.reg.read("HKLM", &Self::key(svc), Some("DelayedAutostart")).ok().flatten();
        Some((dword(&s), d.map_or(0, |x| dword(&x))))
    }
}

impl TweakHandler for ServiceHandler {
    fn kinds(&self) -> &'static [TweakKind] { &[TweakKind::Service] }

    fn status(&self, t: &TweakDefinition, _: &TweakContext) -> Result<TweakStatus, String> {
        Ok(match self.read(t.service.as_deref().unwrap_or("")) { None => TweakStatus::Unknown, Some((4, _)) => TweakStatus::Applied, Some(_) => TweakStatus::NotApplied })
    }

    fn apply(&self, t: &TweakDefinition, _: &TweakContext) -> Result<Value, String> {
        let name = t.service.as_deref().ok_or("Missing service name")?;
        let (start, delayed) = self.read(name).ok_or_else(|| format!("Service {name} does not exist on this computer."))?;
        let r = self.run.run(&argv(&["sc", "config", name, "start=", "disabled"]), 60);
        if !r.success() { return Err(format!("Could not disable service {name}: {}", r.first_line())); }
        Ok(json!({"service": name, "start": start, "delayed": delayed}))
    }

    fn revert(&self, _: &TweakDefinition, snap: &Value, _: &TweakContext) -> Result<(), String> {
        let name = snap["service"].as_str().ok_or("Corrupt snapshot")?;
        let delayed = snap["delayed"].as_i64() == Some(1);
        let mode = match snap["start"].as_i64() { Some(2) => if delayed { "delayed-auto" } else { "auto" }, Some(4) => "disabled", _ => "demand" };
        let r = self.run.run(&argv(&["sc", "config", name, "start=", mode]), 60);
        if r.success() { Ok(()) } else { Err(format!("Could not restore service {name}: {}", r.first_line())) }
    }
}

// ---------------------------------------------------------------- scheduled tasks

pub struct TaskHandler { run: Arc<dyn Runner> }
impl TaskHandler {
    pub fn new(run: Arc<dyn Runner>) -> Self { Self { run } }
    fn enabled(&self, task: &str) -> Option<bool> {
        let r = self.run.run(&argv(&["schtasks", "/query", "/tn", task, "/xml"]), 60);
        r.success().then(|| task_enabled(&r.output))
    }
}

impl TweakHandler for TaskHandler {
    fn kinds(&self) -> &'static [TweakKind] { &[TweakKind::Task] }

    fn status(&self, t: &TweakDefinition, _: &TweakContext) -> Result<TweakStatus, String> {
        let states: Vec<bool> = t.tasks.iter().flatten().filter_map(|x| self.enabled(x)).collect();
        Ok(if states.is_empty() { TweakStatus::Unknown } else if states.iter().any(|x| *x) { TweakStatus::NotApplied } else { TweakStatus::Applied })
    }

    fn apply(&self, t: &TweakDefinition, _: &TweakContext) -> Result<Value, String> {
        let mut changed = Vec::new();
        for task in t.tasks.iter().flatten() {
            let Some(was) = self.enabled(task) else { continue }; // task does not exist in this Windows version
            let r = self.run.run(&argv(&["schtasks", "/change", "/tn", task, "/disable"]), 60);
            if !r.success() { return Err(format!("Could not disable task {task}: {}", r.first_line())); }
            changed.push(json!({"task": task, "wasEnabled": was}));
        }
        if changed.is_empty() { return Err("None of these tasks exist in this version of Windows.".into()); }
        Ok(Value::Array(changed))
    }

    fn revert(&self, _: &TweakDefinition, snap: &Value, _: &TweakContext) -> Result<(), String> {
        for n in snap.as_array().ok_or("Corrupt snapshot")? {
            if n["wasEnabled"].as_bool() != Some(true) { continue; }
            let r = self.run.run(&argv(&["schtasks", "/change", "/tn", n["task"].as_str().unwrap_or(""), "/enable"]), 60);
            if !r.success() { return Err(format!("Could not enable the task: {}", r.first_line())); }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- bcdedit

pub struct BcdHandler { run: Arc<dyn Runner> }
impl BcdHandler {
    pub fn new(run: Arc<dyn Runner>) -> Self { Self { run } }
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        let r = self.run.run(&argv(&["bcdedit", "/enum", "{current}"]), 60);
        if !r.success() { return Err(format!("bcdedit is unavailable (requires administrator): {}", r.first_line())); }
        Ok(bcd_value(&r.output, key))
    }
}

impl TweakHandler for BcdHandler {
    fn kinds(&self) -> &'static [TweakKind] { &[TweakKind::Bcd] }

    fn status(&self, t: &TweakDefinition, _: &TweakContext) -> Result<TweakStatus, String> {
        let b = t.bcd.as_ref().ok_or("Missing bcd data")?;
        Ok(if self.get(&b.key)?.is_some_and(|v| v.eq_ignore_ascii_case(&b.value)) { TweakStatus::Applied } else { TweakStatus::NotApplied })
    }

    fn apply(&self, t: &TweakDefinition, _: &TweakContext) -> Result<Value, String> {
        let b = t.bcd.as_ref().ok_or("Missing bcd data")?;
        let old = self.get(&b.key)?;
        let r = self.run.run(&argv(&["bcdedit", "/set", &b.key, &b.value]), 60);
        if !r.success() { return Err(format!("bcdedit rejected the change: {}", r.first_line())); }
        Ok(json!({"key": b.key, "old": old}))
    }

    fn revert(&self, _: &TweakDefinition, snap: &Value, _: &TweakContext) -> Result<(), String> {
        let key = snap["key"].as_str().ok_or("Corrupt snapshot")?;
        let r = match snap["old"].as_str() {
            None => self.run.run(&argv(&["bcdedit", "/deletevalue", key]), 60),
            Some(old) => self.run.run(&argv(&["bcdedit", "/set", key, old]), 60),
        };
        if r.success() { Ok(()) } else { Err(format!("bcdedit rejected the revert: {}", r.first_line())) }
    }
}

// ---------------------------------------------------------------- NIC advanced properties

pub struct NicHandler { run: Arc<dyn Runner> }
impl NicHandler {
    pub fn new(run: Arc<dyn Runner>) -> Self { Self { run } }

    fn read(&self, keyword: &str) -> Vec<(String, String)> {
        let script = format!("Get-NetAdapter -Physical | Where-Object {{$_.Status -eq 'Up'}} | ForEach-Object {{ \
Get-NetAdapterAdvancedProperty -Name $_.Name -RegistryKeyword '{}' -ErrorAction SilentlyContinue }} | \
ForEach-Object {{ [pscustomobject]@{{N=$_.Name;V=([string]($_.RegistryValue -join ','))}} }} | ConvertTo-Json -Compress", q(keyword));
        let r = self.run.run(&powershell(&script), 40);
        if !r.success() { return vec![]; }
        let (Some(a), Some(b)) = (r.output.find(|c| c == '[' || c == '{'), r.output.rfind(|c| c == ']' || c == '}')) else { return vec![] };
        let Ok(v) = serde_json::from_str::<Value>(&r.output[a..=b]) else { return vec![] };
        let items = match v { Value::Array(a) => a, o @ Value::Object(_) => vec![o], _ => vec![] };
        items.iter().filter_map(|i| {
            let n = i["N"].as_str()?.to_string();
            (!n.is_empty()).then(|| (n, i["V"].as_str().unwrap_or("").to_string()))
        }).collect()
    }

    fn set(&self, adapter: &str, keyword: &str, value: &str) -> Result<(), String> {
        let r = self.run.run(&powershell(&format!(
            "Set-NetAdapterAdvancedProperty -Name '{}' -RegistryKeyword '{}' -RegistryValue '{}' -ErrorAction Stop", q(adapter), q(keyword), q(value))), 60);
        if r.success() { Ok(()) } else { Err(format!("Adapter {adapter}: {}", r.first_line())) }
    }
}

impl TweakHandler for NicHandler {
    fn kinds(&self) -> &'static [TweakKind] { &[TweakKind::Nic] }

    fn status(&self, t: &TweakDefinition, _: &TweakContext) -> Result<TweakStatus, String> {
        let nic = t.nic.as_ref().ok_or("Missing nic data")?;
        for kw in &nic.keywords {
            let vals = self.read(kw);
            if vals.is_empty() || vals.iter().any(|(_, v)| *v != nic.value) { return Ok(TweakStatus::NotApplied); }
        }
        Ok(TweakStatus::Applied)
    }

    fn apply(&self, t: &TweakDefinition, _: &TweakContext) -> Result<Value, String> {
        let nic = t.nic.as_ref().ok_or("Missing nic data")?;
        let mut snap: Vec<(String, String, String)> = Vec::new();
        for kw in &nic.keywords { for (a, v) in self.read(kw) { snap.push((a, kw.clone(), v)); } }
        if snap.is_empty() { return Err("No active network adapter supports this setting.".into()); }
        for (i, (a, kw, _)) in snap.iter().enumerate() {
            if let Err(e) = self.set(a, kw, &nic.value) {
                for (a2, kw2, old) in &snap[..i] { let _ = self.set(a2, kw2, old); }
                return Err(e);
            }
        }
        Ok(Value::Array(snap.iter().map(|(a, k, o)| json!({"adapter": a, "keyword": k, "old": o})).collect()))
    }

    fn revert(&self, _: &TweakDefinition, snap: &Value, _: &TweakContext) -> Result<(), String> {
        for n in snap.as_array().ok_or("Corrupt snapshot")? {
            self.set(n["adapter"].as_str().unwrap_or(""), n["keyword"].as_str().unwrap_or(""), n["old"].as_str().unwrap_or(""))?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- commands

pub struct CmdHandler { run: Arc<dyn Runner>, reg: Arc<dyn RegistryApi> }
impl CmdHandler {
    pub fn new(run: Arc<dyn Runner>, reg: Arc<dyn RegistryApi>) -> Self { Self { run, reg } }
}

impl TweakHandler for CmdHandler {
    fn kinds(&self) -> &'static [TweakKind] { &[TweakKind::Cmd] }

    fn status(&self, t: &TweakDefinition, _: &TweakContext) -> Result<TweakStatus, String> {
        let chk = t.cmd.as_ref().and_then(|c| c.check.as_ref());
        if let Some(r) = chk.and_then(|c| c.reg_dword.as_ref()) {
            let cur = self.reg.read(&r.hive, &r.path, Some(&r.name))?;
            return Ok(if cur.is_some_and(|d| dword(&d) == r.expected) { TweakStatus::Applied } else { TweakStatus::NotApplied });
        }
        if let Some(ps) = chk.and_then(|c| c.ps.as_ref()) {
            let res = self.run.run(&powershell(ps), 30);
            if !res.success() { return Ok(TweakStatus::Unknown); }
            let is_false = res.output.trim().to_lowercase().ends_with("false");
            return Ok(if chk.is_some_and(|c| c.equals_false) == is_false { TweakStatus::Applied } else { TweakStatus::NotApplied });
        }
        Ok(TweakStatus::Unknown) // no reliable read-back: state is whether NEOX made the change
    }

    fn apply(&self, t: &TweakDefinition, _: &TweakContext) -> Result<Value, String> {
        let cmd = t.cmd.as_ref().ok_or("Missing command data")?;
        let timeout = u64::from(t.timeout_seconds.unwrap_or(120));
        for (ran, a) in cmd.on.iter().enumerate() {
            let r = self.run.run(a, timeout);
            if !r.success() {
                for i in 0..ran { if let Some(off) = cmd.off.get(i.min(cmd.off.len().saturating_sub(1))) { self.run.run(off, 120); } }
                return Err(if r.first_line().is_empty() { "The command failed.".into() } else { r.first_line() });
            }
        }
        Ok(json!({"appliedAt": neox_core::timefmt::now_rfc3339()}))
    }

    fn revert(&self, t: &TweakDefinition, _: &Value, _: &TweakContext) -> Result<(), String> {
        for a in &t.cmd.as_ref().ok_or("Missing command data")?.off {
            let r = self.run.run(a, 120);
            if !r.success() { return Err(format!("Revert failed: {}", r.first_line())); }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::AdapterInfo;
    use crate::registry::MemRegistry;
    use crate::runner::fake::Script;
    use crate::runner::RunResult;
    use neox_core::catalog::Catalog;

    struct OneNic;
    impl Adapters for OneNic {
        fn active(&self) -> Vec<AdapterInfo> {
            vec![AdapterInfo { name: "Eth".into(), guid: "AAAA".into(), description: "".into(), speed_mbps: 1000, dns: vec![], addresses: vec![], kind: "ethernet".into() }]
        }
    }
    fn cat() -> Catalog { Catalog::load_embedded().unwrap() }
    fn ctx() -> TweakContext { TweakContext::default() }

    fn reg_handler(reg: Arc<MemRegistry>) -> RegistryHandler {
        RegistryHandler::new(reg, Arc::new(OneNic), Arc::new(|| {}))
    }

    #[test]
    fn registry_handler_applies_verifies_and_reverts_every_plain_reg_tweak() {
        let c = cat();
        for t in c.all().iter().filter(|t| t.kind == TweakKind::Reg) {
            let reg = Arc::new(MemRegistry::new());
            let h = reg_handler(reg.clone());
            assert_eq!(h.status(t, &ctx()).unwrap(), TweakStatus::NotApplied, "{}", t.id);
            let snap = h.apply(t, &ctx()).unwrap_or_else(|e| panic!("{}: {e}", t.id));
            assert_eq!(h.status(t, &ctx()).unwrap(), TweakStatus::Applied, "{}", t.id);
            h.revert(t, &snap, &ctx()).unwrap();
            assert_eq!(h.status(t, &ctx()).unwrap(), TweakStatus::NotApplied, "{}", t.id);
        }
    }

    #[test]
    fn hex_bytes_parses_and_ignores_separators() {
        assert_eq!(hex_bytes("0A0b ff,00"), vec![0x0a, 0x0b, 0xff, 0x00]);
        assert_eq!(hex_bytes(""), Vec::<u8>::new());
        let c = cat();
        let v = &c.find("kb_scancode_gaming").unwrap().values.as_ref().unwrap()[0];
        assert_eq!(v.ty, "binary");
        let b = hex_bytes(&v.as_string());
        assert_eq!(b.len(), 28);
        assert_eq!(&b[8..12], &[4, 0, 0, 0], "entry count includes the terminator");
    }

    #[test]
    fn registry_handler_restores_previous_values_not_defaults() {
        let c = cat();
        let t = c.find("game_mode").unwrap();
        let reg = Arc::new(MemRegistry::new());
        let v = &t.values.as_ref().unwrap()[0];
        reg.write(&v.hive, &v.path, v.name.as_deref(), &RegData::Dword(42)).unwrap();
        let h = reg_handler(reg.clone());
        let snap = h.apply(t, &ctx()).unwrap();
        h.revert(t, &snap, &ctx()).unwrap();
        assert_eq!(reg.read(&v.hive, &v.path, v.name.as_deref()).unwrap(), Some(RegData::Dword(42)));
    }

    #[test]
    fn registry_write_failure_leaves_nothing_half_applied() {
        let c = cat();
        let t = c.all().iter().find(|t| t.kind == TweakKind::Reg && t.values.as_ref().unwrap().len() > 1).unwrap();
        let reg = Arc::new(MemRegistry::new());
        *reg.deny_write.lock().unwrap() = true;
        let h = reg_handler(reg.clone());
        assert!(h.apply(t, &ctx()).is_err());
        assert_eq!(h.status(t, &ctx()).unwrap(), TweakStatus::NotApplied);
    }

    #[test]
    fn per_adapter_tweaks_expand_the_adapter_guid() {
        let c = cat();
        let t = c.find("nagle").unwrap();
        let reg = Arc::new(MemRegistry::new());
        let h = reg_handler(reg.clone());
        h.apply(t, &ctx()).unwrap();
        let path = "SYSTEM\\CurrentControlSet\\Services\\Tcpip\\Parameters\\Interfaces\\{AAAA}";
        assert_eq!(reg.read("HKLM", path, Some("TcpAckFrequency")).unwrap(), Some(RegData::Dword(1)));
    }

    #[test]
    fn per_game_tweaks_require_an_existing_exe() {
        let c = cat();
        let t = c.find("fso_gpu").unwrap();
        let h = reg_handler(Arc::new(MemRegistry::new()));
        assert!(h.apply(t, &ctx()).is_err());
        let me = std::env::current_exe().unwrap().to_string_lossy().to_string();
        assert!(h.apply(t, &TweakContext { game_exe_path: Some(me) }).is_ok());
    }

    #[test]
    fn service_handler_disables_and_restores_the_original_start_type() {
        let c = cat();
        let t = c.find("svc_sysmain").unwrap();
        let reg = Arc::new(MemRegistry::new());
        reg.write("HKLM", "SYSTEM\\CurrentControlSet\\Services\\SysMain", Some("Start"), &RegData::Dword(2)).unwrap();
        reg.write("HKLM", "SYSTEM\\CurrentControlSet\\Services\\SysMain", Some("DelayedAutostart"), &RegData::Dword(1)).unwrap();
        let run = Arc::new(Script::new());
        let h = ServiceHandler::new(run.clone(), reg);
        let snap = h.apply(t, &ctx()).unwrap();
        h.revert(t, &snap, &ctx()).unwrap();
        let calls = run.calls();
        assert_eq!(calls[0], "sc config SysMain start= disabled");
        assert_eq!(calls[1], "sc config SysMain start= delayed-auto");
    }

    #[test]
    fn service_handler_reports_missing_services() {
        let c = cat();
        let h = ServiceHandler::new(Arc::new(Script::new()), Arc::new(MemRegistry::new()));
        let t = c.find("svc_sysmain").unwrap();
        assert_eq!(h.status(t, &ctx()).unwrap(), TweakStatus::Unknown);
        assert!(h.apply(t, &ctx()).unwrap_err().contains("does not exist"));
    }

    #[test]
    fn power_idx_round_trip_uses_ac_value_and_restores_it() {
        let c = cat();
        let t = c.find("cpu_min").unwrap();
        let run = Arc::new(Script::new());
        run.on("powercfg /getactivescheme", RunResult::ok("Power Scheme GUID: 381b4222-f694-41f0-9685-ff5bb260df2e  (Balanced)"));
        run.on("powercfg /query", RunResult::ok("AC Power Setting Index: 0x00000005\nDC Power Setting Index: 0x00000005"));
        let h = PowerHandler::new(run.clone());
        let snap = h.apply(t, &ctx()).unwrap();
        assert_eq!(snap["old"], 5);
        h.revert(t, &snap, &ctx()).unwrap();
        assert!(run.calls().iter().any(|l| l.contains("/setacvalueindex 381b4222-f694-41f0-9685-ff5bb260df2e") && l.ends_with(" 5")));
    }

    #[test]
    fn power_plan_builds_a_custom_plan_when_windows_hides_high_performance() {
        let c = cat();
        let t = c.find("powerPlan").unwrap();
        let run = Arc::new(Script::new());
        run.on("powercfg /getactivescheme", RunResult::ok("Power Scheme GUID: 381b4222-f694-41f0-9685-ff5bb260df2e  (Balanced)"));
        run.on(&format!("powercfg /setactive {HIGH_PERFORMANCE}"), RunResult::err(1, "not found"));
        run.on(&format!("powercfg /duplicatescheme {HIGH_PERFORMANCE}"), RunResult::err(1, "not found"));
        run.on(&format!("powercfg /duplicatescheme {BALANCED}"), RunResult::ok("Power Scheme GUID: 11111111-2222-3333-4444-555555555555  (Balanced)"));
        let h = PowerHandler::new(run.clone());
        let snap = h.apply(t, &ctx()).unwrap();
        assert_eq!(snap["created"], "11111111-2222-3333-4444-555555555555");
        h.revert(t, &snap, &ctx()).unwrap();
        assert!(run.calls().iter().any(|l| l == "powercfg /delete 11111111-2222-3333-4444-555555555555"));
    }

    #[test]
    fn bcd_handler_restores_or_deletes_depending_on_the_original_value() {
        let c = cat();
        let t = c.find("dynamic_tick").unwrap();
        let run = Arc::new(Script::new());
        run.on("bcdedit /enum", RunResult::ok("identifier {current}\nuseplatformtick Yes"));
        let h = BcdHandler::new(run.clone());
        let snap = h.apply(t, &ctx()).unwrap();
        assert!(snap["old"].is_null());
        h.revert(t, &snap, &ctx()).unwrap();
        assert!(run.calls().iter().any(|l| l == "bcdedit /deletevalue disabledynamictick"));
    }

    #[test]
    fn task_handler_skips_missing_tasks_and_restores_only_enabled_ones() {
        let c = cat();
        let t = c.find("task_appraiser").unwrap();
        let run = Arc::new(Script::new());
        run.on("schtasks /query /tn \\Microsoft\\Windows\\Application Experience\\Microsoft Compatibility Appraiser",
               RunResult::ok("<Task><Settings><Enabled>true</Enabled></Settings></Task>"));
        run.on("schtasks /query", RunResult::err(1, "ERROR: The system cannot find the file specified."));
        let h = TaskHandler::new(run.clone());
        let snap = h.apply(t, &ctx()).unwrap();
        assert_eq!(snap.as_array().unwrap().len(), 1);
        h.revert(t, &snap, &ctx()).unwrap();
        assert!(run.calls().iter().any(|l| l.contains("/enable")));
    }

    #[test]
    fn cmd_handler_rolls_back_earlier_commands_when_one_fails() {
        let c = cat();
        let mut t = c.find("hibernate_off").unwrap().clone();
        t.cmd.as_mut().unwrap().on = vec![argv(&["powercfg", "/h", "off"]), argv(&["fail"])];
        let run = Arc::new(Script::new());
        run.on("fail", RunResult::err(1, "nope"));
        let h = CmdHandler::new(run.clone(), Arc::new(MemRegistry::new()));
        assert_eq!(h.apply(&t, &ctx()).unwrap_err(), "nope");
        assert!(run.calls().iter().any(|l| l == "powercfg /h on"));
    }

    #[test]
    fn cmd_handler_reads_back_registry_checks() {
        let c = cat();
        let t = c.find("hibernate_off").unwrap();
        let reg = Arc::new(MemRegistry::new());
        let h = CmdHandler::new(Arc::new(Script::new()), reg.clone());
        assert_eq!(h.status(t, &ctx()).unwrap(), TweakStatus::NotApplied);
        reg.write("HKLM", "SYSTEM\\CurrentControlSet\\Control\\Power", Some("HibernateEnabled"), &RegData::Dword(0)).unwrap();
        assert_eq!(h.status(t, &ctx()).unwrap(), TweakStatus::Applied);
    }

    #[test]
    fn nic_handler_sets_and_restores_per_adapter_values() {
        let c = cat();
        let t = c.find("nic_im_off").unwrap();
        let run = Arc::new(Script::new());
        run.on("powershell -NoProfile -NonInteractive -Command Get-NetAdapter", RunResult::ok(r#"{"N":"Eth","V":"1"}"#));
        let h = NicHandler::new(run.clone());
        let snap = h.apply(t, &ctx()).unwrap();
        h.revert(t, &snap, &ctx()).unwrap();
        let sets: Vec<_> = run.calls().into_iter().filter(|l| l.contains("Set-NetAdapterAdvancedProperty")).collect();
        assert_eq!(sets.len(), 2);
        assert!(sets[0].contains("-RegistryValue '0'"));
        assert!(sets[1].contains("-RegistryValue '1'"));
    }
}
