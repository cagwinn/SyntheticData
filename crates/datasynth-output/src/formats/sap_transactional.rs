//! SAP transactional-table exporters — v4.3.0d.
//!
//! Maps the DataSynth document-flow models to the SAP transactional
//! tables that accompany the GL triple (BKPF/BSEG/ACDOCA) and the
//! master-data set from v4.3.0b/c:
//!
//! | DataSynth model  | SAP header table | SAP item table |
//! |------------------|------------------|----------------|
//! | `PurchaseOrder`  | EKKO             | EKPO           |
//! | `SalesOrder`     | VBAK             | VBAP           |
//! | `Delivery`       | LIKP             | LIPS           |
//! | `GoodsReceipt`   | MKPF             | MSEG           |
//!
//! Each pair produces one row per document (header) and one row per
//! line item (item). MSEG additionally inherits MKPF's MBLNR/MJAHR keys
//! so downstream consumers can reconstruct the header/item relationship
//! without a join on the DataSynth `document_id`.
//!
//! All writers honour the `SapDialect` setting of the parent
//! `SapExportConfig` (delimiter / decimal separator / date format /
//! UTF-8 BOM).

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use chrono::NaiveDate;
use datasynth_core::error::SynthResult;
use datasynth_core::models::documents::{Delivery, GoodsReceipt, PurchaseOrder, SalesOrder};
use rust_decimal::Decimal;

use super::sap::SapExportConfig;

// ---------------------------------------------------------------------------
// Shared helpers (duplicated from sap_master_data so this module stays
// self-contained and future-refactorable).
// ---------------------------------------------------------------------------

