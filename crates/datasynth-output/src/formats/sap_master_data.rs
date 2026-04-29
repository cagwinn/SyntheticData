//! SAP master-data table exporters — v4.3.0b.
//!
//! Complements the document-oriented exporter in `formats::sap` by
//! mapping the DataSynth master-data models (`Vendor`, `Customer`,
//! `Material`) into the SAP master-data tables that accompany the
//! classical three-table triple (BKPF / BSEG / ACDOCA):
//!
//! | DataSynth model | SAP general table | SAP company-code table |
//! |-----------------|-------------------|------------------------|
//! | `Vendor`        | LFA1              | LFB1                   |
//! | `Customer`      | KNA1              | KNB1                   |
//! | `Material`      | MARA              | MARD                   |
//!
//! LFA1 / KNA1 are cross-client "general data" tables (one row per
//! master record). LFB1 / KNB1 carry company-code-specific data
//! (reconciliation account, payment terms, dunning block) — one row
//! per (vendor|customer, company code). MARA is the client-level
//! material master; MARD is the storage-location-level stock view.
//!
//! All exporters honour the `SapDialect` setting of the parent
//! `SapExportConfig` (delimiter / decimal separator / date format /
//! UTF-8 BOM).

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use chrono::Datelike;
use datasynth_core::error::SynthResult;
use datasynth_core::models::{
    ChartOfAccounts, CostCenter, Customer, FixedAsset, GLAccount, Material, ProfitCenter, Vendor,
};

use super::sap::{
    SapCustomer, SapCustomerExportable, SapExportConfig, SapVendor, SapVendorExportable,
};

// ===========================================================================
// LFA1 — Vendor general data (one row per Vendor)
// ===========================================================================

impl SapVendorExportable for Vendor {
    fn to_sap_vendor(&self, client: &str) -> SapVendor {
        SapVendor {
            mandt: client.to_string(),
            lifnr: self.vendor_id.clone(),
            land1: self.country.clone(),
            name1: self.name.clone(),
            name2: None,
            ort01: None,
            pstlz: None,
            stras: None,
            regio: None,
            spras: language_for_country(&self.country).to_string(),
            stcd1: self.tax_id.clone(),
            ktokk: account_group_for_vendor(self),
        }
    }
}

// ===========================================================================
// KNA1 — Customer general data (one row per Customer)
// ===========================================================================

impl SapCustomerExportable for Customer {
    fn to_sap_customer(&self, client: &str) -> SapCustomer {
        SapCustomer {
            mandt: client.to_string(),
            kunnr: self.customer_id.clone(),
            land1: self.country.clone(),
            name1: self.name.clone(),
            name2: None,
            ort01: None,
            pstlz: None,
            stras: None,
            regio: None,
            spras: language_for_country(&self.country).to_string(),
            stcd1: self.tax_id.clone(),
            ktokd: account_group_for_customer(self),
        }
    }
}

// ===========================================================================
// LFB1 — Vendor company-code data (one row per (vendor, company code))
// ===========================================================================

/// SAP LFB1 — vendor company-code view. Carries the reconciliation
/// account, payment terms, dunning procedure, and withholding-tax flag
/// that are company-code-specific on a vendor master record.
#[derive(Debug, Clone)]
pub struct SapVendorCompanyCode {
    pub mandt: String,
    pub lifnr: String,
    pub bukrs: String,
    /// Reconciliation GL account (AKONT) — AP control account in the chart.
    pub akont: String,
    /// Payment terms key (ZTERM) — "Net30" / "Net60" / "2_10_Net_30" etc.
    pub zterm: String,
    /// Dunning procedure (MAHNA) — blank when no dunning applies.
    pub mahna: Option<String>,
    /// Withholding tax code (QSSKZ) — populated when withholding is applicable.
    pub qsskz: Option<String>,
    /// Payment block reason (ZAHLS) — blank = open, "A"/"B"/... = blocked.
    pub zahls: Option<String>,
    /// Vendor sort key (SORTL).
    pub sortl: Option<String>,
    /// Account creation date (ERDAT).
    pub erdat: chrono::NaiveDate,
}

/// Extension trait for building the LFB1 company-code row from a
/// foreign-crate `Vendor`. Rust's orphan rule forbids an inherent impl
/// on `Vendor`, so we expose the method via a trait defined here.
pub trait SapVendorCompanyCodeExportable {
    fn to_sap_vendor_company_code(&self, client: &str, company_code: &str) -> SapVendorCompanyCode;
}

impl SapVendorCompanyCodeExportable for Vendor {
    fn to_sap_vendor_company_code(&self, client: &str, company_code: &str) -> SapVendorCompanyCode {
        SapVendorCompanyCode {
            mandt: client.to_string(),
            lifnr: self.vendor_id.clone(),
            bukrs: company_code.to_string(),
            akont: self
                .reconciliation_account
                .clone()
                .unwrap_or_else(|| "2000".to_string()),
            zterm: format!("{:?}", self.payment_terms),
            mahna: None,
            qsskz: if self.withholding_tax_applicable {
                Some("W1".to_string())
            } else {
                None
            },
            zahls: None,
            sortl: Some(self.vendor_id.clone()),
            erdat: chrono::Utc::now().date_naive(),
        }
    }
}

// ===========================================================================
// KNB1 — Customer company-code data (one row per (customer, company code))
// ===========================================================================

/// SAP KNB1 — customer company-code view. Carries the reconciliation
/// account, payment terms, dunning level / procedure, and credit block
/// that are company-code-specific on a customer master record.
#[derive(Debug, Clone)]
pub struct SapCustomerCompanyCode {
    pub mandt: String,
    pub kunnr: String,
    pub bukrs: String,
    /// Reconciliation GL account (AKONT) — AR control account.
    pub akont: String,
    /// Payment terms key (ZTERM).
    pub zterm: String,
    /// Dunning procedure (MAHNA).
    pub mahna: Option<String>,
    /// Current dunning level (MAHNS, 0–4).
    pub mahns: u8,
    /// Last dunning run date (MADAT).
    pub madat: Option<chrono::NaiveDate>,
    /// Payment block reason (ZAHLS).
    pub zahls: Option<String>,
    /// Credit block indicator (CRDBLK) — 1 = blocked, 0 = open.
    pub crdblk: u8,
    /// Sort key (SORTL).
    pub sortl: Option<String>,
    /// Account creation date (ERDAT).
    pub erdat: chrono::NaiveDate,
}

