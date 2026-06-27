//! Account-name dictionary for consolidated FS labelling — v5.1.
//!
//! In v5.0 each of [`super::balance_sheet::build_consolidated_balance_sheet`]
//! and [`super::income_statement::build_consolidated_income_statement`]
//! carried its own `OnceLock<BTreeMap<&'static str, &'static str>>` of
//! canonical code → label entries.  That worked but had two
//! shortcomings the v5.1 roadmap called out:
//!
//! 1. **No engagement override.**  A real audit engagement supplies its
//!    own chart-of-accounts master (e.g. SKR04 / PCG / a custom client
//!    chart) with localised account names ("Forderungen aus Lieferungen
//!    und Leistungen" instead of "Trade receivables").  The static
//!    English dict shadowed those even when the manifest carried them.
//! 2. **Two parallel copies.**  BS and IS each maintained their own
//!    static dict; codes that appear in both (rare, but possible)
//!    needed to be kept in sync by hand.
//!
//! [`AccountNameDictionary`] consolidates both concerns:
//!
//! - `Default::default()` returns the canonical built-in dict covering
//!   every code the v5.0 BS / IS / CF builders emit.  Any caller that
//!   doesn't have a CoA master (e.g. unit tests) gets backwards-
//!   compatible labelling for free.
//! - [`AccountNameDictionary::from_coa_master`] builds the dict from
//!   the engagement's [`ChartOfAccountsMaster`] for a specified
//!   framework, **falling back to the built-in canonical labels** for
//!   any code the master doesn't carry.  Engagement labels win;
//!   built-in labels fill the gaps; bare codes are the final fallback.
//!
//! The resulting `BTreeMap<String, String>` is plain owned data —
//! cheap to clone, trivial to thread through pure builder functions.

use std::collections::BTreeMap;

use datasynth_core::models::ChartOfAccounts;

use crate::manifest::ChartOfAccountsMaster;

/// Account-code → human-readable label dictionary used by the
/// consolidated FS builders.
///
/// See module-level docs for the lookup-fallback semantics.
#[derive(Debug, Clone)]
pub struct AccountNameDictionary {
    map: BTreeMap<String, String>,
}

impl AccountNameDictionary {
    /// Look up a label, falling back to the code itself if no entry
    /// exists.  Always succeeds.
    pub fn get(&self, code: &str) -> String {
        self.map
            .get(code)
            .cloned()
            .unwrap_or_else(|| code.to_string())
    }

    /// Build a dictionary from an engagement's
    /// [`ChartOfAccountsMaster`] for a given framework label.
    ///
    /// - If `framework` matches an entry in `master.frameworks`, every
    ///   `GLAccount` in that chart contributes a `(code, name)` pair.
    /// - Any canonical code not present in the engagement chart still
    ///   gets the built-in label (so a partial chart doesn't leave
    ///   labels blank).
    /// - When `framework` is missing from the master entirely, the
    ///   result is identical to [`Self::default()`].
    ///
    /// Engagement labels win over built-ins on collision.
    pub fn from_coa_master(master: &ChartOfAccountsMaster, framework: &str) -> Self {
        let mut map = builtin_canonical_map();
        if let Some(coa) = master.frameworks.get(framework) {
            extend_with_chart(&mut map, coa);
        }
        Self { map }
    }

    /// Build a dictionary from a single [`ChartOfAccounts`].  Useful in
    /// tests and when a caller already has the resolved chart at hand
    /// (e.g. the per-entity standalone-FS path).  Built-in canonical
    /// labels still fill gaps.
    pub fn from_chart(coa: &ChartOfAccounts) -> Self {
        let mut map = builtin_canonical_map();
        extend_with_chart(&mut map, coa);
        Self { map }
    }
}

impl Default for AccountNameDictionary {
    fn default() -> Self {
        Self {
            map: builtin_canonical_map(),
        }
    }
}

/// Add every account in `coa` to `map`, overwriting any pre-existing
/// entry for the same code.  Engagement-supplied labels always win.
///
/// Uses [`GLAccount::short_description`] as the label — long
/// descriptions can run to several sentences and are too verbose for
/// FS line labels.
fn extend_with_chart(map: &mut BTreeMap<String, String>, coa: &ChartOfAccounts) {
    for acct in &coa.accounts {
        map.insert(acct.account_number.clone(), acct.short_description.clone());
    }
}

