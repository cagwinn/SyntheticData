//! Generate journal entries from document flows.
//!
//! This module creates proper GL entries from P2P and O2C document chains,
//! ensuring that document flow activity is reflected in the general ledger.
//!
//! # P2P Flow JE Mappings
//! - GoodsReceipt → DR Inventory, CR GR/IR Clearing
//! - VendorInvoice → DR GR/IR Clearing (net), DR Input VAT (tax), CR AP (gross)
//! - Payment → DR AP, CR Cash
//!
//! # O2C Flow JE Mappings
//! - Delivery → DR COGS, CR Inventory
//! - CustomerInvoice → DR AR (gross), CR Revenue (net), CR VAT Payable (tax)
//! - CustomerReceipt → DR Cash, CR AR

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::NaiveDate;
use rand::prelude::*;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use rust_decimal::Decimal;

use datasynth_core::accounts::{
    cash_accounts, control_accounts, expense_accounts, revenue_accounts, suspense_accounts,
    tax_accounts,
};
use datasynth_core::models::{
    documents::{CustomerInvoice, Delivery, GoodsReceipt, Payment, VendorInvoice},
    BusinessProcess, DocumentRef, JournalEntry, JournalEntryHeader, JournalEntryLine,
    TransactionSource,
};
use datasynth_core::uuid_factory::{DeterministicUuidFactory, GeneratorType};

use super::{O2CDocumentChain, P2PDocumentChain};

/// Configuration for document flow JE generation.
#[derive(Debug, Clone)]
pub struct DocumentFlowJeConfig {
    /// Inventory account (default: 1200 from control_accounts::INVENTORY)
    pub inventory_account: String,
    /// GR/IR clearing account (default: 2900 from control_accounts::GR_IR_CLEARING)
    pub gr_ir_clearing_account: String,
    /// Accounts payable control account (default: 2000 from control_accounts::AP_CONTROL)
    pub ap_account: String,
    /// Cash/bank account (default: 1000 from cash_accounts::OPERATING_CASH)
    pub cash_account: String,
    /// Accounts receivable control account (default: 1100 from control_accounts::AR_CONTROL)
    pub ar_account: String,
    /// Revenue account (default: 4000 from revenue_accounts::PRODUCT_REVENUE)
    pub revenue_account: String,
    /// COGS account (default: 5000 from expense_accounts::COGS)
    pub cogs_account: String,
    /// VAT output (payable) account for O2C (default: 2110 from tax_accounts::VAT_PAYABLE)
    pub vat_output_account: String,
    /// VAT input (receivable) account for P2P (default: 1160 from tax_accounts::INPUT_VAT)
    pub vat_input_account: String,
    /// Whether to populate FEC auxiliary and lettrage fields on AP/AR lines.
    /// Only relevant for French GAAP / FEC export.
    pub populate_fec_fields: bool,
    /// SP3.13 W1 — Share of vendor invoices that post directly to expense GL
    /// accounts (bypassing GR/IR Clearing).  corpus ratio is ~70% direct
    /// (services, utilities, travel, professional fees) vs. ~30% via GR/IR
    /// (three-way-match for materials receipts).
    ///
    /// When `loaded_priors.is_some()`, the direct-expense path draws the GL
    /// account from the per-source conditional keyed on "KR", so W1.5
    /// `split_je_expense_lines` can then split it into multiple expense lines.
    ///
    /// Clamped to [0.0, 1.0].  Default: 0.70.
    /// No-op when `loaded_priors` is `None` (priors-disabled path unchanged).
    pub direct_expense_share: f64,
}

impl Default for DocumentFlowJeConfig {
    fn default() -> Self {
        Self {
            inventory_account: control_accounts::INVENTORY.to_string(),
            gr_ir_clearing_account: control_accounts::GR_IR_CLEARING.to_string(),
            ap_account: control_accounts::AP_CONTROL.to_string(),
            cash_account: cash_accounts::OPERATING_CASH.to_string(),
            ar_account: control_accounts::AR_CONTROL.to_string(),
            revenue_account: revenue_accounts::PRODUCT_REVENUE.to_string(),
            cogs_account: expense_accounts::COGS.to_string(),
            vat_output_account: tax_accounts::VAT_PAYABLE.to_string(),
            vat_input_account: tax_accounts::INPUT_VAT.to_string(),
            populate_fec_fields: false,
            direct_expense_share: 0.70,
        }
    }
}

impl DocumentFlowJeConfig {
    /// Create a config for French GAAP (PCG) with FEC field population enabled.
    pub fn french_gaap() -> Self {
        use datasynth_core::pcg;
        Self {
            inventory_account: pcg::control_accounts::INVENTORY.to_string(),
            gr_ir_clearing_account: pcg::control_accounts::GR_IR_CLEARING.to_string(),
            ap_account: pcg::control_accounts::AP_CONTROL.to_string(),
            cash_account: pcg::cash_accounts::BANK_ACCOUNT.to_string(),
            ar_account: pcg::control_accounts::AR_CONTROL.to_string(),
            revenue_account: pcg::revenue_accounts::PRODUCT_REVENUE.to_string(),
            cogs_account: pcg::expense_accounts::COGS.to_string(),
            vat_output_account: pcg::tax_accounts::OUTPUT_VAT.to_string(),
            vat_input_account: pcg::tax_accounts::INPUT_VAT.to_string(),
            populate_fec_fields: true,
            direct_expense_share: 0.70,
        }
    }
}

impl From<&datasynth_core::FrameworkAccounts> for DocumentFlowJeConfig {
    fn from(fa: &datasynth_core::FrameworkAccounts) -> Self {
        Self {
            inventory_account: fa.inventory.clone(),
            gr_ir_clearing_account: fa.gr_ir_clearing.clone(),
            ap_account: fa.ap_control.clone(),
            cash_account: fa.bank_account.clone(),
            ar_account: fa.ar_control.clone(),
            revenue_account: fa.product_revenue.clone(),
            cogs_account: fa.cogs.clone(),
            vat_output_account: fa.vat_payable.clone(),
            vat_input_account: fa.input_vat.clone(),
            populate_fec_fields: fa.audit_export.fec_enabled,
            direct_expense_share: 0.70,
        }
    }
}

/// Maximum number of sub-lines produced by a single expense split.
/// Prevents runaway splits on heavy-tailed distributions.
const MAX_SPLIT_LINES: usize = 8;

/// SP3.13 follow-up — geometric within-bucket draw biased toward `lo`.
///
/// Real KR JEs are concentrated at 3 lines; the prior's bucket [4, 9] would
/// draw uniformly to a mean of ~6.5. A geometric decay with `DECAY = 0.5`
/// shifts the mean to ~4.5, closely matching the corpus shape.
///
/// Probabilities: p(k) = DECAY^(k - lo) / normalizer for k in [lo, hi].
/// Buckets of width ≤ 1 return `lo` directly (no sampling needed).
///
/// Exposed as `pub(crate)` so the inline unit test in `mod tests` can call it.
pub(crate) fn sample_within_bucket<R: rand::Rng>(lo: u32, hi: u32, rng: &mut R) -> u32 {
    if hi == lo {
        return lo;
    }
    const DECAY: f64 = 0.5;
    let n = (hi - lo + 1) as usize;
    let mut weights: Vec<f64> = (0..n).map(|i| DECAY.powi(i as i32)).collect();
    // Normalise in-place.
    let total: f64 = weights.iter().sum();
    for w in &mut weights {
        *w /= total;
    }
    // Cumulative draw.
    let r: f64 = rng.random_range(0.0..1.0);
    let mut cum = 0.0;
    for (i, w) in weights.iter().enumerate() {
        cum += w;
        if r <= cum {
            return lo + i as u32;
        }
    }
    hi
}

/// SP3.13 follow-up — sample target line count from a `LineCountHistogram`
/// using the geometric within-bucket draw instead of the uniform one in
/// `LineCountHistogram::sample_bucket`.
///
/// Phase 1: pick a bucket index weighted by the histogram's probability mass.
/// Phase 2: draw within that bucket's `[lo, hi]` range geometrically
///           (see `sample_within_bucket`).
///
/// The histogram's bucket convention is inclusive lower-bound, exclusive upper-
/// bound. The last bucket has no successor, so its upper bound is treated as
/// equal to the lower bound (single-point bucket).
fn sample_target_lines_geometric<R: rand::Rng>(
    hist: &datasynth_core::distributions::behavioral_priors::LineCountHistogram,
    rng: &mut R,
) -> u32 {
    if hist.buckets.is_empty() {
        return 0;
    }
    // Phase 1: pick bucket by probability mass (same logic as sample_bucket).
    let r: f64 = rng.random_range(0.0..1.0);
    let mut cum = 0.0;
    let mut chosen_idx = hist.buckets.len() - 1;
    for (i, &p) in hist.probabilities.iter().enumerate() {
        cum += p;
        if r <= cum {
            chosen_idx = i;
            break;
        }
    }
    let lo = hist.buckets[chosen_idx];
    // Phase 2: compute upper bound (exclusive) and sample geometrically.
    // The last bucket has no next bucket, so hi = lo (single-point).
    let hi_exclusive = hist.buckets.get(chosen_idx + 1).copied().unwrap_or(lo);
    // Convert exclusive upper bound to inclusive for the geometric draw.
    let hi_inclusive = if hi_exclusive > lo {
        hi_exclusive - 1
    } else {
        lo
    };
    sample_within_bucket(lo, hi_inclusive, rng)
}

/// Generator for creating JEs from document flows.
pub struct DocumentFlowJeGenerator {
    config: DocumentFlowJeConfig,
    uuid_factory: DeterministicUuidFactory,
    /// Lookup map: partner_id → auxiliary GL account number.
    /// When populated (from vendor/customer master data), `set_auxiliary_fields`
    /// uses the framework-specific auxiliary account (e.g., PCG "4010001", SKR04 "33000001")
    /// instead of the raw partner ID.
    auxiliary_account_lookup: HashMap<String, String>,
    /// Cost-center IDs sourced from the generated cost-centers master so
    /// document-flow-derived JEs (P2P / O2C) reference IDs that join
    /// back to `cost_centers.id`.  Falls back to the hardcoded
    /// `COST_CENTER_POOL` const when empty.
    cost_center_pool: Vec<String>,
    /// Profit-center IDs sourced from the generated profit-centers master.
    /// Same population semantics as `cost_center_pool`.
    profit_center_pool: Vec<String>,
    /// SP3.12 — when `Some`, pads fixed-shape JEs with balancing pair-lines
    /// so the lines-per-JE distribution matches the corpus prior.
    /// Set by the orchestrator from the same `LoadedPriors` used by
    /// `JournalEntryGenerator`.  No-op when `None`.
    pub loaded_priors: Option<Arc<crate::priors_loader::LoadedPriors>>,
    /// RNG for priors-driven padding; seeded deterministically from `seed`.
    rng: ChaCha8Rng,
}

impl DocumentFlowJeGenerator {
    /// Create a new document flow JE generator with default config and seed 0.
    pub fn new() -> Self {
        Self::with_config_and_seed(DocumentFlowJeConfig::default(), 0)
    }

