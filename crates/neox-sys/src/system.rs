//! Elevation check, restore points and the mouse refresh used by the `refreshMouse` post step.

use crate::runner::{powershell, Runner};
use neox_core::engine::{Elevation, RestorePoints};
use neox_core::models::{RestartRequirement, TweakResult};
use std::sync::Arc;

/// Elevation via the token's integrity level (S-1-16-12288 = High) from `whoami /groups`: no FFI needed.
pub struct WinElevation { elevated: bool }

impl WinElevation {
    pub fn detect(run: &dyn Runner) -> Self {
        let r = run.run(&["whoami".to_string(), "/groups".to_string()], 15);
        Self { elevated: r.success() && r.output.contains("S-1-16-12288") }
    }
}
impl Elevation for WinElevation { fn is_elevated(&self) -> bool { self.elevated } }

/// Relaunches the app with a UAC prompt (PowerShell `Start-Process -Verb RunAs`). Returns false if the user declines.
pub fn restart_elevated(run: &dyn Runner) -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let script = format!("Start-Process -FilePath '{}' -Verb RunAs -ErrorAction Stop", exe.to_string_lossy().replace('\'', "''"));
    run.run(&powershell(&script), 30).success()
}

pub struct PsRestorePoints { pub run: Arc<dyn Runner> }
impl RestorePoints for PsRestorePoints {
    fn create(&self, description: &str) -> TweakResult {
        let safe = description.replace('\'', "''");
        let r = self.run.run(&powershell(&format!(
            "Checkpoint-Computer -Description '{safe}' -RestorePointType MODIFY_SETTINGS -ErrorAction Stop")), 180);
        if r.success() { TweakResult::ok("Restore point created.", RestartRequirement::None) }
        else { TweakResult::fail(if r.first_line().is_empty() { "System Protection may be turned off.".to_string() } else { r.first_line() }) }
    }
}

/// Applies mouse registry settings immediately, without signing out.
#[cfg(windows)]
pub fn refresh_mouse(reg: &dyn crate::registry::RegistryApi) {
    use crate::registry::RegData;
    #[link(name = "user32")]
    extern "system" {
        fn SystemParametersInfoW(action: u32, param: u32, value: *mut core::ffi::c_void, flags: u32) -> i32;
    }
    let get = |n: &str| -> i32 {
        match reg.read("HKCU", "Control Panel\\Mouse", Some(n)) {
            Ok(Some(RegData::Str(s))) | Ok(Some(RegData::ExpandStr(s))) => s.trim().parse().unwrap_or(0),
            Ok(Some(RegData::Dword(v))) => v as i32,
            _ => 0,
        }
    };
    let mut accel = [get("MouseThreshold1"), get("MouseThreshold2"), get("MouseSpeed")];
    let sens = get("MouseSensitivity");
    // SAFETY: valid pointers for the duration of the calls; SPI_SETMOUSE expects an int[3], SPI_SETMOUSESPEED an int in the pointer value.
    unsafe {
        SystemParametersInfoW(0x0004, 0, accel.as_mut_ptr().cast(), 0x03);
        if (1..=20).contains(&sens) { SystemParametersInfoW(0x0071, 0, sens as usize as *mut core::ffi::c_void, 0x03); }
    }
}

#[cfg(not(windows))]
pub fn refresh_mouse(_reg: &dyn crate::registry::RegistryApi) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::fake::Script;
    use crate::runner::RunResult;

    #[test]
    fn detects_high_integrity_token() {
        let run = Script::new();
        run.on("whoami", RunResult::ok("Mandatory Label\\High Mandatory Level  Label  S-1-16-12288"));
        assert!(WinElevation::detect(&run).is_elevated());
        let run = Script::new();
        run.on("whoami", RunResult::ok("Mandatory Label\\Medium Mandatory Level  Label  S-1-16-8192"));
        assert!(!WinElevation::detect(&run).is_elevated());
    }

    #[test]
    fn restore_point_failure_message_is_explained() {
        let run = Arc::new(Script::new());
        run.on("powershell", RunResult::err(1, ""));
        assert!(!PsRestorePoints { run }.create("x").success);
    }
}
