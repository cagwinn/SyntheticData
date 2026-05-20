//! SAP subledger open/cleared-items tables — v4.3.0d.
//!
//! Emits the classic BSIS/BSAS/BSID/BSAD/BSIK/BSAK pairs that every
//! SAP-to-analytics pipeline expects alongside BSEG/ACDOCA:
//!
//! | Pair       | Subledger     | Open (I=Item)          | Cleared (A=Archive) |
//! |------------|---------------|------------------------|----------------------|
//! | BSIS/BSAS  | GL            | open GL items          | cleared GL items     |
//! | BSID/BSAD  | AR (customer) | open AR items          | cleared AR items     |
//! | BSIK/BSAK  | AP (vendor)   | open AP items          | cleared AP items     |
//!
//! DataSynth sources the rows from:
//! - **GL**: journal-entry lines — an entry is "cleared" when an
//!   intercompany elimination or payment run records a clearing
//!   document; otherwise it's "open".
//! - **AR**: `ARInvoice` — `amount_remaining > 0` → BSID (open), the
//!   per-clearing slices in `clearing_info` → BSAD (cleared). Multiple
//!   partial payments on one invoice produce multiple BSAD rows.
//! - **AP**: `APInvoice` — same semantics, BSIK/BSAK.
//!
//! All writers honour the `SapDialect` setting of the parent
//! `SapExportConfig`.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use chrono::NaiveDate;
use datasynth_core::error::SynthResult;
use datasynth_core::models::subledger::ap::APInvoice;
use datasynth_core::models::subledger::ar::ARInvoice;
use datasynth_core::models::subledger::SubledgerDocumentStatus;
use datasynth_core::models::JournalEntry;
use rust_decimal::Decimal;

use super::sap::SapExportConfig;

// ---------------------------------------------------------------------------
// Shared helpers (local copy; the sap/sap_master_data/sap_transactional
// modules each carry their own copy to avoid coupling)
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

// ===========================================================================
// Shared row structs
// ===========================================================================

/// Common open-item columns shared by BSIS, BSID, BSIK. SAP stores many
/// more columns per table; this carries the 10-ish every analytics
/// consumer actually reads.
#[derive(Debug, Clone)]
pub struct SapOpenItemRow {
    pub mandt: String,
    pub bukrs: String,
    /// GL account (HKONT) for BSIS; AR recon account (UMSKS-empty) for
    /// BSID; AP recon account for BSIK — populated from the invoice's
    /// reconciliation account.
    pub hkont: String,
    /// Customer (KUNNR) / vendor (LIFNR) for BSID / BSIK — `None` for BSIS.
    pub partner: Option<String>,
    pub belnr: String,
    pub buzei: String,
    pub gjahr: u16,
    pub budat: NaiveDate,
    pub bldat: NaiveDate,
    pub waers: String,
    /// Debit/credit indicator (SHKZG).
    pub shkzg: String,
    /// Amount in document currency (WRBTR).
    pub wrbtr: Decimal,
    /// Amount in local currency (DMBTR).
    pub dmbtr: Decimal,
    pub zfbdt: Option<NaiveDate>, // baseline date
    pub zterm: Option<String>,
    /// Due date (NETDT).
    pub netdt: Option<NaiveDate>,
}

/// Common cleared-item columns shared by BSAS, BSAD, BSAK.
#[derive(Debug, Clone)]
pub struct SapClearedItemRow {
    pub mandt: String,
    pub bukrs: String,
    pub hkont: String,
    pub partner: Option<String>,
    pub belnr: String,
    pub buzei: String,
    pub gjahr: u16,
    pub budat: NaiveDate,
    pub bldat: NaiveDate,
    pub waers: String,
    pub shkzg: String,
    pub wrbtr: Decimal,
    pub dmbtr: Decimal,
    /// Clearing document number (AUGBL).
    pub augbl: String,
    /// Clearing date (AUGDT).
    pub augdt: NaiveDate,
    /// Cleared amount (WRSHB).
    pub wrshb: Decimal,
}

