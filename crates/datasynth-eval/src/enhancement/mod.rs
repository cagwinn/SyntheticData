//! Enhancement derivation module for automatic configuration optimization.
//!
//! This module provides tools to analyze evaluation results and derive
//! configuration enhancements that will improve data quality metrics.
//!
//! The enhancement pipeline follows this flow:
//! ```text
//! Evaluation Results → Threshold Check → Gap Analysis → Root Cause → Config Suggestion
//! ```
//!
//! The [`AiTuner`] wraps the rule-based [`AutoTuner`] and [`RecommendationEngine`]
//! with an LLM interpretation layer for intelligent gap analysis and creative
//! config suggestions beyond what rules can derive.

pub mod ai_tuner;
mod auto_tuner;
mod recommendation_engine;

pub use ai_tuner::*;
pub use auto_tuner::*;
pub use recommendation_engine::*;