    /// Create with custom account configuration and seed.
    pub fn with_config_and_seed(config: DocumentFlowJeConfig, seed: u64) -> Self {
        Self {
            config,
            uuid_factory: DeterministicUuidFactory::new(seed, GeneratorType::DocumentFlow),
            auxiliary_account_lookup: HashMap::new(),
            cost_center_pool: Vec::new(),
            profit_center_pool: Vec::new(),
            loaded_priors: None,
            // Offset the seed so this RNG is independent from the uuid_factory seed.
            rng: ChaCha8Rng::seed_from_u64(seed.wrapping_add(0xDF_5312_0000)),
        }
    }

    /// SP3.12 — Wire industry priors so that fixed-shape document-flow JEs are
    /// padded with balancing pair-lines to match the real lines-per-JE prior.
    pub fn set_loaded_priors(&mut self, priors: Arc<crate::priors_loader::LoadedPriors>) {
        self.loaded_priors = Some(priors);
    }

    /// Set the auxiliary account lookup map (partner_id → auxiliary GL account).
    ///
    /// When populated, FEC `auxiliary_account_number` fields will use the
    /// framework-specific auxiliary GL account (e.g., PCG "4010001") instead
    /// of the raw partner ID.
    pub fn set_auxiliary_account_lookup(&mut self, lookup: HashMap<String, String>) {
        self.auxiliary_account_lookup = lookup;
    }

    /// Set the cost-center pool (master-data IDs).  See
    /// `JeGenerator::with_cost_center_pool` for semantics.
    pub fn set_cost_center_pool(&mut self, ids: Vec<String>) {
        self.cost_center_pool = ids;
    }

    /// Set the profit-center pool (master-data IDs).
    pub fn set_profit_center_pool(&mut self, ids: Vec<String>) {
        self.profit_center_pool = ids;
    }

    /// SP3.12 W1.5 — Split the dominant single-account line on the debit (or
    /// credit) side into N sub-lines drawn from the per-source GL-account
    /// conditional, matching the corpus pattern: one invoice → 2-4 expense
    /// lines split across distinct GL accounts.
    ///
    /// # Design
    ///
    /// 1. Sample the target line count from the source-conditional
    ///    `LinesPerJePrior` histogram (e.g. "KR" → ~3 lines, "KZ" → 2 lines).
    /// 2. Find the *splittable* line — the debit line with the highest amount
    ///    that does NOT post to a control account (AP, AR, Inventory, GR/IR,
    ///    Cash, Bank, VAT, Suspense).
    /// 3. Draw N distinct GL accounts from
    ///    `per_source_attribute[doc_type]["gl_account"]`.
    /// 4. Split the original line's amount proportionally across N sub-lines
    ///    (Decimal-exact: last sub-line absorbs rounding residual).
    /// 5. Replace the original line with the N sub-lines and re-number all lines.
    ///
    /// The control-account (AP/AR/Cash/…) line stays a single line — only the
    /// expense/asset side is split.  Balance is preserved by construction.
    ///
    /// No-op when:
    ///   - `loaded_priors` is `None`
    ///   - the per-source GL conditional has < 2 distinct values
    ///   - the entry already has enough lines
    ///   - no splittable line is found
    fn split_je_expense_lines(&mut self, entry: &mut JournalEntry) {
        let priors = match self.loaded_priors.as_ref() {
            Some(p) => p.clone(),
            None => return,
        };

        // 1. Sample target line count from the source-conditional histogram.
        //    SP3.13 follow-up: use geometric within-bucket draw so the sampler
        //    concentrates near the bucket's lower bound instead of drawing
        //    uniformly.  The [4, 9] KR bucket had a uniform mean of ~6.5; with
        //    geometric decay 0.5 the mean shifts to ~4.5, matching real KR
        //    which is concentrated at 3 lines/JE.
        let doc_type = entry.header.document_type.clone();
        let hist = priors
            .lines_per_je
            .by_source
            .get(doc_type.as_str())
            .unwrap_or(&priors.lines_per_je.overall);
        let target = (sample_target_lines_geometric(hist, &mut self.rng) as usize)
            .clamp(2, 2 + MAX_SPLIT_LINES);

        let current = entry.lines.len();
        if current >= target {
            return; // Already has enough lines; no split needed.
        }

        // n_splits: how many sub-lines to produce (replaces the original 1 line)
        let n_splits = (target - current + 1).clamp(2, MAX_SPLIT_LINES);

        // 2. Collect the set of control accounts that must NOT be split.
        let no_split_accounts: HashSet<&str> = [
            self.config.ap_account.as_str(),
            self.config.ar_account.as_str(),
            self.config.cash_account.as_str(),
            self.config.gr_ir_clearing_account.as_str(),
            self.config.inventory_account.as_str(),
            self.config.vat_input_account.as_str(),
            self.config.vat_output_account.as_str(),
            control_accounts::AR_CONTROL,
            control_accounts::AP_CONTROL,
            control_accounts::INVENTORY,
            control_accounts::GR_IR_CLEARING,
            cash_accounts::OPERATING_CASH,
            cash_accounts::BANK_ACCOUNT,
            cash_accounts::WIRE_CLEARING,
            suspense_accounts::GENERAL_SUSPENSE,
        ]
        .into_iter()
        .collect();

        // 3. Find the splittable line: highest-amount debit not in no_split_accounts.
        //    Fall back to highest-amount credit if no qualifying debit exists.
        let splittable_idx: Option<usize> = {
            let mut best_debit: Option<(usize, Decimal)> = None;
            let mut best_credit: Option<(usize, Decimal)> = None;
            for (i, line) in entry.lines.iter().enumerate() {
                if no_split_accounts.contains(line.gl_account.as_str()) {
                    continue;
                }
                if line.debit_amount > Decimal::ZERO {
                    let amt = line.debit_amount;
                    if best_debit.map(|(_, a)| amt > a).unwrap_or(true) {
                        best_debit = Some((i, amt));
                    }
                } else if line.credit_amount > Decimal::ZERO {
                    let amt = line.credit_amount;
                    if best_credit.map(|(_, a)| amt > a).unwrap_or(true) {
                        best_credit = Some((i, amt));
                    }
                }
            }
            best_debit
                .map(|(i, _)| i)
                .or_else(|| best_credit.map(|(i, _)| i))
        };

        let idx = match splittable_idx {
            Some(i) => i,
            None => return, // No splittable line found.
        };

        // 4. Draw N distinct GL accounts from the per-source conditional.
        // SP4.6 — derive line role from the splittable line's direction so that
        // expense/asset accounts are drawn for DR splits and liability/revenue
        // accounts for CR splits.
        let split_line_role = if entry.lines[idx].debit_amount > Decimal::ZERO {
            "DR"
        } else {
            "CR"
        };
        let mut gl_accounts: Vec<String> = Vec::with_capacity(n_splits);
        {
            let mut seen: HashSet<String> = HashSet::new();
            let mut attempts = 0usize;
            while gl_accounts.len() < n_splits && attempts < 50 {
                attempts += 1;
                // SP4.6 — role-aware GL selection. Fall back to source-marginal when
                // the (source, role) conditional is absent or yields nothing.
                let candidate = priors
                    .sample_gl_for_source_role(&doc_type, split_line_role, &mut self.rng)
                    .or_else(|| {
                        priors.sample_attribute_for_source(&doc_type, "gl_account", &mut self.rng)
                    });
                if let Some(gl) = candidate {
                    if !no_split_accounts.contains(gl.as_str()) && seen.insert(gl.clone()) {
                        gl_accounts.push(gl);
                    }
                } else {
                    // No per-source conditional available — fall back gracefully.
                    break;
                }
            }
        }

        if gl_accounts.len() < 2 {
            // Cannot split meaningfully — not enough distinct GL accounts.
            return;
        }
        // Cap to the actual number we managed to draw.
        let n_actual = gl_accounts.len().min(n_splits);
        let gl_accounts = &gl_accounts[..n_actual];

        // 5. Split the original line's amount across n_actual sub-lines.
        //    Each sub-line weight is drawn from uniform(0.5, 1.5), then normalised.
        let original_line = entry.lines[idx].clone();
        let is_debit_split = original_line.debit_amount > Decimal::ZERO;
        let total_amount = if is_debit_split {
            original_line.debit_amount
        } else {
            original_line.credit_amount
        };

        // Draw weights and compute proportional amounts.
        let raw_weights: Vec<f64> = (0..n_actual)
            .map(|_| self.rng.random_range(0.5f64..1.5f64))
            .collect();
        let weight_sum: f64 = raw_weights.iter().sum();
        // Convert total_amount to f64 for proportional split, then back to Decimal.
        let total_f64 = total_amount.to_string().parse::<f64>().unwrap_or(0.0);
        let mut amounts: Vec<Decimal> = raw_weights
            .iter()
            .map(|w| {
                let proportion = w / weight_sum;
                let raw = (proportion * total_f64 * 100.0).round() / 100.0;
                Decimal::new((raw * 100.0) as i64, 2)
            })
            .collect();

        // Exact balance: last sub-line absorbs rounding residual.
        let computed_sum: Decimal = amounts[..n_actual - 1].iter().sum();
        let residual = total_amount - computed_sum;
        if residual > Decimal::ZERO {
            amounts[n_actual - 1] = residual;
        } else if n_actual >= 2 {
            // If residual is zero or negative due to rounding, redistribute to last two.
            let last_two_sum = amounts[n_actual - 2] + amounts[n_actual - 1] + residual;
            if last_two_sum > Decimal::ZERO {
                amounts[n_actual - 2] = last_two_sum / Decimal::from(2);
                amounts[n_actual - 1] = last_two_sum - amounts[n_actual - 2];
            }
        }

        // Guard: if any amount wound up ≤ 0 after rounding, abort the split.
        if amounts.iter().any(|a| *a <= Decimal::ZERO) {
            return;
        }

        // 6. Build the sub-lines and insert them where the original was.
        let doc_id = entry.header.document_id;
        let base_line_no = original_line.line_number;

        // Remove the original line.
        entry.lines.remove(idx);

        for (i, (gl, amount)) in gl_accounts.iter().zip(amounts.iter()).enumerate() {
            let line_no = base_line_no + i as u32;
            let mut split_line = if is_debit_split {
                JournalEntryLine::debit(doc_id, line_no, gl.clone(), *amount)
            } else {
                JournalEntryLine::credit(doc_id, line_no, gl.clone(), *amount)
            };
            // Inherit metadata from the original line.
            split_line.cost_center = original_line.cost_center.clone();
            split_line.profit_center = original_line.profit_center.clone();
            split_line.trading_partner = original_line.trading_partner.clone();
            split_line.text = original_line.text.clone();
            split_line.line_text = original_line.line_text.clone();
            split_line.segment = original_line.segment.clone();
            split_line.functional_area = original_line.functional_area.clone();
            split_line.project_code = original_line.project_code.clone();
            entry.lines.insert(idx + i, split_line);
        }

        // 7. Re-number all lines sequentially after the insertion.
        for (i, line) in entry.lines.iter_mut().enumerate() {
            line.line_number = (i + 1) as u32;
        }
    }

