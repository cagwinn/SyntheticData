//! SAF-T (Standard Audit File for Tax) XML exporter — v4.3.1.
//!
//! SAF-T is an OECD-originated XML format for tax audit data. Many EU
//! jurisdictions have adopted it with per-country variants (namespace,
//! version, and occasional mandatory-field differences). DataSynth emits
//! a structurally-SAF-T file with the following top-level sections:
//!
//! - `Header` — AuditFileVersion, company identification, period
//! - `MasterFiles`:
//!   - `GeneralLedgerAccounts` — from `ChartOfAccounts`
//!   - `Customers` — from `master_data.customers`
//!   - `Suppliers` — from `master_data.vendors`
//!   - `Products` — from `master_data.materials`
//! - `GeneralLedgerEntries` — from `journal_entries`
//!
//! Each supported jurisdiction gets a namespace + version slot:
//!
//! | Code | Country       | Spec version        | Nominal schema     |
//! |------|---------------|---------------------|--------------------|
//! | PT   | Portugal      | SAF-T PT 1.04_01    | mf-SAF-T-PT v1.04  |
//! | PL   | Poland        | JPK_KR v1.0         | JPK schema         |
//! | RO   | Romania       | D406 v3.0           | ANAF D406          |
//! | NO   | Norway        | SAF-T Financial 1.10| Norwegian SAF-T    |
//! | LU   | Luxembourg    | FAIA 2.01           | Luxembourg FAIA    |
//!
//! This implementation emits a structurally-valid XML tree that loads
//! under every jurisdiction's schema when the core columns overlap —
//! it's a starting point, not a fully-conformant per-jurisdiction
//! submission. Audit firms using the output should still validate
//! against the jurisdiction-specific XSD before regulatory filing.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use chrono::NaiveDate;
use datasynth_core::error::SynthResult;
use datasynth_core::models::{ChartOfAccounts, Customer, JournalEntry, Material, Vendor};
use rust_decimal::Decimal;

// ---------------------------------------------------------------------------
// Jurisdictions
// ---------------------------------------------------------------------------

/// Jurisdiction for SAF-T export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SaftJurisdiction {
    Portugal,
    Poland,
    Romania,
    Norway,
    Luxembourg,
}

impl SaftJurisdiction {
    /// Parse from a short ISO-ish code (case-insensitive). Unknown → `None`.
    pub fn from_code(code: &str) -> Option<Self> {
        Some(match code.to_ascii_lowercase().as_str() {
            "pt" | "portugal" => Self::Portugal,
            "pl" | "poland" | "jpk" => Self::Poland,
            "ro" | "romania" | "d406" => Self::Romania,
            "no" | "norway" => Self::Norway,
            "lu" | "luxembourg" | "faia" => Self::Luxembourg,
            _ => return None,
        })
    }

    pub fn country_code(&self) -> &'static str {
        match self {
            Self::Portugal => "PT",
            Self::Poland => "PL",
            Self::Romania => "RO",
            Self::Norway => "NO",
            Self::Luxembourg => "LU",
        }
    }

    /// Nominal SAF-T version string emitted in the Header.AuditFileVersion.
    pub fn version_string(&self) -> &'static str {
        match self {
            Self::Portugal => "1.04_01",
            Self::Poland => "1.0",
            Self::Romania => "3.0",
            Self::Norway => "1.10",
            Self::Luxembourg => "2.01",
        }
    }

    /// XML namespace URI (informational — not schema-validated here).
    pub fn namespace(&self) -> &'static str {
        match self {
            Self::Portugal => "urn:OECD:StandardAuditFile-Tax:PT_1.04_01",
            Self::Poland => "http://jpk.mf.gov.pl/wzor/2016/11/23/11231/",
            Self::Romania => "mfp:anaf:dgti:d406:declaratie:v3",
            Self::Norway => "urn:StandardAuditFile-Taxation-Financial:NO",
            Self::Luxembourg => "urn:OECD:StandardAuditFile-Tax:LU_2.01",
        }
    }

    /// Recommended file name when writing a single file per jurisdiction.
    pub fn filename(&self) -> &'static str {
        match self {
            Self::Portugal => "saft_pt.xml",
            Self::Poland => "jpk_kr.xml",
            Self::Romania => "d406.xml",
            Self::Norway => "saft_no.xml",
            Self::Luxembourg => "faia.xml",
        }
    }
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for a SAF-T export run.
#[derive(Debug, Clone)]
pub struct SaftConfig {
    pub jurisdiction: SaftJurisdiction,
    /// Company tax registration number (NIF/TIN/KRS/...). Jurisdictions vary.
    pub company_tax_id: String,
    /// Human-readable company / legal name.
    pub company_name: String,
    /// Fiscal year covered (e.g. 2024).
    pub fiscal_year: u16,
    /// Start of the reporting period.
    pub start_date: NaiveDate,
    /// End of the reporting period.
    pub end_date: NaiveDate,
    /// Default currency (ISO-4217).
    pub currency_code: String,
}

