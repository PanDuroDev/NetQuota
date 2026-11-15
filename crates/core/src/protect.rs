use crate::identity::AppIdentity;

/// Processes that must never be blocked by default. Blocking these can cut the
/// machine off from DNS, DHCP, domain services or security updates.
const PROTECTED_EXE: &[&str] = &[
    "system",
    "registry",
    "memcompression",
    "smss.exe",
    "csrss.exe",
    "wininit.exe",
    "winlogon.exe",
    "services.exe",
    "lsass.exe",
    "lsm.exe",
    "svchost.exe", // always evaluated per hosted service below
    "msmpeng.exe",
    "mpdefendercoreservice.exe",
    "securityhealthservice.exe",
    "securityhealthsystray.exe",
    "smartscreen.exe",
    "dns.exe",
    "logonui.exe",
];

/// Windows services that own critical networking. Matched case-insensitively.
const PROTECTED_SERVICE: &[&str] = &[
    "dnscache",
    "dhcp",
    "nlasvc",
    "netprofm",
    "nsi",
    "wlansvc",
    "wcmSvc",
    "bits",
    "wuauserv",
    "cryptsvc",
    "trustedinstaller",
    "winmgmt",
    "mpssvc",
    "bfe",
    "windefend",
    "sense",
    "eventlog",
    "rpcss",
];

pub enum Protection {
    /// Hard-blocked by the quota engine; no user override is applied silently.
    Protected(String),
    /// Allowed, but the UI must warn before applying a restriction.
    RequiresConfirmation(String),
    None,
}

pub fn protection_for(app: &AppIdentity) -> Protection {
    let exe = app.exe_name.to_ascii_lowercase();
    let services: Vec<String> = app.services.iter().map(|s| s.to_ascii_lowercase()).collect();

    if let Some(svc) = services
        .iter()
        .find(|s| PROTECTED_SERVICE.iter().any(|p| p.eq_ignore_ascii_case(s)))
    {
        return Protection::Protected(format!(
            "critical Windows networking service: {svc}"
        ));
    }

    if exe == "svchost.exe" || exe == "services.exe" || exe == "lsass.exe" {
        return Protection::Protected(format!("core Windows process: {exe}"));
    }

    if PROTECTED_EXE.iter().any(|p| p.eq_ignore_ascii_case(&exe)) {
        return Protection::Protected(format!("protected system process: {exe}"));
    }

    if is_system_binary(app) {
        return Protection::RequiresConfirmation(
            "system binary; blocking may break Windows features".into(),
        );
    }

    Protection::None
}

pub fn is_system_binary(app: &AppIdentity) -> bool {
    let Some(path) = &app.path else {
        return false;
    };
    let p = path.to_ascii_lowercase();
    p.starts_with(r"c:\windows\system32\")
        || p.starts_with(r"c:\windows\syswow64\")
        || p.starts_with(r"c:\windows\winsxs\")
        || p.starts_with(r"c:\windows\")
}

/// System-critical traffic that the UI should show as `System` rather than as an
/// ordinary application.
pub fn is_system_owned(app: &AppIdentity) -> bool {
    is_system_binary(app) || app.exe_name.eq_ignore_ascii_case("system")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(path: &str, exe: &str, services: &[&str]) -> AppIdentity {
        AppIdentity::new(
            Some(path),
            exe,
            services.iter().map(|s| s.to_string()).collect(),
        )
    }

    #[test]
    fn dns_service_is_protected() {
        let a = app(
            r"C:\Windows\System32\svchost.exe",
            "svchost.exe",
            &["Dnscache"],
        );
        assert!(matches!(protection_for(&a), Protection::Protected(_)));
    }

    #[test]
    fn user_app_is_not_protected() {
        let a = app(r"C:\Program Files\Chrome\chrome.exe", "chrome.exe", &[]);
        assert!(matches!(protection_for(&a), Protection::None));
    }

    #[test]
    fn other_system32_binary_requires_confirmation() {
        let a = app(r"C:\Windows\System32\notepad.exe", "notepad.exe", &[]);
        assert!(matches!(
            protection_for(&a),
            Protection::RequiresConfirmation(_)
        ));
    }

    #[test]
    fn non_critical_svchost_is_still_protected_by_default() {
        let a = app(
            r"C:\Windows\System32\svchost.exe",
            "svchost.exe",
            &["SomeThirdPartyService"],
        );
        assert!(matches!(protection_for(&a), Protection::Protected(_)));
    }
}