/// Extension trait for building the KNB1 company-code row from a
/// foreign-crate `Customer`.
pub trait SapCustomerCompanyCodeExportable {
    fn to_sap_customer_company_code(
        &self,
        client: &str,
        company_code: &str,
    ) -> SapCustomerCompanyCode;
}

impl SapCustomerCompanyCodeExportable for Customer {
    fn to_sap_customer_company_code(
        &self,
        client: &str,
        company_code: &str,
    ) -> SapCustomerCompanyCode {
        SapCustomerCompanyCode {
            mandt: client.to_string(),
            kunnr: self.customer_id.clone(),
            bukrs: company_code.to_string(),
            akont: self
                .reconciliation_account
                .clone()
                .unwrap_or_else(|| "1100".to_string()),
            zterm: format!("{:?}", self.payment_terms),
            mahna: self.dunning_procedure.clone(),
            mahns: self.dunning_level,
            madat: self.last_dunning_date,
            zahls: self.credit_block_reason.clone(),
            crdblk: if self.credit_blocked { 1 } else { 0 },
            sortl: Some(self.customer_id.clone()),
            erdat: chrono::Utc::now().date_naive(),
        }
    }
}

// ===========================================================================
// MARA — Material general data (one row per Material)
// ===========================================================================

/// SAP MARA — material master general view. One row per material,
/// client-wide (cross-plant).
#[derive(Debug, Clone)]
pub struct SapMaterial {
    pub mandt: String,
    /// Material number (MATNR).
    pub matnr: String,
    /// Material type (MTART) — "FERT" (finished), "HALB" (semi-finished),
    /// "ROH" (raw), "HAWA" (trading goods), "DIEN" (service), etc.
    pub mtart: String,
    /// Industry sector (MBRSH) — "C" = chemical, "M" = mechanical engineering,
    /// "1" = retail, defaults to "M" when unknown.
    pub mbrsh: String,
    /// Material group (MATKL).
    pub matkl: String,
    /// Base unit of measure (MEINS).
    pub meins: String,
    /// Gross weight (BRGEW).
    pub brgew: Option<rust_decimal::Decimal>,
    /// Volume (VOLUM).
    pub volum: Option<rust_decimal::Decimal>,
    /// Unit of weight (GEWEI).
    pub gewei: String,
    /// Unit of volume (VOLEH).
    pub voleh: String,
    /// Old material number (BISMT) — blank by default.
    pub bismt: Option<String>,
    /// Creation date (ERSDA).
    pub ersda: chrono::NaiveDate,
    /// Created by user (ERNAM).
    pub ernam: String,
}

/// Extension trait for building MARA rows from a foreign-crate `Material`.
pub trait SapMaterialExportable {
    fn to_sap_material(&self, client: &str) -> SapMaterial;
}

impl SapMaterialExportable for Material {
    fn to_sap_material(&self, client: &str) -> SapMaterial {
        SapMaterial {
            mandt: client.to_string(),
            matnr: self.material_id.clone(),
            mtart: material_type_to_mtart(&self.material_type),
            mbrsh: "M".to_string(),
            matkl: material_group_to_matkl(&self.material_group),
            // UnitOfMeasure has a `code` String field (e.g. "EA", "KG", "L")
            // that maps directly to the SAP MEINS column.
            meins: self.base_uom.code.to_uppercase(),
            brgew: self.weight_kg,
            volum: self.volume_m3,
            gewei: "KG".to_string(),
            voleh: "M3".to_string(),
            bismt: None,
            ersda: chrono::Utc::now().date_naive(),
            ernam: "SYSTEM".to_string(),
        }
    }
}

// ===========================================================================
// MARD — Material storage location data (one row per (material, plant, storage location))
// ===========================================================================

/// SAP MARD — storage-location view. One row per (material, plant,
/// storage location); carries the period stock total that subledger
/// inventory reports pull from.
#[derive(Debug, Clone)]
pub struct SapMaterialStorage {
    pub mandt: String,
    pub matnr: String,
    /// Plant (WERKS).
    pub werks: String,
    /// Storage location (LGORT).
    pub lgort: String,
    /// Total unrestricted-use stock (LABST).
    pub labst: rust_decimal::Decimal,
    /// Stock in quality inspection (INSME).
    pub insme: rust_decimal::Decimal,
    /// Blocked stock (SPEME).
    pub speme: rust_decimal::Decimal,
    /// Safety stock (EISLO).
    pub eislo: rust_decimal::Decimal,
    /// Reorder point (MINBE).
    pub minbe: rust_decimal::Decimal,
}

/// Extension trait for building MARD storage-location rows from a
/// foreign-crate `Material`.
pub trait SapMaterialStorageExportable {
    fn to_sap_material_storage_rows(
        &self,
        client: &str,
        storage_location: &str,
        stock_by_plant: &[(String, rust_decimal::Decimal)],
    ) -> Vec<SapMaterialStorage>;
}

impl SapMaterialStorageExportable for Material {
    fn to_sap_material_storage_rows(
        &self,
        client: &str,
        storage_location: &str,
        stock_by_plant: &[(String, rust_decimal::Decimal)],
    ) -> Vec<SapMaterialStorage> {
        stock_by_plant
            .iter()
            .map(|(plant, qty)| SapMaterialStorage {
                mandt: client.to_string(),
                matnr: self.material_id.clone(),
                werks: plant.clone(),
                lgort: storage_location.to_string(),
                labst: *qty,
                insme: rust_decimal::Decimal::ZERO,
                speme: rust_decimal::Decimal::ZERO,
                eislo: self.safety_stock,
                minbe: self.reorder_point,
            })
            .collect()
    }
}

// ===========================================================================
// File writers
// ===========================================================================

/// Shared helper — opens a file, writes the dialect BOM, returns the buffered writer.
fn open_master_file(cfg: &SapExportConfig, path: &Path) -> SynthResult<BufWriter<File>> {
    let file = File::create(path)?;
    let mut writer = BufWriter::with_capacity(256 * 1024, file);
    let bom = cfg.dialect.bom();
    if !bom.is_empty() {
        writer.write_all(bom)?;
    }
    Ok(writer)
}

