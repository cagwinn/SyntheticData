//! LLM metadata enrichment for synthetic data generation.
//!
//! This module uses the `LlmProvider` trait from `datasynth-core` to generate
//! realistic vendor names, transaction descriptions, memo fields, anomaly
//! explanations, and contextual fraud scheme designs. Each enricher falls back
//! to deterministic templates when the LLM provider returns an error.

pub mod anomaly_designer;
pub mod anomaly_explainer;
pub mod customer_enricher;
pub mod finding_enricher;
pub mod material_enricher;
pub mod transaction_enricher;
pub mod vendor_enricher;

pub use anomaly_designer::{
    AnomalyDesigner, CompanyContext, ControlContext, DesignResult, DesignedScheme, DesignedStage,
    SchemeLibrary,
};
pub use anomaly_explainer::AnomalyLlmExplainer;
pub use customer_enricher::CustomerLlmEnricher;
pub use finding_enricher::FindingLlmEnricher;
pub use material_enricher::MaterialLlmEnricher;
pub use transaction_enricher::TransactionLlmEnricher;
pub use vendor_enricher::VendorLlmEnricher;