// ---------------------------------------------------------------------------
// Data bundle — caller collects from its own pipeline
// ---------------------------------------------------------------------------

/// Input bundle for [`write_saft`]. All slices may be empty; the writer
/// emits the corresponding XML section as empty (still present, so the
/// schema structure is complete).
pub struct SaftData<'a> {
    pub accounts: &'a ChartOfAccounts,
    pub customers: &'a [Customer],
    pub vendors: &'a [Vendor],
    pub materials: &'a [Material],
    pub journal_entries: &'a [JournalEntry],
}

// ---------------------------------------------------------------------------
// XML writer — direct string-based, no external dependency
// ---------------------------------------------------------------------------

/// Escape a text node for XML (minimal).
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

fn write_elem<W: Write>(w: &mut W, tag: &str, value: &str) -> std::io::Result<()> {
    writeln!(w, "    <{tag}>{}</{tag}>", xml_escape(value))
}

fn write_elem_indent<W: Write>(
    w: &mut W,
    indent: usize,
    tag: &str,
    value: &str,
) -> std::io::Result<()> {
    let pad = " ".repeat(indent);
    writeln!(w, "{pad}<{tag}>{}</{tag}>", xml_escape(value))
}

fn fmt_decimal(v: &Decimal) -> String {
    // SAF-T across jurisdictions uniformly expects dot decimal — it's XML,
    // not locale-sensitive CSV.
    v.to_string()
}

fn fmt_date(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// Tiny helper so the CLI can construct `NaiveDate` values without
/// taking a direct `chrono` dep.
pub fn saft_naive_date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day)
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid fallback"))
}

// ---------------------------------------------------------------------------
// Top-level writer
// ---------------------------------------------------------------------------

