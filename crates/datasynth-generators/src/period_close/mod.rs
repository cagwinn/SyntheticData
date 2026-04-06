//! Period close generators.
//!
//! This module provides generators for period-end close processes including:
//! - Close engine for orchestrating the close process
//! - Accrual entry generation
//! - Depreciation run generation
//! - Year-end closing entries
//! - IFRS 8 / ASC 280 segment reporting

mod accruals;
mod cash_flow_enhancer;
mod close_engine;
mod consolidation_generator;
mod depreciation;
mod dividend_generator;
mod financial_statement_generator;
pub mod notes_generator;
mod segment_generator;
mod year_end;

pub use accruals::*;
pub use cash_flow_enhancer::*;
pub use close_engine::*;
pub use consolidation_generator::*;
pub use depreciation::*;
pub use dividend_generator::*;
pub use financial_statement_generator::*;
pub use notes_generator::{EnhancedNotesContext, NotesGenerator, NotesGeneratorContext};
pub use segment_generator::*;
pub use year_end::*;
