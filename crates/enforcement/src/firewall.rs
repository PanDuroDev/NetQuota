//! Windows Firewall integration through the `INetFwPolicy2` COM API.
//!
//! NetQuota never edits rules it does not own. Each rule it creates is recorded
//! in a manifest on disk, and only names present in that manifest are removed.
//! The manifest is what makes `restore_all` safe: a rule added by the user or by
//! another program can never be matched by accident.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use windows::core::BSTR;
use windows::Win32::Foundation::{VARIANT_BOOL, VARIANT_TRUE};
use windows::Win32::NetworkManagement::WindowsFirewall::{
    INetFwPolicy2, INetFwRule, INetFwRules, NetFwPolicy2, NetFwRule, NET_FW_ACTION_BLOCK,
    NET_FW_IP_PROTOCOL_ANY, NET_FW_PROFILE2_ALL, NET_FW_RULE_DIR_OUT,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};

pub const GROUP: &str = "NetQuota";
pub const RULE_PREFIX: &str = "NetQuota\\Block\\";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockState {
    Blocked,
    NotBlocked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnedRule {
    pub name: String,
    pub application: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Manifest {
    rules: Vec<OwnedRule>,
}

pub struct Firewall {
    rules: INetFwRules,
    manifest_path: PathBuf,
    manifest: Manifest,
    dry_run: bool,
}

impl Firewall {
    /// Opens the local firewall policy. Reading needs no elevation; adding and
    /// removing rules does, so the service runs as LocalSystem.
    pub fn connect(manifest_path: impl Into<PathBuf>, dry_run: bool) -> Result<Self> {
        unsafe {
            // S_FALSE means COM was already initialized on this thread; both are
            // fine and must not be balanced with CoUninitialize here.
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let policy: INetFwPolicy2 =
                CoCreateInstance(&NetFwPolicy2, None, CLSCTX_INPROC_SERVER)
                    .context("creating INetFwPolicy2 (is the Windows Firewall service running?)")?;
            let rules = policy.Rules().context("reading firewall rules")?;
            let manifest_path = manifest_path.into();
            let manifest = load_manifest(&manifest_path);
            Ok(Self {
                rules,
                manifest_path,
                manifest,
                dry_run,
            })
        }
    }

    pub fn block(&mut self, application: &Path) -> Result<String> {
        let application = validate_application(application)?;
        let name = rule_name(&application);

        let existing = self.manifest.rules.iter().any(|r| r.name == name);
        if existing && self.rule_active(&name)? {
            return Ok(name);
        }
        if self.dry_run {
            self.record(name.clone(), application);
            return Ok(name);
        }
        if existing {
            // Manifest entry without a live rule (rule was deleted externally).
            self.manifest.rules.retain(|r| r.name != name);
        }

        let rule: INetFwRule = unsafe {
            CoCreateInstance(&NetFwRule, None, CLSCTX_INPROC_SERVER).context("creating INetFwRule")?
        };
        unsafe {
            rule.SetName(&BSTR::from(name.as_str()))?;
            rule.SetGrouping(&BSTR::from(GROUP))?;
            rule.SetDescription(&BSTR::from(format!(
                "NetQuota quota enforcement for {}. Managed automatically; remove through NetQuota.",
                application.display()
            )))?;
            rule.SetApplicationName(&BSTR::from(application.to_string_lossy().as_ref()))?;
            rule.SetAction(NET_FW_ACTION_BLOCK)?;
            rule.SetDirection(NET_FW_RULE_DIR_OUT)?;
            rule.SetProtocol(NET_FW_IP_PROTOCOL_ANY.0)?;
            rule.SetProfiles(NET_FW_PROFILE2_ALL.0)?;
            rule.SetEnabled(VARIANT_TRUE)?;
            rule.SetEdgeTraversal(VARIANT_BOOL(0))?;
            self.rules.Add(&rule).with_context(|| {
                format!("adding firewall rule for {}", application.display())
            })?;
        }
        self.record(name.clone(), application);
        Ok(name)
    }

    pub fn unblock(&mut self, application: &Path) -> Result<bool> {
        let application = validate_application(application)?;
        let name = rule_name(&application);
        self.remove_rule(&name)
    }

    /// Removes every rule NetQuota recorded. Returns how many were removed.
    pub fn restore_all(&mut self) -> Result<usize> {
        let names: Vec<String> = self.manifest.rules.iter().map(|r| r.name.clone()).collect();
        let mut removed = 0;
        for name in names {
            if self.remove_rule(&name)? {
                removed += 1;
            } else {
                // Already gone; just forget it.
                self.manifest.rules.retain(|r| r.name != name);
                self.save()?;
            }
        }
        if self.manifest.rules.is_empty() {
            let _ = std::fs::remove_file(&self.manifest_path);
        }
        Ok(removed)
    }

    pub fn state(&self, application: &Path) -> Result<BlockState> {
        let application = validate_application(application)?;
        let name = rule_name(&application);
        if self.rule_active(&name)? {
            Ok(BlockState::Blocked)
        } else {
            Ok(BlockState::NotBlocked)
        }
    }

    /// False in dry-run mode: rules are simulated and no traffic is affected.
    pub fn is_live(&self) -> bool {
        !self.dry_run
    }

    pub fn list_owned(&self) -> Vec<OwnedRule> {
        self.manifest.rules.clone()
    }

    fn remove_rule(&mut self, name: &str) -> Result<bool> {
        let recorded = self.manifest.rules.iter().any(|r| r.name == name);
        if !recorded {
            // Only rules NetQuota recorded in its manifest are ever removed.
            return Ok(false);
        }
        if self.dry_run {
            self.manifest.rules.retain(|r| r.name != name);
            return Ok(true);
        }
        match unsafe { self.rules.Remove(&BSTR::from(name)) } {
            Ok(()) => {
                self.manifest.rules.retain(|r| r.name != name);
                self.save()?;
                Ok(true)
            }
            Err(error) => {
                // 0x80070490 (element not found) means it is already gone.
                if error.code().0 as u32 == 0x8007_0490 {
                    self.manifest.rules.retain(|r| r.name != name);
                    self.save()?;
                    Ok(false)
                } else {
                    Err(error).context("removing firewall rule")
                }
            }
        }
    }

    fn rule_active(&self, name: &str) -> Result<bool> {
        if self.dry_run {
            return Ok(self.manifest.rules.iter().any(|r| r.name == name));
        }
        match unsafe { self.rules.Item(&BSTR::from(name)) } {
            Ok(rule) => {
                let group = unsafe { rule.Grouping() }
                    .map(|g| g.to_string())
                    .unwrap_or_default();
                let enabled = unsafe { rule.Enabled() }
                    .map(|v| v.as_bool())
                    .unwrap_or(false);
                Ok(group == GROUP && enabled)
            }
            Err(_) => Ok(false),
        }
    }

    fn record(&mut self, name: String, application: PathBuf) {
        self.manifest.rules.retain(|r| r.name != name);
        self.manifest.rules.push(OwnedRule {
            name,
            application: application.to_string_lossy().into_owned(),
        });
        let _ = self.save();
    }

    fn save(&self) -> Result<()> {
        if self.dry_run {
            return Ok(());
        }
        if let Some(parent) = self.manifest_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let temporary = self.manifest_path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec_pretty(&self.manifest)?)
            .with_context(|| format!("writing {}", temporary.display()))?;
        std::fs::rename(&temporary, &self.manifest_path)
            .with_context(|| format!("replacing {}", self.manifest_path.display()))?;
        Ok(())
    }
}

fn load_manifest(path: &Path) -> Manifest {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => Manifest::default(),
    }
}

