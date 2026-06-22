//! Multi-period generation session with checkpoint/resume support.
//!
//! [`GenerationSession`] wraps [`EnhancedOrchestrator`] and drives it through
//! a sequence of [`GenerationPeriod`]s, persisting state to `.dss` files so
//! that long runs can be resumed after interruption.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::Datelike;
use datasynth_config::GeneratorConfig;
use datasynth_core::models::generation_session::{
    add_months, advance_seed, BalanceState, DocumentIdState, EntityCounts, GenerationPeriod,
    PeriodLog, SessionState,
};
use datasynth_core::SynthError;

use crate::enhanced_orchestrator::{EnhancedOrchestrator, PhaseConfig};

type SynthResult<T> = Result<T, SynthError>;

/// Controls how period output directories are laid out.
#[derive(Debug, Clone)]
pub enum OutputMode {
    /// Single output directory (one period).
    Batch(PathBuf),
    /// One sub-directory per period under a root directory.
    MultiPeriod(PathBuf),
}

/// Summary of a single completed period generation.
#[derive(Debug)]
pub struct PeriodResult {
    /// The period that was generated.
    pub period: GenerationPeriod,
    /// Filesystem path where this period's output was written.
    pub output_path: PathBuf,
    /// Number of journal entries generated in this period.
    pub journal_entry_count: usize,
    /// Number of document flow records generated in this period.
    pub document_count: usize,
    /// Number of anomalies injected in this period.
    pub anomaly_count: usize,
    /// Wall-clock duration for generating this period (seconds).
    pub duration_secs: f64,
}

/// A multi-period generation session with checkpoint/resume support.
///
/// The session decomposes the total requested time span into fiscal-year-aligned
/// periods and generates each one sequentially, carrying forward balance and ID
/// state between periods.
#[derive(Debug)]
pub struct GenerationSession {
    config: GeneratorConfig,
    state: SessionState,
    periods: Vec<GenerationPeriod>,
    output_mode: OutputMode,
    phase_config: PhaseConfig,
}

impl GenerationSession {
    /// Create a new session from a config and output path.
    ///
    /// The total time span is decomposed into fiscal-year-aligned periods
    /// based on `config.global.fiscal_year_months` (defaults to `period_months`
    /// if not set, yielding a single period).
    pub fn new(config: GeneratorConfig, output_path: PathBuf) -> SynthResult<Self> {
        let start_date = chrono::NaiveDate::parse_from_str(&config.global.start_date, "%Y-%m-%d")
            .map_err(|e| SynthError::generation(format!("Invalid start_date: {e}")))?;

        let total_months = config.global.period_months;
        let fy_months = config.global.fiscal_year_months.unwrap_or(total_months);
        let periods = GenerationPeriod::compute_periods(start_date, total_months, fy_months);

        let output_mode = if periods.len() > 1 {
            OutputMode::MultiPeriod(output_path)
        } else {
            OutputMode::Batch(output_path)
        };

        let seed = config.global.seed.unwrap_or(42);
        let config_hash = Self::compute_config_hash(&config);

        let state = SessionState {
            rng_seed: seed,
            period_cursor: 0,
            balance_state: BalanceState::default(),
            document_id_state: DocumentIdState::default(),
            entity_counts: EntityCounts::default(),
            generation_log: Vec::new(),
            config_hash,
            carry_forward: Vec::new(),
        };

        Ok(Self {
            config,
            state,
            periods,
            output_mode,
            phase_config: PhaseConfig::default(),
        })
    }

    /// Resume a session from a `.dss` checkpoint file.
    ///
    /// The config hash is verified against the checkpoint to ensure the config
    /// has not changed since the session was last saved.
    pub fn resume(path: &Path, config: GeneratorConfig) -> SynthResult<Self> {
        let data = fs::read_to_string(path)
            .map_err(|e| SynthError::generation(format!("Failed to read .dss: {e}")))?;
        let state: SessionState = serde_json::from_str(&data)
            .map_err(|e| SynthError::generation(format!("Failed to parse .dss: {e}")))?;

        let current_hash = Self::compute_config_hash(&config);
        if state.config_hash != current_hash {
            return Err(SynthError::generation(
                "Config has changed since last checkpoint. Cannot resume with different config."
                    .to_string(),
            ));
        }

        let start_date = chrono::NaiveDate::parse_from_str(&config.global.start_date, "%Y-%m-%d")
            .map_err(|e| SynthError::generation(format!("Invalid start_date: {e}")))?;

        let total_months = config.global.period_months;
        let fy_months = config.global.fiscal_year_months.unwrap_or(total_months);
        let periods = GenerationPeriod::compute_periods(start_date, total_months, fy_months);

        let output_dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let output_mode = if periods.len() > 1 {
            OutputMode::MultiPeriod(output_dir)
        } else {
            OutputMode::Batch(output_dir)
        };

        Ok(Self {
            config,
            state,
            periods,
            output_mode,
            phase_config: PhaseConfig::default(),
        })
    }

