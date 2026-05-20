#![cfg_attr(not(test), deny(clippy::unwrap_used))]
//! # synth-runtime
//!
//! Runtime orchestration, parallel execution, and memory management.
//!
//! This crate provides orchestrators:
//! - `EnhancedOrchestrator`: Full-featured orchestrator with all phases
//! - `StreamingOrchestrator`: Streaming orchestrator for real-time generation
//!
//! And support modules for:
//! - `run_manifest`: Run metadata and reproducibility tracking
//! - `label_export`: Anomaly label export to CSV/JSON formats
//!
//! ## v4.0 cleanup
//!
//! The legacy `GenerationOrchestrator` (basic 2-phase CoA + JE) was
//! removed in v4.0 after a v3.x deprecation window. All production
//! call paths — CLI `generate`, server endpoints —
//! already routed through `EnhancedOrchestrator`. Users embedding
//! `GenerationOrchestrator` directly should migrate to
//! `EnhancedOrchestrator::new(config, PhaseConfig::from_config(&config))`.

pub mod causal_engine;
pub mod config_mutator;
pub mod enhanced_orchestrator;
pub mod generation_session;
pub mod intervention_manager;
pub mod je_network;
pub mod label_export;
pub mod lineage;
pub mod output_writer;
pub mod prov;
pub mod run_manifest;
pub mod scenario_engine;
pub mod shard_context;
#[cfg(feature = "streaming")]
pub mod stream_client;
pub mod stream_pipeline;
pub mod streaming_orchestrator;
pub mod webhooks;

pub use enhanced_orchestrator::*;
pub use label_export::*;
pub use run_manifest::*;
pub use shard_context::ShardContext;
pub use streaming_orchestrator::*;
