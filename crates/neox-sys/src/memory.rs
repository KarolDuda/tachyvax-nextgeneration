//! Memory Cleaner: empties working sets and purges the standby list through documented-by-use NT calls.
//! Needs administrator rights. Nothing is stored or changed permanently: Windows simply re-caches what it needs.

/// Windows commands for NtSetSystemInformation(SystemMemoryListInformation).
pub const EMPTY_WORKING_SETS: i32 = 2;
pub const PURGE_STANDBY_LIST: i32 = 4;

#[cfg(windows)]
pub fn clean() -> Result<(), String> {
    use core::ffi::c_void;
    #[repr(C)] struct Luid { low: u32, high: i32 }
    #[repr(C)] struct TokenPrivileges { count: u32, luid: Luid, attrs: u32 }
    #[link(name = "advapi32")]
    extern "system" {
        fn OpenProcessToken(process: *mut c_void, access: u32, token: *mut *mut c_void) -> i32;
        fn LookupPrivilegeValueW(system: *const u16, name: *const u16, luid: *mut Luid) -> i32;
        fn AdjustTokenPrivileges(token: *mut c_void, disable_all: i32, new: *const TokenPrivileges, len: u32, prev: *mut c_void, ret: *mut u32) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" { fn GetCurrentProcess() -> *mut c_void; fn CloseHandle(h: *mut c_void) -> i32; fn GetLastError() -> u32; }
    #[link(name = "ntdll")]
    extern "system" { fn NtSetSystemInformation(class: u32, info: *mut c_void, len: u32) -> i32; }

    const TOKEN_ADJUST_PRIVILEGES: u32 = 0x20; const TOKEN_QUERY: u32 = 0x8; const SE_PRIVILEGE_ENABLED: u32 = 2;
    const SYSTEM_MEMORY_LIST_INFORMATION: u32 = 80;
    unsafe {
        let mut token = core::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut token) == 0 { return Err("Could not open the process token.".into()); }
        let name: Vec<u16> = "SeProfileSingleProcessPrivilege\0".encode_utf16().collect();
        let mut luid = Luid { low: 0, high: 0 };
        let ok = LookupPrivilegeValueW(core::ptr::null(), name.as_ptr(), &mut luid) != 0;
        let tp = TokenPrivileges { count: 1, luid, attrs: SE_PRIVILEGE_ENABLED };
        let adjusted = ok && AdjustTokenPrivileges(token, 0, &tp, 0, core::ptr::null_mut(), core::ptr::null_mut()) != 0 && GetLastError() == 0;
        CloseHandle(token);
        if !adjusted { return Err("Administrator rights are required.".into()); }
        for mut cmd in [EMPTY_WORKING_SETS, PURGE_STANDBY_LIST] {
            let status = NtSetSystemInformation(SYSTEM_MEMORY_LIST_INFORMATION, &mut cmd as *mut i32 as *mut c_void, 4);
            if status < 0 { return Err(format!("Windows refused the memory command (status 0x{:08X}).", status as u32)); }
        }
    }
    Ok(())
}
#[cfg(not(windows))]
pub fn clean() -> Result<(), String> { Err("Memory Cleaner is only available on Windows.".into()) }

/// Windows session start as unix seconds, from the tick counter (resets on a real restart, not after a Fast Startup shutdown).
#[cfg(windows)]
pub fn boot_unix(now: i64) -> i64 {
    #[link(name = "kernel32")]
    extern "system" { fn GetTickCount64() -> u64; }
    now - (unsafe { GetTickCount64() } / 1000) as i64
}
#[cfg(not(windows))]
pub fn boot_unix(_now: i64) -> i64 { 0 }

#[cfg(test)]
mod tests {
    #[test]
    fn commands_match_the_documented_values() { assert_eq!((super::EMPTY_WORKING_SETS, super::PURGE_STANDBY_LIST), (2, 4)); }
    #[cfg(not(windows))]
    #[test]
    fn non_windows_reports_unavailable() { assert!(super::clean().is_err()); assert_eq!(super::boot_unix(1000), 0); }
}