fn write_open_rows(
    cfg: &SapExportConfig,
    rows: &[SapOpenItemRow],
    include_partner: bool,
    partner_col: &str,
    path: &Path,
) -> SynthResult<()> {
    let mut w = open_file(cfg, path)?;
    let d = cfg.delimiter();
    let mut cols: Vec<&str> = vec!["MANDT", "BUKRS", "HKONT"];
    if include_partner {
        cols.push(partner_col);
    }
    cols.extend_from_slice(&[
        "BELNR", "BUZEI", "GJAHR", "BUDAT", "BLDAT", "WAERS", "SHKZG", "WRBTR", "DMBTR", "ZFBDT",
        "ZTERM", "NETDT",
    ]);
    write_header(&mut w, d, &cols)?;
    for r in rows {
        let mut fields: Vec<String> = vec![r.mandt.clone(), r.bukrs.clone(), r.hkont.clone()];
        if include_partner {
            fields.push(r.partner.clone().unwrap_or_default());
        }
        fields.push(r.belnr.clone());
        fields.push(r.buzei.clone());
        fields.push(r.gjahr.to_string());
        fields.push(cfg.format_date(r.budat));
        fields.push(cfg.format_date(r.bldat));
        fields.push(r.waers.clone());
        fields.push(r.shkzg.clone());
        fields.push(cfg.format_decimal(&r.wrbtr));
        fields.push(cfg.format_decimal(&r.dmbtr));
        fields.push(r.zfbdt.map(|d| cfg.format_date(d)).unwrap_or_default());
        fields.push(r.zterm.clone().unwrap_or_default());
        fields.push(r.netdt.map(|d| cfg.format_date(d)).unwrap_or_default());
        write_row(&mut w, d, &fields)?;
    }
    w.flush()?;
    Ok(())
}

fn write_cleared_rows(
    cfg: &SapExportConfig,
    rows: &[SapClearedItemRow],
    include_partner: bool,
    partner_col: &str,
    path: &Path,
) -> SynthResult<()> {
    let mut w = open_file(cfg, path)?;
    let d = cfg.delimiter();
    let mut cols: Vec<&str> = vec!["MANDT", "BUKRS", "HKONT"];
    if include_partner {
        cols.push(partner_col);
    }
    cols.extend_from_slice(&[
        "BELNR", "BUZEI", "GJAHR", "BUDAT", "BLDAT", "WAERS", "SHKZG", "WRBTR", "DMBTR", "AUGBL",
        "AUGDT", "WRSHB",
    ]);
    write_header(&mut w, d, &cols)?;
    for r in rows {
        let mut fields: Vec<String> = vec![r.mandt.clone(), r.bukrs.clone(), r.hkont.clone()];
        if include_partner {
            fields.push(r.partner.clone().unwrap_or_default());
        }
        fields.push(r.belnr.clone());
        fields.push(r.buzei.clone());
        fields.push(r.gjahr.to_string());
        fields.push(cfg.format_date(r.budat));
        fields.push(cfg.format_date(r.bldat));
        fields.push(r.waers.clone());
        fields.push(r.shkzg.clone());
        fields.push(cfg.format_decimal(&r.wrbtr));
        fields.push(cfg.format_decimal(&r.dmbtr));
        fields.push(r.augbl.clone());
        fields.push(cfg.format_date(r.augdt));
        fields.push(cfg.format_decimal(&r.wrshb));
        write_row(&mut w, d, &fields)?;
    }
    w.flush()?;
    Ok(())
}

// ===========================================================================
// BSIS / BSAS — GL open & cleared items
// ===========================================================================

/// Derive BSIS rows from journal entries. Every non-partner JE line
/// counts as an "open GL item" in the simplistic DataSynth model;
/// clearing is modelled explicitly via the intercompany / payment-run
/// engine. The `include_clearing_doc_ids` arg — a set of document IDs
/// known to have been cleared — filters *out* those entries (they go to
/// BSAS instead).
fn bsis_rows_from_je(client: &str, entries: &[JournalEntry]) -> Vec<SapOpenItemRow> {
    let mut rows = Vec::new();
    for je in entries {
        let header = &je.header;
        for line in &je.lines {
            let (shkzg, wrbtr) = if line.debit_amount > Decimal::ZERO {
                ("S".to_string(), line.debit_amount)
            } else {
                ("H".to_string(), line.credit_amount)
            };
            rows.push(SapOpenItemRow {
                mandt: client.to_string(),
                bukrs: header.company_code.clone(),
                hkont: line.account_code.clone(),
                partner: None,
                belnr: header.document_id.to_string(),
                buzei: format!("{:03}", line.line_number),
                gjahr: header.fiscal_year,
                budat: header.posting_date,
                bldat: header.document_date,
                waers: header.currency.clone(),
                shkzg,
                wrbtr,
                dmbtr: wrbtr,
                zfbdt: Some(header.document_date),
                zterm: None,
                netdt: None,
            });
        }
    }
    rows
}

