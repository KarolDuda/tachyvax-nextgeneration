//! Hardware / OS facts from a single PowerShell CIM query (cached by the caller), plus live CPU/RAM sampling.

use crate::runner::{powershell, Runner};
use neox_core::autotune::HardwareProfile;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CpuInfo { pub name: String, pub cores: u32, pub threads: u32, pub mhz: u32 }
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GpuInfo { pub name: String, pub driver: String, pub vram: i64, pub hz: u32 }
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DiskInfo { pub model: String, #[serde(rename = "type")] pub ty: String, pub size: i64, pub status: String }
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VolumeInfo { pub name: String, pub total: i64, pub free: i64, pub fs: String }
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OsInfo {
    pub caption: String, pub version: String, pub build: String, pub boot: i64,
    #[serde(rename = "totalKb")] pub total_kb: i64, #[serde(rename = "freeKb")] pub free_kb: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SystemSnapshot {
    pub cpu: CpuInfo,
    pub gpus: Vec<GpuInfo>,
    pub disks: Vec<DiskInfo>,
    pub volumes: Vec<VolumeInfo>,
    pub os: OsInfo,
    pub battery: Option<bool>,
}

const SCRIPT: &str = "$os=Get-CimInstance Win32_OperatingSystem; $cpu=Get-CimInstance Win32_Processor | Select-Object -First 1; \
[pscustomobject]@{ \
cpu=@{name=$cpu.Name;cores=[int]$cpu.NumberOfCores;threads=[int]$cpu.NumberOfLogicalProcessors;mhz=[int]$cpu.MaxClockSpeed}; \
gpus=@(Get-CimInstance Win32_VideoController | ForEach-Object {@{name=$_.Name;driver=$_.DriverVersion;vram=[int64]$_.AdapterRAM;hz=[int]$_.CurrentRefreshRate}}); \
disks=@(Get-CimInstance Win32_DiskDrive | ForEach-Object {@{model=$_.Model;type=[string]$_.MediaType;size=[int64]$_.Size;status=$_.Status}}); \
volumes=@(Get-CimInstance Win32_LogicalDisk -Filter 'DriveType=3' | ForEach-Object {@{name=$_.DeviceID;total=[int64]$_.Size;free=[int64]$_.FreeSpace;fs=$_.FileSystem}}); \
os=@{caption=$os.Caption;version=$os.Version;build=[string]$os.BuildNumber;boot=[int64]([DateTimeOffset]$os.LastBootUpTime).ToUnixTimeSeconds();totalKb=[int64]$os.TotalVisibleMemorySize;freeKb=[int64]$os.FreePhysicalMemory}; \
battery=[bool](Get-CimInstance Win32_Battery) } | ConvertTo-Json -Compress -Depth 5";

pub fn parse_snapshot(output: &str) -> Option<SystemSnapshot> {
    let (a, b) = (output.find('{')?, output.rfind('}')?);
    let mut v: serde_json::Value = serde_json::from_str(&output[a..=b]).ok()?;
    // PowerShell collapses single-element arrays in some versions: normalise to arrays.
    for k in ["gpus", "disks", "volumes"] {
        if let Some(x) = v.get_mut(k) { if x.is_object() { *x = serde_json::Value::Array(vec![x.take()]); } }
    }
    serde_json::from_value(v).ok()
}

pub fn scan(run: &dyn Runner) -> SystemSnapshot {
    let r = run.run(&powershell(SCRIPT), 60);
    if r.success() { parse_snapshot(&r.output).unwrap_or_default() } else { SystemSnapshot::default() }
}

impl SystemSnapshot {
    pub fn total_ram_bytes(&self) -> u64 { (self.os.total_kb.max(0) as u64) * 1024 }
    pub fn ram_load_percent(&self) -> i32 {
        if self.os.total_kb <= 0 { return 0; }
        (((self.os.total_kb - self.os.free_kb) as f64 / self.os.total_kb as f64) * 100.0).round() as i32
    }
    pub fn uptime_days(&self, now_unix: i64) -> f64 { if self.os.boot <= 0 { 0.0 } else { ((now_unix - self.os.boot).max(0) as f64) / 86_400.0 } }
    pub fn worst_disk_free_percent(&self) -> f64 {
        self.volumes.iter().filter(|v| v.total > 0).map(|v| v.free as f64 * 100.0 / v.total as f64).fold(100.0, f64::min)
    }

    /// When something cannot be determined a cautious value is used (e.g. unknown battery state = laptop).
    pub fn hardware_profile(&self, logical_cpus_fallback: u32) -> HardwareProfile {
        let discrete = self.gpus.iter().any(|g| {
            let n = g.name.to_lowercase();
            ["nvidia", "geforce", "radeon rx", "radeon pro", "arc a", "arc b"].iter().any(|k| n.contains(k))
        });
        let ssd = self.disks.iter().any(|d| {
            let s = format!("{} {}", d.model, d.ty).to_lowercase();
            ["ssd", "nvme", "solid"].iter().any(|k| s.contains(k))
        });
        HardwareProfile {
            cores: if self.cpu.cores > 0 { self.cpu.cores } else { logical_cpus_fallback },
            threads: if self.cpu.threads > 0 { self.cpu.threads } else { logical_cpus_fallback },
            ram_bytes: self.total_ram_bytes(),
            is_laptop: self.battery.unwrap_or(true),
            has_ssd: ssd, has_discrete_gpu: discrete,
            build: self.os.build.trim().parse().unwrap_or(0),
        }
    }

    pub fn describe(&self, now_unix: i64) -> String {
        let mut s = String::new();
        if !self.cpu.name.is_empty() { s += &format!("CPU: {} ({} cores / {} threads)\n", self.cpu.name.trim(), self.cpu.cores, self.cpu.threads); }
        for g in &self.gpus { s += &format!("GPU: {} | driver {} | {} Hz\n", g.name, g.driver, g.hz); }
        s += &format!("RAM: {:.0} GB\n", self.total_ram_bytes() as f64 / 1_073_741_824.0);
        if !self.os.caption.is_empty() { s += &format!("OS: {} (build {})\nUptime: {:.1} days\n", self.os.caption, self.os.build, self.uptime_days(now_unix)); }
        s.trim().to_string()
    }
}

/// Live CPU load and memory, via kernel32 (no polling cost when nobody is looking: the caller starts/stops sampling).
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveSample { pub cpu_percent: f64, pub ram_percent: i32, pub ram_used_bytes: u64, pub ram_total_bytes: u64 }

pub struct CpuSampler { last: Option<(u64, u64)> }

impl CpuSampler {
    pub fn new() -> Self { Self { last: None } }

    pub fn sample(&mut self) -> LiveSample {
        let (idle, total) = cpu_times();
        let cpu = match self.last {
            Some((pi, pt)) if total > pt => (1.0 - (idle - pi) as f64 / (total - pt) as f64).clamp(0.0, 1.0) * 100.0,
            _ => 0.0,
        };
        self.last = Some((idle, total));
        let (tot, avail, load) = memory();
        LiveSample { cpu_percent: cpu, ram_percent: load, ram_used_bytes: tot.saturating_sub(avail), ram_total_bytes: tot }
    }
}
impl Default for CpuSampler { fn default() -> Self { Self::new() } }

#[cfg(windows)]
mod ffi {
    #[repr(C)]
    pub struct FileTime { pub low: u32, pub high: u32 }
    #[repr(C)]
    pub struct MemoryStatusEx {
        pub length: u32, pub memory_load: u32, pub total_phys: u64, pub avail_phys: u64, pub total_page: u64,
        pub avail_page: u64, pub total_virtual: u64, pub avail_virtual: u64, pub avail_ext_virtual: u64,
    }
    #[link(name = "kernel32")]
    extern "system" {
        pub fn GetSystemTimes(idle: *mut FileTime, kernel: *mut FileTime, user: *mut FileTime) -> i32;
        pub fn GlobalMemoryStatusEx(buf: *mut MemoryStatusEx) -> i32;
    }
}

#[cfg(windows)]
fn cpu_times() -> (u64, u64) {
    use ffi::*;
    let mut i = FileTime { low: 0, high: 0 };
    let mut k = FileTime { low: 0, high: 0 };
    let mut u = FileTime { low: 0, high: 0 };
    // SAFETY: three valid, writable FILETIME structures.
    if unsafe { GetSystemTimes(&mut i, &mut k, &mut u) } == 0 { return (0, 0); }
    let f = |t: &FileTime| (u64::from(t.high) << 32) | u64::from(t.low);
    (f(&i), f(&k) + f(&u)) // kernel time includes idle
}

#[cfg(windows)]
/// (total bytes, available bytes, load percent)
pub fn memory() -> (u64, u64, i32) {
    use ffi::*;
    let mut m = MemoryStatusEx { length: std::mem::size_of::<MemoryStatusEx>() as u32, memory_load: 0, total_phys: 0, avail_phys: 0,
        total_page: 0, avail_page: 0, total_virtual: 0, avail_virtual: 0, avail_ext_virtual: 0 };
    // SAFETY: `m` is a valid MEMORYSTATUSEX with `length` set.
    if unsafe { GlobalMemoryStatusEx(&mut m) } == 0 { return (0, 0, 0); }
    (m.total_phys, m.avail_phys, m.memory_load as i32)
}

#[cfg(not(windows))]
fn cpu_times() -> (u64, u64) { (0, 0) }
#[cfg(not(windows))]
pub fn memory() -> (u64, u64, i32) { (0, 0, 0) }

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"cpu":{"name":"AMD Ryzen 7 5800X","cores":8,"threads":16,"mhz":3800},
    "gpus":{"name":"NVIDIA GeForce RTX 3070","driver":"31.0.15","vram":8589934592,"hz":144},
    "disks":[{"model":"Samsung SSD 980 PRO","type":"Fixed hard disk media","size":1000204886016,"status":"OK"}],
    "volumes":[{"name":"C:","total":1000000,"free":150000,"fs":"NTFS"},{"name":"D:","total":1000000,"free":600000,"fs":"NTFS"}],
    "os":{"caption":"Windows 11 Pro","version":"10.0.22631","build":"22631","boot":1794000000,"totalKb":33554432,"freeKb":16777216},
    "battery":false}"#;

    #[test]
    fn parses_a_desktop_snapshot_even_when_arrays_are_collapsed() {
        let s = parse_snapshot(SAMPLE).unwrap();
        assert_eq!(s.gpus.len(), 1);
        assert_eq!(s.volumes.len(), 2);
        assert_eq!(s.ram_load_percent(), 50);
        assert_eq!(s.worst_disk_free_percent(), 15.0);
        let hw = s.hardware_profile(4);
        assert!(!hw.is_laptop && hw.has_ssd && hw.has_discrete_gpu);
        assert_eq!((hw.cores, hw.threads, hw.build), (8, 16, 22631));
        assert_eq!(hw.ram_bytes, 32 * 1_073_741_824);
    }

    #[test]
    fn unknown_battery_state_is_treated_as_laptop() {
        let s = SystemSnapshot::default();
        let hw = s.hardware_profile(6);
        assert!(hw.is_laptop);
        assert_eq!((hw.cores, hw.threads), (6, 6));
        assert!(!hw.has_ssd && !hw.has_discrete_gpu);
    }

    #[test]
    fn uptime_is_computed_from_boot_time() {
        let s = parse_snapshot(SAMPLE).unwrap();
        assert!((s.uptime_days(1794000000 + 86400 * 3) - 3.0).abs() < 1e-9);
        assert_eq!(s.uptime_days(0), 0.0);
    }

    #[test]
    fn garbage_output_yields_none() { assert!(parse_snapshot("not json").is_none()); }
}
