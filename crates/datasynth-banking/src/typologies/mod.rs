//! AML typology injection module.
//!
//! This module provides injection of various AML patterns:
//! - Structuring / Smurfing
//! - Funnel accounts
//! - Layering chains
//! - Round-tripping
//! - Money mule networks
//! - Fraud patterns (ATO, BEC, fake vendors, APP)
//! - Spoofing mode

mod casino_integration;
mod crypto_integration;
mod false_positive;
mod fraud;
mod funnel;
mod injector;
mod layering;
mod mule;
mod network_generator;
pub mod network_topology;
mod pouch_activity;
mod real_estate_integration;
mod romance_scam;
mod round_tripping;
mod sanctions_evasion;
pub mod sophistication_sampler;
mod spoofing;
mod structuring;
mod synthetic_identity;
mod trade_based_ml;

pub use casino_integration::*;
pub use crypto_integration::*;
pub use false_positive::*;
pub use fraud::*;
pub use funnel::*;
pub use injector::*;
pub use layering::*;
pub use mule::*;
pub use network_generator::*;
pub use pouch_activity::*;
pub use real_estate_integration::*;
pub use romance_scam::*;
pub use round_tripping::*;
pub use sanctions_evasion::*;
pub use spoofing::*;
pub use structuring::*;
pub use synthetic_identity::*;
pub use trade_based_ml::*;