/// Write BSIS (open GL items). Produces one row per JE line.
pub fn write_bsis(cfg: &SapExportConfig, entries: &[JournalEntry], path: &Path) -> SynthResult<()> {
    let rows = bsis_rows_from_je(&cfg.client, entries);
    write_open_rows(cfg, &rows, false, "", path)
}

/// Write BSAS (cleared GL items). DataSynth doesn't currently expose a
/// per-JE clearing marker, so this writes an empty file — callers who
/// want cleared-item detail should use the AR/AP variants (BSAD/BSAK)
/// which do have clearing data. File is still emitted with a header row
/// so ingestion pipelines don't choke on a missing file.
pub fn write_bsas(cfg: &SapExportConfig, path: &Path) -> SynthResult<()> {
    let rows: Vec<SapClearedItemRow> = Vec::new();
    write_cleared_rows(cfg, &rows, false, "", path)
}

// ===========================================================================
// BSID / BSAD — AR (customer) open & cleared items
// ===========================================================================

fn ar_reconciliation_account(inv: &ARInvoice) -> String {
    // GLReference carries the actual GL account used for the invoice
    // posting. For AR this is the reconciliation (control) account.
    inv.gl_reference
        .as_ref()
        .map(|gl| gl.gl_account.clone())
        .unwrap_or_else(|| "1100".to_string())
}

fn bsid_bsad_rows(
    client: &str,
    invoices: &[ARInvoice],
) -> (Vec<SapOpenItemRow>, Vec<SapClearedItemRow>) {
    let mut open_rows = Vec::new();
    let mut cleared_rows = Vec::new();
    for inv in invoices {
        let hkont = ar_reconciliation_account(inv);
        let gjahr = inv
            .posting_date
            .format("%Y")
            .to_string()
            .parse::<u16>()
            .unwrap_or(2024);
        let waers = inv.gross_amount.document_currency.clone();

        // Open portion (amount_remaining)
        if matches!(
            inv.status,
            SubledgerDocumentStatus::Open | SubledgerDocumentStatus::PartiallyCleared
        ) && inv.amount_remaining > Decimal::ZERO
        {
            open_rows.push(SapOpenItemRow {
                mandt: client.to_string(),
                bukrs: inv.company_code.clone(),
                hkont: hkont.clone(),
                partner: Some(inv.customer_id.clone()),
                belnr: inv.invoice_number.clone(),
                buzei: "001".to_string(),
                gjahr,
                budat: inv.posting_date,
                bldat: inv.invoice_date,
                waers: waers.clone(),
                shkzg: "S".to_string(),
                wrbtr: inv.amount_remaining,
                dmbtr: inv.amount_remaining,
                zfbdt: Some(inv.baseline_date),
                zterm: Some(format!("{:?}", inv.payment_terms)),
                netdt: Some(inv.due_date),
            });
        }

        // Cleared slices (one per clearing_info entry)
        for (idx, clr) in inv.clearing_info.iter().enumerate() {
            cleared_rows.push(SapClearedItemRow {
                mandt: client.to_string(),
                bukrs: inv.company_code.clone(),
                hkont: hkont.clone(),
                partner: Some(inv.customer_id.clone()),
                belnr: inv.invoice_number.clone(),
                buzei: format!("{:03}", idx + 1),
                gjahr,
                budat: inv.posting_date,
                bldat: inv.invoice_date,
                waers: waers.clone(),
                shkzg: "S".to_string(),
                wrbtr: clr.clearing_amount,
                dmbtr: clr.clearing_amount,
                augbl: clr.clearing_document.clone(),
                augdt: clr.clearing_date,
                wrshb: clr.clearing_amount,
            });
        }
    }
    (open_rows, cleared_rows)
}