    /// Persist the current session state to a `.dss` file.
    pub fn save(&self, path: &Path) -> SynthResult<()> {
        let data = serde_json::to_string_pretty(&self.state)
            .map_err(|e| SynthError::generation(format!("Failed to serialize state: {e}")))?;
        fs::write(path, data)
            .map_err(|e| SynthError::generation(format!("Failed to write .dss: {e}")))?;
        Ok(())
    }

    /// Generate the next period in the sequence.
    ///
    /// Returns `Ok(None)` if all periods have been generated.
    pub fn generate_next_period(&mut self) -> SynthResult<Option<PeriodResult>> {
        if self.state.period_cursor >= self.periods.len() {
            return Ok(None);
        }

        let period = self.periods[self.state.period_cursor].clone();
        let start = std::time::Instant::now();

        let period_seed = advance_seed(self.state.rng_seed, period.index);

        let mut period_config = self.config.clone();
        period_config.global.start_date = period.start_date.format("%Y-%m-%d").to_string();
        period_config.global.period_months = period.months;
        period_config.global.seed = Some(period_seed);

        let output_path = match &self.output_mode {
            OutputMode::Batch(p) => p.clone(),
            OutputMode::MultiPeriod(p) => p.join(&period.label),
        };

        fs::create_dir_all(&output_path)
            .map_err(|e| SynthError::generation(format!("Failed to create output dir: {e}")))?;

        // In a multi-fiscal-year session the year-end close + carry-forward below is authoritative:
        // it runs the COMPLETE income-statement close (rev/exp → income summary → retained earnings)
        // per fiscal year. Suppress the orchestrator's own one-sided net-income→RE close so RE is
        // not posted twice (the 3200 double-count blocker). A lone-period session (periods.len()==1)
        // runs NO session close, so it keeps the orchestrator's close — unchanged.
        let mut period_phase_config = self.phase_config.clone();
        period_phase_config.skip_income_statement_close = self.periods.len() > 1;
        let orchestrator = EnhancedOrchestrator::new(period_config, period_phase_config)?;
        let mut orchestrator = orchestrator.with_output_path(&output_path);

        // ---------------------------------------------------------------
        // Year-boundary opening carry-forward (inject BEFORE generate()).
        // Seed this FY's openings from the prior FY's POST-CLOSE balance
        // sheet (computed at the end of the prior period and stored on
        // `self.state.carry_forward`). FY1 (period_cursor == 0) is skipped,
        // so it still opens with fresh OpeningBalanceGenerator openings.
        // When the carry-forward is non-empty, the orchestrator's v5.3
        // carryover branch (phase_opening_balances) REPLACES its generated
        // openings with these values. This branch draws no RNG.
        // ---------------------------------------------------------------
        if self.state.period_cursor > 0 {
            let opening_balances = std::mem::take(&mut self.state.carry_forward);
            if !opening_balances.is_empty() {
                orchestrator.set_shard_context(crate::shard_context::ShardContext {
                    entity_code: self
                        .config
                        .companies
                        .first()
                        .map(|c| c.code.clone())
                        .unwrap_or_default(),
                    entity_seed: [0u8; 32],
                    extra_journal_entries: Vec::new(),
                    opening_balances,
                });
            }
        }

        let mut result = orchestrator.generate()?;

        // ---------------------------------------------------------------
        // Year-end close + next-FY carry-forward (multi-FY runs only).
        // Gated on `self.periods.len() > 1` so a lone-period session is a
        // strict no-op (single-FY builds never enter the session at all —
        // see the CLI `use_session` gate). The appended closing entries
        // MUST land in `result.journal_entries` BEFORE the per-period
        // writer below, so the books on disk show the close.
        // ---------------------------------------------------------------
        if self.periods.len() > 1 {
            self.close_and_carry_forward(&period, &mut result);
        }

        // Persist this period's full output tree to its sub-directory. The orchestrator
        // returns the result in-memory; without this explicit write (mirroring the
        // single-generate CLI path) session-mode period dirs are left empty. The
        // orchestrator's NumericModeGuard resets the decimal mode on return, so re-apply
        // it before serialization (same fix as the single-generate path, issue #102).
        datasynth_core::serde_decimal::set_numeric_native(
            self.config.output.numeric_mode == datasynth_config::NumericMode::Native,
        );
        let write_result = crate::output_writer::write_all_output_with_layout(
            &result,
            &output_path,
            self.config.output.export_layout,
            &self.config.output.formats,
            self.config.graph_export.je_network.method,
        );
        datasynth_core::serde_decimal::set_numeric_native(false);
        write_result.map_err(|e| {
            SynthError::generation(format!(
                "Failed to write period '{}' output: {e}",
                period.label
            ))
        })?;

        let duration = start.elapsed().as_secs_f64();

        // Count journal entries from the result vec
        let je_count = result.journal_entries.len();

        // Count documents from the document_flows snapshot
        let doc_count = result.document_flows.purchase_orders.len()
            + result.document_flows.sales_orders.len()
            + result.document_flows.goods_receipts.len()
            + result.document_flows.vendor_invoices.len()
            + result.document_flows.customer_invoices.len()
            + result.document_flows.deliveries.len()
            + result.document_flows.payments.len();

        // Count anomalies from anomaly_labels
        let anomaly_count = result.anomaly_labels.labels.len();

        // ---------------------------------------------------------------
        // Balance carry-forward: aggregate closing GL balances from JEs
        // so the next period starts from this period's closing position.
        // ---------------------------------------------------------------
        {
            use std::collections::HashMap;

            // Build net balance per GL account (debit positive, credit negative).
            let mut gl_net: HashMap<String, f64> = HashMap::new();
            for je in &result.journal_entries {
                for line in &je.lines {
                    let account = line.gl_account.clone();
                    let delta = f64::try_from(line.debit_amount).unwrap_or(0.0)
                        - f64::try_from(line.credit_amount).unwrap_or(0.0);
                    *gl_net.entry(account).or_insert(0.0) += delta;
                }
            }

            // Carry forward as opening balances for the next period.
            // We merge into any existing carry-forward from prior periods.
            for (account, delta) in gl_net {
                *self
                    .state
                    .balance_state
                    .gl_balances
                    .entry(account)
                    .or_insert(0.0) += delta;
            }

            // Derive aggregate subledger totals from the balance map.
            // AR is represented by account 1100, AP by account 2000 (sign convention:
            // positive = debit balance for AR, positive credit balance treated as
            // positive AP by flipping sign).
            self.state.balance_state.ar_total = self
                .state
                .balance_state
                .gl_balances
                .get("1100")
                .copied()
                .unwrap_or(0.0)
                .max(0.0);
            self.state.balance_state.ap_total = (-self
                .state
                .balance_state
                .gl_balances
                .get("2000")
                .copied()
                .unwrap_or(0.0))
            .max(0.0);

            // Retained earnings: sum of all income statement accounts (4xxx–8xxx range).
            // Positive retained earnings arise when revenues (credit-normal) exceed expenses.
            let retained: f64 = self
                .state
                .balance_state
                .gl_balances
                .iter()
                .filter_map(|(acct, &bal)| {
                    acct.parse::<u32>()
                        .ok()
                        .filter(|&n| (4000..=8999).contains(&n))
                        .map(|_| -bal) // credit-normal income accounts are negative in debit-net map
                })
                .sum();
            self.state.balance_state.retained_earnings += retained;

            // Advance document ID counters so each period's IDs are globally unique.
            self.state.document_id_state.next_je_number += je_count as u64;
            self.state.document_id_state.next_po_number +=
                result.document_flows.purchase_orders.len() as u64;
            self.state.document_id_state.next_so_number +=
                result.document_flows.sales_orders.len() as u64;
            self.state.document_id_state.next_invoice_number +=
                (result.document_flows.vendor_invoices.len()
                    + result.document_flows.customer_invoices.len()) as u64;
            self.state.document_id_state.next_payment_number +=
                result.document_flows.payments.len() as u64;
            self.state.document_id_state.next_gr_number +=
                result.document_flows.goods_receipts.len() as u64;
        }

        self.state.generation_log.push(PeriodLog {
            period_label: period.label.clone(),
            journal_entries: je_count,
            documents: doc_count,
            anomalies: anomaly_count,
            duration_secs: duration,
        });

        self.state.period_cursor += 1;

        Ok(Some(PeriodResult {
            period,
            output_path,
            journal_entry_count: je_count,
            document_count: doc_count,
            anomaly_count,
            duration_secs: duration,
        }))
    }