/// The canonical built-in label dictionary — covers every code the
/// v5.0 consolidated BS / IS / CF builders emit.  Combines the prior
/// per-file static maps (in `balance_sheet.rs` and
/// `income_statement.rs`) into a single source of truth.
///
/// Entries kept lexicographic-sorted by code for diff-friendliness.
fn builtin_canonical_map() -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    // ── Cash / current assets ─────────────────────────────────────────
    m.insert("1000".to_string(), "Cash and cash equivalents".to_string());
    m.insert("1010".to_string(), "Bank account".to_string());
    m.insert("1020".to_string(), "Petty cash".to_string());
    m.insert("1100".to_string(), "Trade receivables".to_string());
    m.insert("1150".to_string(), "IC receivables".to_string());
    m.insert("1160".to_string(), "Input VAT".to_string());
    m.insert("1200".to_string(), "Inventory".to_string());
    m.insert("1300".to_string(), "Prepaid expenses".to_string());
    // ── Other current / non-current assets ────────────────────────────
    m.insert("1410".to_string(), "Finished goods".to_string());
    m.insert("1420".to_string(), "Work in process".to_string());
    m.insert("1450".to_string(), "Derivative asset".to_string());
    m.insert("1460".to_string(), "Tax receivable".to_string());
    m.insert(
        "1500".to_string(),
        "Property, plant & equipment".to_string(),
    );
    m.insert("1510".to_string(), "Accumulated depreciation".to_string());
    m.insert("1600".to_string(), "Deferred tax asset".to_string());
    m.insert(
        "1850".to_string(),
        "Investment in associates / JVs".to_string(),
    );
    m.insert("1900".to_string(), "Goodwill".to_string());
    m.insert("1910".to_string(), "Customer relationships".to_string());
    m.insert("1920".to_string(), "Trade name".to_string());
    m.insert("1930".to_string(), "Technology".to_string());
    m.insert("1950".to_string(), "Accumulated amortization".to_string());
    // ── Liabilities ───────────────────────────────────────────────────
    m.insert("2000".to_string(), "Trade payables".to_string());
    m.insert("2050".to_string(), "IC payables".to_string());
    m.insert("2100".to_string(), "Sales tax payable".to_string());
    m.insert("2110".to_string(), "VAT payable".to_string());
    m.insert("2130".to_string(), "Income tax payable".to_string());
    m.insert("2160".to_string(), "Interest payable".to_string());
    m.insert("2200".to_string(), "Accrued expenses".to_string());
    m.insert("2210".to_string(), "Accrued salaries".to_string());
    m.insert("2300".to_string(), "Unearned revenue".to_string());
    m.insert("2400".to_string(), "Short-term debt".to_string());
    m.insert("2450".to_string(), "Provision liability".to_string());
    m.insert("2460".to_string(), "Derivative liability".to_string());
    m.insert("2500".to_string(), "Deferred tax liability".to_string());
    m.insert("2600".to_string(), "Long-term debt".to_string());
    m.insert("2700".to_string(), "IC payable (long-term)".to_string());
    // ── Equity ────────────────────────────────────────────────────────
    m.insert("3000".to_string(), "Common stock".to_string());
    m.insert("3100".to_string(), "Additional paid-in capital".to_string());
    m.insert("3200".to_string(), "Retained earnings (legacy)".to_string());
    m.insert("3300".to_string(), "Retained earnings".to_string());
    // 3400 (equity-method bridge) was retired in v5.1 — the
    // post_elim overlay now posts the equity-method counterparty
    // side directly to retained earnings (3300).  No entry is needed
    // here; if a v5.0 archive happens to carry a 3400 line the code
    // falls back to the bare account number.
    // ── NCI / OCI ─────────────────────────────────────────────────────
    m.insert("3500".to_string(), "Non-controlling interest".to_string());
    m.insert(
        "3510".to_string(),
        "OCI — cash flow hedge reserve".to_string(),
    );
    // ── Revenue ───────────────────────────────────────────────────────
    m.insert("4000".to_string(), "Product revenue".to_string());
    m.insert("4010".to_string(), "Sales discounts".to_string());
    m.insert(
        "4020".to_string(),
        "Sales returns and allowances".to_string(),
    );
    m.insert("4100".to_string(), "Service revenue".to_string());
    m.insert("4500".to_string(), "IC revenue".to_string());
    m.insert("4800".to_string(), "Purchase discount income".to_string());
    m.insert("4850".to_string(), "Bargain purchase gain".to_string());
    m.insert(
        "4900".to_string(),
        "Share of profit of associates".to_string(),
    );
    // ── Cost of sales / operating expenses ────────────────────────────
    m.insert("5000".to_string(), "Cost of goods sold".to_string());
    m.insert("5100".to_string(), "Raw materials".to_string());
    m.insert("5200".to_string(), "Direct labor".to_string());
    m.insert("5300".to_string(), "Manufacturing overhead".to_string());
    m.insert("6000".to_string(), "Depreciation expense".to_string());
    m.insert("6010".to_string(), "Amortization expense".to_string());
    m.insert("6100".to_string(), "Salaries and wages".to_string());
    m.insert("6200".to_string(), "Benefits".to_string());
    m.insert("6300".to_string(), "Rent".to_string());
    m.insert("6400".to_string(), "Utilities".to_string());
    m.insert("6500".to_string(), "Office supplies".to_string());
    m.insert("6600".to_string(), "Travel and entertainment".to_string());
    m.insert("6700".to_string(), "Professional fees".to_string());
    m.insert("6800".to_string(), "Insurance".to_string());
    m.insert("6850".to_string(), "Provision expense".to_string());
    m.insert("6900".to_string(), "Bad debt expense".to_string());
    // ── Below the line ────────────────────────────────────────────────
    m.insert("7100".to_string(), "Depreciation expense".to_string());
    m.insert("7150".to_string(), "Interest expense".to_string());
    m.insert("7400".to_string(), "Purchase discounts".to_string());
    m.insert("7500".to_string(), "FX gain/loss".to_string());
    m.insert("7510".to_string(), "Hedge ineffectiveness".to_string());
    m.insert("8000".to_string(), "Tax expense".to_string());
    m.insert("8100".to_string(), "Deferred tax expense".to_string());
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use datasynth_core::models::{
        AccountSubType, AccountType, CoAComplexity, GLAccount, IndustrySector,
    };

    fn coa_with(code_name_pairs: &[(&str, &str)]) -> ChartOfAccounts {
        let mut coa = ChartOfAccounts::new(
            "TEST_COA".to_string(),
            "Test chart".to_string(),
            "DE".to_string(),
            IndustrySector::Manufacturing,
            CoAComplexity::Small,
        );
        for (code, name) in code_name_pairs {
            coa.add_account(GLAccount::new(
                code.to_string(),
                name.to_string(),
                AccountType::Asset,
                AccountSubType::AccountsReceivable,
            ));
        }
        coa
    }

    #[test]
    fn default_resolves_canonical_codes() {
        let dict = AccountNameDictionary::default();
        assert_eq!(dict.get("1100"), "Trade receivables");
        assert_eq!(dict.get("4000"), "Product revenue");
        // Unknown code falls through to the code itself.
        assert_eq!(dict.get("9999"), "9999");
    }

    #[test]
    fn from_chart_overrides_canonical_labels() {
        // Engagement chart provides a localised label for 1100.
        let coa = coa_with(&[("1100", "Forderungen aus Lieferungen und Leistungen")]);
        let dict = AccountNameDictionary::from_chart(&coa);
        assert_eq!(
            dict.get("1100"),
            "Forderungen aus Lieferungen und Leistungen",
            "engagement label must win over built-in canonical label"
        );
        // Codes not in the engagement chart still get the canonical label.
        assert_eq!(dict.get("4000"), "Product revenue");
    }

    #[test]
    fn empty_chart_falls_back_to_canonical() {
        let coa = coa_with(&[]);
        let dict = AccountNameDictionary::from_chart(&coa);
        // An empty engagement chart shouldn't strip the canonical labels.
        assert_eq!(dict.get("1100"), "Trade receivables");
    }

    #[test]
    fn missing_framework_falls_back_to_canonical() {
        let dict = AccountNameDictionary::from_coa_master(
            &ChartOfAccountsMaster {
                primary_framework: "us_gaap".to_string(),
                frameworks: BTreeMap::new(),
                coa_id: "TEST".to_string(),
            },
            "us_gaap",
        );
        assert_eq!(dict.get("1100"), "Trade receivables");
    }
}
