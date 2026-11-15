//! Wire protocol for the NetQuota service named pipe and for the Chrome native
//! messaging bridge. Frames are a 4-byte little-endian length followed by JSON.

pub mod wire;

use netquota_core::policy::AppPolicy;
use netquota_core::profile::ProfileKind;
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u16 = 1;
/// Hard ceiling for a single IPC frame. Snapshots stay well below this.
pub const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    Desktop,
    ChromeExtension,
    Cli,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Hello {
        version: u16,
        client: ClientKind,
        /// Present for the extension; must match the allow-listed extension id.
        extension_id: Option<String>,
    },
    Ping,
    Snapshot,
    ListApps,
    SetPolicy {
        policy: AppPolicy,
    },
    GrantCredit {
        app_key: String,
        extra_bytes: u64,
        duration_secs: Option<u64>,
    },
    SetPaused {
        app_key: String,
        paused: bool,
    },
    BlockNow {
        app_key: String,
    },
    Unblock {
        app_key: String,
    },
    ApplyProfile {
        profile: ProfileKind,
    },
    SiteUsage {
        site: String,
        down: u64,
        up: u64,
        active_secs: u64,
    },
    ListSites,
    SetSitePolicy {
        site: String,
        limit_bytes: Option<u64>,
        blocked: bool,
    },
    ListEvents {
        limit: u32,
    },
    /// Remove every firewall rule NetQuota owns. Always available.
    EmergencyRestore,
    Shutdown,
}

impl Request {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Request::Hello { version, .. } => {
                if *version != PROTOCOL_VERSION {
                    return Err(format!(
                        "protocol version mismatch: client {version}, service {PROTOCOL_VERSION}"
                    ));
                }
            }
            Request::SetPolicy { policy } => policy.validate()?,
            Request::GrantCredit {
                app_key,
                extra_bytes,
                duration_secs,
            } => {
                validate_key(app_key)?;
                if *extra_bytes == 0 || *extra_bytes > 100_000_000_000_000 {
                    return Err("extra_bytes out of range".into());
                }
                if let Some(secs) = duration_secs {
                    if *secs == 0 || *secs > 31 * 24 * 3600 {
                        return Err("duration_secs out of range".into());
                    }
                }
            }
            Request::SetPaused { app_key, .. }
            | Request::BlockNow { app_key }
            | Request::Unblock { app_key } => validate_key(app_key)?,
            Request::SiteUsage {
                site, down, up, ..
            } => {
                if !is_valid_host(site) {
                    return Err("invalid site".into());
                }
                if *down > 10_000_000_000_000 || *up > 10_000_000_000_000 {
                    return Err("site usage out of range".into());
                }
            }
            Request::SetSitePolicy { site, .. } => {
                if !is_valid_host(site) {
                    return Err("invalid site".into());
                }
            }
            Request::ListEvents { limit } => {
                if *limit > 1_000 {
                    return Err("limit too large".into());
                }
            }
            _ => {}
        }
        Ok(())
    }
}

/// Hostnames only: no scheme, no path, no wildcard. Prevents a compromised
/// extension from storing strings that later become firewall or DNR input.
pub fn is_valid_host(host: &str) -> bool {
    if host.is_empty()
        || host.len() > 253
        || host.starts_with('.')
        || host.ends_with('.')
        || !host.is_ascii()
    {
        return false;
    }
    host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && label
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            && !label.starts_with('-')
            && !label.ends_with('-')
    })
}