fn open_file(cfg: &SapExportConfig, path: &Path) -> SynthResult<BufWriter<File>> {
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

fn opt_date(cfg: &SapExportConfig, d: Option<NaiveDate>) -> String {
    d.map(|x| cfg.format_date(x)).unwrap_or_default()
}

fn dec(cfg: &SapExportConfig, v: &Decimal) -> String {
    cfg.format_decimal(v)
}

// ===========================================================================
// EKKO — Purchasing document header (one row per PurchaseOrder)
// ===========================================================================

/// SAP EKKO — PO header. SAP's full EKKO has ~150 columns; this carries
/// the subset that analytics consumers need and that DataSynth can
/// meaningfully populate.
#[derive(Debug, Clone)]
pub struct SapPoHeader {
    pub mandt: String,
    /// Purchasing document number (EBELN).
    pub ebeln: String,
    /// Company code (BUKRS).
    pub bukrs: String,
    /// Document category (BSTYP) — "F" for PO, "A" for RFQ, "K" for contract.
    pub bstyp: String,
    /// Document type (BSART) — "NB" = standard PO, "UB" = stock transport.
    pub bsart: String,
    /// Vendor (LIFNR).
    pub lifnr: String,
    /// Purchasing organisation (EKORG).
    pub ekorg: String,
    /// Purchasing group (EKGRP).
    pub ekgrp: String,
    /// Creation date (AEDAT).
    pub aedat: NaiveDate,
    /// Document date (BEDAT).
    pub bedat: NaiveDate,
    /// Created by (ERNAM).
    pub ernam: String,
    /// Currency (WAERS).
    pub waers: String,
    /// Terms of payment (ZTERM).
    pub zterm: String,
    /// Incoterms part 1 (INCO1).
    pub inco1: Option<String>,
    /// Incoterms part 2 (INCO2).
    pub inco2: Option<String>,
    /// Reference / external number (IHREZ).
    pub ihrez: Option<String>,
}

/// Extension trait mapping `PurchaseOrder` → SAP EKKO.
pub trait SapPoExportable {
    fn to_sap_ekko(&self, client: &str) -> SapPoHeader;
    fn to_sap_ekpo_rows(&self, client: &str) -> Vec<SapPoItem>;
}

/// SAP EKPO — PO line item (one row per `PurchaseOrderItem`).
#[derive(Debug, Clone)]
pub struct SapPoItem {
    pub mandt: String,
    /// Purchasing document number (EBELN) — matches the parent EKKO row.
    pub ebeln: String,
    /// Item number (EBELP).
    pub ebelp: String,
    /// Material number (MATNR).
    pub matnr: Option<String>,
    /// Short text (TXZ01).
    pub txz01: String,
    /// Item category (PSTYP).
    pub pstyp: String,
    /// Account assignment category (KNTTP).
    pub knttp: String,
    /// Plant (WERKS).
    pub werks: Option<String>,
    /// Storage location (LGORT).
    pub lgort: Option<String>,
    /// Quantity (MENGE).
    pub menge: Decimal,
    /// Unit of measure (MEINS).
    pub meins: String,
    /// Net price (NETPR).
    pub netpr: Decimal,
    /// Price unit (PEINH) — always 1 in DataSynth (price is per single unit).
    pub peinh: u32,
    /// Net order value (NETWR).
    pub netwr: Decimal,
    /// GR-based IV flag (WEBRE).
    pub webre: bool,
    /// Quantity received (WEMNG).
    pub wemng: Decimal,
    /// Quantity invoiced (REMNG).
    pub remng: Decimal,
    /// Delivery date (EINDT).
    pub eindt: Option<NaiveDate>,
    /// Deletion indicator (LOEKZ).
    pub loekz: bool,
}

impl SapPoExportable for PurchaseOrder {
    fn to_sap_ekko(&self, client: &str) -> SapPoHeader {
        SapPoHeader {
            mandt: client.to_string(),
            ebeln: self.header.document_id.clone(),
            bukrs: self.header.company_code.clone(),
            bstyp: "F".to_string(),
            bsart: po_type_to_bsart(&self.po_type),
            lifnr: self.vendor_id.clone(),
            ekorg: self.purchasing_org.clone(),
            ekgrp: self.purchasing_group.clone(),
            aedat: self.header.entry_date,
            bedat: self.header.document_date,
            ernam: self.header.created_by.clone(),
            waers: self.header.currency.clone(),
            zterm: format!("{:?}", self.payment_terms),
            inco1: self.incoterms.clone(),
            inco2: self.incoterms_location.clone(),
            ihrez: self.header.reference.clone(),
        }
    }

    fn to_sap_ekpo_rows(&self, client: &str) -> Vec<SapPoItem> {
        self.items
            .iter()
            .map(|item| SapPoItem {
                mandt: client.to_string(),
                ebeln: self.header.document_id.clone(),
                ebelp: format!("{:05}", item.base.line_number),
                matnr: item.base.material_id.clone(),
                txz01: item.base.description.clone(),
                pstyp: match item.item_category.as_str() {
                    "SERVICE" => "9",
                    "LIMIT" => "B",
                    _ => "0",
                }
                .to_string(),
                knttp: item.account_assignment_category.clone(),
                werks: item.base.plant.clone(),
                lgort: item.base.storage_location.clone(),
                menge: item.base.quantity,
                meins: item.base.uom.clone(),
                netpr: item.base.unit_price,
                peinh: 1,
                netwr: item.base.net_amount,
                webre: item.gr_based_iv,
                wemng: item.quantity_received,
                remng: item.quantity_invoiced,
                eindt: item.requested_date,
                loekz: self.is_closed,
            })
            .collect()
    }
}

fn po_type_to_bsart(pt: &datasynth_core::documents::PurchaseOrderType) -> String {
    use datasynth_core::documents::PurchaseOrderType;
    match pt {
        PurchaseOrderType::Standard => "NB",
        PurchaseOrderType::Framework => "FO",
        PurchaseOrderType::Service => "DB",
        PurchaseOrderType::StockTransfer => "UB",
        PurchaseOrderType::Subcontracting => "LB",
        PurchaseOrderType::Consignment => "K",
    }
    .to_string()
}

/// Write EKKO.
pub fn write_ekko(cfg: &SapExportConfig, pos: &[PurchaseOrder], path: &Path) -> SynthResult<()> {
    let mut w = open_file(cfg, path)?;
    let d = cfg.delimiter();
    write_header(
        &mut w,
        d,
        &[
            "MANDT", "EBELN", "BUKRS", "BSTYP", "BSART", "LIFNR", "EKORG", "EKGRP", "AEDAT",
            "BEDAT", "ERNAM", "WAERS", "ZTERM", "INCO1", "INCO2", "IHREZ",
        ],
    )?;
    for po in pos {
        let s = po.to_sap_ekko(&cfg.client);
        let fields = vec![
            s.mandt,
            s.ebeln,
            s.bukrs,
            s.bstyp,
            s.bsart,
            s.lifnr,
            s.ekorg,
            s.ekgrp,
            cfg.format_date(s.aedat),
            cfg.format_date(s.bedat),
            s.ernam,
            s.waers,
            escape(&s.zterm),
            s.inco1.unwrap_or_default(),
            s.inco2.unwrap_or_default(),
            s.ihrez.unwrap_or_default(),
        ];
        write_row(&mut w, d, &fields)?;
    }
    w.flush()?;
    Ok(())
}

/// Write EKPO.
pub fn write_ekpo(cfg: &SapExportConfig, pos: &[PurchaseOrder], path: &Path) -> SynthResult<()> {
    let mut w = open_file(cfg, path)?;
    let d = cfg.delimiter();
    write_header(
        &mut w,
        d,
        &[
            "MANDT", "EBELN", "EBELP", "MATNR", "TXZ01", "PSTYP", "KNTTP", "WERKS", "LGORT",
            "MENGE", "MEINS", "NETPR", "PEINH", "NETWR", "WEBRE", "WEMNG", "REMNG", "EINDT",
            "LOEKZ",
        ],
    )?;
    for po in pos {
        for s in po.to_sap_ekpo_rows(&cfg.client) {
            let fields = vec![
                s.mandt,
                s.ebeln,
                s.ebelp,
                s.matnr.unwrap_or_default(),
                escape(&s.txz01),
                s.pstyp,
                s.knttp,
                s.werks.unwrap_or_default(),
                s.lgort.unwrap_or_default(),
                dec(cfg, &s.menge),
                s.meins,
                dec(cfg, &s.netpr),
                s.peinh.to_string(),
                dec(cfg, &s.netwr),
                if s.webre {
                    "X".to_string()
                } else {
                    String::new()
                },
                dec(cfg, &s.wemng),
                dec(cfg, &s.remng),
                opt_date(cfg, s.eindt),
                if s.loekz {
                    "X".to_string()
                } else {
                    String::new()
                },
            ];
            write_row(&mut w, d, &fields)?;
        }
    }
    w.flush()?;
    Ok(())
}

// ===========================================================================
// VBAK / VBAP — Sales order header / item
// ===========================================================================

/// SAP VBAK — sales-order header.
#[derive(Debug, Clone)]
pub struct SapSoHeader {
    pub mandt: String,
    /// Sales document (VBELN).
    pub vbeln: String,
    /// Sales document type (AUART).
    pub auart: String,
    /// Sales organisation (VKORG).
    pub vkorg: String,
    /// Distribution channel (VTWEG).
    pub vtweg: Option<String>,
    /// Division (SPART).
    pub spart: Option<String>,
    /// Sold-to party (KUNNR).
    pub kunnr: String,
    /// Document date (AUDAT).
    pub audat: NaiveDate,
    /// Net document value (NETWR).
    pub netwr: Decimal,
    /// Currency (WAERK).
    pub waerk: String,
    /// Order reason (AUGRU).
    pub augru: Option<String>,
    /// Requested delivery date (VDATU).
    pub vdatu: Option<NaiveDate>,
    /// Payment terms (ZTERM).
    pub zterm: String,
    /// Incoterms part 1 (INCO1).
    pub inco1: Option<String>,
}

/// SAP VBAP — sales-order item.
#[derive(Debug, Clone)]
pub struct SapSoItem {
    pub mandt: String,
    pub vbeln: String,
    pub posnr: String,
    pub matnr: Option<String>,
    pub arktx: String,
    /// Item category (PSTYV).
    pub pstyv: String,
    /// Order quantity (KWMENG).
    pub kwmeng: Decimal,
    /// Unit of measure (VRKME).
    pub vrkme: String,
    /// Net price (NETPR).
    pub netpr: Decimal,
    /// Net value (NETWR).
    pub netwr: Decimal,
    /// Plant (WERKS).
    pub werks: Option<String>,
    /// Storage location (LGORT).
    pub lgort: Option<String>,
    /// Requested delivery date (EDATU).
    pub edatu: Option<NaiveDate>,
    /// Rejection reason (ABGRU).
    pub abgru: Option<String>,
}

/// Extension trait `SalesOrder` → SAP VBAK / VBAP.
pub trait SapSoExportable {
    fn to_sap_vbak(&self, client: &str) -> SapSoHeader;
    fn to_sap_vbap_rows(&self, client: &str) -> Vec<SapSoItem>;
}

impl SapSoExportable for SalesOrder {
    fn to_sap_vbak(&self, client: &str) -> SapSoHeader {
        SapSoHeader {
            mandt: client.to_string(),
            vbeln: self.header.document_id.clone(),
            auart: so_type_to_auart(&self.so_type),
            vkorg: self.sales_org.clone(),
            vtweg: Some(self.distribution_channel.clone()),
            spart: Some(self.division.clone()),
            kunnr: self.customer_id.clone(),
            audat: self.header.document_date,
            netwr: self.total_net_amount,
            waerk: self.header.currency.clone(),
            augru: None,
            vdatu: self.requested_delivery_date,
            zterm: format!("{:?}", self.payment_terms),
            inco1: self.incoterms.clone(),
        }
    }

    fn to_sap_vbap_rows(&self, client: &str) -> Vec<SapSoItem> {
        self.items
            .iter()
            .map(|item| SapSoItem {
                mandt: client.to_string(),
                vbeln: self.header.document_id.clone(),
                posnr: format!("{:06}", item.base.line_number),
                matnr: item.base.material_id.clone(),
                arktx: item.base.description.clone(),
                pstyv: "TAN".to_string(),
                kwmeng: item.base.quantity,
                vrkme: item.base.uom.clone(),
                netpr: item.base.unit_price,
                netwr: item.base.net_amount,
                werks: item.base.plant.clone(),
                lgort: item.base.storage_location.clone(),
                edatu: item.base.delivery_date,
                abgru: None,
            })
            .collect()
    }
}

fn so_type_to_auart(so: &datasynth_core::documents::SalesOrderType) -> String {
    use datasynth_core::documents::SalesOrderType;
    match so {
        SalesOrderType::Standard => "OR",
        SalesOrderType::Rush => "SO",
        SalesOrderType::CashSale => "BV",
        SalesOrderType::Return => "RE",
        SalesOrderType::FreeOfCharge => "FD",
        SalesOrderType::Consignment => "KB",
        SalesOrderType::Service => "DS",
        SalesOrderType::CreditMemoRequest => "G2",
        SalesOrderType::DebitMemoRequest => "L2",
    }
    .to_string()
}

/// Write VBAK.
pub fn write_vbak(cfg: &SapExportConfig, sos: &[SalesOrder], path: &Path) -> SynthResult<()> {
    let mut w = open_file(cfg, path)?;
    let d = cfg.delimiter();
    write_header(
        &mut w,
        d,
        &[
            "MANDT", "VBELN", "AUART", "VKORG", "VTWEG", "SPART", "KUNNR", "AUDAT", "NETWR",
            "WAERK", "AUGRU", "VDATU", "ZTERM", "INCO1",
        ],
    )?;
    for so in sos {
        let s = so.to_sap_vbak(&cfg.client);
        let fields = vec![
            s.mandt,
            s.vbeln,
            s.auart,
            s.vkorg,
            s.vtweg.unwrap_or_default(),
            s.spart.unwrap_or_default(),
            s.kunnr,
            cfg.format_date(s.audat),
            dec(cfg, &s.netwr),
            s.waerk,
            s.augru.unwrap_or_default(),
            opt_date(cfg, s.vdatu),
            escape(&s.zterm),
            s.inco1.unwrap_or_default(),
        ];
        write_row(&mut w, d, &fields)?;
    }
    w.flush()?;
    Ok(())
}

/// Write VBAP.
pub fn write_vbap(cfg: &SapExportConfig, sos: &[SalesOrder], path: &Path) -> SynthResult<()> {
    let mut w = open_file(cfg, path)?;
    let d = cfg.delimiter();
    write_header(
        &mut w,
        d,
        &[
            "MANDT", "VBELN", "POSNR", "MATNR", "ARKTX", "PSTYV", "KWMENG", "VRKME", "NETPR",
            "NETWR", "WERKS", "LGORT", "EDATU", "ABGRU",
        ],
    )?;
    for so in sos {
        for s in so.to_sap_vbap_rows(&cfg.client) {
            let fields = vec![
                s.mandt,
                s.vbeln,
                s.posnr,
                s.matnr.unwrap_or_default(),
                escape(&s.arktx),
                s.pstyv,
                dec(cfg, &s.kwmeng),
                s.vrkme,
                dec(cfg, &s.netpr),
                dec(cfg, &s.netwr),
                s.werks.unwrap_or_default(),
                s.lgort.unwrap_or_default(),
                opt_date(cfg, s.edatu),
                s.abgru.unwrap_or_default(),
            ];
            write_row(&mut w, d, &fields)?;
        }
    }
    w.flush()?;
    Ok(())
}

// ===========================================================================
// LIKP / LIPS — Delivery header / item
// ===========================================================================

/// SAP LIKP — delivery header.
#[derive(Debug, Clone)]
pub struct SapDeliveryHeader {
    pub mandt: String,
    /// Delivery document number (VBELN).
    pub vbeln: String,
    /// Delivery type (LFART) — "LF" = outbound, "EL" = inbound.
    pub lfart: String,
    /// Shipping point (VSTEL).
    pub vstel: Option<String>,
    /// Route (ROUTE).
    pub route: Option<String>,
    /// Ship-to party (KUNNR).
    pub kunnr: String,
    /// Delivery date (WADAT).
    pub wadat: Option<NaiveDate>,
    /// Planned goods issue date (WADAT_IST).
    pub wadat_ist: Option<NaiveDate>,
    /// Currency (WAERK).
    pub waerk: String,
    /// Carrier (SDABW).
    pub sdabw: Option<String>,
    /// Document date (ERDAT).
    pub erdat: NaiveDate,
    /// Delivery status (LIFSK) — blank = unblocked, anything else = blocked.
    pub lifsk: Option<String>,
}

/// SAP LIPS — delivery item.
#[derive(Debug, Clone)]
pub struct SapDeliveryItem {
    pub mandt: String,
    pub vbeln: String,
    pub posnr: String,
    pub matnr: Option<String>,
    pub arktx: String,
    /// Delivery quantity (LFIMG).
    pub lfimg: Decimal,
    /// Unit of measure (VRKME).
    pub vrkme: String,
    /// Plant (WERKS).
    pub werks: Option<String>,
    /// Storage location (LGORT).
    pub lgort: Option<String>,
    /// Reference PO (VGBEL).
    pub vgbel: Option<String>,
    /// Reference PO item (VGPOS).
    pub vgpos: Option<String>,
    /// Batch (CHARG).
    pub charg: Option<String>,
}

/// Extension trait `Delivery` → SAP LIKP / LIPS.
pub trait SapDeliveryExportable {
    fn to_sap_likp(&self, client: &str) -> SapDeliveryHeader;
    fn to_sap_lips_rows(&self, client: &str) -> Vec<SapDeliveryItem>;
}

impl SapDeliveryExportable for Delivery {
    fn to_sap_likp(&self, client: &str) -> SapDeliveryHeader {
        SapDeliveryHeader {
            mandt: client.to_string(),
            vbeln: self.header.document_id.clone(),
            lfart: delivery_type_to_lfart(&self.delivery_type),
            vstel: Some(self.shipping_point.clone()),
            route: self.route.clone(),
            kunnr: self.customer_id.clone(),
            wadat: self.delivery_date,
            wadat_ist: self.actual_gi_date,
            waerk: self.header.currency.clone(),
            sdabw: self.carrier.clone(),
            erdat: self.header.entry_date,
            lifsk: None,
        }
    }

    fn to_sap_lips_rows(&self, client: &str) -> Vec<SapDeliveryItem> {
        self.items
            .iter()
            .map(|item| SapDeliveryItem {
                mandt: client.to_string(),
                vbeln: self.header.document_id.clone(),
                posnr: format!("{:06}", item.base.line_number),
                matnr: item.base.material_id.clone(),
                arktx: item.base.description.clone(),
                lfimg: item.base.quantity,
                vrkme: item.base.uom.clone(),
                werks: item.base.plant.clone(),
                lgort: item.base.storage_location.clone(),
                vgbel: item.sales_order_id.clone(),
                vgpos: item.so_item.map(|p| format!("{:05}", p)),
                charg: item.batch.clone(),
            })
            .collect()
    }
}

fn delivery_type_to_lfart(dt: &datasynth_core::documents::DeliveryType) -> String {
    use datasynth_core::documents::DeliveryType;
    match dt {
        DeliveryType::Outbound => "LF",
        DeliveryType::Return => "LR",
        DeliveryType::StockTransfer => "UL",
        DeliveryType::Replenishment => "NL",
        DeliveryType::ConsignmentIssue => "LK",
        DeliveryType::ConsignmentReturn => "RL",
    }
    .to_string()
}

/// Write LIKP.
pub fn write_likp(cfg: &SapExportConfig, deliveries: &[Delivery], path: &Path) -> SynthResult<()> {
    let mut w = open_file(cfg, path)?;
    let d = cfg.delimiter();
    write_header(
        &mut w,
        d,
        &[
            "MANDT",
            "VBELN",
            "LFART",
            "VSTEL",
            "ROUTE",
            "KUNNR",
            "WADAT",
            "WADAT_IST",
            "WAERK",
            "SDABW",
            "ERDAT",
            "LIFSK",
        ],
    )?;
    for dlv in deliveries {
        let s = dlv.to_sap_likp(&cfg.client);
        let fields = vec![
            s.mandt,
            s.vbeln,
            s.lfart,
            s.vstel.unwrap_or_default(),
            s.route.unwrap_or_default(),
            s.kunnr,
            opt_date(cfg, s.wadat),
            opt_date(cfg, s.wadat_ist),
            s.waerk,
            s.sdabw.unwrap_or_default(),
            cfg.format_date(s.erdat),
            s.lifsk.unwrap_or_default(),
        ];
        write_row(&mut w, d, &fields)?;
    }
    w.flush()?;
    Ok(())
}

/// Write LIPS.
pub fn write_lips(cfg: &SapExportConfig, deliveries: &[Delivery], path: &Path) -> SynthResult<()> {
    let mut w = open_file(cfg, path)?;
    let d = cfg.delimiter();
    write_header(
        &mut w,
        d,
        &[
            "MANDT", "VBELN", "POSNR", "MATNR", "ARKTX", "LFIMG", "VRKME", "WERKS", "LGORT",
            "VGBEL", "VGPOS", "CHARG",
        ],
    )?;
    for dlv in deliveries {
        for s in dlv.to_sap_lips_rows(&cfg.client) {
            let fields = vec![
                s.mandt,
                s.vbeln,
                s.posnr,
                s.matnr.unwrap_or_default(),
                escape(&s.arktx),
                dec(cfg, &s.lfimg),
                s.vrkme,
                s.werks.unwrap_or_default(),
                s.lgort.unwrap_or_default(),
                s.vgbel.unwrap_or_default(),
                s.vgpos.unwrap_or_default(),
                s.charg.unwrap_or_default(),
            ];
            write_row(&mut w, d, &fields)?;
        }
    }
    w.flush()?;
    Ok(())
}

// ===========================================================================
// MKPF / MSEG — Material document header / item
// ===========================================================================
//
// MKPF/MSEG is SAP's inventory-movement table pair. One MKPF per material
// document; one MSEG per line item. DataSynth's closest analogue is
// `GoodsReceipt` (with its own items), which already represents a
// material-document-equivalent event.

/// SAP MKPF — material document header.
#[derive(Debug, Clone)]
pub struct SapMatDocHeader {
    pub mandt: String,
    /// Material document number (MBLNR).
    pub mblnr: String,
    /// Material document year (MJAHR).
    pub mjahr: u16,
    /// Transaction type (VGART) — "WE" goods receipt, "WA" goods issue.
    pub vgart: String,
    /// Posting date (BUDAT).
    pub budat: NaiveDate,
    /// Document date (BLDAT).
    pub bldat: NaiveDate,
    /// Reference (XBLNR).
    pub xblnr: Option<String>,
    /// User (USNAM).
    pub usnam: String,
    /// Document text (BKTXT).
    pub bktxt: Option<String>,
}

/// SAP MSEG — material document line item.
#[derive(Debug, Clone)]
pub struct SapMatDocItem {
    pub mandt: String,
    pub mblnr: String,
    pub mjahr: u16,
    /// Item number (ZEILE).
    pub zeile: String,
    /// Movement type (BWART).
    pub bwart: String,
    /// Material number (MATNR).
    pub matnr: Option<String>,
    /// Plant (WERKS).
    pub werks: Option<String>,
    /// Storage location (LGORT).
    pub lgort: Option<String>,
    /// Batch (CHARG).
    pub charg: Option<String>,
    /// Quantity (MENGE).
    pub menge: Decimal,
    /// Unit of measure (MEINS).
    pub meins: String,
    /// Net value (DMBTR).
    pub dmbtr: Decimal,
    /// Currency (WAERS).
    pub waers: String,
    /// Debit/credit indicator (SHKZG).
    pub shkzg: String,
    /// Reference PO (EBELN).
    pub ebeln: Option<String>,
    /// Reference PO item (EBELP).
    pub ebelp: Option<String>,
}

/// Extension trait `GoodsReceipt` → SAP MKPF / MSEG.
pub trait SapMatDocExportable {
    fn to_sap_mkpf(&self, client: &str) -> SapMatDocHeader;
    fn to_sap_mseg_rows(&self, client: &str) -> Vec<SapMatDocItem>;
}

impl SapMatDocExportable for GoodsReceipt {
    fn to_sap_mkpf(&self, client: &str) -> SapMatDocHeader {
        SapMatDocHeader {
            mandt: client.to_string(),
            mblnr: self.header.document_id.clone(),
            mjahr: self.header.fiscal_year,
            vgart: "WE".to_string(),
            budat: self
                .header
                .posting_date
                .unwrap_or(self.header.document_date),
            bldat: self.header.document_date,
            xblnr: self.header.reference.clone(),
            usnam: self.header.created_by.clone(),
            bktxt: self.header.header_text.clone(),
        }
    }

    fn to_sap_mseg_rows(&self, client: &str) -> Vec<SapMatDocItem> {
        let year = self.header.fiscal_year;
        self.items
            .iter()
            .map(|item| SapMatDocItem {
                mandt: client.to_string(),
                mblnr: self.header.document_id.clone(),
                mjahr: year,
                zeile: format!("{:04}", item.base.line_number),
                // Most GR is a 101 (receipt into warehouse); returns would be 102.
                bwart: "101".to_string(),
                matnr: item.base.material_id.clone(),
                werks: item.base.plant.clone(),
                lgort: item.base.storage_location.clone(),
                charg: item.batch.clone(),
                menge: item.base.quantity,
                meins: item.base.uom.clone(),
                dmbtr: item.base.net_amount,
                waers: self.header.currency.clone(),
                shkzg: "S".to_string(),
                ebeln: item.po_number.clone(),
                ebelp: item.po_item.map(|p| format!("{:05}", p)),
            })
            .collect()
    }
}

/// Write MKPF.
pub fn write_mkpf(cfg: &SapExportConfig, grs: &[GoodsReceipt], path: &Path) -> SynthResult<()> {
    let mut w = open_file(cfg, path)?;
    let d = cfg.delimiter();
    write_header(
        &mut w,
        d,
        &[
            "MANDT", "MBLNR", "MJAHR", "VGART", "BUDAT", "BLDAT", "XBLNR", "USNAM", "BKTXT",
        ],
    )?;
    for gr in grs {
        let s = gr.to_sap_mkpf(&cfg.client);
        let fields = vec![
            s.mandt,
            s.mblnr,
            s.mjahr.to_string(),
            s.vgart,
            cfg.format_date(s.budat),
            cfg.format_date(s.bldat),
            s.xblnr.unwrap_or_default(),
            s.usnam,
            escape(&s.bktxt.unwrap_or_default()),
        ];
        write_row(&mut w, d, &fields)?;
    }
    w.flush()?;
    Ok(())
}

/// Write MSEG.
pub fn write_mseg(cfg: &SapExportConfig, grs: &[GoodsReceipt], path: &Path) -> SynthResult<()> {
    let mut w = open_file(cfg, path)?;
    let d = cfg.delimiter();
    write_header(
        &mut w,
        d,
        &[
            "MANDT", "MBLNR", "MJAHR", "ZEILE", "BWART", "MATNR", "WERKS", "LGORT", "CHARG",
            "MENGE", "MEINS", "DMBTR", "WAERS", "SHKZG", "EBELN", "EBELP",
        ],
    )?;
    for gr in grs {
        for s in gr.to_sap_mseg_rows(&cfg.client) {
            let fields = vec![
                s.mandt,
                s.mblnr,
                s.mjahr.to_string(),
                s.zeile,
                s.bwart,
                s.matnr.unwrap_or_default(),
                s.werks.unwrap_or_default(),
                s.lgort.unwrap_or_default(),
                s.charg.unwrap_or_default(),
                dec(cfg, &s.menge),
                s.meins,
                dec(cfg, &s.dmbtr),
                s.waers,
                s.shkzg,
                s.ebeln.unwrap_or_default(),
                s.ebelp.unwrap_or_default(),
            ];
            write_row(&mut w, d, &fields)?;
        }
    }
    w.flush()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::sap::SapDialect;
    use super::*;

    fn cfg() -> SapExportConfig {
        SapExportConfig {
            dialect: SapDialect::Hana,
            ..SapExportConfig::default()
        }
    }

    #[test]
    fn po_type_mapping_covers_all_variants() {
        use datasynth_core::documents::PurchaseOrderType;
        for pt in [
            PurchaseOrderType::Standard,
            PurchaseOrderType::Framework,
            PurchaseOrderType::Service,
            PurchaseOrderType::StockTransfer,
            PurchaseOrderType::Subcontracting,
            PurchaseOrderType::Consignment,
        ] {
            let code = po_type_to_bsart(&pt);
            assert!(!code.is_empty(), "BSART must be non-empty for {pt:?}");
        }
    }

    #[test]
    fn so_type_mapping_covers_all_variants() {
        use datasynth_core::documents::SalesOrderType;
        for so in [
            SalesOrderType::Standard,
            SalesOrderType::Rush,
            SalesOrderType::CashSale,
            SalesOrderType::Return,
            SalesOrderType::FreeOfCharge,
            SalesOrderType::Consignment,
            SalesOrderType::Service,
            SalesOrderType::CreditMemoRequest,
            SalesOrderType::DebitMemoRequest,
        ] {
            let code = so_type_to_auart(&so);
            assert!(!code.is_empty(), "AUART must be non-empty for {so:?}");
        }
    }

    #[test]
    fn delivery_type_mapping_covers_all_variants() {
        use datasynth_core::documents::DeliveryType;
        for dt in [
            DeliveryType::Outbound,
            DeliveryType::Return,
            DeliveryType::StockTransfer,
            DeliveryType::Replenishment,
            DeliveryType::ConsignmentIssue,
            DeliveryType::ConsignmentReturn,
        ] {
            let code = delivery_type_to_lfart(&dt);
            assert!(!code.is_empty(), "LFART must be non-empty for {dt:?}");
        }
    }

    #[test]
    fn dialect_format_decimal_roundtrips_through_helper() {
        let cfg = cfg();
        let v = Decimal::new(12345, 2);
        assert_eq!(dec(&cfg, &v), "123,45");
    }
}
