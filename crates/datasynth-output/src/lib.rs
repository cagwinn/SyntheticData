#![deny(clippy::unwrap_used)]
//! # synth-output
//!
//! Output sinks for CSV, Parquet, JSON, and streaming formats.
//! Also provides ERP-specific export formats for SAP, Oracle EBS, and NetSuite.

use std::path::{Path, PathBuf};

pub mod compressed;
pub mod control_export;
pub mod csv_sink;
pub mod esg_export;
pub mod fast_csv;
pub mod formats;
pub mod json_sink;
pub mod parquet_sink;
pub mod project_accounting_export;
pub mod streaming;
pub mod tax_export;
pub mod treasury_export;

/// Output routing config for the enhanced orchestrator's file-writer.
///
/// In single-entity mode (default) [`OutputRootConfig::flat`] preserves the
/// pre-v5.0 layout: all generated files land directly under `root_dir`.
///
/// In group-audit shard mode, the runner sets `per_entity_subtree: true`
/// and `entity_code: Some(code)` so each entity's archive is written under
/// `{root_dir}/entities/{code}/`, leaving `{root_dir}/` itself available
/// for group-wide artifacts (consolidated FS, aggregate summary, etc).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputRootConfig {
    /// Root output directory as configured by the caller.
    pub root_dir: PathBuf,
    /// When `true`, output is routed under `{root_dir}/entities/{entity_code}/`
    /// so group-wide artifacts (consolidated FS, summary) can live alongside
    /// per-entity shard archives at `{root_dir}/`.
    pub per_entity_subtree: bool,
    /// Entity code identifying the shard. Ignored when `per_entity_subtree`
    /// is `false`. When `per_entity_subtree` is `true` but this is `None`,
    /// [`OutputRootConfig::effective_dir`] safely falls back to `root_dir`.
    pub entity_code: Option<String>,
}

impl OutputRootConfig {
    /// Flat single-entity layout: files written directly under `root_dir`.
    pub fn flat(root_dir: impl Into<PathBuf>) -> Self {
        Self {
            root_dir: root_dir.into(),
            per_entity_subtree: false,
            entity_code: None,
        }
    }

    /// Per-entity subtree layout: files written under
    /// `{root_dir}/entities/{entity_code}/`.
    pub fn per_entity(root_dir: impl Into<PathBuf>, entity_code: impl Into<String>) -> Self {
        Self {
            root_dir: root_dir.into(),
            per_entity_subtree: true,
            entity_code: Some(entity_code.into()),
        }
    }

    /// Resolve the effective `output_dir` path for the file-writer. This
    /// is the directory into which all the existing `write_*` helpers
    /// compose their `.join("subdir")` paths.
    ///
    /// Defensive fallback: if `per_entity_subtree` is `true` but
    /// `entity_code` is `None`, we revert to `root_dir` rather than write
    /// into a nameless `{root_dir}/entities/` directory.
    pub fn effective_dir(&self) -> PathBuf {
        match (self.per_entity_subtree, &self.entity_code) {
            (true, Some(code)) => self.root_dir.join("entities").join(code),
            (false, _) | (true, None) => self.root_dir.clone(),
        }
    }

    /// Root directory borrowed as a `&Path` — useful for group-wide
    /// artifacts (consolidated FS, aggregate summary) that must sit
    /// alongside the per-entity subtrees rather than inside one of them.
    pub fn root_path(&self) -> &Path {
        &self.root_dir
    }
}

impl Default for OutputRootConfig {
    fn default() -> Self {
        Self::flat(PathBuf::from("."))
    }
}