    /// Append standard year-end closing entries to `result`, then compute the
    /// POST-close balance-sheet carry-forward for the next fiscal year.
    ///
    /// Steps:
    ///   1. Build the close trial balance from `result.journal_entries` as a
    ///      NATURAL-MAGNITUDE positive map (revenue credit-normal → positive,
    ///      expense debit-normal → positive). The close generator adds each
    ///      account's value directly as the side amount on its natural side
    ///      (see `year_end.rs::close_revenue_accounts`/`close_expense_accounts`),
    ///      so a raw debit-net would roll revenue the wrong way.
    ///   2. Run [`YearEndCloseGenerator`] over that TB, NAMESPACE the closing
    ///      entries' references per FY (the generator's counter resets each
    ///      `new`, so `YECL-*-00000001` would collide across years), and
    ///      `extend` them onto `result.journal_entries` BEFORE the period
    ///      writer runs.
    ///   3. Recompute the post-close GL net (now including the closing
    ///      entries), keep ONLY balance-sheet accounts, decompose each into a
    ///      debit/credit side typed (contra-aware) off the CoA, sort by
    ///      account code, and store on `self.state.carry_forward` for the next
    ///      FY's opening injection.
    ///
    /// Only called for multi-FY runs (`self.periods.len() > 1`); draws no RNG.
    fn close_and_carry_forward(
        &mut self,
        period: &GenerationPeriod,
        result: &mut crate::enhanced_orchestrator::EnhancedGenerationResult,
    ) {
        use datasynth_core::models::balance::{AccountType as BalAccountType, EntityOpeningBalance};
        use datasynth_core::models::{AccountType as CoaAccountType, YearEndClosingSpec};
        use datasynth_core::FrameworkAccounts;
        use datasynth_generators::period_close::{YearEndCloseConfig, YearEndCloseGenerator};
        use rust_decimal::Decimal;
        use std::collections::HashMap;

        let company_code = self
            .config
            .companies
            .first()
            .map(|c| c.code.clone())
            .unwrap_or_default();
        let fiscal_year = period.end_date.year();

        // --- Classify revenue/expense off the emitted CoA (robust across ----
        // frameworks — SKR04/PCG don't use 4=revenue/5-6=expense prefixes).
        // The full account code IS the "prefix" the close matches on, so
        // `account.starts_with(code)` matches exactly.
        let mut revenue_accounts: Vec<String> = Vec::new();
        let mut expense_accounts: Vec<String> = Vec::new();
        for acct in &result.chart_of_accounts.accounts {
            match acct.account_type {
                CoaAccountType::Revenue => revenue_accounts.push(acct.account_number.clone()),
                CoaAccountType::Expense => expense_accounts.push(acct.account_number.clone()),
                _ => {}
            }
        }

        // --- Income-summary / RE / dividends accounts come from the framework
        // map (no dedicated CoA sub_type for income-summary / dividends). The
        // framework string mirrors `EnhancedOrchestrator::resolve_framework_str`
        // (country first, then the accounting-standards label) so these codes
        // match the chart the orchestrator actually emitted.
        let fa = FrameworkAccounts::for_framework(self.resolve_framework_str());

        // --- (1) Build the close TB as natural-magnitude POSITIVE per account.
        // Sum signed debit-net per account, then sign each by its normal side
        // so revenue/expense are positive magnitudes (what the close expects).
        let mut net: HashMap<String, Decimal> = HashMap::new();
        for je in &result.journal_entries {
            for line in &je.lines {
                *net.entry(line.gl_account.clone()).or_insert(Decimal::ZERO) +=
                    line.debit_amount - line.credit_amount;
            }
        }
        let mut close_tb: HashMap<String, Decimal> = HashMap::with_capacity(net.len());
        for (code, debit_net) in &net {
            // natural magnitude = debit-net for debit-normal, credit-net for
            // credit-normal. net_balance() already encodes the side per type.
            let acct_type = self.balance_account_type(result, code);
            let magnitude = if Self::is_debit_normal(acct_type) {
                *debit_net
            } else {
                -*debit_net
            };
            close_tb.insert(code.clone(), magnitude);
        }

        let spec = YearEndClosingSpec {
            company_code: company_code.clone(),
            fiscal_year,
            revenue_accounts,
            expense_accounts,
            income_summary_account: fa.income_summary.clone(),
            retained_earnings_account: fa.retained_earnings.clone(),
            dividend_account: Some(fa.dividends_paid.clone()),
        };

        // --- (2) Run the close, namespace, and append. ----------------------
        let mut close_gen = YearEndCloseGenerator::new(YearEndCloseConfig::from(&fa));
        let mut close = close_gen.generate_year_end_close(&company_code, fiscal_year, &close_tb, &spec);
        debug_assert!(
            close.all_entries_balanced(),
            "year-end closing entries must balance"
        );

        // NAMESPACE the closing-entry references per FY: the generator's
        // `entry_counter` resets to 0 each `new`, so `YECL-REV-00000001`
        // collides across years. Prefix the header.reference + each line's
        // reference with the FY label (a pure function of period.index, so it
        // reproduces). The header.document_id (UUID) is already unique.
        for je in &mut close.closing_entries {
            if let Some(r) = je.header.reference.take() {
                je.header.reference = Some(format!("{}-{r}", period.label));
            }
            for line in &mut je.lines {
                if let Some(r) = line.reference.take() {
                    line.reference = Some(format!("{}-{r}", period.label));
                }
            }
        }

        result
            .journal_entries
            .extend(close.closing_entries.iter().cloned());

        // --- (3) Build the next-FY carry-forward from the POST-close GL net.
        // Re-net the now-extended JE set so the closing entries are included
        // (revenue/expense net to ~0 post-close; net income has rolled into
        // retained earnings / equity, so the BS-only set satisfies A=L+E).
        let mut post: HashMap<String, Decimal> = HashMap::new();
        for je in &result.journal_entries {
            for line in &je.lines {
                *post.entry(line.gl_account.clone()).or_insert(Decimal::ZERO) +=
                    line.debit_amount - line.credit_amount;
            }
        }

        let mut carry: Vec<EntityOpeningBalance> = Vec::new();
        for (code, debit_net) in &post {
            if *debit_net == Decimal::ZERO {
                continue;
            }
            let account_type = self.balance_account_type(result, code);
            // Keep ONLY balance-sheet accounts; drop Revenue/Expense (they
            // reset to zero next FY — net income is already in retained
            // earnings via the close).
            if matches!(account_type, BalAccountType::Revenue | BalAccountType::Expense) {
                continue;
            }
            // Decompose the signed debit-net into a single side. For a
            // debit-normal type a positive debit-net is a debit balance; for a
            // credit-normal type a positive debit-net (i.e. a negative
            // credit-balance) is a debit too. At most one side is non-zero.
            let (debit, credit) = if *debit_net >= Decimal::ZERO {
                (*debit_net, Decimal::ZERO)
            } else {
                (Decimal::ZERO, -*debit_net)
            };
            carry.push(EntityOpeningBalance {
                account_code: code.clone(),
                account_type,
                debit,
                credit,
            });
        }
        // Deterministic order (HashMap iteration is otherwise non-deterministic).
        carry.sort_by(|a, b| a.account_code.cmp(&b.account_code));
        self.state.carry_forward = carry;
    }

