//! Process identity: executable path, creation time and publisher.
//!
//! Identity matters for enforcement. A quota is attached to the executable
//! path, while a running instance is `(pid, creation time)` so PID reuse cannot
//! inherit another application's traffic.

use std::ffi::c_void;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
};
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowThreadProcessId, IsWindowVisible,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInfo {
    pub pid: u32,
    pub path: Option<String>,
    pub exe_name: String,
    /// Process creation time, in Unix milliseconds. Distinguishes new processes
    /// from reused PIDs.
    pub started_ms: i64,
    pub publisher: Option<String>,
}

impl ProcessInfo {
    pub fn app_key(&self, services: &[String]) -> String {
        netquota_core::identity::app_key(self.path.as_deref(), &self.exe_name, services)
    }
}

fn filetime_to_unix_ms(ft: FILETIME) -> i64 {
    let ticks = ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64;
    // 100-ns ticks since 1601-01-01.
    (ticks / 10_000) as i64 - 11_644_473_600_000
}

fn basename(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or(path).to_string()
}

pub fn process_info(pid: u32) -> Option<ProcessInfo> {
    if pid == 0 {
        return None;
    }
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let result = query_process(handle, pid);
        let _ = CloseHandle(handle);
        result
    }
}

unsafe fn query_process(handle: HANDLE, pid: u32) -> Option<ProcessInfo> {
    let mut buffer = [0u16; 32_768];
    let mut size = buffer.len() as u32;
    let path = match QueryFullProcessImageNameW(
        handle,
        PROCESS_NAME_WIN32,
        PWSTR(buffer.as_mut_ptr()),
        &mut size,
    ) {
        Ok(()) => Some(String::from_utf16_lossy(&buffer[..size as usize])),
        Err(_) => None,
    };
    let started_ms = read_start_time(handle).unwrap_or(0);
    let exe_name = path
        .as_deref()
        .map(basename)
        .unwrap_or_else(|| format!("pid-{pid}"));
    let publisher = path.as_deref().and_then(publisher_of);
    Some(ProcessInfo {
        pid,
        path,
        exe_name,
        started_ms,
        publisher,
    })
}

unsafe fn read_start_time(handle: HANDLE) -> Option<i64> {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user).ok()?;
    Some(filetime_to_unix_ms(creation))
}

/// PID owning the foreground window, when it is visible.
pub fn foreground_pid() -> Option<u32> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() || !IsWindowVisible(hwnd).as_bool() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        (pid != 0).then_some(pid)
    }
}

fn wide_cstr(value: &[u16]) -> String {
    let end = value.iter().position(|&c| c == 0).unwrap_or(value.len());
    String::from_utf16_lossy(&value[..end]).trim().to_string()
}

/// Company name from the executable's version resource. This is metadata the
/// file carries; it is not a signature check and is labelled as such in the UI.
fn publisher_of(path: &str) -> Option<String> {
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let pcw = PCWSTR(wide.as_ptr());
    unsafe {
        let size = GetFileVersionInfoSizeW(pcw, None);
        if size == 0 || size > 8 * 1024 * 1024 {
            return None;
        }
        let mut block = vec![0u8; size as usize];
        GetFileVersionInfoW(pcw, 0, size, block.as_mut_ptr() as *mut c_void).ok()?;

        let mut translations: *mut c_void = std::ptr::null_mut();
        let mut translations_len = 0u32;
        let translation_key: Vec<u16> = "\\VarFileInfo\\Translation\0".encode_utf16().collect();
        let ok = VerQueryValueW(
            block.as_ptr() as *const c_void,
            PCWSTR(translation_key.as_ptr()),
            &mut translations,
            &mut translations_len,
        );
        if ok.as_bool() && !translations.is_null() && translations_len >= 4 {
            let pairs = translations as *const u16;
            let count = (translations_len / 4) as usize;
            for i in 0..count.min(16) {
                let lang = std::ptr::read_unaligned(pairs.add(i * 2));
                let codepage = std::ptr::read_unaligned(pairs.add(i * 2 + 1));
                let query = format!("\\StringFileInfo\\{lang:04x}{codepage:04x}\\CompanyName\0");
                let wide_query: Vec<u16> = query.encode_utf16().collect();
                let mut value: *mut c_void = std::ptr::null_mut();
                let mut value_len = 0u32;
                if VerQueryValueW(
                    block.as_ptr() as *const c_void,
                    PCWSTR(wide_query.as_ptr()),
                    &mut value,
                    &mut value_len,
                )
                .as_bool()
                    && !value.is_null()
                    && value_len > 1
                {
                    let chars = std::slice::from_raw_parts(
                        value as *const u16,
                        value_len as usize / 2,
                    );
                    let name = wide_cstr(chars);
                    if !name.is_empty() {
                        return Some(name);
                    }
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_process_is_resolvable() {
        let info = process_info(std::process::id()).expect("self is queryable");
        assert!(info.exe_name.to_ascii_lowercase().ends_with(".exe"));
        assert!(info.started_ms > 1_500_000_000_000);
        assert!(info.path.is_some());
    }

    #[test]
    fn pid_zero_and_bogus_pids_return_none() {
        assert!(process_info(0).is_none());
        // Very high PIDs are not in use on a normal desktop.
        assert!(process_info(u32::MAX - 1).is_none());
    }

    #[test]
    fn same_pid_reports_stable_creation_time() {
        let pid = std::process::id();
        let a = process_info(pid).unwrap();
        let b = process_info(pid).unwrap();
        assert_eq!(a.started_ms, b.started_ms);
        assert_eq!(a.path, b.path);
    }

    #[test]
    fn filetime_conversion_matches_epoch_floor() {
        assert_eq!(filetime_to_unix_ms(FILETIME::default()), -11_644_473_600_000);
    }
}