/// Write BSID (open AR items, one per unpaid invoice).
pub fn write_bsid(cfg: &SapExportConfig, invoices: &[ARInvoice], path: &Path) -> SynthResult<()> {
    let (open_rows, _) = bsid_bsad_rows(&cfg.client, invoices);
    write_open_rows(cfg, &open_rows, true, "KUNNR", path)
}

/// Write BSAD (cleared AR items, one per payment application).
pub fn write_bsad(cfg: &SapExportConfig, invoices: &[ARInvoice], path: &Path) -> SynthResult<()> {
    let (_, cleared_rows) = bsid_bsad_rows(&cfg.client, invoices);
    write_cleared_rows(cfg, &cleared_rows, true, "KUNNR", path)
}

// ===========================================================================
// BSIK / BSAK — AP (vendor) open & cleared items
// ===========================================================================

fn ap_reconciliation_account(inv: &APInvoice) -> String {
    inv.gl_reference
        .as_ref()
        .map(|gl| gl.gl_account.clone())
        .unwrap_or_else(|| "2000".to_string())
}

fn bsik_bsak_rows(
    client: &str,
    invoices: &[APInvoice],
) -> (Vec<SapOpenItemRow>, Vec<SapClearedItemRow>) {
    let mut open_rows = Vec::new();
    let mut cleared_rows = Vec::new();
    for inv in invoices {
        let hkont = ap_reconciliation_account(inv);
        let gjahr = inv
            .posting_date
            .format("%Y")
            .to_string()
            .parse::<u16>()
            .unwrap_or(2024);
        let waers = inv.gross_amount.document_currency.clone();

        if matches!(
            inv.status,
            SubledgerDocumentStatus::Open | SubledgerDocumentStatus::PartiallyCleared
        ) && inv.amount_remaining > Decimal::ZERO
        {
            open_rows.push(SapOpenItemRow {
                mandt: client.to_string(),
                bukrs: inv.company_code.clone(),
                hkont: hkont.clone(),
                partner: Some(inv.vendor_id.clone()),
                belnr: inv.invoice_number.clone(),
                buzei: "001".to_string(),
                gjahr,
                budat: inv.posting_date,
                bldat: inv.invoice_date,
                waers: waers.clone(),
                shkzg: "H".to_string(),
                wrbtr: inv.amount_remaining,
                dmbtr: inv.amount_remaining,
                zfbdt: Some(inv.baseline_date),
                zterm: Some(format!("{:?}", inv.payment_terms)),
                netdt: Some(inv.due_date),
            });
        }

        for (idx, clr) in inv.clearing_info.iter().enumerate() {
            cleared_rows.push(SapClearedItemRow {
                mandt: client.to_string(),
                bukrs: inv.company_code.clone(),
                hkont: hkont.clone(),
                partner: Some(inv.vendor_id.clone()),
                belnr: inv.invoice_number.clone(),
                buzei: format!("{:03}", idx + 1),
                gjahr,
                budat: inv.posting_date,
                bldat: inv.invoice_date,
                waers: waers.clone(),
                shkzg: "H".to_string(),
                wrbtr: clr.clearing_amount,
                dmbtr: clr.clearing_amount,
                augbl: clr.clearing_document.clone(),
                augdt: clr.clearing_date,
                wrshb: clr.clearing_amount,
            });
        }
    }
    (open_rows, cleared_rows)
}

/// Write BSIK (open AP items).
pub fn write_bsik(cfg: &SapExportConfig, invoices: &[APInvoice], path: &Path) -> SynthResult<()> {
    let (open_rows, _) = bsik_bsak_rows(&cfg.client, invoices);
    write_open_rows(cfg, &open_rows, true, "LIFNR", path)
}

/// Write BSAK (cleared AP items).
pub fn write_bsak(cfg: &SapExportConfig, invoices: &[APInvoice], path: &Path) -> SynthResult<()> {
    let (_, cleared_rows) = bsik_bsak_rows(&cfg.client, invoices);
    write_cleared_rows(cfg, &cleared_rows, true, "LIFNR", path)
}

#[cfg(test)]
mod tests {
    use super::super::sap::SapDialect;
    use super::*;

    #[test]
    fn shared_helpers_compile() {
        let cfg = SapExportConfig {
            dialect: SapDialect::Hana,
            ..SapExportConfig::default()
        };
        assert_eq!(cfg.dialect.delimiter(), ';');
    }
}
