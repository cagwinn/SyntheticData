//! Dividend declaration generator.
//!
//! Generates dividend declarations with paired journal entries:
//! - Declaration JE: DR Retained Earnings / CR Dividends Payable
//! - Payment JE: DR Dividends Payable / CR Operating Cash

use chrono::NaiveDate;
use datasynth_core::accounts::{cash_accounts, dividend_accounts, equity_accounts};
use datasynth_core::models::{DividendDeclaration, JournalEntry, JournalEntryLine};
use datasynth_core::utils::seeded_rng;
use datasynth_core::uuid_factory::{DeterministicUuidFactory, GeneratorType};
use rand_chacha::ChaCha8Rng;
use rust_decimal::Decimal;

/// Result of generating a dividend cycle.
pub struct DividendResult {
    /// Dividend declarations generated.
    pub declarations: Vec<DividendDeclaration>,
    /// Journal entries (declaration JE first, then payment JE).
    pub journal_entries: Vec<JournalEntry>,
}

/// Generates dividend declarations with paired journal entries.
pub struct DividendGenerator {
    /// RNG for any future randomised fields.
    #[allow(dead_code)]
    rng: ChaCha8Rng,
    uuid_factory: DeterministicUuidFactory,
}

impl DividendGenerator {
    /// Create a new dividend generator with the given seed.
    pub fn new(seed: u64) -> Self {
        Self {
            rng: seeded_rng(seed, 0),
            uuid_factory: DeterministicUuidFactory::new(seed, GeneratorType::PeriodClose),
        }
    }

    /// Generate a dividend declaration and its paired journal entries.
    ///
    /// # Arguments
    /// * `entity_code`       – Company code declaring the dividend.
    /// * `declaration_date`  – Board declaration date.
    /// * `per_share_amount`  – Dividend per share.
    /// * `total_amount`      – Total cash dividend declared.
    /// * `currency`          – ISO 4217 currency code.
    pub fn generate(
        &mut self,
        entity_code: &str,
        declaration_date: NaiveDate,
        per_share_amount: Decimal,
        total_amount: Decimal,
        currency: &str,
    ) -> DividendResult {
        let id = self.uuid_factory.next().to_string();

        let record_date = declaration_date + chrono::Duration::days(14);
        let payment_date = record_date + chrono::Duration::days(30);

        let declaration = DividendDeclaration {
            id: id.clone(),
            entity_code: entity_code.to_string(),
            declaration_date,
            record_date,
            payment_date,
            per_share_amount,
            total_amount,
            currency: currency.to_string(),
        };

        // --- Declaration JE ---------------------------------------------------
        // DR Retained Earnings ("3200")  |  CR Dividends Payable ("2170")
        let decl_doc_id = format!("JE-DIV-DECL-{id}");
        let mut decl_je = JournalEntry::new_simple(
            decl_doc_id,
            entity_code.to_string(),
            declaration_date,
            format!("Dividend declaration – {entity_code}"),
        );
        decl_je.add_line(JournalEntryLine {
            line_number: 1,
            gl_account: equity_accounts::RETAINED_EARNINGS.to_string(),
            debit_amount: total_amount,
            credit_amount: Decimal::ZERO,
            reference: Some(id.clone()),
            ..Default::default()
        });
        decl_je.add_line(JournalEntryLine {
            line_number: 2,
            gl_account: dividend_accounts::DIVIDENDS_PAYABLE.to_string(),
            debit_amount: Decimal::ZERO,
            credit_amount: total_amount,
            reference: Some(id.clone()),
            ..Default::default()
        });

        // --- Payment JE -------------------------------------------------------
        // DR Dividends Payable ("2170")  |  CR Operating Cash ("1000")
        let pay_doc_id = format!("JE-DIV-PAY-{id}");
        let mut pay_je = JournalEntry::new_simple(
            pay_doc_id,
            entity_code.to_string(),
            payment_date,
            format!("Dividend payment – {entity_code}"),
        );
        pay_je.add_line(JournalEntryLine {
            line_number: 1,
            gl_account: dividend_accounts::DIVIDENDS_PAYABLE.to_string(),
            debit_amount: total_amount,
            credit_amount: Decimal::ZERO,
            reference: Some(id.clone()),
            ..Default::default()
        });
        pay_je.add_line(JournalEntryLine {
            line_number: 2,
            gl_account: cash_accounts::OPERATING_CASH.to_string(),
            debit_amount: Decimal::ZERO,
            credit_amount: total_amount,
            reference: Some(id.clone()),
            ..Default::default()
        });

        DividendResult {
            declarations: vec![declaration],
            journal_entries: vec![decl_je, pay_je],
        }
    }
}