pub use compressed::{CompressedWriter, CompressionConfig};
pub use control_export::*;
pub use csv_sink::*;
pub use esg_export::*;
pub use formats::{
    saft_naive_date, write_anla, write_bsad, write_bsak, write_bsas, write_bsid, write_bsik,
    write_bsis, write_cepc, write_csks, write_ekko, write_ekpo, write_fec_csv,
    write_gobd_accounts_csv, write_gobd_index_xml, write_gobd_journal_csv, write_kna1, write_knb1,
    write_lfa1, write_lfb1, write_likp, write_lips, write_mara, write_mard, write_mkpf, write_mseg,
    write_saft, write_ska1, write_skb1, write_vbak, write_vbap, NetSuiteExporter,
    NetSuiteJournalEntry, NetSuiteJournalLine, OracleExporter, OracleJeHeader, OracleJeLine,
    SaftConfig, SaftData, SaftJurisdiction, SapAsset, SapAssetExportable, SapClearedItemRow,
    SapCostCenter, SapCostCenterExportable, SapCustomer, SapCustomerCompanyCode,
    SapCustomerCompanyCodeExportable, SapCustomerExportable, SapDeliveryExportable,
    SapDeliveryHeader, SapDeliveryItem, SapDialect, SapExportConfig, SapExporter,
    SapGlAccountCompanyCode, SapGlAccountExportable, SapGlAccountGeneral, SapMatDocExportable,
    SapMatDocHeader, SapMatDocItem, SapMaterial, SapMaterialExportable, SapMaterialStorage,
    SapMaterialStorageExportable, SapOpenItemRow, SapPoExportable, SapPoHeader, SapPoItem,
    SapProfitCenter, SapProfitCenterExportable, SapSoExportable, SapSoHeader, SapSoItem,
    SapTableType, SapVendor, SapVendorCompanyCode, SapVendorCompanyCodeExportable,
    SapVendorExportable, XbrlExporter,
};
pub use json_sink::*;
pub use parquet_sink::*;
pub use project_accounting_export::*;
pub use streaming::{
    CsvStreamingSink, JsonStreamingSink, NdjsonStreamingSink, ParquetStreamingSink,
};
pub use tax_export::*;
pub use treasury_export::*;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test_helpers;

#[cfg(test)]
mod output_root_config_tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn flat_mode_returns_root_dir() {
        let cfg = OutputRootConfig::flat("/tmp/out");
        assert_eq!(cfg.effective_dir(), Path::new("/tmp/out"));
        assert!(!cfg.per_entity_subtree);
        assert!(cfg.entity_code.is_none());
    }

    #[test]
    fn per_entity_mode_routes_under_entities_code() {
        let cfg = OutputRootConfig::per_entity("/tmp/out", "NESTLE_SA");
        assert_eq!(
            cfg.effective_dir(),
            Path::new("/tmp/out/entities/NESTLE_SA")
        );
        assert!(cfg.per_entity_subtree);
        assert_eq!(cfg.entity_code.as_deref(), Some("NESTLE_SA"));
    }

    #[test]
    fn per_entity_true_without_code_falls_back_to_root() {
        // Defensive: if a caller sets per_entity_subtree: true but forgets
        // to set entity_code, we must not write to a bogus
        // "/tmp/out/entities/" path — silently revert to flat.
        let cfg = OutputRootConfig {
            root_dir: "/tmp/out".into(),
            per_entity_subtree: true,
            entity_code: None,
        };
        assert_eq!(cfg.effective_dir(), Path::new("/tmp/out"));
    }

    #[test]
    fn default_is_flat_at_cwd() {
        let cfg = OutputRootConfig::default();
        assert_eq!(cfg.effective_dir(), Path::new("."));
        assert!(!cfg.per_entity_subtree);
        assert!(cfg.entity_code.is_none());
    }

    #[test]
    fn root_path_always_returns_root_regardless_of_mode() {
        // root_path() exposes the configured root for group-wide artifacts
        // (consolidated FS, aggregate summary) that must NOT land inside
        // one entity's subtree.
        let flat = OutputRootConfig::flat("/tmp/out");
        assert_eq!(flat.root_path(), Path::new("/tmp/out"));

        let per_entity = OutputRootConfig::per_entity("/tmp/out", "ENT_A");
        assert_eq!(per_entity.root_path(), Path::new("/tmp/out"));
    }
}
