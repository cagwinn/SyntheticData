//! Banking/KYC/AML evaluation module.
//!
//! Validates banking data including KYC profile completeness
//! and AML typology coherence and detectability.

pub mod account_lifecycle;
pub mod aml_detectability;
pub mod cross_layer_coherence;
pub mod device_fingerprint;
pub mod false_positive_quality;
pub mod kyc_completeness;
pub mod network_structure;
pub mod sanctions_screening;
pub mod sophistication_distribution;
pub mod velocity_quality;

pub use account_lifecycle::{
    LifecycleAnalysis, LifecycleAnalyzer, LifecycleEndState, LifecycleThresholds, TransitionRecord,
};
pub use aml_detectability::{
    AmlDetectabilityAnalysis, AmlDetectabilityAnalyzer, AmlTransactionData, TypologyData,
};
pub use cross_layer_coherence::{
    BankTxnLinks, CrossLayerCoherenceAnalysis, CrossLayerCoherenceAnalyzer, CrossLayerThresholds,
    PaymentRef,
};
pub use device_fingerprint::{
    DeviceFingerprintAnalysis, DeviceFingerprintAnalyzer, DeviceFingerprintThresholds,
    DeviceObservation,
};
pub use false_positive_quality::{
    FalsePositiveAnalysis, FalsePositiveAnalyzer, FalsePositiveThresholds, LabelData,
};
pub use kyc_completeness::{KycCompletenessAnalysis, KycCompletenessAnalyzer, KycProfileData};
pub use network_structure::{
    NetworkNodeObservation, NetworkStructureAnalysis, NetworkStructureAnalyzer,
    NetworkStructureThresholds,
};
pub use sanctions_screening::{
    SanctionsScreeningAnalysis, SanctionsScreeningAnalyzer, SanctionsScreeningThresholds,
    ScreeningObservation,
};
pub use sophistication_distribution::{
    SophisticationAnalysis, SophisticationAnalyzer, SophisticationObservation,
    SophisticationThresholds,
};
pub use velocity_quality::{
    VelocityFeaturesData, VelocityQualityAnalysis, VelocityQualityAnalyzer,
    VelocityQualityThresholds,
};

use serde::{Deserialize, Serialize};

/// Combined banking evaluation results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BankingEvaluation {
    /// KYC completeness analysis.
    pub kyc: Option<KycCompletenessAnalysis>,
    /// AML detectability analysis.
    pub aml: Option<AmlDetectabilityAnalysis>,
    /// Cross-layer coherence analysis (payment↔bank txn).
    #[serde(default)]
    pub cross_layer: Option<CrossLayerCoherenceAnalysis>,
    /// Velocity feature quality analysis.
    #[serde(default)]
    pub velocity: Option<VelocityQualityAnalysis>,
    /// False positive quality analysis.
    #[serde(default)]
    pub false_positive: Option<FalsePositiveAnalysis>,
    /// Overall pass/fail.
    pub passes: bool,
    /// Issues found.
    pub issues: Vec<String>,
}

impl BankingEvaluation {
    /// Create a new empty evaluation.
    pub fn new() -> Self {
        Self {
            kyc: None,
            aml: None,
            cross_layer: None,
            velocity: None,
            false_positive: None,
            passes: true,
            issues: Vec::new(),
        }
    }

    /// Check thresholds and update pass status.
    pub fn check_thresholds(&mut self) {
        self.issues.clear();
        if let Some(ref kyc) = self.kyc {
            if !kyc.passes {
                self.issues.extend(kyc.issues.clone());
            }
        }
        if let Some(ref aml) = self.aml {
            if !aml.passes {
                self.issues.extend(aml.issues.clone());
            }
        }
        if let Some(ref cl) = self.cross_layer {
            if !cl.passes {
                self.issues.extend(cl.issues.clone());
            }
        }
        if let Some(ref v) = self.velocity {
            if !v.passes {
                self.issues.extend(v.issues.clone());
            }
        }
        if let Some(ref fp) = self.false_positive {
            if !fp.passes {
                self.issues.extend(fp.issues.clone());
            }
        }
        self.passes = self.issues.is_empty();
    }
}

impl Default for BankingEvaluation {
    fn default() -> Self {
        Self::new()
    }
}
