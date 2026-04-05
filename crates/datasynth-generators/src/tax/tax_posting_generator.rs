//! Tax GL Posting Journal Entry Generator.
//!
//! Converts TaxLine records into balanced GL journal entries:
//! - Customer invoices → Output VAT posting (DR AR / CR VAT Payable)
//! - Deductible vendor invoices → Input VAT posting (DR Input VAT / CR AP)
//! - Non-deductible vendor invoices → skipped (expense already absorbs tax)
//! - Other document types (JournalEntry, Payment, PayrollRun) → skipped

use chrono::NaiveDate;

use datasynth_core::accounts::{control_accounts, tax_accounts};
use datasynth_core::models::journal_entry::{
    BusinessProcess, JournalEntry, JournalEntryHeader, JournalEntryLine, TransactionSource,
};
use datasynth_core::models::{TaxLine, TaxableDocumentType};

// ---------------------------------------------------------------------------
// Generator
// ---------------------------------------------------------------------------

/// Generates GL journal entries for tax postings derived from TaxLine records.
pub struct TaxPostingGenerator;

impl TaxPostingGenerator {
    /// Generate GL journal entries for each eligible TaxLine.
    ///
    /// # Rules
    ///
    /// | Document type       | Deductible | Action                                    |
    /// |---------------------|------------|-------------------------------------------|
    /// | CustomerInvoice     | any        | DR AR Control / CR VAT Payable            |
    /// | VendorInvoice       | true       | DR Input VAT / CR AP Control              |
    /// | VendorInvoice       | false      | Skipped – tax absorbed into expense       |
    /// | JournalEntry        | any        | Skipped                                   |
    /// | Payment             | any        | Skipped                                   |
    /// | PayrollRun          | any        | Skipped                                   |
    pub fn generate_tax_posting_jes(
        tax_lines: &[TaxLine],
        company_code: &str,
        posting_date: NaiveDate,
    ) -> Vec<JournalEntry> {
        let mut jes = Vec::new();

        for tax_line in tax_lines {
            // Skip zero-amount lines – nothing to post.
            if tax_line.tax_amount.is_zero() {
                continue;
            }

            match tax_line.document_type {
                TaxableDocumentType::CustomerInvoice => {
                    jes.push(Self::output_vat_je(tax_line, company_code, posting_date));
                }
                TaxableDocumentType::VendorInvoice if tax_line.is_deductible => {
                    jes.push(Self::input_vat_je(tax_line, company_code, posting_date));
                }
                // Non-deductible vendor tax or unsupported doc types → skip.
                TaxableDocumentType::VendorInvoice
                | TaxableDocumentType::JournalEntry
                | TaxableDocumentType::Payment
                | TaxableDocumentType::PayrollRun => {}
            }
        }

        jes
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Output VAT posting for a customer invoice tax line.
    ///
    /// DR AR Control ("1100") / CR VAT Payable ("2110")
    fn output_vat_je(
        tax_line: &TaxLine,
        company_code: &str,
        posting_date: NaiveDate,
    ) -> JournalEntry {
        let doc_id_str = format!("JE-TAX-OUT-{}", tax_line.id);
        let mut je = Self::build_je(
            doc_id_str.clone(),
            company_code,
            posting_date,
            format!(
                "Output VAT posting for customer invoice {}",
                tax_line.document_id
            ),
        );

        let uuid = je.header.document_id;
        let amount = tax_line.tax_amount;
        je.add_line(JournalEntryLine::debit(
            uuid,
            1,
            control_accounts::AR_CONTROL.to_string(),
            amount,
        ));
        je.add_line(JournalEntryLine::credit(
            uuid,
            2,
            tax_accounts::VAT_PAYABLE.to_string(),
            amount,
        ));
        je
    }

    /// Input VAT posting for a deductible vendor invoice tax line.
    ///
    /// DR Input VAT ("1160") / CR AP Control ("2000")
    fn input_vat_je(
        tax_line: &TaxLine,
        company_code: &str,
        posting_date: NaiveDate,
    ) -> JournalEntry {
        let doc_id_str = format!("JE-TAX-IN-{}", tax_line.id);
        let mut je = Self::build_je(
            doc_id_str.clone(),
            company_code,
            posting_date,
            format!(
                "Input VAT posting for vendor invoice {}",
                tax_line.document_id
            ),
        );

        let uuid = je.header.document_id;
        let amount = tax_line.tax_amount;
        je.add_line(JournalEntryLine::debit(
            uuid,
            1,
            tax_accounts::INPUT_VAT.to_string(),
            amount,
        ));
        je.add_line(JournalEntryLine::credit(
            uuid,
            2,
            control_accounts::AP_CONTROL.to_string(),
            amount,
        ));
        je
    }

    /// Build a base `JournalEntry` with tax-posting metadata.
    fn build_je(
        _doc_id_str: String,
        company_code: &str,
        posting_date: NaiveDate,
        description: String,
    ) -> JournalEntry {
        let mut header = JournalEntryHeader::new(company_code.to_string(), posting_date);
        header.document_type = "TAX_POSTING".to_string();
        header.created_by = "TAX_POSTING_ENGINE".to_string();
        header.source = TransactionSource::Automated;
        header.business_process = Some(BusinessProcess::R2R);
        header.header_text = Some(description);
        JournalEntry::new(header)
    }
}