fn validate_application(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err(anyhow!("application path must be absolute: {}", path.display()));
    }
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if !extension.eq_ignore_ascii_case("exe") {
        return Err(anyhow!("only .exe files can be blocked: {}", path.display()));
    }
    if path.to_string_lossy().contains('"') || path.to_string_lossy().contains('\n') {
        return Err(anyhow!("application path contains invalid characters"));
    }
    // A deleted executable can still leave a blocked rule behind, so a missing
    // file is allowed: recovery must be possible after an uninstall.
    Ok(path.to_path_buf())
}

/// Stable per-executable rule name, so blocking the same path twice is
/// idempotent and unblock needs no lookup table.
pub fn rule_name(application: &Path) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in application.to_string_lossy().to_lowercase().bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{RULE_PREFIX}{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_manifest(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("netquota-test-{}-{}", std::process::id(), name))
            .join("firewall.json")
    }

    #[test]
    fn rule_names_are_stable_and_path_specific() {
        let a = Path::new("C:\\Program Files\\App\\app.exe");
        let b = Path::new("C:\\Program Files\\Other\\app.exe");
        assert_eq!(rule_name(a), rule_name(a));
        assert_ne!(rule_name(a), rule_name(b));
        assert!(rule_name(a).starts_with(RULE_PREFIX));
        // Case-insensitive on purpose: Windows paths are.
        assert_eq!(rule_name(a), rule_name(Path::new("c:\\program files\\app\\APP.EXE")));
    }

    #[test]
    fn dry_run_never_touches_the_firewall() {
        let manifest = temp_manifest("dry");
        let _ = std::fs::remove_dir_all(manifest.parent().unwrap());
        let mut firewall = Firewall::connect(&manifest, true).expect("COM available");
        let app = std::env::current_exe().unwrap();
        assert_eq!(firewall.state(&app).unwrap(), BlockState::NotBlocked);
        let name = firewall.block(&app).unwrap();
        assert_eq!(firewall.state(&app).unwrap(), BlockState::Blocked);
        assert_eq!(firewall.list_owned().len(), 1);
        assert!(firewall.unblock(&app).unwrap());
        assert_eq!(firewall.state(&app).unwrap(), BlockState::NotBlocked);
        assert!(firewall.list_owned().is_empty());
        // Nothing was persisted in dry-run mode.
        assert!(!manifest.exists());
        assert_eq!(name, rule_name(&app));
    }

    #[test]
    fn unowned_rule_is_never_removed() {
        let manifest = temp_manifest("unowned");
        let mut firewall = Firewall::connect(&manifest, true).unwrap();
        // A name that is not in the manifest is skipped, never removed.
        assert!(!firewall.remove_rule("SomeOtherVendor\\Rule").unwrap());
    }

    #[test]
    fn path_validation_rejects_relative_and_non_exe() {
        assert!(validate_application(Path::new("app.exe")).is_err());
        assert!(validate_application(Path::new("C:\\Windows\\notepad.dll")).is_err());
        assert!(validate_application(Path::new("C:\\Windows\\notepad.exe")).is_ok());
    }

    #[test]
    fn real_firewall_rules_are_readable_without_elevation() {
        let firewall = Firewall::connect(temp_manifest("read"), false).expect("policy readable");
        // This is a real query against the local policy store.
        let count = unsafe { firewall.rules.Count() }.expect("rule count");
        assert!(count >= 0);
    }
}