    /// Build an account description lookup from the configured accounts.
    fn account_description_map(&self) -> HashMap<String, String> {
        let mut map = HashMap::new();
        map.insert(
            self.config.inventory_account.clone(),
            "Inventory".to_string(),
        );
        map.insert(
            self.config.gr_ir_clearing_account.clone(),
            "GR/IR Clearing".to_string(),
        );
        map.insert(
            self.config.ap_account.clone(),
            "Accounts Payable".to_string(),
        );
        map.insert(
            self.config.cash_account.clone(),
            "Cash and Cash Equivalents".to_string(),
        );
        map.insert(
            self.config.ar_account.clone(),
            "Accounts Receivable".to_string(),
        );
        map.insert(
            self.config.revenue_account.clone(),
            "Product Revenue".to_string(),
        );
        map.insert(
            self.config.cogs_account.clone(),
            "Cost of Goods Sold".to_string(),
        );
        map.insert(
            self.config.vat_output_account.clone(),
            "VAT Payable".to_string(),
        );
        map.insert(
            self.config.vat_input_account.clone(),
            "Input VAT".to_string(),
        );
        map
    }

    /// Cost center pool used for expense account enrichment.
    const COST_CENTER_POOL: &'static [&'static str] =
        &["CC1000", "CC2000", "CC3000", "CC4000", "CC5000"];

    /// Enrich journal entry line items with account descriptions, cost centers,
    /// profit centers, value dates, line text, and assignment fields.
    ///
    /// Uses the configured accounts to derive descriptions, since the document
    /// flow JE generator does not have access to the full chart of accounts.
    fn enrich_line_items(&self, entry: &mut JournalEntry) {
        // T2-D Lever 1b: emit the SAP document-type code in the `source` column
        // for document-flow-derived JEs, instead of falling back to the coarse
        // `TransactionSource` enum (csv_sink). `document_type` is already the SAP
        // code (WE/KR/KZ/WL/DR/DZ). Closes the ~8.5% `automated` source residual
        // left by Lever 1 (experiments/ml/FINDINGS.md sec.7).
        if entry.header.sap_source_code.is_none() && !entry.header.document_type.is_empty() {
            entry.header.sap_source_code = Some(entry.header.document_type.clone());
        }
        let desc_map = self.account_description_map();
        let posting_date = entry.header.posting_date;
        let company_code = &entry.header.company_code;
        let header_text = entry.header.header_text.clone();
        let business_process = entry.header.business_process;

        // Derive a deterministic index from document_id for cost center selection
        let doc_id_bytes = entry.header.document_id.as_bytes();
        let mut cc_seed: usize = 0;
        for &b in doc_id_bytes {
            cc_seed = cc_seed.wrapping_add(b as usize);
        }

        for (i, line) in entry.lines.iter_mut().enumerate() {
            // 1. account_description from known accounts
            if line.account_description.is_none() {
                line.account_description = desc_map.get(&line.gl_account).cloned();
            }

            // 2. cost_center for expense accounts (5xxx/6xxx).
            //    When the orchestrator wired a master-data pool via
            //    `set_cost_center_pool`, draw from it filtered to the
            //    entry's company; otherwise fall back to the hardcoded
            //    `COST_CENTER_POOL`.
            if line.cost_center.is_none() {
                let first_char = line.gl_account.chars().next().unwrap_or('0');
                if first_char == '5' || first_char == '6' {
                    if !self.cost_center_pool.is_empty() {
                        let needle = format!("-{company_code}-");
                        let candidates: Vec<&String> = self
                            .cost_center_pool
                            .iter()
                            .filter(|id| id.contains(&needle))
                            .collect();
                        let pool: Vec<&String> = if candidates.is_empty() {
                            self.cost_center_pool.iter().collect()
                        } else {
                            candidates
                        };
                        let idx = cc_seed.wrapping_add(i) % pool.len();
                        line.cost_center = Some(pool[idx].clone());
                    } else {
                        let idx = cc_seed.wrapping_add(i) % Self::COST_CENTER_POOL.len();
                        line.cost_center = Some(Self::COST_CENTER_POOL[idx].to_string());
                    }
                }
            }

            // 3. profit_center: master pool when available, else
            //    derived from company code + business process (legacy).
            if line.profit_center.is_none() {
                if !self.profit_center_pool.is_empty() {
                    let needle = format!("-{company_code}-");
                    let candidates: Vec<&String> = self
                        .profit_center_pool
                        .iter()
                        .filter(|id| id.contains(&needle))
                        .collect();
                    let pool: Vec<&String> = if candidates.is_empty() {
                        self.profit_center_pool.iter().collect()
                    } else {
                        candidates
                    };
                    let idx = cc_seed.wrapping_add(i) % pool.len();
                    line.profit_center = Some(pool[idx].clone());
                } else {
                    let suffix = match business_process {
                        Some(BusinessProcess::P2P) => "-P2P",
                        Some(BusinessProcess::O2C) => "-O2C",
                        _ => "",
                    };
                    line.profit_center = Some(format!("PC-{company_code}{suffix}"));
                }
            }

            // 4. line_text: fall back to header_text
            if line.line_text.is_none() {
                line.line_text = header_text.clone();
            }

            // 5. value_date for AR/AP accounts
            if line.value_date.is_none()
                && (line.gl_account == self.config.ar_account
                    || line.gl_account == self.config.ap_account)
            {
                line.value_date = Some(posting_date);
            }

            // 6. assignment for AP/AR lines - extract partner ID from header text
            if line.assignment.is_none()
                && (line.gl_account == self.config.ap_account
                    || line.gl_account == self.config.ar_account)
            {
                if let Some(ref ht) = header_text {
                    if let Some(partner_part) = ht.rsplit(" - ").next() {
                        line.assignment = Some(partner_part.to_string());
                    }
                }
            }
        }
    }

    /// Set auxiliary account fields on AP/AR lines when FEC population is enabled.
    ///
    /// Only sets the fields if `populate_fec_fields` is true and the line's
    /// GL account matches the configured AP or AR control account.
    ///
    /// When an auxiliary account lookup is available, uses the framework-specific
    /// auxiliary GL account (e.g., PCG "4010001", SKR04 "33000001") instead of
    /// the raw partner ID.
    fn set_auxiliary_fields(
        &self,
        line: &mut JournalEntryLine,
        partner_id: &str,
        partner_label: &str,
    ) {
        if !self.config.populate_fec_fields {
            return;
        }
        if line.gl_account == self.config.ap_account || line.gl_account == self.config.ar_account {
            // Prefer the framework-specific auxiliary GL account from the lookup map;
            // fall back to the raw partner ID if not found.
            let aux_account = self
                .auxiliary_account_lookup
                .get(partner_id)
                .cloned()
                .unwrap_or_else(|| partner_id.to_string());
            line.auxiliary_account_number = Some(aux_account);
            line.auxiliary_account_label = Some(partner_label.to_string());
        }
    }

    /// Apply lettrage (matching) codes to all AP/AR lines in a set of entries.
    ///
    /// Only sets lettrage if `populate_fec_fields` is true. The lettrage code
    /// is derived from the chain ID (e.g. PO or SO document ID) and the date
    /// is typically the final payment's posting date.
    fn apply_lettrage(
        &self,
        entries: &mut [JournalEntry],
        chain_id: &str,
        lettrage_date: NaiveDate,
    ) {
        if !self.config.populate_fec_fields {
            return;
        }
        let code = format!("LTR-{}", &chain_id[..chain_id.len().min(8)]);
        for entry in entries.iter_mut() {
            for line in entry.lines.iter_mut() {
                if line.gl_account == self.config.ap_account
                    || line.gl_account == self.config.ar_account
                {
                    line.lettrage = Some(code.clone());
                    line.lettrage_date = Some(lettrage_date);
                }
            }
        }
    }

    /// **v5.8.0** — wire line-level `predecessor_line_id` pointers along
    /// a chain of JEs.
    ///
    /// For each adjacent pair `(prev, curr)` in `entries`, scan every
    /// line in `curr` and look for a line in `prev` with the same
    /// `gl_account`. When found, set `curr_line.predecessor_line_id =
    /// derive_transaction_id(prev_line)`.
    ///
    /// This corresponds to the natural P2P / O2C booking flow:
    ///
    /// - **P2P**: GR (DR Inventory / CR GR/IR) → Invoice (DR GR/IR /
    ///   CR AP) → Payment (DR AP / CR Cash). The GR/IR-credit line of
    ///   the GR JE is the predecessor of the GR/IR-debit line of the
    ///   Invoice JE; the AP-credit line of the Invoice JE is the
    ///   predecessor of the AP-debit line of the Payment JE.
    /// - **O2C**: Delivery (DR COGS / CR Inventory) → Invoice (DR AR /
    ///   CR Revenue) → Receipt (DR Cash / CR AR). AR-credit on Invoice
    ///   is predecessor of AR-debit on Receipt.
    ///
    /// Position-by-`gl_account` matching is intentionally simple and
    /// unambiguous on the canonical chain shapes; ties (multiple lines
    /// of the same gl_account in `prev`) match to the first occurrence
    /// — deterministic but lossy on multi-position chains. Adequate
    /// for the v5.8.0 MVP edge-list export; a strict 1-to-1 matcher is
    /// future work.
    fn wire_predecessor_chain(entries: &mut [JournalEntry]) {
        if entries.len() < 2 {
            return;
        }
        for i in 1..entries.len() {
            // Snapshot prev's lines as (gl_account, transaction_id) pairs
            // so we can mutate `entries[i]` without borrow conflict.
            let prev_lines: Vec<(String, String)> = entries[i - 1]
                .lines
                .iter()
                .map(|l| {
                    let tx_id = l.transaction_id.clone().unwrap_or_else(|| {
                        datasynth_core::models::JournalEntryLine::derive_transaction_id(
                            l.document_id,
                            l.line_number,
                        )
                    });
                    (l.gl_account.clone(), tx_id)
                })
                .collect();

            for line in entries[i].lines.iter_mut() {
                if line.predecessor_line_id.is_some() {
                    continue;
                }
                if let Some((_, tx_id)) =
                    prev_lines.iter().find(|(acct, _)| acct == &line.gl_account)
                {
                    line.predecessor_line_id = Some(tx_id.clone());
                }
            }
        }
    }

    /// Generate all JEs from a P2P document chain.
    pub fn generate_from_p2p_chain(&mut self, chain: &P2PDocumentChain) -> Vec<JournalEntry> {
        let mut entries = Vec::new();

        // Generate JEs for goods receipts
        for gr in &chain.goods_receipts {
            if let Some(je) = self.generate_from_goods_receipt(gr) {
                entries.push(je);
            }
        }

        // Generate JE for vendor invoice
        if let Some(ref invoice) = chain.vendor_invoice {
            if let Some(je) = self.generate_from_vendor_invoice(invoice) {
                entries.push(je);
            }
        }

        // Generate JE for payment
        if let Some(ref payment) = chain.payment {
            if let Some(je) = self.generate_from_ap_payment(payment) {
                entries.push(je);
            }
        }

        // Generate JEs for remainder payments
        for payment in &chain.remainder_payments {
            if let Some(je) = self.generate_from_ap_payment(payment) {
                entries.push(je);
            }
        }

        // Apply lettrage on complete P2P chains (invoice + payment both present)
        if self.config.populate_fec_fields && chain.is_complete {
            if let Some(ref payment) = chain.payment {
                let posting_date = payment
                    .header
                    .posting_date
                    .unwrap_or(payment.header.document_date);
                self.apply_lettrage(
                    &mut entries,
                    &chain.purchase_order.header.document_id,
                    posting_date,
                );
            }
        }

        // v5.8.0 — wire line-level predecessor pointers along the chain so
        // graphs/je_network.csv can trace booking chains across JEs.
        Self::wire_predecessor_chain(&mut entries);

        entries
    }

    /// Generate all JEs from an O2C document chain.
    pub fn generate_from_o2c_chain(&mut self, chain: &O2CDocumentChain) -> Vec<JournalEntry> {
        let mut entries = Vec::new();

        // Generate JEs for deliveries
        for delivery in &chain.deliveries {
            if let Some(je) = self.generate_from_delivery(delivery) {
                entries.push(je);
            }
        }

        // Generate JE for customer invoice
        if let Some(ref invoice) = chain.customer_invoice {
            if let Some(je) = self.generate_from_customer_invoice(invoice) {
                entries.push(je);
            }
        }

        // Generate JE for customer receipt
        if let Some(ref receipt) = chain.customer_receipt {
            if let Some(je) = self.generate_from_ar_receipt(receipt) {
                entries.push(je);
            }
        }

        // Generate JEs for remainder receipts (follow-up to partial payments)
        for receipt in &chain.remainder_receipts {
            if let Some(je) = self.generate_from_ar_receipt(receipt) {
                entries.push(je);
            }
        }

        // Apply lettrage on complete O2C chains (invoice + receipt both present)
        if self.config.populate_fec_fields && chain.customer_receipt.is_some() {
            if let Some(ref receipt) = chain.customer_receipt {
                let posting_date = receipt
                    .header
                    .posting_date
                    .unwrap_or(receipt.header.document_date);
                self.apply_lettrage(
                    &mut entries,
                    &chain.sales_order.header.document_id,
                    posting_date,
                );
            }
        }

        // v5.8.0 — wire line-level predecessor pointers along the chain.
        Self::wire_predecessor_chain(&mut entries);

        entries
    }

    /// Generate JE from Goods Receipt.
    /// DR Inventory, CR GR/IR Clearing
    pub fn generate_from_goods_receipt(&mut self, gr: &GoodsReceipt) -> Option<JournalEntry> {
        if gr.items.is_empty() {
            return None;
        }

        let document_id = self.uuid_factory.next();

        // Use the total_value from the GR, or calculate from line items
        let total_amount = if gr.total_value > Decimal::ZERO {
            gr.total_value
        } else {
            gr.items
                .iter()
                .map(|item| item.base.net_amount)
                .sum::<Decimal>()
        };

        if total_amount == Decimal::ZERO {
            return None;
        }

        // Use posting_date or fall back to document_date
        let posting_date = gr.header.posting_date.unwrap_or(gr.header.document_date);

        let mut header = JournalEntryHeader::with_deterministic_id(
            gr.header.company_code.clone(),
            posting_date,
            document_id,
        );
        header.source = TransactionSource::Automated;
        header.business_process = Some(BusinessProcess::P2P);
        header.document_type = "WE".to_string();
        header.reference = Some(format!("GR:{}", gr.header.document_id));
        header.source_document = Some(DocumentRef::GoodsReceipt(gr.header.document_id.clone()));
        header.header_text = Some(format!(
            "Goods Receipt {} - {}",
            gr.header.document_id,
            gr.vendor_id.as_deref().unwrap_or("Unknown")
        ));

        let mut entry = JournalEntry::new(header);

        // DR Inventory
        let debit_line = JournalEntryLine::debit(
            entry.header.document_id,
            1,
            self.config.inventory_account.clone(),
            total_amount,
        );
        entry.add_line(debit_line);

        // CR GR/IR Clearing — vendor-linked; populate trading_partner when known
        let mut credit_line = JournalEntryLine::credit(
            entry.header.document_id,
            2,
            self.config.gr_ir_clearing_account.clone(),
            total_amount,
        );
        credit_line.trading_partner = gr.vendor_id.clone();
        entry.add_line(credit_line);

        self.enrich_line_items(&mut entry);
        self.split_je_expense_lines(&mut entry);
        Some(entry)
    }

    /// Generate JE from Vendor Invoice.
    ///
    /// **Canonical three-way-match path** (priors disabled, or direct-expense
    /// draw fails):
    /// - DR GR/IR Clearing = net amount  (or payable when no VAT)
    /// - DR Input VAT      = tax amount  (only when VAT > 0)
    /// - CR AP              = gross (payable) amount
    ///
    /// **SP3.13 W1 — Direct-expense path** (priors enabled, random draw <
    /// `direct_expense_share`; ~70 % of real KR invoices are services /
    /// utilities / travel that bypass GR/IR):
    /// - DR <expense GL>   = net amount  (or payable when no VAT)
    /// - DR Input VAT      = tax amount  (only when VAT > 0)
    /// - CR AP              = gross (payable) amount
    ///
    /// The expense GL is drawn from the per-source "KR" conditional in
    /// `loaded_priors`, enabling W1.5 `split_je_expense_lines` to split it
    /// into multiple cost-account lines matching the corpus shape.
    pub fn generate_from_vendor_invoice(
        &mut self,
        invoice: &VendorInvoice,
    ) -> Option<JournalEntry> {
        if invoice.payable_amount == Decimal::ZERO {
            return None;
        }

        let document_id = self.uuid_factory.next();

        // Use posting_date or fall back to document_date
        let posting_date = invoice
            .header
            .posting_date
            .unwrap_or(invoice.header.document_date);

        let mut header = JournalEntryHeader::with_deterministic_id(
            invoice.header.company_code.clone(),
            posting_date,
            document_id,
        );
        header.source = TransactionSource::Automated;
        header.business_process = Some(BusinessProcess::P2P);
        header.document_type = "KR".to_string();
        header.reference = Some(format!("VI:{}", invoice.header.document_id));
        header.source_document = Some(DocumentRef::VendorInvoice(
            invoice.header.document_id.clone(),
        ));
        header.header_text = Some(format!(
            "Vendor Invoice {} - {}",
            invoice.vendor_invoice_number, invoice.vendor_id
        ));

        let mut entry = JournalEntry::new(header);

        let has_vat = invoice.tax_amount > Decimal::ZERO;
        // Net debit amount: if VAT present use net_amount so that
        //   DR net + DR VAT = CR gross (payable_amount).
        let debit_net_amount = if has_vat {
            invoice.net_amount
        } else {
            invoice.payable_amount
        };

        // SP3.13 W1 — Decide whether to post directly to an expense account
        // (bypassing GR/IR Clearing).  Gated on `loaded_priors` being set so
        // the priors-disabled path is byte-identical to the pre-SP3.13 behaviour.
        let direct_expense_share = self.config.direct_expense_share.clamp(0.0, 1.0);
        let use_direct_expense = self.loaded_priors.is_some()
            && self.rng.random_range(0.0_f64..1.0_f64) < direct_expense_share;

        if use_direct_expense {
            // Direct-expense path: DR <expense GL> / optional DR VAT / CR AP.
            // The single expense DR will be further split by W1.5
            // `split_je_expense_lines` when the prior has enough distinct GLs.
            // SP4.6 — role-aware GL selection: KR DR lines should draw expense
            // accounts (5/6xxx), not AP accounts.  Try (KR, DR) first, then the
            // source-marginal, then hard-code to a generic expense GL.
            let expense_gl = self
                .loaded_priors
                .as_ref()
                .and_then(|p| p.sample_gl_for_source_role("KR", "DR", &mut self.rng))
                .or_else(|| {
                    self.loaded_priors.as_ref().and_then(|p| {
                        p.sample_attribute_for_source("KR", "gl_account", &mut self.rng)
                    })
                })
                // Fall back to a generic operating expense GL when the
                // conditional draw fails (sparse prior or no "KR" entry).
                .unwrap_or_else(|| "6000".to_string());

            // DR Expense (net amount, or payable when no VAT)
            let debit_expense =
                JournalEntryLine::debit(entry.header.document_id, 1, expense_gl, debit_net_amount);
            entry.add_line(debit_expense);

            // DR Input VAT (only when tax is non-zero)
            if has_vat {
                let vat_line = JournalEntryLine::debit(
                    entry.header.document_id,
                    2,
                    self.config.vat_input_account.clone(),
                    invoice.tax_amount,
                );
                entry.add_line(vat_line);
            }

            // CR Accounts Payable (gross / payable amount)
            let mut credit_ap = JournalEntryLine::credit(
                entry.header.document_id,
                if has_vat { 3 } else { 2 },
                self.config.ap_account.clone(),
                invoice.payable_amount,
            );
            self.set_auxiliary_fields(&mut credit_ap, &invoice.vendor_id, &invoice.vendor_id);
            credit_ap.trading_partner = Some(invoice.vendor_id.clone());
            entry.add_line(credit_ap);
        } else {
            // Canonical three-way-match path (existing behaviour, unchanged).
            // DR GR/IR Clearing (net amount when VAT present, else payable)
            let debit_clearing = JournalEntryLine::debit(
                entry.header.document_id,
                1,
                self.config.gr_ir_clearing_account.clone(),
                debit_net_amount,
            );
            entry.add_line(debit_clearing);

            // DR Input VAT (only when tax is non-zero)
            if has_vat {
                let vat_line = JournalEntryLine::debit(
                    entry.header.document_id,
                    2,
                    self.config.vat_input_account.clone(),
                    invoice.tax_amount,
                );
                entry.add_line(vat_line);
            }

            // CR Accounts Payable (gross / payable amount)
            let mut credit_line = JournalEntryLine::credit(
                entry.header.document_id,
                if has_vat { 3 } else { 2 },
                self.config.ap_account.clone(),
                invoice.payable_amount,
            );
            self.set_auxiliary_fields(&mut credit_line, &invoice.vendor_id, &invoice.vendor_id);
            credit_line.trading_partner = Some(invoice.vendor_id.clone());
            entry.add_line(credit_line);
        }

        self.enrich_line_items(&mut entry);
        self.split_je_expense_lines(&mut entry);
        Some(entry)
    }

    /// Generate JE from AP Payment.
    /// DR AP, CR Cash
    pub fn generate_from_ap_payment(&mut self, payment: &Payment) -> Option<JournalEntry> {
        if payment.amount == Decimal::ZERO {
            return None;
        }

        let document_id = self.uuid_factory.next();

        // Use posting_date or fall back to document_date
        let posting_date = payment
            .header
            .posting_date
            .unwrap_or(payment.header.document_date);

        let mut header = JournalEntryHeader::with_deterministic_id(
            payment.header.company_code.clone(),
            posting_date,
            document_id,
        );
        header.source = TransactionSource::Automated;
        header.business_process = Some(BusinessProcess::P2P);
        header.document_type = "KZ".to_string();
        header.reference = Some(format!("PAY:{}", payment.header.document_id));
        header.source_document = Some(DocumentRef::Payment(payment.header.document_id.clone()));
        header.header_text = Some(format!(
            "Payment {} - {}",
            payment.header.document_id, payment.business_partner_id
        ));

        let mut entry = JournalEntry::new(header);

        // DR Accounts Payable — vendor-linked; populate trading_partner
        let mut debit_line = JournalEntryLine::debit(
            entry.header.document_id,
            1,
            self.config.ap_account.clone(),
            payment.amount,
        );
        self.set_auxiliary_fields(
            &mut debit_line,
            &payment.business_partner_id,
            &payment.business_partner_id,
        );
        debit_line.trading_partner = Some(payment.business_partner_id.clone());
        entry.add_line(debit_line);

        // CR Cash/Bank
        let credit_line = JournalEntryLine::credit(
            entry.header.document_id,
            2,
            self.config.cash_account.clone(),
            payment.amount,
        );
        entry.add_line(credit_line);

        self.enrich_line_items(&mut entry);
        self.split_je_expense_lines(&mut entry);
        Some(entry)
    }

    /// Generate JE from Delivery.
    /// DR COGS, CR Inventory
    pub fn generate_from_delivery(&mut self, delivery: &Delivery) -> Option<JournalEntry> {
        if delivery.items.is_empty() {
            return None;
        }

        let document_id = self.uuid_factory.next();

        // Calculate total cost from line items
        let total_cost = delivery
            .items
            .iter()
            .map(|item| item.base.net_amount)
            .sum::<Decimal>();

        if total_cost == Decimal::ZERO {
            return None;
        }

        // Use posting_date or fall back to document_date
        let posting_date = delivery
            .header
            .posting_date
            .unwrap_or(delivery.header.document_date);

        let mut header = JournalEntryHeader::with_deterministic_id(
            delivery.header.company_code.clone(),
            posting_date,
            document_id,
        );
        header.source = TransactionSource::Automated;
        header.business_process = Some(BusinessProcess::O2C);
        header.document_type = "WL".to_string();
        header.reference = Some(format!("DEL:{}", delivery.header.document_id));
        header.source_document = Some(DocumentRef::Delivery(delivery.header.document_id.clone()));
        header.header_text = Some(format!(
            "Delivery {} - {}",
            delivery.header.document_id, delivery.customer_id
        ));

        let mut entry = JournalEntry::new(header);

        // DR COGS
        let debit_line = JournalEntryLine::debit(
            entry.header.document_id,
            1,
            self.config.cogs_account.clone(),
            total_cost,
        );
        entry.add_line(debit_line);

        // CR Inventory
        let credit_line = JournalEntryLine::credit(
            entry.header.document_id,
            2,
            self.config.inventory_account.clone(),
            total_cost,
        );
        entry.add_line(credit_line);

        self.enrich_line_items(&mut entry);
        self.split_je_expense_lines(&mut entry);
        Some(entry)
    }

    /// Generate JE from Customer Invoice.
    ///
    /// When the invoice carries tax (`total_tax_amount > 0`), the entry is split:
    /// - DR AR          = gross amount
    /// - CR Revenue     = net amount
    /// - CR VAT Payable = tax amount
    ///
    /// When there is no tax, the original two-line entry is produced:
    /// - DR AR      = gross amount
    /// - CR Revenue = gross amount
    pub fn generate_from_customer_invoice(
        &mut self,
        invoice: &CustomerInvoice,
    ) -> Option<JournalEntry> {
        if invoice.total_gross_amount == Decimal::ZERO {
            return None;
        }

        let document_id = self.uuid_factory.next();

        // Use posting_date or fall back to document_date
        let posting_date = invoice
            .header
            .posting_date
            .unwrap_or(invoice.header.document_date);

        let mut header = JournalEntryHeader::with_deterministic_id(
            invoice.header.company_code.clone(),
            posting_date,
            document_id,
        );
        header.source = TransactionSource::Automated;
        header.business_process = Some(BusinessProcess::O2C);
        header.document_type = "DR".to_string();
        header.reference = Some(format!("CI:{}", invoice.header.document_id));
        header.source_document = Some(DocumentRef::CustomerInvoice(
            invoice.header.document_id.clone(),
        ));
        header.header_text = Some(format!(
            "Customer Invoice {} - {}",
            invoice.header.document_id, invoice.customer_id
        ));

        let mut entry = JournalEntry::new(header);

        // DR Accounts Receivable (gross amount) — customer-linked; populate trading_partner
        let mut debit_line = JournalEntryLine::debit(
            entry.header.document_id,
            1,
            self.config.ar_account.clone(),
            invoice.total_gross_amount,
        );
        self.set_auxiliary_fields(&mut debit_line, &invoice.customer_id, &invoice.customer_id);
        debit_line.trading_partner = Some(invoice.customer_id.clone());
        entry.add_line(debit_line);

        // CR Revenue (net amount when VAT present, else gross)
        let revenue_amount = if invoice.total_tax_amount > Decimal::ZERO {
            invoice.total_net_amount
        } else {
            invoice.total_gross_amount
        };
        let credit_line = JournalEntryLine::credit(
            entry.header.document_id,
            2,
            self.config.revenue_account.clone(),
            revenue_amount,
        );
        entry.add_line(credit_line);

        // CR VAT Payable (only when tax is non-zero)
        if invoice.total_tax_amount > Decimal::ZERO {
            let vat_line = JournalEntryLine::credit(
                entry.header.document_id,
                3,
                self.config.vat_output_account.clone(),
                invoice.total_tax_amount,
            );
            entry.add_line(vat_line);
        }

        self.enrich_line_items(&mut entry);
        self.split_je_expense_lines(&mut entry);
        Some(entry)
    }

    /// Generate JE from AR Receipt (Customer Payment).
    /// DR Cash, CR AR
    pub fn generate_from_ar_receipt(&mut self, payment: &Payment) -> Option<JournalEntry> {
        if payment.amount == Decimal::ZERO {
            return None;
        }

        let document_id = self.uuid_factory.next();

        // Use posting_date or fall back to document_date
        let posting_date = payment
            .header
            .posting_date
            .unwrap_or(payment.header.document_date);

        let mut header = JournalEntryHeader::with_deterministic_id(
            payment.header.company_code.clone(),
            posting_date,
            document_id,
        );
        header.source = TransactionSource::Automated;
        header.business_process = Some(BusinessProcess::O2C);
        header.document_type = "DZ".to_string();
        header.reference = Some(format!("RCP:{}", payment.header.document_id));
        header.source_document = Some(DocumentRef::Receipt(payment.header.document_id.clone()));
        header.header_text = Some(format!(
            "Customer Receipt {} - {}",
            payment.header.document_id, payment.business_partner_id
        ));

        let mut entry = JournalEntry::new(header);

        // DR Cash/Bank
        let debit_line = JournalEntryLine::debit(
            entry.header.document_id,
            1,
            self.config.cash_account.clone(),
            payment.amount,
        );
        entry.add_line(debit_line);

        // CR Accounts Receivable — customer-linked; populate trading_partner
        let mut credit_line = JournalEntryLine::credit(
            entry.header.document_id,
            2,
            self.config.ar_account.clone(),
            payment.amount,
        );
        self.set_auxiliary_fields(
            &mut credit_line,
            &payment.business_partner_id,
            &payment.business_partner_id,
        );
        credit_line.trading_partner = Some(payment.business_partner_id.clone());
        entry.add_line(credit_line);

        self.enrich_line_items(&mut entry);
        self.split_je_expense_lines(&mut entry);
        Some(entry)
    }
}

