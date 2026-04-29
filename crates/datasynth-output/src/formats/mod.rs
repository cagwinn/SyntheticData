//! ERP-specific output format modules.
//!
//! This module provides export functionality for common ERP systems:
//! - SAP S/4HANA (BKPF, BSEG, ACDOCA tables)
//! - Oracle EBS (GL_JE_HEADERS, GL_JE_LINES)
//! - NetSuite (Journal entries with NetSuite-specific fields)
//! - FEC (Fichier des Écritures Comptables) for French GAAP
//! - GoBD (Grundsätze zur ordnungsmäßigen Führung und Aufbewahrung) for German GAAP

pub mod fec;
pub mod gobd;
pub mod netsuite;
pub mod oracle;
pub mod saft;
pub mod sap;
pub mod sap_master_data;
pub mod sap_subledger;
pub mod sap_transactional;
pub mod xbrl;

pub use fec::write_fec_csv;
pub use gobd::{write_gobd_accounts_csv, write_gobd_index_xml, write_gobd_journal_csv};
pub use netsuite::{NetSuiteExporter, NetSuiteJournalEntry, NetSuiteJournalLine};
pub use oracle::{OracleExporter, OracleJeHeader, OracleJeLine};
pub use saft::{saft_naive_date, write_saft, SaftConfig, SaftData, SaftJurisdiction};
pub use sap::{
    SapCustomer, SapCustomerExportable, SapDialect, SapExportConfig, SapExporter, SapTableType,
    SapVendor, SapVendorExportable,
};
pub use sap_master_data::{
    write_anla, write_cepc, write_csks, write_kna1, write_knb1, write_lfa1, write_lfb1, write_mara,
    write_mard, write_ska1, write_skb1, SapAsset, SapAssetExportable, SapCostCenter,
    SapCostCenterExportable, SapCustomerCompanyCode, SapCustomerCompanyCodeExportable,
    SapGlAccountCompanyCode, SapGlAccountExportable, SapGlAccountGeneral, SapMaterial,
    SapMaterialExportable, SapMaterialStorage, SapMaterialStorageExportable, SapProfitCenter,
    SapProfitCenterExportable, SapVendorCompanyCode, SapVendorCompanyCodeExportable,
};
pub use sap_subledger::{
    write_bsad, write_bsak, write_bsas, write_bsid, write_bsik, write_bsis, SapClearedItemRow,
    SapOpenItemRow,
};
pub use sap_transactional::{
    write_ekko, write_ekpo, write_likp, write_lips, write_mkpf, write_mseg, write_vbak, write_vbap,
    SapDeliveryExportable, SapDeliveryHeader, SapDeliveryItem, SapMatDocExportable,
    SapMatDocHeader, SapMatDocItem, SapPoExportable, SapPoHeader, SapPoItem, SapSoExportable,
    SapSoHeader, SapSoItem,
};
pub use xbrl::XbrlExporter;