fn write_row<W: Write>(writer: &mut W, delim: char, fields: &[String]) -> std::io::Result<()> {
    for (i, f) in fields.iter().enumerate() {
        if i > 0 {
            write!(writer, "{delim}")?;
        }
        write!(writer, "{f}")?;
    }
    writeln!(writer)
}

fn write_header<W: Write>(writer: &mut W, delim: char, cols: &[&str]) -> std::io::Result<()> {
    for (i, c) in cols.iter().enumerate() {
        if i > 0 {
            write!(writer, "{delim}")?;
        }
        write!(writer, "{c}")?;
    }
    writeln!(writer)
}

fn escape(field: &str) -> String {
    if field.contains(',') || field.contains(';') || field.contains('"') || field.contains('\n') {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

/// Write LFA1 (vendor general data). One row per vendor.
pub fn write_lfa1(cfg: &SapExportConfig, vendors: &[Vendor], path: &Path) -> SynthResult<()> {
    let mut writer = open_master_file(cfg, path)?;
    let delim = cfg.delimiter();
    write_header(
        &mut writer,
        delim,
        &[
            "MANDT", "LIFNR", "LAND1", "NAME1", "NAME2", "ORT01", "PSTLZ", "STRAS", "REGIO",
            "SPRAS", "STCD1", "KTOKK",
        ],
    )?;
    for v in vendors {
        let s = v.to_sap_vendor(&cfg.client);
        let fields: Vec<String> = vec![
            s.mandt,
            s.lifnr,
            s.land1,
            escape(&s.name1),
            escape(&s.name2.unwrap_or_default()),
            escape(&s.ort01.unwrap_or_default()),
            s.pstlz.unwrap_or_default(),
            escape(&s.stras.unwrap_or_default()),
            s.regio.unwrap_or_default(),
            s.spras,
            s.stcd1.unwrap_or_default(),
            s.ktokk,
        ];
        write_row(&mut writer, delim, &fields)?;
    }
    writer.flush()?;
    Ok(())
}

/// Write LFB1 (vendor company-code data). Requires a list of company
/// codes to emit the per-vendor-per-company rows.
pub fn write_lfb1(
    cfg: &SapExportConfig,
    vendors: &[Vendor],
    company_codes: &[String],
    path: &Path,
) -> SynthResult<()> {
    let mut writer = open_master_file(cfg, path)?;
    let delim = cfg.delimiter();
    write_header(
        &mut writer,
        delim,
        &[
            "MANDT", "LIFNR", "BUKRS", "AKONT", "ZTERM", "MAHNA", "QSSKZ", "ZAHLS", "SORTL",
            "ERDAT",
        ],
    )?;
    for v in vendors {
        for company in company_codes {
            let s = v.to_sap_vendor_company_code(&cfg.client, company);
            let fields: Vec<String> = vec![
                s.mandt,
                s.lifnr,
                s.bukrs,
                s.akont,
                s.zterm,
                s.mahna.unwrap_or_default(),
                s.qsskz.unwrap_or_default(),
                s.zahls.unwrap_or_default(),
                s.sortl.unwrap_or_default(),
                cfg.format_date(s.erdat),
            ];
            write_row(&mut writer, delim, &fields)?;
        }
    }
    writer.flush()?;
    Ok(())
}

/// Write KNA1 (customer general data).
pub fn write_kna1(cfg: &SapExportConfig, customers: &[Customer], path: &Path) -> SynthResult<()> {
    let mut writer = open_master_file(cfg, path)?;
    let delim = cfg.delimiter();
    write_header(
        &mut writer,
        delim,
        &[
            "MANDT", "KUNNR", "LAND1", "NAME1", "NAME2", "ORT01", "PSTLZ", "STRAS", "REGIO",
            "SPRAS", "STCD1", "KTOKD",
        ],
    )?;
    for c in customers {
        let s = c.to_sap_customer(&cfg.client);
        let fields: Vec<String> = vec![
            s.mandt,
            s.kunnr,
            s.land1,
            escape(&s.name1),
            escape(&s.name2.unwrap_or_default()),
            escape(&s.ort01.unwrap_or_default()),
            s.pstlz.unwrap_or_default(),
            escape(&s.stras.unwrap_or_default()),
            s.regio.unwrap_or_default(),
            s.spras,
            s.stcd1.unwrap_or_default(),
            s.ktokd,
        ];
        write_row(&mut writer, delim, &fields)?;
    }
    writer.flush()?;
    Ok(())
}

/// Write KNB1 (customer company-code data).
pub fn write_knb1(
    cfg: &SapExportConfig,
    customers: &[Customer],
    company_codes: &[String],
    path: &Path,
) -> SynthResult<()> {
    let mut writer = open_master_file(cfg, path)?;
    let delim = cfg.delimiter();
    write_header(
        &mut writer,
        delim,
        &[
            "MANDT", "KUNNR", "BUKRS", "AKONT", "ZTERM", "MAHNA", "MAHNS", "MADAT", "ZAHLS",
            "CRDBLK", "SORTL", "ERDAT",
        ],
    )?;
    for c in customers {
        for company in company_codes {
            let s = c.to_sap_customer_company_code(&cfg.client, company);
            let fields: Vec<String> = vec![
                s.mandt,
                s.kunnr,
                s.bukrs,
                s.akont,
                s.zterm,
                s.mahna.unwrap_or_default(),
                s.mahns.to_string(),
                s.madat.map(|d| cfg.format_date(d)).unwrap_or_default(),
                s.zahls.unwrap_or_default(),
                s.crdblk.to_string(),
                s.sortl.unwrap_or_default(),
                cfg.format_date(s.erdat),
            ];
            write_row(&mut writer, delim, &fields)?;
        }
    }
    writer.flush()?;
    Ok(())
}

/// Write MARA (material general data).
pub fn write_mara(cfg: &SapExportConfig, materials: &[Material], path: &Path) -> SynthResult<()> {
    let mut writer = open_master_file(cfg, path)?;
    let delim = cfg.delimiter();
    write_header(
        &mut writer,
        delim,
        &[
            "MANDT", "MATNR", "MTART", "MBRSH", "MATKL", "MEINS", "BRGEW", "VOLUM", "GEWEI",
            "VOLEH", "BISMT", "ERSDA", "ERNAM",
        ],
    )?;
    for m in materials {
        let s = m.to_sap_material(&cfg.client);
        let fields: Vec<String> = vec![
            s.mandt,
            s.matnr,
            s.mtart,
            s.mbrsh,
            s.matkl,
            s.meins,
            s.brgew
                .as_ref()
                .map(|d| cfg.format_decimal(d))
                .unwrap_or_default(),
            s.volum
                .as_ref()
                .map(|d| cfg.format_decimal(d))
                .unwrap_or_default(),
            s.gewei,
            s.voleh,
            s.bismt.unwrap_or_default(),
            cfg.format_date(s.ersda),
            s.ernam,
        ];
        write_row(&mut writer, delim, &fields)?;
    }
    writer.flush()?;
    Ok(())
}

/// Write MARD (material storage-location data). Uses a synthetic
/// `"0001"` storage location when none is supplied — SAP's default.
pub fn write_mard(cfg: &SapExportConfig, materials: &[Material], path: &Path) -> SynthResult<()> {
    let mut writer = open_master_file(cfg, path)?;
    let delim = cfg.delimiter();
    write_header(
        &mut writer,
        delim,
        &[
            "MANDT", "MATNR", "WERKS", "LGORT", "LABST", "INSME", "SPEME", "EISLO", "MINBE",
        ],
    )?;
    for m in materials {
        // Each listed plant emits a MARD row with zero stock — the CLI can
        // overlay real stock from `InventoryPosition` data if needed.
        let stock: Vec<(String, rust_decimal::Decimal)> = m
            .plants
            .iter()
            .map(|p| (p.clone(), rust_decimal::Decimal::ZERO))
            .collect();
        for s in m.to_sap_material_storage_rows(&cfg.client, "0001", &stock) {
            let fields: Vec<String> = vec![
                s.mandt,
                s.matnr,
                s.werks,
                s.lgort,
                cfg.format_decimal(&s.labst),
                cfg.format_decimal(&s.insme),
                cfg.format_decimal(&s.speme),
                cfg.format_decimal(&s.eislo),
                cfg.format_decimal(&s.minbe),
            ];
            write_row(&mut writer, delim, &fields)?;
        }
    }
    writer.flush()?;
    Ok(())
}

// ===========================================================================
// Internal helpers
// ===========================================================================

/// Map a DataSynth `MaterialGroup` to the SAP MATKL column (4-char codes).
fn material_group_to_matkl(g: &datasynth_core::models::MaterialGroup) -> String {
    use datasynth_core::models::MaterialGroup;
    match g {
        MaterialGroup::Electronics => "ELEC",
        MaterialGroup::Mechanical => "MECH",
        MaterialGroup::Chemicals | MaterialGroup::Chemical => "CHEM",
        MaterialGroup::OfficeSupplies => "OFFC",
        MaterialGroup::ItEquipment => "ITEQ",
        MaterialGroup::Furniture => "FURN",
        MaterialGroup::PackagingMaterials => "PACK",
        MaterialGroup::SafetyEquipment => "SAFE",
        MaterialGroup::Tools => "TOOL",
        MaterialGroup::Services => "SERV",
        MaterialGroup::Consumables => "CONS",
        MaterialGroup::FinishedGoods => "FINI",
    }
    .to_string()
}

/// Map a DataSynth `MaterialType` to the SAP MTART code.
fn material_type_to_mtart(mt: &datasynth_core::models::MaterialType) -> String {
    use datasynth_core::models::MaterialType;
    match mt {
        MaterialType::RawMaterial => "ROH",
        MaterialType::SemiFinished => "HALB",
        MaterialType::FinishedGood => "FERT",
        MaterialType::TradingGood => "HAWA",
        MaterialType::OperatingSupplies => "HIBE",
        MaterialType::SparePart => "ERSA",
        MaterialType::Packaging => "VERP",
        MaterialType::Service => "DIEN",
    }
    .to_string()
}

/// Pick a default SAP KTOKK vendor account group based on vendor type.
fn account_group_for_vendor(v: &Vendor) -> String {
    use datasynth_core::models::VendorType;
    match v.vendor_type {
        VendorType::Supplier => "LIEF",
        VendorType::ServiceProvider | VendorType::ProfessionalServices => "SERV",
        VendorType::Technology => "TECH",
        VendorType::Logistics => "LOGI",
        VendorType::Contractor => "CONT",
        VendorType::RealEstate => "REST",
        VendorType::Financial => "FINA",
        VendorType::Utility => "UTIL",
        VendorType::EmployeeReimbursement => "EMPL",
    }
    .to_string()
}

/// Pick a default SAP KTOKD customer account group based on customer type.
fn account_group_for_customer(c: &Customer) -> String {
    use datasynth_core::models::CustomerType;
    match c.customer_type {
        CustomerType::Corporate => "KUNA",
        CustomerType::SmallBusiness => "KUN1",
        CustomerType::Consumer => "CPDB",
        CustomerType::Government => "GOVT",
        CustomerType::NonProfit => "NPRF",
        CustomerType::Intercompany => "INTR",
        CustomerType::Distributor => "DIST",
    }
    .to_string()
}

/// Minimal ISO-country → SAP-language (SPRAS) map. Falls back to "E"
/// (English) for anything not explicitly listed.
fn language_for_country(iso: &str) -> &'static str {
    match iso {
        "DE" | "AT" | "CH" => "D",
        "FR" | "BE" | "LU" => "F",
        "ES" | "MX" | "AR" | "CO" | "CL" => "S",
        "IT" => "I",
        "PT" | "BR" => "P",
        "CN" => "1",
        "JP" => "J",
        "RU" => "R",
        "PL" => "L",
        _ => "E",
    }
}

#[allow(dead_code)]
fn today_ymd() -> String {
    let now = chrono::Utc::now().date_naive();
    format!("{:04}{:02}{:02}", now.year(), now.month(), now.day())
}

// ===========================================================================
// v4.3.0c — Asset / Cost-center / GL-account masters
// ===========================================================================

/// SAP ANLA — asset master general data (one row per FixedAsset).
#[derive(Debug, Clone)]
pub struct SapAsset {
    pub mandt: String,
    pub bukrs: String,
    /// Main asset number (ANLN1).
    pub anln1: String,
    /// Asset sub-number (ANLN2) — "0000" for the main record.
    pub anln2: String,
    /// Asset class (ANLKL).
    pub anlkl: String,
    /// Description (TXT50).
    pub txt50: String,
    /// Capitalisation date (AKTIV).
    pub aktiv: Option<chrono::NaiveDate>,
    /// Acquisition date (ZUGDT).
    pub zugdt: chrono::NaiveDate,
    /// Deactivation / retirement date (DEAKT).
    pub deakt: Option<chrono::NaiveDate>,
    /// Cost centre (KOSTL).
    pub kostl: Option<String>,
    /// Serial number (SERNR).
    pub sernr: Option<String>,
    /// Manufacturer (HERST).
    pub herst: Option<String>,
    /// Creation date (ERDAT).
    pub erdat: chrono::NaiveDate,
    /// Created by (ERNAM).
    pub ernam: String,
}

/// Extension trait for mapping `FixedAsset` → SAP ANLA.
pub trait SapAssetExportable {
    fn to_sap_asset(&self, client: &str) -> SapAsset;
}

impl SapAssetExportable for FixedAsset {
    fn to_sap_asset(&self, client: &str) -> SapAsset {
        SapAsset {
            mandt: client.to_string(),
            bukrs: self.company_code.clone(),
            anln1: self.asset_id.clone(),
            anln2: format!("{:04}", self.sub_number),
            anlkl: asset_class_to_anlkl(&self.asset_class),
            txt50: self.description.clone(),
            aktiv: self.capitalized_date,
            zugdt: self.acquisition_date,
            deakt: self.disposal_date,
            kostl: self.cost_center.clone(),
            sernr: self.serial_number.clone(),
            herst: self.manufacturer.clone(),
            erdat: self.acquisition_date,
            ernam: "SYSTEM".to_string(),
        }
    }
}

/// Write ANLA (fixed-asset master).
pub fn write_anla(cfg: &SapExportConfig, assets: &[FixedAsset], path: &Path) -> SynthResult<()> {
    let mut writer = open_master_file(cfg, path)?;
    let delim = cfg.delimiter();
    write_header(
        &mut writer,
        delim,
        &[
            "MANDT", "BUKRS", "ANLN1", "ANLN2", "ANLKL", "TXT50", "AKTIV", "ZUGDT", "DEAKT",
            "KOSTL", "SERNR", "HERST", "ERDAT", "ERNAM",
        ],
    )?;
    for a in assets {
        let s = a.to_sap_asset(&cfg.client);
        let fields: Vec<String> = vec![
            s.mandt,
            s.bukrs,
            s.anln1,
            s.anln2,
            s.anlkl,
            escape(&s.txt50),
            s.aktiv.map(|d| cfg.format_date(d)).unwrap_or_default(),
            cfg.format_date(s.zugdt),
            s.deakt.map(|d| cfg.format_date(d)).unwrap_or_default(),
            s.kostl.unwrap_or_default(),
            s.sernr.unwrap_or_default(),
            escape(&s.herst.unwrap_or_default()),
            cfg.format_date(s.erdat),
            s.ernam,
        ];
        write_row(&mut writer, delim, &fields)?;
    }
    writer.flush()?;
    Ok(())
}

// ===========================================================================
// CSKS — Cost center master
// ===========================================================================

/// SAP CSKS — cost-centre master. One row per (controlling area, cost
/// centre, validity period). DataSynth emits one row per cost centre
/// using the company code as the controlling area (1:1 mapping in
/// small demos; enterprises typically split).
#[derive(Debug, Clone)]
pub struct SapCostCenter {
    pub mandt: String,
    /// Controlling area (KOKRS) — defaults to the company code.
    pub kokrs: String,
    /// Cost-centre ID (KOSTL).
    pub kostl: String,
    /// Valid-from (DATBI … DATAB in SAP; the validity-to is stored in DATBI
    /// which we leave as 9999-12-31 for "open ended").
    pub datbi: chrono::NaiveDate,
    pub datab: chrono::NaiveDate,
    /// Name / description (KTEXT — via CSKT table in real SAP, flattened
    /// here for analytics convenience).
    pub ktext: String,
    /// Category — `1` = overhead, `F` = production, etc. (KOSAR).
    pub kosar: String,
    /// Responsible person (VERAK_USER).
    pub verak_user: Option<String>,
    /// Blocked-for-actuals flag (BKZKP).
    pub bkzkp: bool,
}

/// Extension trait for mapping `CostCenter` → SAP CSKS.
pub trait SapCostCenterExportable {
    fn to_sap_cost_center(&self, client: &str) -> SapCostCenter;
}

impl SapCostCenterExportable for CostCenter {
    fn to_sap_cost_center(&self, client: &str) -> SapCostCenter {
        SapCostCenter {
            mandt: client.to_string(),
            kokrs: self.company_code.clone(),
            kostl: self.id.clone(),
            datbi: chrono::NaiveDate::from_ymd_opt(9999, 12, 31)
                .expect("9999-12-31 is a valid date"),
            datab: chrono::NaiveDate::from_ymd_opt(2000, 1, 1).expect("2000-01-01 is a valid date"),
            ktext: self.name.clone(),
            kosar: cost_center_category_to_kosar(&self.category),
            verak_user: self.responsible_person.clone(),
            bkzkp: !self.is_active,
        }
    }
}

/// Write CSKS (cost-centre master).
pub fn write_csks(
    cfg: &SapExportConfig,
    cost_centers: &[CostCenter],
    path: &Path,
) -> SynthResult<()> {
    let mut writer = open_master_file(cfg, path)?;
    let delim = cfg.delimiter();
    write_header(
        &mut writer,
        delim,
        &[
            "MANDT",
            "KOKRS",
            "KOSTL",
            "DATBI",
            "DATAB",
            "KTEXT",
            "KOSAR",
            "VERAK_USER",
            "BKZKP",
        ],
    )?;
    for cc in cost_centers {
        let s = cc.to_sap_cost_center(&cfg.client);
        let fields: Vec<String> = vec![
            s.mandt,
            s.kokrs,
            s.kostl,
            cfg.format_date(s.datbi),
            cfg.format_date(s.datab),
            escape(&s.ktext),
            s.kosar,
            s.verak_user.unwrap_or_default(),
            if s.bkzkp {
                "X".to_string()
            } else {
                String::new()
            },
        ];
        write_row(&mut writer, delim, &fields)?;
    }
    writer.flush()?;
    Ok(())
}

// ===========================================================================
// CEPC — Profit centre master (v5.1)
// ===========================================================================

/// SAP CEPC — profit-centre master.  One row per (controlling area,
/// profit centre, validity period).  DataSynth emits one row per
/// profit centre using the company code as the controlling area
/// (1:1 mapping in small demos; enterprises typically split).
///
/// CEPC is the canonical SAP table for profit-centre master data in
/// the CO-PCA (Profit Centre Accounting) module.  The `PRCTR` key
/// joins to `BSEG.PRCTR` / `ACDOCA.PRCTR` on every line item that
/// carries a profit-centre attribution.
#[derive(Debug, Clone)]
pub struct SapProfitCenter {
    pub mandt: String,
    /// Controlling area (KOKRS) — defaults to the company code.
    pub kokrs: String,
    /// Profit centre ID (PRCTR).
    pub prctr: String,
    /// Valid-to (DATBI) — `9999-12-31` for "open ended".
    pub datbi: chrono::NaiveDate,
    /// Valid-from (DATAB).
    pub datab: chrono::NaiveDate,
    /// Description / name (KTEXT — flattened from CEPCT in real SAP).
    pub ktext: String,
    /// Responsible person (VERAK_USER).
    pub verak_user: Option<String>,
    /// Lock indicator (LOKKZ): `X` when the centre is inactive.
    pub lokkz: bool,
    /// Department / segment code (ABTEI) — propagated from the
    /// `ProfitCenter.segment_code` so consumers can group by IFRS 8
    /// reportable segment without joining a sidecar.
    pub abtei: Option<String>,
    /// Hierarchy node (HIE_KIND) — encodes whether this is a top-level
    /// node ("S" for summary) or a leaf ("D" for detail).
    pub hie_kind: String,
}

/// Extension trait for mapping `ProfitCenter` → SAP CEPC.
pub trait SapProfitCenterExportable {
    fn to_sap_profit_center(&self, client: &str) -> SapProfitCenter;
}

impl SapProfitCenterExportable for ProfitCenter {
    fn to_sap_profit_center(&self, client: &str) -> SapProfitCenter {
        SapProfitCenter {
            mandt: client.to_string(),
            kokrs: self.company_code.clone(),
            prctr: self.id.clone(),
            datbi: chrono::NaiveDate::from_ymd_opt(9999, 12, 31)
                .expect("9999-12-31 is a valid date"),
            datab: chrono::NaiveDate::from_ymd_opt(2000, 1, 1).expect("2000-01-01 is a valid date"),
            ktext: self.name.clone(),
            verak_user: self.responsible_person.clone(),
            lokkz: !self.is_active,
            abtei: self.segment_code.clone(),
            hie_kind: if self.level == 1 { "S" } else { "D" }.to_string(),
        }
    }
}

/// Write CEPC (profit-centre master).
///
/// v5.1: closes the v5.0.1 documentation-only mitigation of Gap 6.
/// CEPC is now a first-class master-data table alongside CSKS — the
/// CLI's `output.sap.tables` config no longer warns when `cepc` is
/// requested.
pub fn write_cepc(
    cfg: &SapExportConfig,
    profit_centers: &[ProfitCenter],
    path: &Path,
) -> SynthResult<()> {
    let mut writer = open_master_file(cfg, path)?;
    let delim = cfg.delimiter();
    write_header(
        &mut writer,
        delim,
        &[
            "MANDT",
            "KOKRS",
            "PRCTR",
            "DATBI",
            "DATAB",
            "KTEXT",
            "VERAK_USER",
            "LOKKZ",
            "ABTEI",
            "HIE_KIND",
        ],
    )?;
    for pc in profit_centers {
        let s = pc.to_sap_profit_center(&cfg.client);
        let fields: Vec<String> = vec![
            s.mandt,
            s.kokrs,
            s.prctr,
            cfg.format_date(s.datbi),
            cfg.format_date(s.datab),
            escape(&s.ktext),
            s.verak_user.unwrap_or_default(),
            if s.lokkz {
                "X".to_string()
            } else {
                String::new()
            },
            s.abtei.unwrap_or_default(),
            s.hie_kind,
        ];
        write_row(&mut writer, delim, &fields)?;
    }
    writer.flush()?;
    Ok(())
}

// ===========================================================================
// SKA1 / SKB1 — GL account masters
// ===========================================================================

/// SAP SKA1 — chart-of-accounts-level GL account (client-wide).
#[derive(Debug, Clone)]
pub struct SapGlAccountGeneral {
    pub mandt: String,
    /// Chart of accounts code (KTOPL).
    pub ktopl: String,
    /// GL account number (SAKNR).
    pub saknr: String,
    /// Account group (KTOKS).
    pub ktoks: String,
    /// Balance sheet (1) or P&L (2) (XBILK).
    pub xbilk: u8,
    /// P&L statement account type (GVTYP) — blank for balance-sheet accounts.
    pub gvtyp: Option<String>,
    /// Creation date (ERDAT).
    pub erdat: chrono::NaiveDate,
    /// Created by (ERNAM).
    pub ernam: String,
}

/// SAP SKB1 — company-code-level GL account data.
#[derive(Debug, Clone)]
pub struct SapGlAccountCompanyCode {
    pub mandt: String,
    pub bukrs: String,
    pub saknr: String,
    /// Account currency (WAERS).
    pub waers: String,
    /// Open-item management flag (XOPVW).
    pub xopvw: bool,
    /// Line-item display flag (XKRES).
    pub xkres: bool,
    /// Posting blocked flag (XSPEB).
    pub xspeb: bool,
    /// Tax category (MWSKZ).
    pub mwskz: Option<String>,
    /// Reconciliation account type (MITKZ) — "D" (debtor), "K" (vendor),
    /// "A" (asset), blank for regular GL accounts.
    pub mitkz: Option<String>,
}

/// Extension trait for mapping `GLAccount` → SAP SKA1.
pub trait SapGlAccountExportable {
    fn to_sap_gl_general(&self, client: &str, ktopl: &str) -> SapGlAccountGeneral;
    fn to_sap_gl_company_code(
        &self,
        client: &str,
        company_code: &str,
        currency: &str,
    ) -> SapGlAccountCompanyCode;
}

impl SapGlAccountExportable for GLAccount {
    fn to_sap_gl_general(&self, client: &str, ktopl: &str) -> SapGlAccountGeneral {
        use datasynth_core::models::AccountType;
        let xbilk = matches!(
            self.account_type,
            AccountType::Asset | AccountType::Liability | AccountType::Equity
        );
        SapGlAccountGeneral {
            mandt: client.to_string(),
            ktopl: ktopl.to_string(),
            saknr: self.account_number.clone(),
            ktoks: self.account_group.clone(),
            xbilk: if xbilk { 1 } else { 2 },
            gvtyp: if !xbilk { Some("H".to_string()) } else { None },
            erdat: chrono::Utc::now().date_naive(),
            ernam: "SYSTEM".to_string(),
        }
    }

    fn to_sap_gl_company_code(
        &self,
        client: &str,
        company_code: &str,
        currency: &str,
    ) -> SapGlAccountCompanyCode {
        use datasynth_core::models::AccountType;
        let mitkz = if self.is_control_account {
            match self.account_type {
                AccountType::Asset => Some("D".to_string()), // AR control
                AccountType::Liability => Some("K".to_string()), // AP control
                _ => None,
            }
        } else {
            None
        };
        SapGlAccountCompanyCode {
            mandt: client.to_string(),
            bukrs: company_code.to_string(),
            saknr: self.account_number.clone(),
            waers: currency.to_string(),
            xopvw: self.is_control_account || self.is_suspense_account,
            xkres: self.is_postable,
            xspeb: self.is_blocked,
            mwskz: None,
            mitkz,
        }
    }
}

/// Write SKA1 (chart-of-accounts-level GL master).
///
/// Takes a `ChartOfAccounts` rather than a `&[GLAccount]` so the
/// `KTOPL` column (chart-of-accounts code) is available — SAP separates
/// the chart of accounts (country/group-specific, like INT/SKR04/CAUS)
/// from the individual account rows.
pub fn write_ska1(cfg: &SapExportConfig, coa: &ChartOfAccounts, path: &Path) -> SynthResult<()> {
    let mut writer = open_master_file(cfg, path)?;
    let delim = cfg.delimiter();
    write_header(
        &mut writer,
        delim,
        &[
            "MANDT", "KTOPL", "SAKNR", "KTOKS", "XBILK", "GVTYP", "ERDAT", "ERNAM",
        ],
    )?;
    let ktopl = coa.coa_id.as_str();
    for acct in &coa.accounts {
        let s = acct.to_sap_gl_general(&cfg.client, ktopl);
        let fields: Vec<String> = vec![
            s.mandt,
            s.ktopl,
            s.saknr,
            s.ktoks,
            s.xbilk.to_string(),
            s.gvtyp.unwrap_or_default(),
            cfg.format_date(s.erdat),
            s.ernam,
        ];
        write_row(&mut writer, delim, &fields)?;
    }
    writer.flush()?;
    Ok(())
}

/// Write SKB1 (company-code-level GL master) — one row per (account, company).
pub fn write_skb1(
    cfg: &SapExportConfig,
    coa: &ChartOfAccounts,
    company_codes: &[String],
    path: &Path,
) -> SynthResult<()> {
    let mut writer = open_master_file(cfg, path)?;
    let delim = cfg.delimiter();
    write_header(
        &mut writer,
        delim,
        &[
            "MANDT", "BUKRS", "SAKNR", "WAERS", "XOPVW", "XKRES", "XSPEB", "MWSKZ", "MITKZ",
        ],
    )?;
    for acct in &coa.accounts {
        for company in company_codes {
            let s = acct.to_sap_gl_company_code(&cfg.client, company, &cfg.local_currency);
            let fields: Vec<String> = vec![
                s.mandt,
                s.bukrs,
                s.saknr,
                s.waers,
                if s.xopvw {
                    "X".to_string()
                } else {
                    String::new()
                },
                if s.xkres {
                    "X".to_string()
                } else {
                    String::new()
                },
                if s.xspeb {
                    "X".to_string()
                } else {
                    String::new()
                },
                s.mwskz.unwrap_or_default(),
                s.mitkz.unwrap_or_default(),
            ];
            write_row(&mut writer, delim, &fields)?;
        }
    }
    writer.flush()?;
    Ok(())
}

// ===========================================================================
// Helper mappings for v4.3.0c
// ===========================================================================

/// Map `FixedAsset.asset_class` → SAP ANLKL asset-class code (4-digit).
fn asset_class_to_anlkl(class: &datasynth_core::models::AssetClass) -> String {
    use datasynth_core::models::AssetClass;
    match class {
        AssetClass::Buildings | AssetClass::BuildingImprovements => "1000",
        AssetClass::Land => "1100",
        AssetClass::MachineryEquipment | AssetClass::Machinery => "2000",
        AssetClass::ComputerHardware | AssetClass::ItEquipment => "5000",
        AssetClass::FurnitureFixtures | AssetClass::Furniture => "4000",
        AssetClass::Vehicles => "3000",
        AssetClass::LeaseholdImprovements => "4500",
        AssetClass::Intangibles | AssetClass::Software => "7000",
        AssetClass::ConstructionInProgress => "8000",
        AssetClass::LowValueAssets => "9000",
    }
    .to_string()
}

/// Map `CostCenter.category` → SAP KOSAR cost-centre category code.
fn cost_center_category_to_kosar(category: &datasynth_core::models::CostCenterCategory) -> String {
    use datasynth_core::models::CostCenterCategory;
    match category {
        CostCenterCategory::Production => "F",
        CostCenterCategory::Administration => "H",
        CostCenterCategory::Sales => "V",
        CostCenterCategory::RAndD => "E",
        CostCenterCategory::Corporate => "1",
    }
    .to_string()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::super::sap::SapDialect;
    use super::*;
    use datasynth_core::models::{CustomerType, MaterialType, VendorType};
    use tempfile::TempDir;

    fn sample_vendor() -> Vendor {
        let mut v = Vendor::new("V-0001", "Müller GmbH", VendorType::Supplier);
        v.country = "DE".to_string();
        v.tax_id = Some("DE123456789".to_string());
        v.reconciliation_account = Some("2000".to_string());
        v.withholding_tax_applicable = true;
        v
    }

    fn sample_customer() -> Customer {
        let mut c = Customer::new("C-0001", "Retail Corp", CustomerType::Corporate);
        c.country = "US".to_string();
        c.reconciliation_account = Some("1100".to_string());
        c.dunning_level = 2;
        c.credit_blocked = true;
        c.credit_block_reason = Some("A".to_string());
        c
    }

    fn sample_material() -> Material {
        let mut m = Material::new("MAT-0001", "Steel coil 1.5mm", MaterialType::RawMaterial);
        m.weight_kg = Some(rust_decimal::Decimal::new(15, 0));
        m.plants = vec!["PLNT01".to_string(), "PLNT02".to_string()];
        m
    }

    #[test]
    fn lfa1_row_round_trip_maps_core_fields() {
        let v = sample_vendor();
        let row = v.to_sap_vendor("100");
        assert_eq!(row.mandt, "100");
        assert_eq!(row.lifnr, "V-0001");
        assert_eq!(row.land1, "DE");
        assert_eq!(row.name1, "Müller GmbH");
        assert_eq!(row.spras, "D", "DE country must map to SPRAS=D (German)");
        assert_eq!(row.stcd1.as_deref(), Some("DE123456789"));
        assert_eq!(row.ktokk, "LIEF");
    }

    #[test]
    fn kna1_row_round_trip_maps_core_fields() {
        let c = sample_customer();
        let row = c.to_sap_customer("100");
        assert_eq!(row.kunnr, "C-0001");
        assert_eq!(row.land1, "US");
        assert_eq!(row.spras, "E");
        assert_eq!(row.ktokd, "KUNA");
    }

    #[test]
    fn lfb1_row_carries_reconciliation_account_and_withholding() {
        let v = sample_vendor();
        let row = v.to_sap_vendor_company_code("100", "C001");
        assert_eq!(row.bukrs, "C001");
        assert_eq!(row.akont, "2000");
        assert_eq!(
            row.qsskz.as_deref(),
            Some("W1"),
            "withholding-applicable vendor must emit a QSSKZ code"
        );
    }

    #[test]
    fn knb1_row_carries_dunning_and_credit_block() {
        let c = sample_customer();
        let row = c.to_sap_customer_company_code("100", "C001");
        assert_eq!(row.bukrs, "C001");
        assert_eq!(row.akont, "1100");
        assert_eq!(row.mahns, 2);
        assert_eq!(row.crdblk, 1, "credit-blocked customer must emit CRDBLK=1");
        assert_eq!(row.zahls.as_deref(), Some("A"));
    }

    #[test]
    fn mara_row_maps_material_type_to_mtart() {
        let m = sample_material();
        let row = m.to_sap_material("100");
        assert_eq!(row.matnr, "MAT-0001");
        assert_eq!(row.mtart, "ROH", "raw-material type must map to MTART=ROH");
    }

    #[test]
    fn mard_row_emitted_per_plant() {
        let m = sample_material();
        let stock: Vec<(String, rust_decimal::Decimal)> = vec![
            ("PLNT01".to_string(), rust_decimal::Decimal::new(100, 0)),
            ("PLNT02".to_string(), rust_decimal::Decimal::new(50, 0)),
        ];
        let rows = m.to_sap_material_storage_rows("100", "0001", &stock);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].werks, "PLNT01");
        assert_eq!(rows[0].labst, rust_decimal::Decimal::new(100, 0));
        assert_eq!(rows[1].werks, "PLNT02");
    }

    #[test]
    fn cepc_emits_one_row_per_profit_center_with_segment_propagated() {
        use datasynth_core::models::{ProfitCenter, ProfitCenterCategory};
        let tmp = TempDir::new().unwrap();
        let cfg = SapExportConfig::default();
        let pcs = vec![
            ProfitCenter::top_level("PC-EMEA", "EMEA", "C001", ProfitCenterCategory::Region)
                .with_segment("SEG-EMEA"),
            ProfitCenter::sub_unit(
                "PC-EMEA-DACH",
                "DACH",
                "PC-EMEA",
                "C001",
                ProfitCenterCategory::Region,
            )
            .with_segment("SEG-EMEA"),
        ];
        let path = tmp.path().join("cepc.csv");
        write_cepc(&cfg, &pcs, &path).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3, "header + 2 profit-centre rows");
        assert!(lines[0].starts_with("MANDT"));
        assert!(lines[0].contains("PRCTR"));
        assert!(lines[0].contains("ABTEI"));
        assert!(lines[0].contains("HIE_KIND"));

        // First data row is the level-1 segment node (HIE_KIND=S).
        assert!(lines[1].contains("PC-EMEA"));
        assert!(lines[1].contains("SEG-EMEA"));
        assert!(
            lines[1].ends_with(",S") || lines[1].contains(",S,") || lines[1].contains("\tS"),
            "level-1 should map to HIE_KIND=S, got: {}",
            lines[1]
        );

        // Second data row is the level-2 sub-unit (HIE_KIND=D), shares segment.
        assert!(lines[2].contains("PC-EMEA-DACH"));
        assert!(lines[2].contains("SEG-EMEA"));
    }

    #[test]
    fn master_files_written_with_hana_dialect_use_semicolon_and_bom() {
        let tmp = TempDir::new().unwrap();
        let cfg = SapExportConfig {
            dialect: SapDialect::Hana,
            ..SapExportConfig::default()
        };
        let vendors = vec![sample_vendor()];
        let lfa1_path = tmp.path().join("lfa1.csv");
        write_lfa1(&cfg, &vendors, &lfa1_path).unwrap();

        let bytes = std::fs::read(&lfa1_path).unwrap();
        assert_eq!(
            &bytes[..3],
            [0xEF, 0xBB, 0xBF],
            "Hana dialect must prefix master-data files with a UTF-8 BOM"
        );
        let text = std::str::from_utf8(&bytes[3..]).unwrap();
        assert!(text.lines().next().unwrap().contains(';'));
    }
}