fn validate_key(key: &str) -> Result<(), String> {
    if key.is_empty() || key.len() > 1_024 {
        return Err("invalid app key".into());
    }
    if key.contains(['\0', '\n', '\r']) {
        return Err("invalid characters in app key".into());
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Hello {
        version: u16,
        service_version: String,
        /// False when the process is not elevated, so enforcement is unavailable.
        enforcement_ready: bool,
        monitoring: MonitoringStatus,
    },
    Pong,
    Snapshot {
        snapshot: wire::Snapshot,
    },
    Apps {
        apps: Vec<wire::AppUsage>,
    },
    PolicyOk,
    Sites {
        sites: Vec<wire::SiteUsage>,
    },
    Events {
        events: Vec<wire::EventRecord>,
    },
    Restored {
        removed_rules: u32,
    },
    Error {
        code: ErrorCode,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitoringStatus {
    Running,
    Degraded,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadRequest,
    Unauthorized,
    Unsupported,
    NotFound,
    Internal,
}

/// 4-byte little-endian length prefix followed by UTF-8 JSON.
pub fn encode_frame<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    let json = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    if json.len() > MAX_FRAME_BYTES {
        return Err("frame too large".into());
    }
    let mut out = Vec::with_capacity(json.len() + 4);
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&json);
    Ok(out)
}

/// Split one frame from a buffer. Returns `None` when more bytes are needed.
pub fn take_frame(buffer: &mut Vec<u8>) -> Result<Option<Vec<u8>>, String> {
    if buffer.len() < 4 {
        return Ok(None);
    }
    let len = u32::from_le_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]) as usize;
    if len > MAX_FRAME_BYTES {
        return Err(format!("frame length {len} exceeds limit"));
    }
    if buffer.len() < 4 + len {
        return Ok(None);
    }
    let body: Vec<u8> = buffer[4..4 + len].to_vec();
    buffer.drain(..4 + len);
    Ok(Some(body))
}

pub fn parse_request(frame: &[u8]) -> Result<Request, String> {
    let request: Request = serde_json::from_slice(frame).map_err(|e| format!("bad request: {e}"))?;
    request.validate()?;
    Ok(request)
}

pub fn parse_response(frame: &[u8]) -> Result<Response, String> {
    serde_json::from_slice(frame).map_err(|e| format!("bad response: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use netquota_core::quota::Limit;

    #[test]
    fn frame_round_trips() {
        let req = Request::Ping;
        let mut bytes = encode_frame(&req).unwrap();
        let frame = take_frame(&mut bytes).unwrap().unwrap();
        assert!(bytes.is_empty());
        assert_eq!(parse_request(&frame).unwrap(), req);
    }

    #[test]
    fn partial_frame_waits_for_more_data() {
        let mut bytes = encode_frame(&Request::Snapshot).unwrap();
        let tail = bytes.split_off(bytes.len() - 2);
        assert!(take_frame(&mut bytes).unwrap().is_none());
        bytes.extend_from_slice(&tail);
        assert!(take_frame(&mut bytes).unwrap().is_some());
    }

    #[test]
    fn oversized_frame_is_rejected_without_allocating() {
        let mut bytes = ((MAX_FRAME_BYTES as u32) + 1).to_le_bytes().to_vec();
        assert!(take_frame(&mut bytes).is_err());
    }

    #[test]
    fn invalid_json_is_rejected() {
        assert!(parse_request(b"{\"type\":\"nope\"}").is_err());
        assert!(parse_request(b"not json").is_err());
    }

    #[test]
    fn version_mismatch_is_rejected() {
        let req = Request::Hello {
            version: PROTOCOL_VERSION + 1,
            client: ClientKind::Desktop,
            extension_id: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn host_validation_blocks_injection() {
        assert!(is_valid_host("youtube.com"));
        assert!(is_valid_host("sub.domain.co.uk"));
        assert!(!is_valid_host("https://youtube.com"));
        assert!(!is_valid_host("youtube.com/../x"));
        assert!(!is_valid_host(""));
        assert!(!is_valid_host("-bad.com"));
        assert!(!is_valid_host("*.youtube.com"));
        assert!(!is_valid_host("exämple.com"));
    }

    #[test]
    fn absurd_credit_is_rejected() {
        let req = Request::GrantCredit {
            app_key: "a".into(),
            extra_bytes: u64::MAX,
            duration_secs: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn zero_credit_is_rejected() {
        let req = Request::GrantCredit {
            app_key: "a".into(),
            extra_bytes: 0,
            duration_secs: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn policy_is_validated_over_the_wire() {
        let mut policy = AppPolicy::new("c:\\app.exe");
        policy.period = netquota_core::PeriodKind::CustomDays { days: 0 };
        let req = Request::SetPolicy { policy };
        assert!(req.validate().is_err());
    }

    #[test]
    fn valid_policy_passes() {
        let mut policy = AppPolicy::new("c:\\app.exe");
        policy.limit = Limit {
            total: Some(1_000),
            download: None,
            upload: Some(500),
        };
        policy.upload_kbps = Some(50);
        assert!(Request::SetPolicy { policy }.validate().is_ok());
    }
}
