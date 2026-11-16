//! Metered connection detection through WinRT connectivity APIs.

use serde::{Deserialize, Serialize};
use windows::Networking::Connectivity::{NetworkCostType, NetworkInformation};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionCost {
    pub metered: bool,
    pub roaming: bool,
    pub known: bool,
}

/// Reads the current internet connection cost. Returns `known: false` when the
/// platform cannot answer, so policies can decide to stay permissive instead of
/// guessing.
pub fn connection_cost() -> ConnectionCost {
    match query() {
        Ok(cost) => cost,
        Err(error) => {
            tracing::debug!(%error, "connection cost unavailable");
            ConnectionCost::default()
        }
    }
}

fn query() -> windows::core::Result<ConnectionCost> {
    let profile = NetworkInformation::GetInternetConnectionProfile()?;
    let cost = profile.GetConnectionCost()?;
    let cost_type = cost.NetworkCostType()?;
    let metered = matches!(cost_type, NetworkCostType::Fixed | NetworkCostType::Variable);
    Ok(ConnectionCost {
        metered,
        roaming: cost.Roaming().unwrap_or(false),
        known: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_query_never_panics() {
        let cost = connection_cost();
        if !cost.known {
            assert!(!cost.metered);
        }
    }
}