    /// Map a GL account code to the 8-variant balance [`AccountType`], preferring
    /// the emitted CoA (contra-aware via `sub_type`) and falling back to the
    /// leading-digit heuristic when the account is absent from the chart.
    fn balance_account_type(
        &self,
        result: &crate::enhanced_orchestrator::EnhancedGenerationResult,
        code: &str,
    ) -> datasynth_core::models::balance::AccountType {
        use datasynth_core::models::balance::AccountType as BalAccountType;
        use datasynth_core::models::{AccountSubType, AccountType as CoaAccountType};

        if let Some(acct) = result.chart_of_accounts.get_account(code) {
            // Contra accounts can't be recovered from the 6-variant type alone
            // (the engine folds accumulated depreciation into Asset and
            // treasury stock into Equity); override off the sub_type so
            // net_balance() signs them correctly.
            return match acct.sub_type {
                AccountSubType::AccumulatedDepreciation => BalAccountType::ContraAsset,
                AccountSubType::TreasuryStock => BalAccountType::ContraEquity,
                _ => match acct.account_type {
                    CoaAccountType::Asset => BalAccountType::Asset,
                    CoaAccountType::Liability => BalAccountType::Liability,
                    CoaAccountType::Equity => BalAccountType::Equity,
                    CoaAccountType::Revenue => BalAccountType::Revenue,
                    CoaAccountType::Expense => BalAccountType::Expense,
                    // Statistical accounts carry no real balance; treat as Asset
                    // (debit-normal) — they net to zero and are dropped from the
                    // BS-only carry-forward anyway.
                    CoaAccountType::Statistical => BalAccountType::Asset,
                },
            };
        }
        BalAccountType::from_account_code(code)
    }

