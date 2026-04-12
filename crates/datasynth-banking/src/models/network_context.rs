//! Network context for multi-party AML scenarios.
//!
//! Links transactions and customers that participate in the same criminal
//! network (structuring rings, mule chains, shell company pyramids).

use datasynth_core::models::banking::AmlTypology;
use serde::{Deserialize, Serialize};

/// Network context attached to transactions that are part of multi-party scenarios.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkContext {
    /// Unique network identifier linking all participants
    pub network_id: String,
    /// This participant's role in the network
    pub network_role: NetworkRole,
    /// Co-occurring typologies in this scenario (e.g., structuring + mule + layering)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub co_occurring_typologies: Vec<AmlTypology>,
    /// Total number of participants in the network
    pub network_size: u32,
}

/// Role within a criminal network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkRole {
    /// Organizes and directs the network
    Coordinator,
    /// Makes small deposits on behalf of the coordinator (structuring)
    Smurf,
    /// Receives and forwards funds in a mule chain
    Middleman,
    /// Final cash-out point in a mule/layering chain
    CashOut,
    /// Shell company entity used for layering
    ShellEntity,
    /// Recruits mules or smurfs
    Recruiter,
    /// Ultimate beneficiary of laundered funds
    Beneficiary,
}

impl NetworkRole {
    /// Risk weight for this role (higher = more central to the network).
    pub fn risk_weight(&self) -> f64 {
        match self {
            Self::Coordinator => 2.0,
            Self::Recruiter => 1.8,
            Self::CashOut => 1.5,
            Self::ShellEntity => 1.4,
            Self::Beneficiary => 1.3,
            Self::Middleman => 1.2,
            Self::Smurf => 1.0,
        }
    }
}
