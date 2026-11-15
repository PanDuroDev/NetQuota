use serde::{Deserialize, Serialize};

/// Stable identity of an executable. The key is what policies are attached to,
/// so it survives process restarts and PID reuse.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AppIdentity {
    /// Stable policy key: normalised executable path, plus hosted service names
    /// for shared hosts such as `svchost.exe`.
    pub key: String,
    pub exe_name: String,
    pub path: Option<String>,
    pub publisher: Option<String>,
    /// Services hosted by this process, resolved from the Service Control Manager.
    pub services: Vec<String>,
}

impl AppIdentity {
    pub fn new(path: Option<&str>, exe_name: &str, services: Vec<String>) -> Self {
        Self {
            key: app_key(path, exe_name, &services),
            exe_name: exe_name.to_string(),
            path: path.map(|p| p.to_string()),
            publisher: None,
            services,
        }
    }

    /// Shared hosts are only distinguishable by the services they contain
    /// (`svchost.exe` instances are otherwise identical).
    pub fn is_shared_host(&self) -> bool {
        !self.services.is_empty()
    }

    pub fn display_name(&self) -> String {
        if self.services.is_empty() {
            self.exe_name.clone()
        } else {
            format!("{} ({})", self.exe_name, self.services.join(", "))
        }
    }
}

pub fn normalize_path(path: &str) -> String {
    path.trim().replace('/', "\\").to_lowercase()
}

pub fn app_key(path: Option<&str>, exe_name: &str, services: &[String]) -> String {
    let base = match path {
        Some(p) if !p.trim().is_empty() => normalize_path(p),
        _ => format!("name:{}", exe_name.trim().to_lowercase()),
    };
    if services.is_empty() {
        base
    } else {
        let mut names: Vec<String> = services.iter().map(|s| s.to_lowercase()).collect();
        names.sort();
        names.dedup();
        format!("{base}#{}", names.join("+"))
    }
}

/// A running process instance. `started_ms` (creation time) distinguishes a new
/// process from a reused PID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProcessRef {
    pub pid: u32,
    pub started_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_exe_in_different_folders_gets_different_keys() {
        let a = app_key(
            Some(r"C:\Program Files\App\app.exe"),
            "app.exe",
            &[],
        );
        let b = app_key(Some(r"D:\Tools\App\app.exe"), "app.exe", &[]);
        assert_ne!(a, b);
    }

    #[test]
    fn path_case_and_separators_are_normalised() {
        assert_eq!(
            app_key(Some(r"C:/Windows/System32/SVCHOST.EXE"), "svchost.exe", &[]),
            app_key(Some(r"c:\windows\system32\svchost.exe"), "svchost.exe", &[])
        );
    }

    #[test]
    fn svchost_instances_are_split_by_service_set() {
        let dns = app_key(
            Some(r"C:\Windows\System32\svchost.exe"),
            "svchost.exe",
            &["Dnscache".into()],
        );
        let dhcp = app_key(
            Some(r"C:\Windows\System32\svchost.exe"),
            "svchost.exe",
            &["Dhcp".into()],
        );
        assert_ne!(dns, dhcp);
        assert!(dns.contains("dnscache"));
    }

    #[test]
    fn service_order_does_not_change_the_key() {
        let a = app_key(
            Some(r"C:\x\svchost.exe"),
            "svchost.exe",
            &["B".into(), "A".into()],
        );
        let b = app_key(
            Some(r"C:\x\svchost.exe"),
            "svchost.exe",
            &["A".into(), "B".into()],
        );
        assert_eq!(a, b);
    }

    #[test]
    fn missing_path_falls_back_to_name_key() {
        let k = app_key(None, "mystery.exe", &[]);
        assert_eq!(k, "name:mystery.exe");
    }
}