    /// True if a balance [`AccountType`] is debit-normal (its natural-side
    /// magnitude equals its debit-net), mirroring `EntityOpeningBalance::net_balance`.
    fn is_debit_normal(account_type: datasynth_core::models::balance::AccountType) -> bool {
        use datasynth_core::models::balance::AccountType;
        matches!(
            account_type,
            AccountType::Asset
                | AccountType::ContraLiability
                | AccountType::ContraEquity
                | AccountType::Expense
        )
    }

    /// Resolve the framework string the same way the orchestrator does
    /// (country first, then the accounting-standards label), so the
    /// `FrameworkAccounts` codes used by the close match the emitted chart.
    /// Replicated here because the orchestrator's `resolve_framework_str` is
    /// private.
    fn resolve_framework_str(&self) -> &'static str {
        let country = self
            .config
            .companies
            .first()
            .map(|c| c.country.as_str())
            .unwrap_or("US")
            .to_ascii_uppercase();
        match country.as_str() {
            "DE" | "AT" => "german_gaap",
            "FR" | "BE" | "LU" => "french_gaap",
            _ => {
                if self.config.accounting_standards.enabled {
                    use datasynth_config::schema::AccountingFrameworkConfig as Fw;
                    match self.config.accounting_standards.framework {
                        Some(Fw::FrenchGaap) => return "french_gaap",
                        Some(Fw::GermanGaap) => return "german_gaap",
                        Some(Fw::Ifrs) => return "ifrs",
                        Some(Fw::DualReporting) => return "dual_reporting",
                        Some(Fw::UsGaap) | None => {}
                    }
                }
                "us_gaap"
            }
        }
    }

    /// Generate all remaining periods in the sequence.
    pub fn generate_all(&mut self) -> SynthResult<Vec<PeriodResult>> {
        let mut results = Vec::new();
        while let Some(result) = self.generate_next_period()? {
            results.push(result);
        }
        Ok(results)
    }

    /// Extend the session with additional months and generate them.
    pub fn generate_delta(&mut self, additional_months: u32) -> SynthResult<Vec<PeriodResult>> {
        let last_end = if let Some(last_period) = self.periods.last() {
            add_months(last_period.end_date, 1)
        } else {
            chrono::NaiveDate::parse_from_str(&self.config.global.start_date, "%Y-%m-%d")
                .map_err(|e| SynthError::generation(format!("Invalid start_date: {e}")))?
        };

        let fy_months = self
            .config
            .global
            .fiscal_year_months
            .unwrap_or(self.config.global.period_months);
        let new_periods = GenerationPeriod::compute_periods(last_end, additional_months, fy_months);

        let base_index = self.periods.len();
        let new_periods: Vec<GenerationPeriod> = new_periods
            .into_iter()
            .enumerate()
            .map(|(i, mut p)| {
                p.index = base_index + i;
                p
            })
            .collect();

        self.periods.extend(new_periods);
        self.generate_all()
    }

    /// Read-only access to the session state.
    pub fn state(&self) -> &SessionState {
        &self.state
    }

    /// Read-only access to the period list.
    pub fn periods(&self) -> &[GenerationPeriod] {
        &self.periods
    }

    /// Number of periods that have not yet been generated.
    pub fn remaining_periods(&self) -> usize {
        self.periods.len().saturating_sub(self.state.period_cursor)
    }

    /// Compute a hash of the config for drift detection.
    fn compute_config_hash(config: &GeneratorConfig) -> String {
        use std::hash::{Hash, Hasher};
        let json = serde_json::to_string(config).unwrap_or_default();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        json.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_config() -> GeneratorConfig {
        serde_yaml::from_str(
            r#"
global:
  seed: 42
  industry: retail
  start_date: "2024-01-01"
  period_months: 12
companies:
  - code: "C001"
    name: "Test Corp"
    currency: "USD"
    country: "US"
    annual_transaction_volume: ten_k
chart_of_accounts:
  complexity: small
output:
  output_directory: "./output"
"#,
        )
        .expect("minimal config should parse")
    }

    #[test]
    fn test_session_new_single_period() {
        let config = minimal_config();
        let session =
            GenerationSession::new(config, PathBuf::from("/tmp/test_session_single")).unwrap();
        assert_eq!(session.periods().len(), 1);
        assert_eq!(session.remaining_periods(), 1);
    }

    #[test]
    fn test_session_new_multi_period() {
        let mut config = minimal_config();
        config.global.period_months = 36;
        config.global.fiscal_year_months = Some(12);
        let session =
            GenerationSession::new(config, PathBuf::from("/tmp/test_session_multi")).unwrap();
        assert_eq!(session.periods().len(), 3);
        assert_eq!(session.remaining_periods(), 3);
    }

    #[test]
    fn test_session_save_and_resume() {
        let config = minimal_config();
        let session =
            GenerationSession::new(config.clone(), PathBuf::from("/tmp/test_session_save"))
                .unwrap();
        let tmp = std::env::temp_dir().join("test_gen_session.dss");
        session.save(&tmp).unwrap();
        let resumed = GenerationSession::resume(&tmp, config).unwrap();
        assert_eq!(resumed.state().period_cursor, 0);
        assert_eq!(resumed.state().rng_seed, 42);
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn test_session_resume_config_mismatch() {
        let config = minimal_config();
        let session =
            GenerationSession::new(config.clone(), PathBuf::from("/tmp/test_session_mismatch"))
                .unwrap();
        let tmp = std::env::temp_dir().join("test_gen_session_mismatch.dss");
        session.save(&tmp).unwrap();
        let mut different = config;
        different.global.seed = Some(999);
        let result = GenerationSession::resume(&tmp, different);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Config has changed"),
            "Expected config drift error, got: {}",
            err_msg
        );
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn test_session_remaining_periods() {
        let config = minimal_config();
        let session =
            GenerationSession::new(config, PathBuf::from("/tmp/test_session_remaining")).unwrap();
        assert_eq!(session.remaining_periods(), 1);
    }

    #[test]
    fn test_session_config_hash_deterministic() {
        let config = minimal_config();
        let h1 = GenerationSession::compute_config_hash(&config);
        let h2 = GenerationSession::compute_config_hash(&config);
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_session_config_hash_changes_on_mutation() {
        let config = minimal_config();
        let h1 = GenerationSession::compute_config_hash(&config);
        let mut modified = config;
        modified.global.seed = Some(999);
        let h2 = GenerationSession::compute_config_hash(&modified);
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_session_output_mode_batch_for_single_period() {
        let config = minimal_config();
        let session =
            GenerationSession::new(config, PathBuf::from("/tmp/test_batch_mode")).unwrap();
        assert!(matches!(session.output_mode, OutputMode::Batch(_)));
    }

    #[test]
    fn test_session_output_mode_multi_for_multiple_periods() {
        let mut config = minimal_config();
        config.global.period_months = 24;
        config.global.fiscal_year_months = Some(12);
        let session =
            GenerationSession::new(config, PathBuf::from("/tmp/test_multi_mode")).unwrap();
        assert!(matches!(session.output_mode, OutputMode::MultiPeriod(_)));
    }
}
