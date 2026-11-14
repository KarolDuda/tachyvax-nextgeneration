//! Process execution without a shell: arguments are passed separately (no injection), with a timeout and no console window.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct RunResult { pub code: i32, pub output: String }

impl RunResult {
    pub fn success(&self) -> bool { self.code == 0 }
    pub fn first_line(&self) -> String { self.output.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").to_string() }
    pub fn ok(output: &str) -> Self { Self { code: 0, output: output.into() } }
    pub fn err(code: i32, output: &str) -> Self { Self { code, output: output.into() } }
}

pub trait Runner: Send + Sync {
    fn run(&self, argv: &[String], timeout_secs: u64) -> RunResult;
}

pub fn argv(parts: &[&str]) -> Vec<String> { parts.iter().map(|s| s.to_string()).collect() }

pub fn powershell(script: &str) -> Vec<String> {
    argv(&["powershell", "-NoProfile", "-NonInteractive", "-Command", script])
}

pub struct ProcRunner;

impl Runner for ProcRunner {
    fn run(&self, argv: &[String], timeout_secs: u64) -> RunResult {
        let Some(program) = argv.first() else { return RunResult::err(-1, "Empty command.") };
        let mut cmd = Command::new(program);
        cmd.args(&argv[1..]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(windows)]
        { use std::os::windows::process::CommandExt; cmd.creation_flags(0x0800_0000); } // CREATE_NO_WINDOW
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return RunResult::err(-1, &format!("Cannot start {program}: {e}")),
        };
        let mut out = child.stdout.take();
        let mut err = child.stderr.take();
        let t_out = std::thread::spawn(move || { let mut b = Vec::new(); if let Some(o) = out.as_mut() { let _ = o.read_to_end(&mut b); } b });
        let t_err = std::thread::spawn(move || { let mut b = Vec::new(); if let Some(o) = err.as_mut() { let _ = o.read_to_end(&mut b); } b });

        let deadline = Instant::now() + Duration::from_secs(timeout_secs.max(1));
        let code = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status.code().unwrap_or(-1),
                Ok(None) if Instant::now() >= deadline => { let _ = child.kill(); let _ = child.wait(); break -2; }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(_) => break -1,
            }
        };
        let mut text = String::from_utf8_lossy(&t_out.join().unwrap_or_default()).into_owned();
        text.push_str(&String::from_utf8_lossy(&t_err.join().unwrap_or_default()));
        if code == -2 { text = "The command timed out.".into(); }
        RunResult { code, output: text.trim().to_string() }
    }
}

#[cfg(test)]
pub mod fake {
    use super::*;
    use std::sync::Mutex;

    /// Scripted runner: responds by the first matching prefix (joined argv) and records every call.
    pub struct Script { pub rules: Mutex<Vec<(String, RunResult)>>, pub calls: Mutex<Vec<String>> }
    impl Script {
        pub fn new() -> Self { Self { rules: Mutex::new(vec![]), calls: Mutex::new(vec![]) } }
        pub fn on(&self, prefix: &str, r: RunResult) -> &Self { self.rules.lock().unwrap().push((prefix.into(), r)); self }
        pub fn calls(&self) -> Vec<String> { self.calls.lock().unwrap().clone() }
    }
    impl Runner for Script {
        fn run(&self, argv: &[String], _: u64) -> RunResult {
            let line = argv.join(" ");
            self.calls.lock().unwrap().push(line.clone());
            self.rules.lock().unwrap().iter().find(|(p, _)| line.starts_with(p.as_str())).map(|(_, r)| r.clone())
                .unwrap_or_else(|| RunResult::ok(""))
        }
    }
}
