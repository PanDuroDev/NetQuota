//! Global interface counters from the IP Helper API. This provider is readable
//! without elevation, so it also works in the desktop app when the service is
//! not installed, and it serves as a sanity check on per-process totals.

use std::ffi::c_void;

use anyhow::{anyhow, Result};
use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_ROW2, MIB_IF_TABLE2};
use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;

pub const IF_TYPE_ETHERNET_CSMACD: u32 = 6;
pub const IF_TYPE_IEEE80211: u32 = 71;
pub const IF_TYPE_SOFTWARE_LOOPBACK: u32 = 24;
pub const IF_TYPE_TUNNEL: u32 = 131;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum InterfaceKind {
    Ethernet,
    Wifi,
    Vpn,
    Other,
}

impl InterfaceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            InterfaceKind::Ethernet => "ethernet",
            InterfaceKind::Wifi => "wifi",
            InterfaceKind::Vpn => "vpn",
            InterfaceKind::Other => "other",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InterfaceStats {
    pub index: u32,
    pub alias: String,
    pub kind: InterfaceKind,
    pub in_octets: u64,
    pub out_octets: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GlobalSnapshot {
    pub down: u64,
    pub up: u64,
    pub interfaces: Vec<InterfaceStats>,
    pub primary_alias: Option<String>,
    pub primary_kind: Option<InterfaceKind>,
    pub vpn_present: bool,
}

fn wide_to_string(value: &[u16]) -> String {
    let end = value.iter().position(|&c| c == 0).unwrap_or(value.len());
    String::from_utf16_lossy(&value[..end])
}

/// Windows exposes each physical adapter once per NDIS filter layer ("-WFP
/// Native MAC Layer", "-QoS Packet Scheduler"). Those rows repeat the same
/// octet counters, so summing them multiplies the real traffic by the number of
/// filters. The layered rows are skipped; the base row keeps the counters.
///
/// shortcut: name/description matching, upgrade to `GetAdaptersAddresses`
/// interface-type filtering if a driver ever hides the filter layers.
fn is_layered(row: &MIB_IF_ROW2) -> bool {
    let alias = wide_to_string(&row.Alias).to_ascii_lowercase();
    let description = wide_to_string(&row.Description).to_ascii_lowercase();
    alias.contains("-wfp ") || alias.contains("-qos packet scheduler")
        || description.contains("wfp native") || description.contains("qos packet scheduler")
}

fn kind_for(row: &MIB_IF_ROW2) -> InterfaceKind {
    match row.Type {
        IF_TYPE_IEEE80211 => InterfaceKind::Wifi,
        IF_TYPE_TUNNEL => InterfaceKind::Vpn,
        IF_TYPE_ETHERNET_CSMACD => InterfaceKind::Ethernet,
        _ => {
            let desc = wide_to_string(&row.Description).to_ascii_lowercase();
            if desc.contains("vpn")
                || desc.contains("wireguard")
                || desc.contains("tailscale")
                || desc.contains("tap-")
                || desc.starts_with("tun")
            {
                InterfaceKind::Vpn
            } else if desc.contains("wi-fi") || desc.contains("wireless") {
                InterfaceKind::Wifi
            } else {
                InterfaceKind::Other
            }
        }
    }
}

pub fn read_interfaces() -> Result<Vec<InterfaceStats>> {
    unsafe {
        let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        let status = GetIfTable2(&mut table);
        if status.0 != 0 || table.is_null() {
            return Err(anyhow!("GetIfTable2 failed with code {}", status.0));
        }
        let count = (*table).NumEntries as usize;
        // The table is a variable-length array; the header declares one row.
        let rows = std::slice::from_raw_parts((*table).Table.as_ptr(), count);
        let mut out = Vec::with_capacity(count);
        for row in rows {
            if row.OperStatus != IfOperStatusUp
                || row.Type == IF_TYPE_SOFTWARE_LOOPBACK
                || is_layered(row)
            {
                continue;
            }
            let alias = wide_to_string(&row.Alias);
            out.push(InterfaceStats {
                index: row.InterfaceIndex,
                alias: if alias.is_empty() {
                    wide_to_string(&row.Description)
                } else {
                    alias
                },
                kind: kind_for(row),
                in_octets: row.InOctets,
                out_octets: row.OutOctets,
            });
        }
        FreeMibTable(table as *const c_void);
        Ok(out)
    }
}

/// Sum interface counters once. Tunnel interfaces are counted separately so a
/// VPN is not double-counted together with the physical adapter underneath it.
pub fn snapshot() -> Result<GlobalSnapshot> {
    let interfaces = read_interfaces()?;
    let physical: Vec<&InterfaceStats> = interfaces
        .iter()
        .filter(|i| i.kind != InterfaceKind::Vpn)
        .collect();
    let chosen: Vec<&InterfaceStats> = if physical.is_empty() {
        interfaces.iter().collect()
    } else {
        physical
    };

    let down = chosen.iter().fold(0u64, |a, i| a.saturating_add(i.in_octets));
    let up = chosen
        .iter()
        .fold(0u64, |a, i| a.saturating_add(i.out_octets));
    let primary = chosen
        .iter()
        .max_by_key(|i| i.in_octets.saturating_add(i.out_octets));

    Ok(GlobalSnapshot {
        down,
        up,
        vpn_present: interfaces.iter().any(|i| i.kind == InterfaceKind::Vpn),
        primary_alias: primary.map(|i| i.alias.clone()),
        primary_kind: primary.map(|i| i.kind),
        interfaces,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_strings_stop_at_nul() {
        let mut buf = [0u16; 8];
        for (i, c) in "Wi-Fi".encode_utf16().enumerate() {
            buf[i] = c;
        }
        assert_eq!(wide_to_string(&buf), "Wi-Fi");
    }

    #[test]
    fn real_interface_read_does_not_fail() {
        // This is a genuine read of the machine's interface table, not a mock.
        let snap = snapshot().expect("interface table readable without elevation");
        assert!(snap.interfaces.len() < 512);
        if let Some(kind) = snap.primary_kind {
            assert!(matches!(
                kind,
                InterfaceKind::Ethernet | InterfaceKind::Wifi | InterfaceKind::Vpn | InterfaceKind::Other
            ));
        }
    }
}