/// Write a SAF-T XML file at `path`.
pub fn write_saft(cfg: &SaftConfig, data: &SaftData<'_>, path: &Path) -> SynthResult<()> {
    let file = File::create(path)?;
    let mut w = BufWriter::with_capacity(512 * 1024, file);

    writeln!(w, r#"<?xml version="1.0" encoding="UTF-8"?>"#)?;
    writeln!(w, r#"<AuditFile xmlns="{}">"#, cfg.jurisdiction.namespace())?;

    write_header(&mut w, cfg)?;
    write_master_files(&mut w, cfg, data)?;
    write_general_ledger_entries(&mut w, cfg, data)?;

    writeln!(w, "</AuditFile>")?;
    w.flush()?;
    Ok(())
}

fn write_header<W: Write>(w: &mut W, cfg: &SaftConfig) -> std::io::Result<()> {
    writeln!(w, "  <Header>")?;
    write_elem(w, "AuditFileVersion", cfg.jurisdiction.version_string())?;
    write_elem(w, "CompanyID", &cfg.company_tax_id)?;
    write_elem(w, "TaxRegistrationNumber", &cfg.company_tax_id)?;
    write_elem(w, "TaxAccountingBasis", "F")?; // F = standard accounting basis
    write_elem(w, "CompanyName", &cfg.company_name)?;
    write_elem(w, "FiscalYear", &cfg.fiscal_year.to_string())?;
    write_elem(w, "StartDate", &fmt_date(cfg.start_date))?;
    write_elem(w, "EndDate", &fmt_date(cfg.end_date))?;
    write_elem(w, "CurrencyCode", &cfg.currency_code)?;
    write_elem(w, "DateCreated", &fmt_date(chrono::Utc::now().date_naive()))?;
    write_elem(w, "SoftwareCompanyName", "DataSynth")?;
    write_elem(w, "ProductID", "DataSynth/4.3.1")?;
    write_elem(w, "ProductVersion", "4.3.1")?;
    writeln!(w, "  </Header>")?;
    Ok(())
}

fn write_master_files<W: Write>(
    w: &mut W,
    cfg: &SaftConfig,
    data: &SaftData<'_>,
) -> std::io::Result<()> {
    writeln!(w, "  <MasterFiles>")?;

    // GeneralLedgerAccounts
    writeln!(w, "    <GeneralLedgerAccounts>")?;
    for acct in &data.accounts.accounts {
        writeln!(w, "      <Account>")?;
        write_elem_indent(w, 8, "AccountID", &acct.account_number)?;
        write_elem_indent(w, 8, "AccountDescription", &acct.short_description)?;
        write_elem_indent(w, 8, "AccountType", &format!("{:?}", acct.account_type))?;
        write_elem_indent(w, 8, "OpeningDebitBalance", "0.00")?;
        write_elem_indent(w, 8, "OpeningCreditBalance", "0.00")?;
        write_elem_indent(w, 8, "ClosingDebitBalance", "0.00")?;
        write_elem_indent(w, 8, "ClosingCreditBalance", "0.00")?;
        writeln!(w, "      </Account>")?;
    }
    writeln!(w, "    </GeneralLedgerAccounts>")?;

    // Customers
    writeln!(w, "    <Customer>")?;
    for c in data.customers {
        writeln!(w, "      <CustomerEntry>")?;
        write_elem_indent(w, 8, "CustomerID", &c.customer_id)?;
        write_elem_indent(
            w,
            8,
            "AccountID",
            c.account_number.as_deref().unwrap_or("DESC"),
        )?;
        write_elem_indent(
            w,
            8,
            "CustomerTaxID",
            c.tax_id.as_deref().unwrap_or("Desconhecido"),
        )?;
        write_elem_indent(w, 8, "CompanyName", &c.name)?;
        writeln!(w, "        <BillingAddress>")?;
        write_elem_indent(w, 10, "AddressDetail", "N/A")?;
        write_elem_indent(w, 10, "City", "N/A")?;
        write_elem_indent(w, 10, "PostalCode", "0000-000")?;
        write_elem_indent(w, 10, "Country", &c.country)?;
        writeln!(w, "        </BillingAddress>")?;
        write_elem_indent(w, 8, "SelfBillingIndicator", "0")?;
        writeln!(w, "      </CustomerEntry>")?;
    }
    writeln!(w, "    </Customer>")?;

    // Suppliers (vendors)
    writeln!(w, "    <Supplier>")?;
    for v in data.vendors {
        writeln!(w, "      <SupplierEntry>")?;
        write_elem_indent(w, 8, "SupplierID", &v.vendor_id)?;
        write_elem_indent(
            w,
            8,
            "AccountID",
            v.account_number.as_deref().unwrap_or("DESC"),
        )?;
        write_elem_indent(
            w,
            8,
            "SupplierTaxID",
            v.tax_id.as_deref().unwrap_or("Desconhecido"),
        )?;
        write_elem_indent(w, 8, "CompanyName", &v.name)?;
        writeln!(w, "        <BillingAddress>")?;
        write_elem_indent(w, 10, "AddressDetail", "N/A")?;
        write_elem_indent(w, 10, "City", "N/A")?;
        write_elem_indent(w, 10, "PostalCode", "0000-000")?;
        write_elem_indent(w, 10, "Country", &v.country)?;
        writeln!(w, "        </BillingAddress>")?;
        write_elem_indent(w, 8, "SelfBillingIndicator", "0")?;
        writeln!(w, "      </SupplierEntry>")?;
    }
    writeln!(w, "    </Supplier>")?;

    // Products (materials)
    writeln!(w, "    <Product>")?;
    for m in data.materials {
        writeln!(w, "      <ProductEntry>")?;
        write_elem_indent(w, 8, "ProductType", "P")?; // P = Product
        write_elem_indent(w, 8, "ProductCode", &m.material_id)?;
        write_elem_indent(w, 8, "ProductDescription", &m.description)?;
        write_elem_indent(w, 8, "ProductNumberCode", &m.material_id)?;
        writeln!(w, "      </ProductEntry>")?;
    }
    writeln!(w, "    </Product>")?;

    // TaxTable — minimal single-row stub. Most jurisdictions populate
    // this from actual tax codes in the dataset; out of scope for v4.3.1.
    writeln!(w, "    <TaxTable>")?;
    writeln!(w, "      <TaxTableEntry>")?;
    write_elem_indent(w, 8, "TaxType", "IVA")?;
    write_elem_indent(w, 8, "TaxCountryRegion", cfg.jurisdiction.country_code())?;
    write_elem_indent(w, 8, "TaxCode", "NOR")?;
    write_elem_indent(w, 8, "Description", "Standard rate (stub)")?;
    write_elem_indent(w, 8, "TaxPercentage", "0.00")?;
    writeln!(w, "      </TaxTableEntry>")?;
    writeln!(w, "    </TaxTable>")?;

    writeln!(w, "  </MasterFiles>")?;
    Ok(())
}

fn write_general_ledger_entries<W: Write>(
    w: &mut W,
    _cfg: &SaftConfig,
    data: &SaftData<'_>,
) -> std::io::Result<()> {
    // Derive aggregate counters.
    let number_of_entries = data.journal_entries.len();
    let mut total_debit = Decimal::ZERO;
    let mut total_credit = Decimal::ZERO;
    for je in data.journal_entries {
        for line in &je.lines {
            total_debit += line.debit_amount;
            total_credit += line.credit_amount;
        }
    }

    writeln!(w, "  <GeneralLedgerEntries>")?;
    write_elem_indent(w, 4, "NumberOfEntries", &number_of_entries.to_string())?;
    write_elem_indent(w, 4, "TotalDebit", &fmt_decimal(&total_debit))?;
    write_elem_indent(w, 4, "TotalCredit", &fmt_decimal(&total_credit))?;

    // One Journal per fiscal period (keeps the file small & grouped).
    // Real SAF-T subdivides by journal type; DataSynth collapses into a
    // single "GEN" journal and emits every JE as a transaction under it.
    writeln!(w, "    <Journal>")?;
    write_elem_indent(w, 6, "JournalID", "GEN")?;
    write_elem_indent(w, 6, "Description", "General journal")?;

    for je in data.journal_entries {
        writeln!(w, "      <Transaction>")?;
        write_elem_indent(w, 8, "TransactionID", &je.header.document_id.to_string())?;
        write_elem_indent(w, 8, "Period", &format!("{:02}", je.header.fiscal_period))?;
        write_elem_indent(w, 8, "TransactionDate", &fmt_date(je.header.document_date))?;
        write_elem_indent(w, 8, "SourceID", &je.header.created_by)?;
        write_elem_indent(
            w,
            8,
            "Description",
            je.header.header_text.as_deref().unwrap_or(""),
        )?;
        write_elem_indent(
            w,
            8,
            "DocArchivalNumber",
            &je.header.reference.clone().unwrap_or_default(),
        )?;
        write_elem_indent(w, 8, "TransactionType", "N")?; // N = normal

        let je_posting_date = je.header.posting_date;
        write_elem_indent(w, 8, "GLPostingDate", &fmt_date(je_posting_date))?;

        writeln!(w, "        <Lines>")?;
        for line in &je.lines {
            let side_tag = if line.debit_amount > Decimal::ZERO {
                "DebitLine"
            } else {
                "CreditLine"
            };
            let amount = if line.debit_amount > Decimal::ZERO {
                line.debit_amount
            } else {
                line.credit_amount
            };
            writeln!(w, "          <{side_tag}>")?;
            write_elem_indent(w, 12, "RecordID", &line.line_number.to_string())?;
            write_elem_indent(w, 12, "AccountID", &line.account_code)?;
            write_elem_indent(w, 12, "SystemEntryDate", &fmt_date(je_posting_date))?;
            write_elem_indent(
                w,
                12,
                "Description",
                line.line_text.as_deref().unwrap_or(""),
            )?;
            write_elem_indent(w, 12, "Amount", &fmt_decimal(&amount))?;
            writeln!(w, "          </{side_tag}>")?;
        }
        writeln!(w, "        </Lines>")?;

        writeln!(w, "      </Transaction>")?;
    }

    writeln!(w, "    </Journal>")?;
    writeln!(w, "  </GeneralLedgerEntries>")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jurisdiction_parses_case_insensitive() {
        assert_eq!(
            SaftJurisdiction::from_code("pt"),
            Some(SaftJurisdiction::Portugal)
        );
        assert_eq!(
            SaftJurisdiction::from_code("PT"),
            Some(SaftJurisdiction::Portugal)
        );
        assert_eq!(
            SaftJurisdiction::from_code("Norway"),
            Some(SaftJurisdiction::Norway)
        );
        assert_eq!(
            SaftJurisdiction::from_code("faia"),
            Some(SaftJurisdiction::Luxembourg)
        );
        assert_eq!(SaftJurisdiction::from_code("xx"), None);
    }

    #[test]
    fn xml_escape_handles_specials() {
        assert_eq!(xml_escape("A & B"), "A &amp; B");
        assert_eq!(xml_escape("<tag>"), "&lt;tag&gt;");
        assert_eq!(xml_escape("O'Brien \"AB\""), "O&apos;Brien &quot;AB&quot;");
    }

    #[test]
    fn each_jurisdiction_has_filename_and_version() {
        for j in [
            SaftJurisdiction::Portugal,
            SaftJurisdiction::Poland,
            SaftJurisdiction::Romania,
            SaftJurisdiction::Norway,
            SaftJurisdiction::Luxembourg,
        ] {
            assert!(!j.country_code().is_empty());
            assert!(!j.version_string().is_empty());
            assert!(!j.namespace().is_empty());
            assert!(j.filename().ends_with(".xml"));
        }
    }
}