impl Default for DocumentFlowJeGenerator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use datasynth_core::models::documents::{GoodsReceiptItem, MovementType};

    fn create_test_gr() -> GoodsReceipt {
        let mut gr = GoodsReceipt::from_purchase_order(
            "GR-001".to_string(),
            "1000",
            "PO-001",
            "V-001",
            "P1000",
            "0001",
            2024,
            1,
            NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
            "JSMITH",
        );

        let item = GoodsReceiptItem::from_po(
            10,
            "Test Material",
            Decimal::from(100),
            Decimal::from(50),
            "PO-001",
            10,
        )
        .with_movement_type(MovementType::GrForPo);

        gr.add_item(item);
        gr.post("JSMITH", NaiveDate::from_ymd_opt(2024, 1, 15).unwrap());

        gr
    }

    fn create_test_vendor_invoice() -> VendorInvoice {
        use datasynth_core::models::documents::VendorInvoiceItem;

        let mut invoice = VendorInvoice::new(
            "VI-001".to_string(),
            "1000",
            "V-001",
            "INV-12345".to_string(),
            2024,
            1,
            NaiveDate::from_ymd_opt(2024, 1, 20).unwrap(),
            "JSMITH",
        );

        let item = VendorInvoiceItem::from_po_gr(
            10,
            "Test Material",
            Decimal::from(100),
            Decimal::from(50),
            "PO-001",
            10,
            Some("GR-001".to_string()),
            Some(10),
        );

        invoice.add_item(item);
        invoice.post("JSMITH", NaiveDate::from_ymd_opt(2024, 1, 20).unwrap());

        invoice
    }

    fn create_test_payment() -> Payment {
        let mut payment = Payment::new_ap_payment(
            "PAY-001".to_string(),
            "1000",
            "V-001",
            Decimal::from(5000),
            2024,
            2,
            NaiveDate::from_ymd_opt(2024, 2, 15).unwrap(),
            "JSMITH",
        );

        payment.post("JSMITH", NaiveDate::from_ymd_opt(2024, 2, 15).unwrap());

        payment
    }

    #[test]
    fn test_generate_from_goods_receipt() {
        let mut generator = DocumentFlowJeGenerator::new();
        let gr = create_test_gr();

        let je = generator.generate_from_goods_receipt(&gr);

        assert!(je.is_some());
        let je = je.unwrap();

        // Should be balanced
        assert!(je.is_balanced());

        // Should have 2 lines
        assert_eq!(je.line_count(), 2);

        // DR should be inventory, CR should be GR/IR
        assert!(je.total_debit() > Decimal::ZERO);
        assert_eq!(je.total_debit(), je.total_credit());

        // Should reference source document
        assert!(je.header.reference.is_some());
        assert!(je.header.reference.as_ref().unwrap().contains("GR:"));

        // T2-D Lever 1b: the `source` column carries the SAP doc-type code
        // (= document_type "WE"), not the coarse TransactionSource enum.
        assert_eq!(je.header.sap_source_code.as_deref(), Some("WE"));
        assert_eq!(je.header.document_type, "WE");
    }

    #[test]
    fn test_generate_from_vendor_invoice() {
        let mut generator = DocumentFlowJeGenerator::new();
        let invoice = create_test_vendor_invoice();

        let je = generator.generate_from_vendor_invoice(&invoice);

        assert!(je.is_some());
        let je = je.unwrap();

        assert!(je.is_balanced());
        assert_eq!(je.line_count(), 2);
        assert!(je.header.reference.as_ref().unwrap().contains("VI:"));
    }

    #[test]
    fn test_generate_from_ap_payment() {
        let mut generator = DocumentFlowJeGenerator::new();
        let payment = create_test_payment();

        let je = generator.generate_from_ap_payment(&payment);

        assert!(je.is_some());
        let je = je.unwrap();

        assert!(je.is_balanced());
        assert_eq!(je.line_count(), 2);
        assert!(je.header.reference.as_ref().unwrap().contains("PAY:"));
    }

    #[test]
    fn test_all_entries_are_balanced() {
        let mut generator = DocumentFlowJeGenerator::new();

        let gr = create_test_gr();
        let invoice = create_test_vendor_invoice();
        let payment = create_test_payment();

        let entries = vec![
            generator.generate_from_goods_receipt(&gr),
            generator.generate_from_vendor_invoice(&invoice),
            generator.generate_from_ap_payment(&payment),
        ];

        for entry in entries.into_iter().flatten() {
            assert!(
                entry.is_balanced(),
                "Entry {} is not balanced",
                entry.header.document_id
            );
        }
    }

    // ====================================================================
    // FEC compliance tests
    // ====================================================================

    #[test]
    fn test_french_gaap_auxiliary_on_ap_ar_lines_only() {
        // French GAAP config sets auxiliary fields on AP/AR lines only
        let mut generator =
            DocumentFlowJeGenerator::with_config_and_seed(DocumentFlowJeConfig::french_gaap(), 42);

        // Vendor invoice: AP line should have auxiliary, GR/IR line should not
        let invoice = create_test_vendor_invoice();
        let je = generator.generate_from_vendor_invoice(&invoice).unwrap();

        // Line 1 = DR GR/IR Clearing → no auxiliary
        assert!(
            je.lines[0].auxiliary_account_number.is_none(),
            "GR/IR clearing line should not have auxiliary"
        );

        // Line 2 = CR AP → has auxiliary
        assert_eq!(
            je.lines[1].auxiliary_account_number.as_deref(),
            Some("V-001"),
            "AP line should have vendor ID as auxiliary"
        );
        assert_eq!(
            je.lines[1].auxiliary_account_label.as_deref(),
            Some("V-001"),
        );
    }

    #[test]
    fn test_french_gaap_lettrage_on_complete_p2p_chain() {
        use datasynth_core::models::documents::PurchaseOrder;

        let mut generator =
            DocumentFlowJeGenerator::with_config_and_seed(DocumentFlowJeConfig::french_gaap(), 42);

        let po = PurchaseOrder::new(
            "PO-001",
            "1000",
            "V-001",
            2024,
            1,
            NaiveDate::from_ymd_opt(2024, 1, 10).unwrap(),
            "JSMITH",
        );

        let chain = P2PDocumentChain {
            purchase_order: po,
            goods_receipts: vec![create_test_gr()],
            vendor_invoice: Some(create_test_vendor_invoice()),
            payment: Some(create_test_payment()),
            remainder_payments: Vec::new(),
            is_complete: true,
            three_way_match_passed: true,
            payment_timing: None,
        };

        let entries = generator.generate_from_p2p_chain(&chain);
        assert!(!entries.is_empty());

        // All AP lines should share the same lettrage code
        let ap_account = &generator.config.ap_account;
        let mut lettrage_codes: Vec<&str> = Vec::new();
        for entry in &entries {
            for line in &entry.lines {
                if &line.gl_account == ap_account {
                    assert!(
                        line.lettrage.is_some(),
                        "AP line should have lettrage on complete chain"
                    );
                    assert!(line.lettrage_date.is_some());
                    lettrage_codes.push(line.lettrage.as_deref().unwrap());
                } else {
                    assert!(
                        line.lettrage.is_none(),
                        "Non-AP line should not have lettrage"
                    );
                }
            }
        }

        // All AP lettrage codes should be the same
        assert!(!lettrage_codes.is_empty());
        assert!(
            lettrage_codes.iter().all(|c| *c == lettrage_codes[0]),
            "All AP lines should share the same lettrage code"
        );
        assert!(lettrage_codes[0].starts_with("LTR-"));
    }

    #[test]
    fn test_incomplete_chain_has_no_lettrage() {
        use datasynth_core::models::documents::PurchaseOrder;

        let mut generator =
            DocumentFlowJeGenerator::with_config_and_seed(DocumentFlowJeConfig::french_gaap(), 42);

        let po = PurchaseOrder::new(
            "PO-002",
            "1000",
            "V-001",
            2024,
            1,
            NaiveDate::from_ymd_opt(2024, 1, 10).unwrap(),
            "JSMITH",
        );

        // Incomplete chain: no payment
        let chain = P2PDocumentChain {
            purchase_order: po,
            goods_receipts: vec![create_test_gr()],
            vendor_invoice: Some(create_test_vendor_invoice()),
            payment: None,
            remainder_payments: Vec::new(),
            is_complete: false,
            three_way_match_passed: false,
            payment_timing: None,
        };

        let entries = generator.generate_from_p2p_chain(&chain);

        for entry in &entries {
            for line in &entry.lines {
                assert!(
                    line.lettrage.is_none(),
                    "Incomplete chain should have no lettrage"
                );
            }
        }
    }

    #[test]
    fn test_default_config_no_fec_fields() {
        // Default config (non-French) should leave all FEC fields as None
        let mut generator = DocumentFlowJeGenerator::new();

        let invoice = create_test_vendor_invoice();
        let je = generator.generate_from_vendor_invoice(&invoice).unwrap();

        for line in &je.lines {
            assert!(line.auxiliary_account_number.is_none());
            assert!(line.auxiliary_account_label.is_none());
            assert!(line.lettrage.is_none());
            assert!(line.lettrage_date.is_none());
        }
    }

    #[test]
    fn test_auxiliary_lookup_uses_gl_account_instead_of_partner_id() {
        // When auxiliary lookup is populated, FEC auxiliary_account_number should
        // use the framework-specific GL account instead of the raw partner ID.
        let mut generator =
            DocumentFlowJeGenerator::with_config_and_seed(DocumentFlowJeConfig::french_gaap(), 42);

        let mut lookup = HashMap::new();
        lookup.insert("V-001".to_string(), "4010001".to_string());
        generator.set_auxiliary_account_lookup(lookup);

        let invoice = create_test_vendor_invoice();
        let je = generator.generate_from_vendor_invoice(&invoice).unwrap();

        // AP line should use the auxiliary GL account from lookup, not "V-001"
        assert_eq!(
            je.lines[1].auxiliary_account_number.as_deref(),
            Some("4010001"),
            "AP line should use auxiliary GL account from lookup"
        );
        // Label should still be the partner ID (human-readable)
        assert_eq!(
            je.lines[1].auxiliary_account_label.as_deref(),
            Some("V-001"),
        );
    }

    #[test]
    fn test_auxiliary_lookup_fallback_to_partner_id() {
        // When the auxiliary lookup exists but doesn't contain the partner,
        // should fall back to raw partner ID.
        let mut generator =
            DocumentFlowJeGenerator::with_config_and_seed(DocumentFlowJeConfig::french_gaap(), 42);

        // Lookup has a different vendor, not V-001
        let mut lookup = HashMap::new();
        lookup.insert("V-999".to_string(), "4019999".to_string());
        generator.set_auxiliary_account_lookup(lookup);

        let invoice = create_test_vendor_invoice();
        let je = generator.generate_from_vendor_invoice(&invoice).unwrap();

        // V-001 not in lookup, so should fall back to raw partner ID
        assert_eq!(
            je.lines[1].auxiliary_account_number.as_deref(),
            Some("V-001"),
            "Should fall back to partner ID when not in lookup"
        );
    }

    #[test]
    fn test_auxiliary_lookup_works_for_customer_receipt() {
        // Verify the lookup also works for O2C AR receipt lines.
        let mut generator =
            DocumentFlowJeGenerator::with_config_and_seed(DocumentFlowJeConfig::french_gaap(), 42);

        let mut lookup = HashMap::new();
        lookup.insert("C-001".to_string(), "4110001".to_string());
        generator.set_auxiliary_account_lookup(lookup);

        let mut receipt = Payment::new_ar_receipt(
            "RCP-001".to_string(),
            "1000",
            "C-001",
            Decimal::from(3000),
            2024,
            3,
            NaiveDate::from_ymd_opt(2024, 3, 15).unwrap(),
            "JSMITH",
        );
        receipt.post("JSMITH", NaiveDate::from_ymd_opt(2024, 3, 15).unwrap());

        let je = generator.generate_from_ar_receipt(&receipt).unwrap();

        // AR line (line 2 = CR AR) should use the auxiliary GL account from lookup
        assert_eq!(
            je.lines[1].auxiliary_account_number.as_deref(),
            Some("4110001"),
            "AR line should use auxiliary GL account from lookup"
        );
    }

    // ====================================================================
    // VAT / tax splitting tests
    // ====================================================================

    /// Helper: create a customer invoice with tax on its line items.
    fn create_test_customer_invoice_with_tax() -> CustomerInvoice {
        use datasynth_core::models::documents::CustomerInvoiceItem;

        let mut invoice = CustomerInvoice::new(
            "CI-001",
            "1000",
            "C-001",
            2024,
            1,
            NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
            NaiveDate::from_ymd_opt(2024, 2, 14).unwrap(),
            "JSMITH",
        );

        // 10 units * 100 = 1000 net, 100 tax => 1100 gross
        let mut item =
            CustomerInvoiceItem::new(1, "Product A", Decimal::from(10), Decimal::from(100));
        item.base.tax_amount = Decimal::from(100);
        invoice.add_item(item);
        invoice.post("JSMITH", NaiveDate::from_ymd_opt(2024, 1, 15).unwrap());

        invoice
    }

    /// Helper: create a customer invoice without any tax.
    fn create_test_customer_invoice_no_tax() -> CustomerInvoice {
        use datasynth_core::models::documents::CustomerInvoiceItem;

        let mut invoice = CustomerInvoice::new(
            "CI-002",
            "1000",
            "C-002",
            2024,
            1,
            NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
            NaiveDate::from_ymd_opt(2024, 2, 14).unwrap(),
            "JSMITH",
        );

        let item = CustomerInvoiceItem::new(1, "Product B", Decimal::from(10), Decimal::from(100));
        invoice.add_item(item);
        invoice.post("JSMITH", NaiveDate::from_ymd_opt(2024, 1, 15).unwrap());

        invoice
    }

    /// Helper: create a vendor invoice with tax on its line items.
    fn create_test_vendor_invoice_with_tax() -> VendorInvoice {
        use datasynth_core::models::documents::VendorInvoiceItem;

        let mut invoice = VendorInvoice::new(
            "VI-002".to_string(),
            "1000",
            "V-001",
            "INV-TAX-001".to_string(),
            2024,
            1,
            NaiveDate::from_ymd_opt(2024, 1, 20).unwrap(),
            "JSMITH",
        );

        // 100 qty * 50 price = 5000 net, 500 tax => 5500 gross = payable
        let item = VendorInvoiceItem::from_po_gr(
            10,
            "Test Material",
            Decimal::from(100),
            Decimal::from(50),
            "PO-001",
            10,
            Some("GR-001".to_string()),
            Some(10),
        )
        .with_tax("VAT10", Decimal::from(500));

        invoice.add_item(item);
        invoice.post("JSMITH", NaiveDate::from_ymd_opt(2024, 1, 20).unwrap());

        invoice
    }

    #[test]
    fn test_customer_invoice_with_tax_produces_three_lines() {
        let mut generator = DocumentFlowJeGenerator::new();
        let invoice = create_test_customer_invoice_with_tax();

        assert_eq!(invoice.total_net_amount, Decimal::from(1000));
        assert_eq!(invoice.total_tax_amount, Decimal::from(100));
        assert_eq!(invoice.total_gross_amount, Decimal::from(1100));

        let je = generator.generate_from_customer_invoice(&invoice).unwrap();

        // Should have 3 lines: DR AR, CR Revenue, CR VAT
        assert_eq!(
            je.line_count(),
            3,
            "Expected 3 JE lines for invoice with tax"
        );
        assert!(je.is_balanced(), "Entry must be balanced");

        // Line 1: DR AR = gross (1100)
        assert_eq!(je.lines[0].gl_account, control_accounts::AR_CONTROL);
        assert_eq!(je.lines[0].debit_amount, Decimal::from(1100));
        assert_eq!(je.lines[0].credit_amount, Decimal::ZERO);

        // Line 2: CR Revenue = net (1000)
        assert_eq!(je.lines[1].gl_account, revenue_accounts::PRODUCT_REVENUE);
        assert_eq!(je.lines[1].credit_amount, Decimal::from(1000));
        assert_eq!(je.lines[1].debit_amount, Decimal::ZERO);

        // Line 3: CR VAT Payable = tax (100)
        assert_eq!(je.lines[2].gl_account, tax_accounts::VAT_PAYABLE);
        assert_eq!(je.lines[2].credit_amount, Decimal::from(100));
        assert_eq!(je.lines[2].debit_amount, Decimal::ZERO);
    }

    #[test]
    fn test_customer_invoice_no_tax_produces_two_lines() {
        let mut generator = DocumentFlowJeGenerator::new();
        let invoice = create_test_customer_invoice_no_tax();

        assert_eq!(invoice.total_tax_amount, Decimal::ZERO);
        assert_eq!(invoice.total_net_amount, Decimal::from(1000));
        assert_eq!(invoice.total_gross_amount, Decimal::from(1000));

        let je = generator.generate_from_customer_invoice(&invoice).unwrap();

        // Should have 2 lines (no VAT line)
        assert_eq!(
            je.line_count(),
            2,
            "Expected 2 JE lines for invoice without tax"
        );
        assert!(je.is_balanced(), "Entry must be balanced");

        // Line 1: DR AR = gross (1000)
        assert_eq!(je.lines[0].gl_account, control_accounts::AR_CONTROL);
        assert_eq!(je.lines[0].debit_amount, Decimal::from(1000));

        // Line 2: CR Revenue = gross (1000)  — same as gross when no tax
        assert_eq!(je.lines[1].gl_account, revenue_accounts::PRODUCT_REVENUE);
        assert_eq!(je.lines[1].credit_amount, Decimal::from(1000));
    }

    #[test]
    fn test_vendor_invoice_with_tax_produces_three_lines() {
        let mut generator = DocumentFlowJeGenerator::new();
        let invoice = create_test_vendor_invoice_with_tax();

        assert_eq!(invoice.net_amount, Decimal::from(5000));
        assert_eq!(invoice.tax_amount, Decimal::from(500));
        assert_eq!(invoice.gross_amount, Decimal::from(5500));
        assert_eq!(invoice.payable_amount, Decimal::from(5500));

        let je = generator.generate_from_vendor_invoice(&invoice).unwrap();

        // Should have 3 lines: DR GR/IR, DR Input VAT, CR AP
        assert_eq!(
            je.line_count(),
            3,
            "Expected 3 JE lines for vendor invoice with tax"
        );
        assert!(je.is_balanced(), "Entry must be balanced");

        // Line 1: DR GR/IR Clearing = net (5000)
        assert_eq!(je.lines[0].gl_account, control_accounts::GR_IR_CLEARING);
        assert_eq!(je.lines[0].debit_amount, Decimal::from(5000));
        assert_eq!(je.lines[0].credit_amount, Decimal::ZERO);

        // Line 2: DR Input VAT = tax (500)
        assert_eq!(je.lines[1].gl_account, tax_accounts::INPUT_VAT);
        assert_eq!(je.lines[1].debit_amount, Decimal::from(500));
        assert_eq!(je.lines[1].credit_amount, Decimal::ZERO);

        // Line 3: CR AP = gross (5500)
        assert_eq!(je.lines[2].gl_account, control_accounts::AP_CONTROL);
        assert_eq!(je.lines[2].credit_amount, Decimal::from(5500));
        assert_eq!(je.lines[2].debit_amount, Decimal::ZERO);
    }

    #[test]
    fn test_vendor_invoice_no_tax_produces_two_lines() {
        // The existing create_test_vendor_invoice() has no tax
        let mut generator = DocumentFlowJeGenerator::new();
        let invoice = create_test_vendor_invoice();

        assert_eq!(invoice.tax_amount, Decimal::ZERO);

        let je = generator.generate_from_vendor_invoice(&invoice).unwrap();

        // Should have 2 lines (unchanged behavior)
        assert_eq!(
            je.line_count(),
            2,
            "Expected 2 JE lines for vendor invoice without tax"
        );
        assert!(je.is_balanced(), "Entry must be balanced");

        // Line 1: DR GR/IR Clearing = payable
        assert_eq!(je.lines[0].gl_account, control_accounts::GR_IR_CLEARING);
        assert_eq!(je.lines[0].debit_amount, invoice.payable_amount);

        // Line 2: CR AP = payable
        assert_eq!(je.lines[1].gl_account, control_accounts::AP_CONTROL);
        assert_eq!(je.lines[1].credit_amount, invoice.payable_amount);
    }

    #[test]
    fn test_vat_accounts_configurable() {
        // Verify that VAT accounts can be customized via config
        let config = DocumentFlowJeConfig {
            vat_output_account: "2999".to_string(),
            vat_input_account: "1999".to_string(),
            ..Default::default()
        };

        let mut generator = DocumentFlowJeGenerator::with_config_and_seed(config, 42);

        // Customer invoice with tax
        let ci = create_test_customer_invoice_with_tax();
        let je = generator.generate_from_customer_invoice(&ci).unwrap();
        assert_eq!(
            je.lines[2].gl_account, "2999",
            "VAT output account should be configurable"
        );

        // Vendor invoice with tax
        let vi = create_test_vendor_invoice_with_tax();
        let je = generator.generate_from_vendor_invoice(&vi).unwrap();
        assert_eq!(
            je.lines[1].gl_account, "1999",
            "VAT input account should be configurable"
        );
    }

    #[test]
    fn test_vat_entries_from_framework_accounts() {
        // FrameworkAccounts should propagate VAT accounts into DocumentFlowJeConfig
        let fa = datasynth_core::FrameworkAccounts::us_gaap();
        let config = DocumentFlowJeConfig::from(&fa);

        assert_eq!(config.vat_output_account, tax_accounts::VAT_PAYABLE);
        assert_eq!(config.vat_input_account, tax_accounts::INPUT_VAT);

        let fa_fr = datasynth_core::FrameworkAccounts::french_gaap();
        let config_fr = DocumentFlowJeConfig::from(&fa_fr);

        assert_eq!(config_fr.vat_output_account, "445710");
        assert_eq!(config_fr.vat_input_account, "445660");
    }

    #[test]
    fn test_french_gaap_vat_accounts() {
        let config = DocumentFlowJeConfig::french_gaap();
        assert_eq!(config.vat_output_account, "445710"); // PCG OUTPUT_VAT
        assert_eq!(config.vat_input_account, "445660"); // PCG INPUT_VAT
    }

    #[test]
    fn test_vat_balanced_with_multiple_items() {
        // Multiple line items with different tax amounts must still balance
        use datasynth_core::models::documents::CustomerInvoiceItem;

        let mut invoice = CustomerInvoice::new(
            "CI-003",
            "1000",
            "C-003",
            2024,
            1,
            NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
            NaiveDate::from_ymd_opt(2024, 2, 14).unwrap(),
            "JSMITH",
        );

        // Item 1: 500 net, 50 tax
        let mut item1 = CustomerInvoiceItem::new(1, "A", Decimal::from(5), Decimal::from(100));
        item1.base.tax_amount = Decimal::from(50);
        invoice.add_item(item1);

        // Item 2: 300 net, 30 tax
        let mut item2 = CustomerInvoiceItem::new(2, "B", Decimal::from(3), Decimal::from(100));
        item2.base.tax_amount = Decimal::from(30);
        invoice.add_item(item2);

        invoice.post("JSMITH", NaiveDate::from_ymd_opt(2024, 1, 15).unwrap());

        // net=800, tax=80, gross=880
        assert_eq!(invoice.total_net_amount, Decimal::from(800));
        assert_eq!(invoice.total_tax_amount, Decimal::from(80));
        assert_eq!(invoice.total_gross_amount, Decimal::from(880));

        let mut generator = DocumentFlowJeGenerator::new();
        let je = generator.generate_from_customer_invoice(&invoice).unwrap();

        assert_eq!(je.line_count(), 3);
        assert!(je.is_balanced());
        assert_eq!(je.total_debit(), Decimal::from(880));
        assert_eq!(je.total_credit(), Decimal::from(880));
    }

    #[test]
    fn test_document_types_per_source_document() {
        let mut generator = DocumentFlowJeGenerator::new();

        let gr = create_test_gr();
        let invoice = create_test_vendor_invoice();
        let payment = create_test_payment();

        let gr_je = generator.generate_from_goods_receipt(&gr).unwrap();
        assert_eq!(
            gr_je.header.document_type, "WE",
            "Goods receipt should be WE"
        );

        let vi_je = generator.generate_from_vendor_invoice(&invoice).unwrap();
        assert_eq!(
            vi_je.header.document_type, "KR",
            "Vendor invoice should be KR"
        );

        let pay_je = generator.generate_from_ap_payment(&payment).unwrap();
        assert_eq!(pay_je.header.document_type, "KZ", "AP payment should be KZ");

        // Collect distinct document types
        let types: std::collections::HashSet<&str> = [
            gr_je.header.document_type.as_str(),
            vi_je.header.document_type.as_str(),
            pay_je.header.document_type.as_str(),
        ]
        .into_iter()
        .collect();

        assert!(
            types.len() >= 3,
            "Expected at least 3 distinct document types from P2P flow, got {:?}",
            types,
        );
    }

    #[test]
    fn test_enrichment_account_descriptions_populated() {
        let mut generator = DocumentFlowJeGenerator::new();
        let gr = create_test_gr();
        let invoice = create_test_vendor_invoice();
        let payment = create_test_payment();

        let gr_je = generator.generate_from_goods_receipt(&gr).unwrap();
        let vi_je = generator.generate_from_vendor_invoice(&invoice).unwrap();
        let pay_je = generator.generate_from_ap_payment(&payment).unwrap();

        // All lines in all JEs should have account descriptions
        for je in [&gr_je, &vi_je, &pay_je] {
            for line in &je.lines {
                assert!(
                    line.account_description.is_some(),
                    "Line for account {} should have description, entry doc {}",
                    line.gl_account,
                    je.header.document_id,
                );
            }
        }

        // GR JE: Inventory and GR/IR Clearing
        assert_eq!(
            gr_je.lines[0].account_description.as_deref(),
            Some("Inventory"),
        );
        assert_eq!(
            gr_je.lines[1].account_description.as_deref(),
            Some("GR/IR Clearing"),
        );
    }

    #[test]
    fn test_enrichment_profit_center_and_line_text() {
        let mut generator = DocumentFlowJeGenerator::new();
        let gr = create_test_gr();

        let je = generator.generate_from_goods_receipt(&gr).unwrap();

        for line in &je.lines {
            // All lines should have profit_center
            assert!(
                line.profit_center.is_some(),
                "Line {} should have profit_center",
                line.gl_account,
            );
            let pc = line.profit_center.as_ref().unwrap();
            assert!(
                pc.starts_with("PC-"),
                "Profit center should start with PC-, got {}",
                pc,
            );

            // All lines should have line_text (from header fallback)
            assert!(
                line.line_text.is_some(),
                "Line {} should have line_text",
                line.gl_account,
            );
        }
    }

    #[test]
    fn test_enrichment_cost_center_for_expense_accounts() {
        let mut generator = DocumentFlowJeGenerator::new();

        // Create a delivery which produces COGS (5000) entries
        use datasynth_core::models::documents::{Delivery, DeliveryItem};
        let mut delivery = Delivery::new(
            "DEL-001".to_string(),
            "1000",
            "SO-001",
            "C-001",
            2024,
            1,
            NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
            "JSMITH",
        );
        let item = DeliveryItem::from_sales_order(
            10,
            "Test Material",
            Decimal::from(100),
            Decimal::from(50),
            "SO-001",
            10,
        );
        delivery.add_item(item);
        delivery.post_goods_issue("JSMITH", NaiveDate::from_ymd_opt(2024, 1, 15).unwrap());

        let je = generator.generate_from_delivery(&delivery).unwrap();

        // COGS line (5000) should have cost_center
        let cogs_line = je.lines.iter().find(|l| l.gl_account == "5000").unwrap();
        assert!(
            cogs_line.cost_center.is_some(),
            "COGS line should have cost_center assigned",
        );
        let cc = cogs_line.cost_center.as_ref().unwrap();
        assert!(
            cc.starts_with("CC"),
            "Cost center should start with CC, got {}",
            cc,
        );

        // Inventory line (1200) should NOT have cost_center
        let inv_line = je.lines.iter().find(|l| l.gl_account == "1200").unwrap();
        assert!(
            inv_line.cost_center.is_none(),
            "Non-expense line should not have cost_center",
        );
    }

    #[test]
    fn test_enrichment_value_date_for_ap_ar() {
        let mut generator = DocumentFlowJeGenerator::new();

        let invoice = create_test_vendor_invoice();
        let je = generator.generate_from_vendor_invoice(&invoice).unwrap();

        // AP line should have value_date
        let ap_line = je.lines.iter().find(|l| l.gl_account == "2000").unwrap();
        assert!(
            ap_line.value_date.is_some(),
            "AP line should have value_date set",
        );
        assert_eq!(ap_line.value_date, Some(je.header.posting_date));

        // GR/IR clearing line should NOT have value_date
        let clearing_line = je.lines.iter().find(|l| l.gl_account == "2900").unwrap();
        assert!(
            clearing_line.value_date.is_none(),
            "Non-AP/AR line should not have value_date",
        );
    }

    // ====================================================================
    // SP3 T8 — P2P trading_partner propagation
    // ====================================================================

    /// Task 8: P2P JE lines that are directly vendor-linked carry
    /// `trading_partner = Some(vendor_id)`.
    #[test]
    fn p2p_generator_populates_trading_partner_for_vendor_lines() {
        let mut generator = DocumentFlowJeGenerator::new();

        // GR: GR/IR clearing line (line 2) should carry the vendor ID
        let gr = create_test_gr();
        let gr_je = generator.generate_from_goods_receipt(&gr).unwrap();
        let gr_ir_line = gr_je
            .lines
            .iter()
            .find(|l| l.gl_account == control_accounts::GR_IR_CLEARING)
            .expect("GR/IR clearing line missing");
        assert_eq!(
            gr_ir_line.trading_partner.as_deref(),
            Some("V-001"),
            "GR/IR clearing line should carry vendor_id as trading_partner"
        );
        // Inventory DR line should NOT carry trading_partner
        let inv_line = gr_je
            .lines
            .iter()
            .find(|l| l.gl_account == control_accounts::INVENTORY)
            .expect("Inventory line missing");
        assert!(
            inv_line.trading_partner.is_none(),
            "Inventory DR line should not carry trading_partner"
        );

        // Vendor invoice: AP credit line should carry vendor_id; GR/IR debit should not
        let invoice = create_test_vendor_invoice();
        let vi_je = generator.generate_from_vendor_invoice(&invoice).unwrap();
        let ap_line = vi_je
            .lines
            .iter()
            .find(|l| l.gl_account == control_accounts::AP_CONTROL)
            .expect("AP line missing");
        assert_eq!(
            ap_line.trading_partner.as_deref(),
            Some("V-001"),
            "AP line should carry vendor_id as trading_partner"
        );
        let gr_ir_dr = vi_je
            .lines
            .iter()
            .find(|l| l.gl_account == control_accounts::GR_IR_CLEARING)
            .expect("GR/IR debit line missing");
        assert!(
            gr_ir_dr.trading_partner.is_none(),
            "GR/IR debit on vendor invoice should not carry trading_partner"
        );

        // AP payment: AP debit line should carry vendor_id; Cash CR should not
        let payment = create_test_payment();
        let pay_je = generator.generate_from_ap_payment(&payment).unwrap();
        let ap_dr = pay_je
            .lines
            .iter()
            .find(|l| l.gl_account == control_accounts::AP_CONTROL)
            .expect("AP debit line missing on payment");
        assert_eq!(
            ap_dr.trading_partner.as_deref(),
            Some("V-001"),
            "AP debit line on payment should carry vendor_id as trading_partner"
        );
        let cash_line = pay_je
            .lines
            .iter()
            .find(|l| l.gl_account == cash_accounts::OPERATING_CASH)
            .expect("Cash line missing on payment");
        assert!(
            cash_line.trading_partner.is_none(),
            "Cash CR line should not carry trading_partner"
        );
    }

    // ====================================================================
    // SP3 T9 — O2C trading_partner propagation
    // ====================================================================

    /// Task 9: O2C JE lines that are directly customer-linked carry
    /// `trading_partner = Some(customer_id)`.
    #[test]
    fn o2c_generator_populates_trading_partner_for_customer_lines() {
        let mut generator = DocumentFlowJeGenerator::new();

        // Customer invoice: AR DR line should carry customer_id; Revenue/VAT CRs should not
        let invoice = create_test_customer_invoice_with_tax();
        let ci_je = generator.generate_from_customer_invoice(&invoice).unwrap();
        let ar_line = ci_je
            .lines
            .iter()
            .find(|l| l.gl_account == control_accounts::AR_CONTROL)
            .expect("AR line missing");
        assert_eq!(
            ar_line.trading_partner.as_deref(),
            Some("C-001"),
            "AR line should carry customer_id as trading_partner"
        );
        let rev_line = ci_je
            .lines
            .iter()
            .find(|l| l.gl_account == revenue_accounts::PRODUCT_REVENUE)
            .expect("Revenue line missing");
        assert!(
            rev_line.trading_partner.is_none(),
            "Revenue CR line should not carry trading_partner"
        );
        let vat_line = ci_je
            .lines
            .iter()
            .find(|l| l.gl_account == tax_accounts::VAT_PAYABLE)
            .expect("VAT line missing");
        assert!(
            vat_line.trading_partner.is_none(),
            "VAT CR line should not carry trading_partner"
        );

        // AR receipt: AR CR line should carry customer_id; Cash DR should not
        let mut receipt = Payment::new_ar_receipt(
            "RCP-T9".to_string(),
            "1000",
            "C-001",
            Decimal::from(1100),
            2024,
            2,
            NaiveDate::from_ymd_opt(2024, 2, 28).unwrap(),
            "JSMITH",
        );
        receipt.post("JSMITH", NaiveDate::from_ymd_opt(2024, 2, 28).unwrap());

        let rcp_je = generator.generate_from_ar_receipt(&receipt).unwrap();
        let ar_cr = rcp_je
            .lines
            .iter()
            .find(|l| l.gl_account == control_accounts::AR_CONTROL)
            .expect("AR CR line missing on receipt");
        assert_eq!(
            ar_cr.trading_partner.as_deref(),
            Some("C-001"),
            "AR CR line on receipt should carry customer_id as trading_partner"
        );
        let cash_dr = rcp_je
            .lines
            .iter()
            .find(|l| l.gl_account == cash_accounts::OPERATING_CASH)
            .expect("Cash DR line missing on receipt");
        assert!(
            cash_dr.trading_partner.is_none(),
            "Cash DR line on receipt should not carry trading_partner"
        );

        // Delivery: COGS and Inventory lines should NOT carry trading_partner
        use datasynth_core::models::documents::{Delivery, DeliveryItem};
        let mut delivery = Delivery::new(
            "DEL-T9".to_string(),
            "1000",
            "SO-001",
            "C-001",
            2024,
            1,
            NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
            "JSMITH",
        );
        let item = DeliveryItem::from_sales_order(
            10,
            "Material X",
            Decimal::from(100),
            Decimal::from(50),
            "SO-001",
            10,
        );
        delivery.add_item(item);
        delivery.post_goods_issue("JSMITH", NaiveDate::from_ymd_opt(2024, 1, 15).unwrap());

        let del_je = generator.generate_from_delivery(&delivery).unwrap();
        for line in &del_je.lines {
            assert!(
                line.trading_partner.is_none(),
                "Delivery JE lines (COGS/Inventory) should not carry trading_partner; \
                 account {} has {:?}",
                line.gl_account,
                line.trading_partner,
            );
        }
    }

    /// SP3.13 W1.5 follow-up — geometric within-bucket draw concentrates the
    /// sampled value near the bucket's lower bound instead of drawing uniformly.
    ///
    /// For the [4, 9] bucket (lo=4, hi=9) with DECAY=0.5:
    /// - Expected: mean < 5.5 (vs. uniform mean 6.5)
    /// - Expected: ≥40 % of draws land exactly at the lower bound (4)
    #[test]
    fn sp3_13_w1_5_bucket_tightening_concentrates_target_near_lo() {
        use rand::SeedableRng;
        use rand_chacha::ChaCha8Rng;
        use std::collections::HashMap;

        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let mut counts: HashMap<u32, i32> = HashMap::new();
        for _ in 0..10_000 {
            let v = super::sample_within_bucket(4, 9, &mut rng);
            *counts.entry(v).or_insert(0) += 1;
        }

        let total: f64 = counts.values().sum::<i32>() as f64;
        let mean: f64 = counts
            .iter()
            .map(|(&k, &c)| k as f64 * (c as f64 / total))
            .sum();
        assert!(
            mean < 5.5,
            "geometric within-bucket draw should pull mean below 5.5 (uniform mean is 6.5), got {mean:.3}"
        );

        let count_lo = counts.get(&4).copied().unwrap_or(0);
        let lo_fraction = count_lo as f64 / total;
        assert!(
            lo_fraction > 0.40,
            "expected ≥40 % of draws at lower bound (4), got {lo_fraction:.3} ({count_lo}/10000)"
        );
    }
}
