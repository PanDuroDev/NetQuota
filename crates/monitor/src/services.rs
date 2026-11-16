//! Maps PIDs to the Windows services they host. `svchost.exe` instances are
//! shared containers, so attribution needs the hosted service names rather than
//! the image name.

use std::collections::HashMap;

use windows::core::PCWSTR;
use windows::Win32::System::Services::{
    CloseServiceHandle, EnumServicesStatusExW, OpenSCManagerW, ENUM_SERVICE_STATUS_PROCESSW,
    SC_ENUM_PROCESS_INFO, SC_MANAGER_CONNECT, SC_MANAGER_ENUMERATE_SERVICE, SERVICE_STATE_ALL,
    SERVICE_WIN32,
};

const MAX_ROUNDS: usize = 8;

pub fn pid_service_map() -> HashMap<u32, Vec<String>> {
    let mut map: HashMap<u32, Vec<String>> = HashMap::new();
    unsafe {
        let scm = match OpenSCManagerW(
            PCWSTR::null(),
            PCWSTR::null(),
            SC_MANAGER_CONNECT | SC_MANAGER_ENUMERATE_SERVICE,
        ) {
            Ok(handle) => handle,
            Err(error) => {
                // Enumerating services needs only a connect right, so this fails
                // mainly on a heavily locked down machine.
                tracing::debug!(%error, "cannot open service control manager");
                return map;
            }
        };

        let mut resume = 0u32;
        for _ in 0..MAX_ROUNDS {
            let mut needed = 0u32;
            let mut returned = 0u32;
            // Probe for the required buffer size. ERROR_MORE_DATA is expected.
            let _ = EnumServicesStatusExW(
                scm,
                SC_ENUM_PROCESS_INFO,
                SERVICE_WIN32,
                SERVICE_STATE_ALL,
                None,
                &mut needed,
                &mut returned,
                Some(&mut resume),
                PCWSTR::null(),
            );
            if needed == 0 {
                break;
            }

            let words = (needed as usize).div_ceil(8) + 512;
            let mut buffer = vec![0u64; words];
            let bytes = std::slice::from_raw_parts_mut(
                buffer.as_mut_ptr() as *mut u8,
                buffer.len() * std::mem::size_of::<u64>(),
            );
            let mut needed2 = 0u32;
            let mut returned2 = 0u32;
            if EnumServicesStatusExW(
                scm,
                SC_ENUM_PROCESS_INFO,
                SERVICE_WIN32,
                SERVICE_STATE_ALL,
                Some(bytes),
                &mut needed2,
                &mut returned2,
                Some(&mut resume),
                PCWSTR::null(),
            )
            .is_err()
            {
                break;
            }

            let entries = std::slice::from_raw_parts(
                buffer.as_ptr() as *const ENUM_SERVICE_STATUS_PROCESSW,
                returned2 as usize,
            );
            for entry in entries {
                let name = if entry.lpServiceName.is_null() {
                    continue;
                } else {
                    entry.lpServiceName.to_string().unwrap_or_default()
                };
                let pid = entry.ServiceStatusProcess.dwProcessId;
                if pid != 0 && !name.is_empty() {
                    map.entry(pid).or_default().push(name);
                }
            }
            if returned2 == 0 {
                break;
            }
        }
        let _ = CloseServiceHandle(scm);
    }
    for names in map.values_mut() {
        names.sort();
        names.dedup();
    }
    map
}

/// Service names hosted by one PID, sorted for a stable application key.
pub fn services_for(map: &HashMap<u32, Vec<String>>, pid: u32) -> Vec<String> {
    map.get(&pid).cloned().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enumeration_is_consistent_and_bounded() {
        let map = pid_service_map();
        // A normal Windows machine has dozens of service-hosting processes. A
        // locked down CI machine may legitimately return nothing.
        assert!(map.len() < 4096);
        for (pid, names) in &map {
            assert_ne!(*pid, 0);
            assert!(!names.is_empty());
            let mut sorted = names.clone();
            sorted.sort();
            assert_eq!(&sorted, names);
        }
    }
}
