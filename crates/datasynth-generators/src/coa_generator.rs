//! Chart of Accounts generator.

use tracing::debug;

use datasynth_core::accounts::{
    asset_class_accounts, cash_accounts, control_accounts, dividend_accounts, dormant_accounts,
    equity_accounts, expense_accounts, intangible_accounts, inventory_accounts, liability_accounts,
    manufacturing_accounts, provision_accounts, revenue_accounts, suspense_accounts, tax_accounts,
    treasury_accounts,
};
use datasynth_core::models::*;
use datasynth_core::pcg_loader;
use datasynth_core::skr_loader;
use datasynth_core::traits::Generator;
use datasynth_core::utils::seeded_rng;
use rand_chacha::ChaCha8Rng;

/// Accounting framework for CoA generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CoAFramework {
    /// US GAAP (4-digit accounts)
    #[default]
    UsGaap,
    /// French GAAP / Plan Comptable Général (6-digit accounts)
    FrenchPcg,
    /// German GAAP / SKR04 (4-digit accounts)
    GermanSkr04,
}

/// Generator for Chart of Accounts.
pub struct ChartOfAccountsGenerator {
    rng: ChaCha8Rng,
    seed: u64,
    complexity: CoAComplexity,
    industry: IndustrySector,
    count: u64,
    /// Accounting framework for CoA generation.
    coa_framework: CoAFramework,
}

impl ChartOfAccountsGenerator {
    /// Create a new CoA generator.
    pub fn new(complexity: CoAComplexity, industry: IndustrySector, seed: u64) -> Self {
        Self {
            rng: seeded_rng(seed, 0),
            seed,
            complexity,
            industry,
            count: 0,
            coa_framework: CoAFramework::UsGaap,
        }
    }

    /// Use French GAAP (Plan Comptable Général) account structure.
    ///
    /// Deprecated: use `with_coa_framework(CoAFramework::FrenchPcg)` instead.
    pub fn with_french_pcg(mut self, use_pcg: bool) -> Self {
        if use_pcg {
            self.coa_framework = CoAFramework::FrenchPcg;
        }
        self
    }

    /// Set the accounting framework for CoA generation.
    pub fn with_coa_framework(mut self, framework: CoAFramework) -> Self {
        self.coa_framework = framework;
        self
    }

    /// Generate a complete chart of accounts.
    pub fn generate(&mut self) -> ChartOfAccounts {
        debug!(
            complexity = ?self.complexity,
            industry = ?self.industry,
            seed = self.seed,
            framework = ?self.coa_framework,
            "Generating chart of accounts"
        );

        self.count += 1;
        let mut coa = match self.coa_framework {
            CoAFramework::UsGaap => self.generate_default(),
            CoAFramework::FrenchPcg => self.generate_pcg(),
            CoAFramework::GermanSkr04 => self.generate_skr(),
        };

        // v5.0.1 fix (Gap 5): stamp `accounting_framework` on every emitted
        // GLAccount so the per-row column matches the sidecar
        // `coa_meta.accounting_framework`. Until this fix the field
        // defaulted to `None` even though the generator knew the framework.
        let framework_label = match self.coa_framework {
            CoAFramework::UsGaap => "us_gaap",
            CoAFramework::FrenchPcg => "french_pcg",
            CoAFramework::GermanSkr04 => "german_skr04",
        };
        for account in coa.accounts.iter_mut() {
            account.accounting_framework = Some(framework_label.to_string());
        }
        coa
    }

    /// Generate default (US-style) chart of accounts.
    fn generate_default(&mut self) -> ChartOfAccounts {
        let target_count = self.complexity.target_count();
        let mut coa = ChartOfAccounts::new(
            format!("COA_{:?}_{}", self.industry, self.complexity.target_count()),
            format!("{:?} Chart of Accounts", self.industry),
            "US".to_string(),
            self.industry,
            self.complexity,
        );

        // Seed canonical accounts first so other generators can find them
        Self::seed_canonical_accounts(&mut coa);

        // Generate additional accounts by type
        self.generate_asset_accounts(&mut coa, target_count / 5);
        self.generate_liability_accounts(&mut coa, target_count / 6);
        self.generate_equity_accounts(&mut coa, target_count / 10);
        self.generate_revenue_accounts(&mut coa, target_count / 5);
        self.generate_expense_accounts(&mut coa, target_count / 4);
        self.generate_suspense_accounts(&mut coa);
        coa
    }

    /// Generate Plan Comptable Général (French GAAP) chart of accounts.
    /// Uses the comprehensive PCG 2024 structure from [arrhes/PCG](https://github.com/arrhes/PCG) when available.
    fn generate_pcg(&mut self) -> ChartOfAccounts {
        match pcg_loader::build_chart_of_accounts_from_pcg_2024(self.complexity, self.industry) {
            Ok(coa) => coa,
            Err(_) => self.generate_pcg_fallback(),
        }
    }

    /// Generate SKR04 (German GAAP) chart of accounts.
    /// Uses the embedded SKR04 2024 structure when available.
    fn generate_skr(&mut self) -> ChartOfAccounts {
        match skr_loader::build_chart_of_accounts_from_skr04(self.complexity, self.industry) {
            Ok(coa) => coa,
            Err(_) => self.generate_skr_fallback(),
        }
    }

    /// Fallback simplified SKR04 when the embedded 2024 JSON cannot be loaded.
    fn generate_skr_fallback(&mut self) -> ChartOfAccounts {
        use datasynth_core::skr;
        let target_count = self.complexity.target_count();
        let mut coa = ChartOfAccounts::new(
            format!("COA_SKR04_{:?}_{}", self.industry, target_count),
            format!("Standardkontenrahmen 04 – {:?}", self.industry),
            "DE".to_string(),
            self.industry,
            self.complexity,
        );
        coa.account_format = "####".to_string();

        // Seed key SKR04 accounts
        let key_accounts = [
            (
                skr::control_accounts::AR_CONTROL,
                "Forderungen aus L+L",
                AccountType::Asset,
                AccountSubType::AccountsReceivable,
            ),
            (
                skr::control_accounts::AP_CONTROL,
                "Verbindlichkeiten aus L+L",
                AccountType::Liability,
                AccountSubType::AccountsPayable,
            ),
            (
                skr::control_accounts::INVENTORY,
                "Vorräte",
                AccountType::Asset,
                AccountSubType::Inventory,
            ),
            (
                skr::control_accounts::FIXED_ASSETS,
                "Sachanlagen",
                AccountType::Asset,
                AccountSubType::FixedAssets,
            ),
            (
                skr::cash_accounts::OPERATING_CASH,
                "Bank",
                AccountType::Asset,
                AccountSubType::Cash,
            ),
            (
                skr::cash_accounts::PETTY_CASH,
                "Kasse",
                AccountType::Asset,
                AccountSubType::Cash,
            ),
            (
                skr::equity_accounts::COMMON_STOCK,
                "Gezeichnetes Kapital",
                AccountType::Equity,
                AccountSubType::CommonStock,
            ),
            (
                skr::equity_accounts::RETAINED_EARNINGS,
                "Gewinnvortrag",
                AccountType::Equity,
                AccountSubType::RetainedEarnings,
            ),
            (
                skr::revenue_accounts::PRODUCT_REVENUE,
                "Umsatzerlöse",
                AccountType::Revenue,
                AccountSubType::ProductRevenue,
            ),
            (
                skr::revenue_accounts::SERVICE_REVENUE,
                "Erlöse Leistungen",
                AccountType::Revenue,
                AccountSubType::ServiceRevenue,
            ),
            (
                skr::expense_accounts::COGS,
                "Materialaufwand",
                AccountType::Expense,
                AccountSubType::CostOfGoodsSold,
            ),
            (
                skr::expense_accounts::SALARIES_WAGES,
                "Löhne und Gehälter",
                AccountType::Expense,
                AccountSubType::OperatingExpenses,
            ),
            (
                skr::expense_accounts::DEPRECIATION,
                "Abschreibungen",
                AccountType::Expense,
                AccountSubType::DepreciationExpense,
            ),
            (
                skr::expense_accounts::RENT,
                "Miete",
                AccountType::Expense,
                AccountSubType::OperatingExpenses,
            ),
        ];

        for (code, name, acc_type, sub_type) in key_accounts {
            let mut account =
                GLAccount::new(code.to_string(), name.to_string(), acc_type, sub_type);
            account.requires_cost_center = acc_type == AccountType::Expense;
            coa.add_account(account);
        }

        // Add additional accounts to reach target count
        let mut num = 4100u32;
        while coa.account_count() < target_count && num < 9900 {
            let code = format!("{num:04}");
            if coa.get_account(&code).is_none() {
                let class = (num / 1000) as u8;
                let (acc_type, sub_type) = match class {
                    0..=1 => (AccountType::Asset, AccountSubType::OtherAssets),
                    2 => (AccountType::Equity, AccountSubType::RetainedEarnings),
                    3 => (AccountType::Liability, AccountSubType::OtherLiabilities),
                    4 => (AccountType::Revenue, AccountSubType::OtherIncome),
                    5 => (AccountType::Expense, AccountSubType::CostOfGoodsSold),
                    6 => (AccountType::Expense, AccountSubType::OperatingExpenses),
                    7 => (AccountType::Expense, AccountSubType::InterestExpense),
                    _ => (AccountType::Asset, AccountSubType::SuspenseClearing),
                };
                coa.add_account(GLAccount::new(
                    code,
                    format!("Konto {num}"),
                    acc_type,
                    sub_type,
                ));
            }
            num += 10;
        }

        coa
    }

    /// Fallback simplified PCG when the embedded 2024 JSON cannot be loaded.
    fn generate_pcg_fallback(&mut self) -> ChartOfAccounts {
        let target_count = self.complexity.target_count();
        let mut coa = ChartOfAccounts::new(
            format!("COA_PCG_{:?}_{}", self.industry, target_count),
            format!("Plan Comptable Général – {:?}", self.industry),
            "FR".to_string(),
            self.industry,
            self.complexity,
        );
        coa.account_format = "######".to_string();

        self.generate_pcg_class_1(&mut coa, target_count / 10);
        self.generate_pcg_class_2(&mut coa, target_count / 6);
        self.generate_pcg_class_3(&mut coa, target_count / 8);
        self.generate_pcg_class_4(&mut coa, target_count / 5);
        self.generate_pcg_class_5(&mut coa, target_count / 12);
        self.generate_pcg_class_6(&mut coa, target_count / 4);
        self.generate_pcg_class_7(&mut coa, target_count / 5);
        self.generate_pcg_class_8(&mut coa);

        coa
    }

    fn generate_pcg_class_1(&mut self, coa: &mut ChartOfAccounts, count: usize) {
        let items = [
            (
                101,
                "Capital",
                AccountType::Equity,
                AccountSubType::CommonStock,
            ),
            (
                129,
                "Résultat",
                AccountType::Equity,
                AccountSubType::RetainedEarnings,
            ),
            (
                164,
                "Emprunts",
                AccountType::Liability,
                AccountSubType::LongTermDebt,
            ),
            (
                151,
                "Provisions pour risques",
                AccountType::Liability,
                AccountSubType::AccruedLiabilities,
            ),
        ];
        for (base, name, acc_type, sub_type) in items {
            for i in 0..count.max(1) {
                let num = base * 1000 + (i as u32 % 100);
                coa.add_account(GLAccount::new(
                    format!("{num:06}"),
                    format!("{} {}", name, i + 1),
                    acc_type,
                    sub_type,
                ));
            }
        }
    }

    fn generate_pcg_class_2(&mut self, coa: &mut ChartOfAccounts, count: usize) {
        for i in 0..count.max(1) {
            let num = 215000 + (i as u32 % 100);
            coa.add_account(GLAccount::new(
                format!("{num:06}"),
                format!("Immobilisations {}", i + 1),
                AccountType::Asset,
                AccountSubType::FixedAssets,
            ));
        }
        for i in 0..(count / 2).max(1) {
            let num = 281000 + (i as u32 % 100);
            coa.add_account(GLAccount::new(
                format!("{num:06}"),
                format!("Amortissements {}", i + 1),
                AccountType::Asset,
                AccountSubType::AccumulatedDepreciation,
            ));
        }
    }

    fn generate_pcg_class_3(&mut self, coa: &mut ChartOfAccounts, count: usize) {
        for i in 0..count.max(1) {
            let num = 310000 + (i as u32 % 1000);
            coa.add_account(GLAccount::new(
                format!("{num:06}"),
                format!("Stocks {}", i + 1),
                AccountType::Asset,
                AccountSubType::Inventory,
            ));
        }
    }

    fn generate_pcg_class_4(&mut self, coa: &mut ChartOfAccounts, count: usize) {
        for i in 0..count.max(1) {
            let num = 411000 + (i as u32 % 1000);
            coa.add_account(GLAccount::new(
                format!("{num:06}"),
                format!("Clients {}", i + 1),
                AccountType::Asset,
                AccountSubType::AccountsReceivable,
            ));
        }
        for i in 0..count.max(1) {
            let num = 401000 + (i as u32 % 1000);
            coa.add_account(GLAccount::new(
                format!("{num:06}"),
                format!("Fournisseurs {}", i + 1),
                AccountType::Liability,
                AccountSubType::AccountsPayable,
            ));
        }
        let clearing = GLAccount::new(
            "408000".to_string(),
            "Fournisseurs – non encore reçus".to_string(),
            AccountType::Liability,
            AccountSubType::GoodsReceivedClearing,
        );
        coa.add_account(clearing);
    }

    fn generate_pcg_class_5(&mut self, coa: &mut ChartOfAccounts, count: usize) {
        let bases = [
            (512, "Banque"),
            (530, "Caisse"),
            (511, "Valeurs à l'encaissement"),
        ];
        for (base, name) in bases {
            for i in 0..(count / 3).max(1) {
                let num = base * 1000 + (i as u32 % 100);
                coa.add_account(GLAccount::new(
                    format!("{num:06}"),
                    format!("{} {}", name, i + 1),
                    AccountType::Asset,
                    AccountSubType::Cash,
                ));
            }
        }
    }

    fn generate_pcg_class_6(&mut self, coa: &mut ChartOfAccounts, count: usize) {
        let bases = [
            (603, "Achats"),
            (641, "Rémunérations"),
            (681, "DAP"),
            (613, "Loyers"),
            (661, "Charges financières"),
        ];
        for (base, name) in bases {
            for i in 0..(count / 5).max(1) {
                let num = base * 1000 + (i as u32 % 100);
                let mut account = GLAccount::new(
                    format!("{num:06}"),
                    format!("{} {}", name, i + 1),
                    AccountType::Expense,
                    AccountSubType::OperatingExpenses,
                );
                account.requires_cost_center = true;
                coa.add_account(account);
            }
        }
    }

    fn generate_pcg_class_7(&mut self, coa: &mut ChartOfAccounts, count: usize) {
        let bases = [
            (701, "Ventes"),
            (706, "Prestations"),
            (758, "Produits divers"),
        ];
        for (base, name) in bases {
            for i in 0..(count / 3).max(1) {
                let num = base * 1000 + (i as u32 % 100);
                coa.add_account(GLAccount::new(
                    format!("{num:06}"),
                    format!("{} {}", name, i + 1),
                    AccountType::Revenue,
                    AccountSubType::ProductRevenue,
                ));
            }
        }
    }

    fn generate_pcg_class_8(&mut self, coa: &mut ChartOfAccounts) {
        coa.add_account(GLAccount::new(
            "808000".to_string(),
            "Comptes spéciaux".to_string(),
            AccountType::Asset,
            AccountSubType::SuspenseClearing,
        ));
    }

    /// Insert all canonical accounts from `datasynth_core::accounts` into the CoA.
    ///
    /// These are the well-known account numbers (4-digit, 1000-9300 range) that
    /// other generators reference. They are added before auto-generated accounts
    /// (which start at 100000+) so there are no collisions.
    fn seed_canonical_accounts(coa: &mut ChartOfAccounts) {
        // --- Cash accounts (1000-series, Asset / Cash) ---
        coa.add_account(GLAccount::new(
            cash_accounts::OPERATING_CASH.to_string(),
            "Operating Cash".to_string(),
            AccountType::Asset,
            AccountSubType::Cash,
        ));
        coa.add_account(GLAccount::new(
            cash_accounts::BANK_ACCOUNT.to_string(),
            "Bank Account".to_string(),
            AccountType::Asset,
            AccountSubType::Cash,
        ));
        coa.add_account(GLAccount::new(
            cash_accounts::PETTY_CASH.to_string(),
            "Petty Cash".to_string(),
            AccountType::Asset,
            AccountSubType::Cash,
        ));
        coa.add_account(GLAccount::new(
            cash_accounts::WIRE_CLEARING.to_string(),
            "Wire Transfer Clearing".to_string(),
            AccountType::Asset,
            AccountSubType::BankClearing,
        ));

        // --- Control accounts (Asset side) ---
        {
            let mut acct = GLAccount::new(
                control_accounts::AR_CONTROL.to_string(),
                "Accounts Receivable Control".to_string(),
                AccountType::Asset,
                AccountSubType::AccountsReceivable,
            );
            acct.is_control_account = true;
            coa.add_account(acct);
        }
        {
            let mut acct = GLAccount::new(
                control_accounts::IC_AR_CLEARING.to_string(),
                "Intercompany AR Clearing".to_string(),
                AccountType::Asset,
                AccountSubType::AccountsReceivable,
            );
            acct.is_control_account = true;
            coa.add_account(acct);
        }
        coa.add_account(GLAccount::new(
            control_accounts::INVENTORY.to_string(),
            "Inventory".to_string(),
            AccountType::Asset,
            AccountSubType::Inventory,
        ));
        coa.add_account(GLAccount::new(
            control_accounts::FIXED_ASSETS.to_string(),
            "Fixed Assets".to_string(),
            AccountType::Asset,
            AccountSubType::FixedAssets,
        ));
        coa.add_account(GLAccount::new(
            control_accounts::ACCUMULATED_DEPRECIATION.to_string(),
            "Accumulated Depreciation".to_string(),
            AccountType::Asset,
            AccountSubType::AccumulatedDepreciation,
        ));

        // --- Tax asset accounts ---
        coa.add_account(GLAccount::new(
            tax_accounts::INPUT_VAT.to_string(),
            "Input VAT".to_string(),
            AccountType::Asset,
            AccountSubType::OtherReceivables,
        ));
        coa.add_account(GLAccount::new(
            tax_accounts::DEFERRED_TAX_ASSET.to_string(),
            "Deferred Tax Asset".to_string(),
            AccountType::Asset,
            AccountSubType::OtherAssets,
        ));

        // --- Liability / Control accounts (2000-series) ---
        {
            let mut acct = GLAccount::new(
                control_accounts::AP_CONTROL.to_string(),
                "Accounts Payable Control".to_string(),
                AccountType::Liability,
                AccountSubType::AccountsPayable,
            );
            acct.is_control_account = true;
            coa.add_account(acct);
        }
        {
            let mut acct = GLAccount::new(
                control_accounts::IC_AP_CLEARING.to_string(),
                "Intercompany AP Clearing".to_string(),
                AccountType::Liability,
                AccountSubType::AccountsPayable,
            );
            acct.is_control_account = true;
            coa.add_account(acct);
        }
        coa.add_account(GLAccount::new(
            tax_accounts::SALES_TAX_PAYABLE.to_string(),
            "Sales Tax Payable".to_string(),
            AccountType::Liability,
            AccountSubType::TaxLiabilities,
        ));
        coa.add_account(GLAccount::new(
            tax_accounts::VAT_PAYABLE.to_string(),
            "VAT Payable".to_string(),
            AccountType::Liability,
            AccountSubType::TaxLiabilities,
        ));
        coa.add_account(GLAccount::new(
            tax_accounts::WITHHOLDING_TAX_PAYABLE.to_string(),
            "Withholding Tax Payable".to_string(),
            AccountType::Liability,
            AccountSubType::TaxLiabilities,
        ));
        coa.add_account(GLAccount::new(
            liability_accounts::ACCRUED_EXPENSES.to_string(),
            "Accrued Expenses".to_string(),
            AccountType::Liability,
            AccountSubType::AccruedLiabilities,
        ));
        coa.add_account(GLAccount::new(
            liability_accounts::ACCRUED_SALARIES.to_string(),
            "Accrued Salaries".to_string(),
            AccountType::Liability,
            AccountSubType::AccruedLiabilities,
        ));
        coa.add_account(GLAccount::new(
            liability_accounts::ACCRUED_BENEFITS.to_string(),
            "Accrued Benefits".to_string(),
            AccountType::Liability,
            AccountSubType::AccruedLiabilities,
        ));
        coa.add_account(GLAccount::new(
            liability_accounts::UNEARNED_REVENUE.to_string(),
            "Unearned Revenue".to_string(),
            AccountType::Liability,
            AccountSubType::DeferredRevenue,
        ));
        coa.add_account(GLAccount::new(
            liability_accounts::SHORT_TERM_DEBT.to_string(),
            "Short-Term Debt".to_string(),
            AccountType::Liability,
            AccountSubType::ShortTermDebt,
        ));
        coa.add_account(GLAccount::new(
            tax_accounts::DEFERRED_TAX_LIABILITY.to_string(),
            "Deferred Tax Liability".to_string(),
            AccountType::Liability,
            AccountSubType::TaxLiabilities,
        ));
        coa.add_account(GLAccount::new(
            liability_accounts::LONG_TERM_DEBT.to_string(),
            "Long-Term Debt".to_string(),
            AccountType::Liability,
            AccountSubType::LongTermDebt,
        ));
        coa.add_account(GLAccount::new(
            liability_accounts::IC_PAYABLE.to_string(),
            "Intercompany Payable".to_string(),
            AccountType::Liability,
            AccountSubType::OtherLiabilities,
        ));
        {
            let mut acct = GLAccount::new(
                control_accounts::GR_IR_CLEARING.to_string(),
                "GR/IR Clearing".to_string(),
                AccountType::Liability,
                AccountSubType::GoodsReceivedClearing,
            );
            acct.is_suspense_account = true;
            coa.add_account(acct);
        }

        // --- Equity accounts (3000-series) ---
        coa.add_account(GLAccount::new(
            equity_accounts::COMMON_STOCK.to_string(),
            "Common Stock".to_string(),
            AccountType::Equity,
            AccountSubType::CommonStock,
        ));
        coa.add_account(GLAccount::new(
            equity_accounts::APIC.to_string(),
            "Additional Paid-In Capital".to_string(),
            AccountType::Equity,
            AccountSubType::AdditionalPaidInCapital,
        ));
        coa.add_account(GLAccount::new(
            equity_accounts::RETAINED_EARNINGS.to_string(),
            "Retained Earnings".to_string(),
            AccountType::Equity,
            AccountSubType::RetainedEarnings,
        ));
        coa.add_account(GLAccount::new(
            equity_accounts::CURRENT_YEAR_EARNINGS.to_string(),
            "Current Year Earnings".to_string(),
            AccountType::Equity,
            AccountSubType::NetIncome,
        ));
        coa.add_account(GLAccount::new(
            equity_accounts::TREASURY_STOCK.to_string(),
            "Treasury Stock".to_string(),
            AccountType::Equity,
            AccountSubType::TreasuryStock,
        ));
        coa.add_account(GLAccount::new(
            equity_accounts::CTA.to_string(),
            "Currency Translation Adjustment".to_string(),
            AccountType::Equity,
            AccountSubType::OtherComprehensiveIncome,
        ));

        // --- Revenue accounts (4000-series) ---
        coa.add_account(GLAccount::new(
            revenue_accounts::PRODUCT_REVENUE.to_string(),
            "Product Revenue".to_string(),
            AccountType::Revenue,
            AccountSubType::ProductRevenue,
        ));
        coa.add_account(GLAccount::new(
            revenue_accounts::SALES_DISCOUNTS.to_string(),
            "Sales Discounts".to_string(),
            AccountType::Revenue,
            AccountSubType::ProductRevenue,
        ));
        coa.add_account(GLAccount::new(
            revenue_accounts::SALES_RETURNS.to_string(),
            "Sales Returns and Allowances".to_string(),
            AccountType::Revenue,
            AccountSubType::ProductRevenue,
        ));
        coa.add_account(GLAccount::new(
            revenue_accounts::SERVICE_REVENUE.to_string(),
            "Service Revenue".to_string(),
            AccountType::Revenue,
            AccountSubType::ServiceRevenue,
        ));
        coa.add_account(GLAccount::new(
            revenue_accounts::IC_REVENUE.to_string(),
            "Intercompany Revenue".to_string(),
            AccountType::Revenue,
            AccountSubType::OtherIncome,
        ));
        coa.add_account(GLAccount::new(
            revenue_accounts::OTHER_REVENUE.to_string(),
            "Other Revenue".to_string(),
            AccountType::Revenue,
            AccountSubType::OtherIncome,
        ));

        // --- Expense accounts (5000-7xxx series) ---
        {
            let mut acct = GLAccount::new(
                expense_accounts::COGS.to_string(),
                "Cost of Goods Sold".to_string(),
                AccountType::Expense,
                AccountSubType::CostOfGoodsSold,
            );
            acct.requires_cost_center = true;
            coa.add_account(acct);
        }
        {
            let mut acct = GLAccount::new(
                expense_accounts::RAW_MATERIALS.to_string(),
                "Raw Materials".to_string(),
                AccountType::Expense,
                AccountSubType::CostOfGoodsSold,
            );
            acct.requires_cost_center = true;
            coa.add_account(acct);
        }
        {
            let mut acct = GLAccount::new(
                expense_accounts::DIRECT_LABOR.to_string(),
                "Direct Labor".to_string(),
                AccountType::Expense,
                AccountSubType::CostOfGoodsSold,
            );
            acct.requires_cost_center = true;
            coa.add_account(acct);
        }
        {
            let mut acct = GLAccount::new(
                expense_accounts::MANUFACTURING_OVERHEAD.to_string(),
                "Manufacturing Overhead".to_string(),
                AccountType::Expense,
                AccountSubType::CostOfGoodsSold,
            );
            acct.requires_cost_center = true;
            coa.add_account(acct);
        }
        {
            let mut acct = GLAccount::new(
                expense_accounts::DEPRECIATION.to_string(),
                "Depreciation Expense".to_string(),
                AccountType::Expense,
                AccountSubType::DepreciationExpense,
            );
            acct.requires_cost_center = true;
            coa.add_account(acct);
        }
        {
            let mut acct = GLAccount::new(
                expense_accounts::SALARIES_WAGES.to_string(),
                "Salaries and Wages".to_string(),
                AccountType::Expense,
                AccountSubType::OperatingExpenses,
            );
            acct.requires_cost_center = true;
            coa.add_account(acct);
        }
        {
            let mut acct = GLAccount::new(
                expense_accounts::BENEFITS.to_string(),
                "Benefits Expense".to_string(),
                AccountType::Expense,
                AccountSubType::OperatingExpenses,
            );
            acct.requires_cost_center = true;
            coa.add_account(acct);
        }
        {
            let mut acct = GLAccount::new(
                expense_accounts::RENT.to_string(),
                "Rent Expense".to_string(),
                AccountType::Expense,
                AccountSubType::OperatingExpenses,
            );
            acct.requires_cost_center = true;
            coa.add_account(acct);
        }
        {
            let mut acct = GLAccount::new(
                expense_accounts::UTILITIES.to_string(),
                "Utilities Expense".to_string(),
                AccountType::Expense,
                AccountSubType::OperatingExpenses,
            );
            acct.requires_cost_center = true;
            coa.add_account(acct);
        }
        {
            let mut acct = GLAccount::new(
                expense_accounts::OFFICE_SUPPLIES.to_string(),
                "Office Supplies".to_string(),
                AccountType::Expense,
                AccountSubType::AdministrativeExpenses,
            );
            acct.requires_cost_center = true;
            coa.add_account(acct);
        }
        {
            let mut acct = GLAccount::new(
                expense_accounts::TRAVEL_ENTERTAINMENT.to_string(),
                "Travel and Entertainment".to_string(),
                AccountType::Expense,
                AccountSubType::SellingExpenses,
            );
            acct.requires_cost_center = true;
            coa.add_account(acct);
        }
        {
            let mut acct = GLAccount::new(
                expense_accounts::PROFESSIONAL_FEES.to_string(),
                "Professional Fees".to_string(),
                AccountType::Expense,
                AccountSubType::AdministrativeExpenses,
            );
            acct.requires_cost_center = true;
            coa.add_account(acct);
        }
        {
            let mut acct = GLAccount::new(
                expense_accounts::INSURANCE.to_string(),
                "Insurance Expense".to_string(),
                AccountType::Expense,
                AccountSubType::OperatingExpenses,
            );
            acct.requires_cost_center = true;
            coa.add_account(acct);
        }
        {
            let mut acct = GLAccount::new(
                expense_accounts::BAD_DEBT.to_string(),
                "Bad Debt Expense".to_string(),
                AccountType::Expense,
                AccountSubType::OperatingExpenses,
            );
            acct.requires_cost_center = true;
            coa.add_account(acct);
        }
        {
            let mut acct = GLAccount::new(
                expense_accounts::INTEREST_EXPENSE.to_string(),
                "Interest Expense".to_string(),
                AccountType::Expense,
                AccountSubType::InterestExpense,
            );
            acct.requires_cost_center = true;
            coa.add_account(acct);
        }
        {
            let mut acct = GLAccount::new(
                expense_accounts::PURCHASE_DISCOUNTS.to_string(),
                "Purchase Discounts".to_string(),
                AccountType::Expense,
                AccountSubType::OtherExpenses,
            );
            acct.requires_cost_center = true;
            coa.add_account(acct);
        }
        {
            let mut acct = GLAccount::new(
                expense_accounts::FX_GAIN_LOSS.to_string(),
                "FX Gain/Loss".to_string(),
                AccountType::Expense,
                AccountSubType::ForeignExchangeLoss,
            );
            acct.requires_cost_center = true;
            coa.add_account(acct);
        }

        // --- Tax expense (8000-series) ---
        {
            let mut acct = GLAccount::new(
                tax_accounts::TAX_EXPENSE.to_string(),
                "Tax Expense".to_string(),
                AccountType::Expense,
                AccountSubType::TaxExpense,
            );
            acct.requires_cost_center = true;
            coa.add_account(acct);
        }

        // --- Asset class accounts (FA subledger acquisition + acc. depreciation) ---
        Self::seed_asset_class_accounts(coa);

        // --- Manufacturing accounts (WIP, finished goods, variances, warranty) ---
        Self::seed_manufacturing_accounts(coa);

        // --- Intangible assets (goodwill, customer relationships, etc.) ---
        Self::seed_intangible_accounts(coa);

        // --- Treasury / hedging / debt accounts ---
        Self::seed_treasury_accounts(coa);

        // --- Provisions (IAS 37 / ASC 450) ---
        Self::seed_provision_accounts(coa);

        // --- Dividend accounts (declared / payable) ---
        Self::seed_dividend_accounts(coa);

        // --- Pension / share-based compensation ---
        Self::seed_compensation_accounts(coa);

        // --- Inventory subledger sub-accounts (write-up income / write-down expense) ---
        Self::seed_inventory_subledger_accounts(coa);

        // --- Tax accounts not seeded above (income tax payable, tax receivable) ---
        Self::seed_additional_tax_accounts(coa);

        // --- Equity accounts not seeded above (income summary, dividends paid) ---
        Self::seed_additional_equity_accounts(coa);

        // --- Dormant / blocked legacy accounts (anomaly-strategy targets) ---
        Self::seed_dormant_accounts(coa);

        // --- Suspense / Clearing accounts (9000-series) ---
        {
            let mut acct = GLAccount::new(
                suspense_accounts::GENERAL_SUSPENSE.to_string(),
                "General Suspense".to_string(),
                AccountType::Asset,
                AccountSubType::SuspenseClearing,
            );
            acct.is_suspense_account = true;
            coa.add_account(acct);
        }
        {
            let mut acct = GLAccount::new(
                suspense_accounts::PAYROLL_CLEARING.to_string(),
                "Payroll Clearing".to_string(),
                AccountType::Asset,
                AccountSubType::SuspenseClearing,
            );
            acct.is_suspense_account = true;
            coa.add_account(acct);
        }
        {
            let mut acct = GLAccount::new(
                suspense_accounts::BANK_RECONCILIATION_SUSPENSE.to_string(),
                "Bank Reconciliation Suspense".to_string(),
                AccountType::Asset,
                AccountSubType::BankClearing,
            );
            acct.is_suspense_account = true;
            coa.add_account(acct);
        }
        {
            let mut acct = GLAccount::new(
                suspense_accounts::IC_ELIMINATION_SUSPENSE.to_string(),
                "IC Elimination Suspense".to_string(),
                AccountType::Asset,
                AccountSubType::IntercompanyClearing,
            );
            acct.is_suspense_account = true;
            coa.add_account(acct);
        }
    }

    /// Seed asset-class acquisition + accumulated-depreciation contras +
    /// FA-subledger expense / disposal accounts so JEs from the fixed-asset
    /// generator always resolve.
    fn seed_asset_class_accounts(coa: &mut ChartOfAccounts) {
        let acquisitions = [
            (asset_class_accounts::LAND, "Land"),
            (asset_class_accounts::BUILDINGS, "Buildings"),
            (
                asset_class_accounts::BUILDING_IMPROVEMENTS,
                "Building Improvements",
            ),
            (
                asset_class_accounts::MACHINERY_EQUIPMENT,
                "Machinery & Equipment",
            ),
            (asset_class_accounts::VEHICLES, "Vehicles"),
            (asset_class_accounts::OFFICE_EQUIPMENT, "Office Equipment"),
            (asset_class_accounts::COMPUTER_HARDWARE, "Computer Hardware"),
            (
                asset_class_accounts::SOFTWARE_INTANGIBLES,
                "Software / Intangibles",
            ),
            (
                asset_class_accounts::FURNITURE_FIXTURES,
                "Furniture & Fixtures",
            ),
            (
                asset_class_accounts::LEASEHOLD_IMPROVEMENTS,
                "Leasehold Improvements",
            ),
            (asset_class_accounts::OTHER_ASSETS, "Other Fixed Assets"),
            (asset_class_accounts::LOW_VALUE_ASSETS, "Low-Value Assets"),
            (
                asset_class_accounts::CONSTRUCTION_IN_PROGRESS,
                "Construction in Progress",
            ),
        ];
        for (number, name) in acquisitions {
            coa.add_account(GLAccount::new(
                number.to_string(),
                name.to_string(),
                AccountType::Asset,
                AccountSubType::FixedAssets,
            ));
        }

        let depreciation_contras = [
            (asset_class_accounts::ACC_DEP_LAND, "Acc. Dep. — Land"),
            (
                asset_class_accounts::ACC_DEP_BUILDINGS,
                "Acc. Dep. — Buildings",
            ),
            (
                asset_class_accounts::ACC_DEP_MACHINERY,
                "Acc. Dep. — Machinery",
            ),
            (
                asset_class_accounts::ACC_DEP_VEHICLES,
                "Acc. Dep. — Vehicles",
            ),
            (
                asset_class_accounts::ACC_DEP_OFFICE_EQUIPMENT,
                "Acc. Dep. — Office Equipment",
            ),
            (
                asset_class_accounts::ACC_DEP_SOFTWARE,
                "Acc. Dep. — Software / Intangibles",
            ),
            (
                asset_class_accounts::ACC_DEP_FURNITURE,
                "Acc. Dep. — Furniture",
            ),
            (
                asset_class_accounts::ACC_DEP_LEASEHOLD,
                "Acc. Dep. — Leasehold Improvements",
            ),
            (asset_class_accounts::ACC_DEP_OTHER, "Acc. Dep. — Other"),
            (asset_class_accounts::ACC_DEP_CIP, "Acc. Dep. — CIP"),
        ];
        for (number, name) in depreciation_contras {
            coa.add_account(GLAccount::new(
                number.to_string(),
                name.to_string(),
                AccountType::Asset,
                AccountSubType::AccumulatedDepreciation,
            ));
        }

        // FA-subledger depreciation expense (7100) — distinct from the
        // GL-level DEPRECIATION (6000) which is also seeded.
        {
            let mut acct = GLAccount::new(
                asset_class_accounts::DEPRECIATION_EXPENSE.to_string(),
                "FA Depreciation Expense".to_string(),
                AccountType::Expense,
                AccountSubType::DepreciationExpense,
            );
            acct.requires_cost_center = true;
            coa.add_account(acct);
        }
        coa.add_account(GLAccount::new(
            asset_class_accounts::GAIN_ON_DISPOSAL.to_string(),
            "Gain on Disposal of Fixed Assets".to_string(),
            AccountType::Revenue,
            AccountSubType::GainOnSale,
        ));
        coa.add_account(GLAccount::new(
            asset_class_accounts::LOSS_ON_DISPOSAL.to_string(),
            "Loss on Disposal of Fixed Assets".to_string(),
            AccountType::Expense,
            AccountSubType::LossOnSale,
        ));
    }

    /// Seed manufacturing-cost-flow accounts (WIP, finished goods, variances, warranty).
    fn seed_manufacturing_accounts(coa: &mut ChartOfAccounts) {
        coa.add_account(GLAccount::new(
            manufacturing_accounts::FINISHED_GOODS.to_string(),
            "Finished Goods".to_string(),
            AccountType::Asset,
            AccountSubType::Inventory,
        ));
        coa.add_account(GLAccount::new(
            manufacturing_accounts::WIP.to_string(),
            "Work in Process".to_string(),
            AccountType::Asset,
            AccountSubType::Inventory,
        ));
        coa.add_account(GLAccount::new(
            manufacturing_accounts::LABOR_ACCRUAL.to_string(),
            "Labor Accrual".to_string(),
            AccountType::Liability,
            AccountSubType::AccruedLiabilities,
        ));
        coa.add_account(GLAccount::new(
            manufacturing_accounts::WARRANTY_PROVISION.to_string(),
            "Warranty Provision".to_string(),
            AccountType::Liability,
            AccountSubType::OtherLiabilities,
        ));

        let variance_accounts = [
            (
                manufacturing_accounts::SCRAP_EXPENSE,
                "Scrap Expense",
                AccountSubType::CostOfGoodsSold,
            ),
            (
                manufacturing_accounts::OVERHEAD_APPLIED,
                "Overhead Applied",
                AccountSubType::CostOfGoodsSold,
            ),
            (
                manufacturing_accounts::MATERIAL_PRICE_VARIANCE,
                "Material Price Variance",
                AccountSubType::CostOfGoodsSold,
            ),
            (
                manufacturing_accounts::MATERIAL_USAGE_VARIANCE,
                "Material Usage Variance",
                AccountSubType::CostOfGoodsSold,
            ),
            (
                manufacturing_accounts::LABOR_RATE_VARIANCE,
                "Labor Rate Variance",
                AccountSubType::CostOfGoodsSold,
            ),
            (
                manufacturing_accounts::LABOR_EFFICIENCY_VARIANCE,
                "Labor Efficiency Variance",
                AccountSubType::CostOfGoodsSold,
            ),
            (
                manufacturing_accounts::OVERHEAD_VOLUME_VARIANCE,
                "Overhead Volume Variance",
                AccountSubType::CostOfGoodsSold,
            ),
            (
                manufacturing_accounts::WARRANTY_EXPENSE,
                "Warranty Expense",
                AccountSubType::OperatingExpenses,
            ),
        ];
        for (number, name, sub) in variance_accounts {
            let mut acct = GLAccount::new(
                number.to_string(),
                name.to_string(),
                AccountType::Expense,
                sub,
            );
            acct.requires_cost_center = true;
            coa.add_account(acct);
        }
    }

    /// Seed intangible-asset accounts (goodwill, customer relationships, etc.).
    fn seed_intangible_accounts(coa: &mut ChartOfAccounts) {
        let intangibles = [
            (
                intangible_accounts::GOODWILL,
                "Goodwill",
                AccountType::Asset,
                AccountSubType::IntangibleAssets,
            ),
            (
                intangible_accounts::CUSTOMER_RELATIONSHIPS,
                "Customer Relationships",
                AccountType::Asset,
                AccountSubType::IntangibleAssets,
            ),
            (
                intangible_accounts::TRADE_NAME,
                "Trade Name / Brand",
                AccountType::Asset,
                AccountSubType::IntangibleAssets,
            ),
            (
                intangible_accounts::TECHNOLOGY,
                "Technology / Developed Software",
                AccountType::Asset,
                AccountSubType::IntangibleAssets,
            ),
            (
                intangible_accounts::ACCUMULATED_AMORTIZATION,
                "Accumulated Amortization",
                AccountType::Asset,
                AccountSubType::AccumulatedDepreciation,
            ),
            (
                intangible_accounts::AMORTIZATION_EXPENSE,
                "Amortization Expense — Intangibles",
                AccountType::Expense,
                AccountSubType::AmortizationExpense,
            ),
            (
                intangible_accounts::BARGAIN_PURCHASE_GAIN,
                "Bargain Purchase Gain",
                AccountType::Revenue,
                AccountSubType::OtherIncome,
            ),
        ];
        for (number, name, ty, sub) in intangibles {
            coa.add_account(GLAccount::new(
                number.to_string(),
                name.to_string(),
                ty,
                sub,
            ));
        }
    }

    /// Seed treasury / hedging / debt accounts.
    fn seed_treasury_accounts(coa: &mut ChartOfAccounts) {
        let entries = [
            (
                treasury_accounts::INTEREST_PAYABLE,
                "Interest Payable",
                AccountType::Liability,
                AccountSubType::AccruedLiabilities,
            ),
            (
                treasury_accounts::DEBT_PREMIUM,
                "Debt Premium",
                AccountType::Liability,
                AccountSubType::LongTermDebt,
            ),
            (
                treasury_accounts::DEBT_DISCOUNT,
                "Debt Discount",
                AccountType::Liability,
                AccountSubType::LongTermDebt,
            ),
            (
                treasury_accounts::DERIVATIVE_ASSET,
                "Derivative Asset",
                AccountType::Asset,
                AccountSubType::OtherAssets,
            ),
            (
                treasury_accounts::DERIVATIVE_LIABILITY,
                "Derivative Liability",
                AccountType::Liability,
                AccountSubType::OtherLiabilities,
            ),
            (
                treasury_accounts::OCI_CASH_FLOW_HEDGE,
                "OCI — Cash Flow Hedge Reserve",
                AccountType::Equity,
                AccountSubType::OtherComprehensiveIncome,
            ),
            (
                treasury_accounts::HEDGE_INEFFECTIVENESS,
                "Hedge Ineffectiveness",
                AccountType::Expense,
                AccountSubType::OtherExpenses,
            ),
            (
                treasury_accounts::CASH_POOL_IC_RECEIVABLE,
                "IC Receivable — Cash Pool",
                AccountType::Asset,
                AccountSubType::AccountsReceivable,
            ),
            (
                treasury_accounts::CASH_POOL_IC_PAYABLE,
                "IC Payable — Cash Pool",
                AccountType::Liability,
                AccountSubType::AccountsPayable,
            ),
        ];
        for (number, name, ty, sub) in entries {
            coa.add_account(GLAccount::new(
                number.to_string(),
                name.to_string(),
                ty,
                sub,
            ));
        }
    }

    /// Seed provision accounts (IAS 37 / ASC 450).
    fn seed_provision_accounts(coa: &mut ChartOfAccounts) {
        coa.add_account(GLAccount::new(
            provision_accounts::PROVISION_LIABILITY.to_string(),
            "Provision Liability".to_string(),
            AccountType::Liability,
            AccountSubType::OtherLiabilities,
        ));
        let mut prov_exp = GLAccount::new(
            provision_accounts::PROVISION_EXPENSE.to_string(),
            "Provision Expense".to_string(),
            AccountType::Expense,
            AccountSubType::OperatingExpenses,
        );
        prov_exp.requires_cost_center = true;
        coa.add_account(prov_exp);
    }

    /// Seed dividend accounts.
    fn seed_dividend_accounts(coa: &mut ChartOfAccounts) {
        coa.add_account(GLAccount::new(
            dividend_accounts::DIVIDENDS_PAYABLE.to_string(),
            "Dividends Payable".to_string(),
            AccountType::Liability,
            AccountSubType::OtherLiabilities,
        ));
        coa.add_account(GLAccount::new(
            dividend_accounts::DIVIDENDS_DECLARED.to_string(),
            "Dividends Declared".to_string(),
            AccountType::Equity,
            AccountSubType::RetainedEarnings,
        ));
    }

    /// Seed pension and share-based compensation accounts.
    fn seed_compensation_accounts(coa: &mut ChartOfAccounts) {
        coa.add_account(GLAccount::new(
            liability_accounts::NET_PENSION_LIABILITY.to_string(),
            "Net Pension Liability".to_string(),
            AccountType::Liability,
            AccountSubType::PensionLiabilities,
        ));
        coa.add_account(GLAccount::new(
            equity_accounts::OCI_REMEASUREMENTS.to_string(),
            "OCI — Pension Remeasurements".to_string(),
            AccountType::Equity,
            AccountSubType::OtherComprehensiveIncome,
        ));
        coa.add_account(GLAccount::new(
            equity_accounts::APIC_STOCK_COMP.to_string(),
            "APIC — Stock Compensation".to_string(),
            AccountType::Equity,
            AccountSubType::AdditionalPaidInCapital,
        ));
        let mut pension_exp = GLAccount::new(
            expense_accounts::PENSION_EXPENSE.to_string(),
            "Pension Expense".to_string(),
            AccountType::Expense,
            AccountSubType::OperatingExpenses,
        );
        pension_exp.requires_cost_center = true;
        coa.add_account(pension_exp);
        let mut stock_comp = GLAccount::new(
            expense_accounts::STOCK_COMP_EXPENSE.to_string(),
            "Stock-Based Compensation Expense".to_string(),
            AccountType::Expense,
            AccountSubType::OperatingExpenses,
        );
        stock_comp.requires_cost_center = true;
        coa.add_account(stock_comp);
    }

    /// Seed inventory subledger sub-accounts beyond the GL control (1200).
    fn seed_inventory_subledger_accounts(coa: &mut ChartOfAccounts) {
        coa.add_account(GLAccount::new(
            inventory_accounts::WRITEUP_INCOME.to_string(),
            "Inventory Write-up Income".to_string(),
            AccountType::Revenue,
            AccountSubType::OtherIncome,
        ));
        let mut writedown = GLAccount::new(
            inventory_accounts::WRITEDOWN_EXPENSE.to_string(),
            "Inventory Write-down Expense".to_string(),
            AccountType::Expense,
            AccountSubType::CostOfGoodsSold,
        );
        writedown.requires_cost_center = true;
        coa.add_account(writedown);
    }

    /// Seed tax accounts not already covered by `seed_canonical_accounts`.
    fn seed_additional_tax_accounts(coa: &mut ChartOfAccounts) {
        coa.add_account(GLAccount::new(
            tax_accounts::INCOME_TAX_PAYABLE.to_string(),
            "Income Tax Payable".to_string(),
            AccountType::Liability,
            AccountSubType::TaxLiabilities,
        ));
        coa.add_account(GLAccount::new(
            tax_accounts::TAX_RECEIVABLE.to_string(),
            "Tax Receivable".to_string(),
            AccountType::Asset,
            AccountSubType::OtherReceivables,
        ));
    }

    /// Seed dormant / blocked legacy accounts.
    ///
    /// These mirror the targets of the `DormantAccountActivity` anomaly
    /// strategy in `datasynth_generators::anomaly::strategies`. Real
    /// COAs retain such accounts (legacy suspense, predecessor-system
    /// clearing, obsolete migration accounts, retained QA / test
    /// accounts) as `is_blocked = true` so audit trails remain
    /// resolvable; we follow the same pattern. With `is_postable =
    /// false`, normal generators won't pick them up — only the anomaly
    /// strategy does, which is exactly the fraud-signature behaviour
    /// being modelled.
    fn seed_dormant_accounts(coa: &mut ChartOfAccounts) {
        let entries = [
            (
                dormant_accounts::LEGACY_SUSPENSE,
                "Legacy Suspense (migrated)",
                AccountType::Asset,
                AccountSubType::SuspenseClearing,
            ),
            (
                dormant_accounts::LEGACY_CLEARING,
                "Legacy Clearing (predecessor system)",
                AccountType::Liability,
                AccountSubType::OtherLiabilities,
            ),
            (
                dormant_accounts::OBSOLETE,
                "Obsolete Account",
                AccountType::Equity,
                AccountSubType::OtherComprehensiveIncome,
            ),
            (
                dormant_accounts::TEST_ACCOUNT,
                "Test Account (QA residue)",
                AccountType::Asset,
                AccountSubType::SuspenseClearing,
            ),
        ];
        for (number, name, ty, sub) in entries {
            let mut acct = GLAccount::new(number.to_string(), name.to_string(), ty, sub);
            acct.is_blocked = true;
            acct.is_postable = false;
            acct.is_suspense_account = matches!(sub, AccountSubType::SuspenseClearing);
            coa.add_account(acct);
        }
    }

    /// Seed equity accounts not already covered by `seed_canonical_accounts`.
    fn seed_additional_equity_accounts(coa: &mut ChartOfAccounts) {
        coa.add_account(GLAccount::new(
            equity_accounts::INCOME_SUMMARY.to_string(),
            "Income Summary".to_string(),
            AccountType::Equity,
            AccountSubType::NetIncome,
        ));
        coa.add_account(GLAccount::new(
            equity_accounts::DIVIDENDS_PAID.to_string(),
            "Dividends Paid".to_string(),
            AccountType::Equity,
            AccountSubType::RetainedEarnings,
        ));
    }

    fn generate_asset_accounts(&mut self, coa: &mut ChartOfAccounts, count: usize) {
        let sub_types = vec![
            (AccountSubType::Cash, "Cash", 0.15),
            (
                AccountSubType::AccountsReceivable,
                "Accounts Receivable",
                0.20,
            ),
            (AccountSubType::Inventory, "Inventory", 0.15),
            (AccountSubType::PrepaidExpenses, "Prepaid Expenses", 0.10),
            (AccountSubType::FixedAssets, "Fixed Assets", 0.25),
            (
                AccountSubType::AccumulatedDepreciation,
                "Accumulated Depreciation",
                0.10,
            ),
            (AccountSubType::OtherAssets, "Other Assets", 0.05),
        ];

        let mut account_num = 100000u32;
        for (sub_type, name_prefix, weight) in sub_types {
            let sub_count = ((count as f64) * weight).round() as usize;
            for i in 0..sub_count.max(1) {
                let account = GLAccount::new(
                    format!("{account_num}"),
                    format!("{} {}", name_prefix, i + 1),
                    AccountType::Asset,
                    sub_type,
                );
                coa.add_account(account);
                account_num += 10;
            }
        }
    }

    fn generate_liability_accounts(&mut self, coa: &mut ChartOfAccounts, count: usize) {
        let sub_types = vec![
            (AccountSubType::AccountsPayable, "Accounts Payable", 0.25),
            (
                AccountSubType::AccruedLiabilities,
                "Accrued Liabilities",
                0.20,
            ),
            (AccountSubType::ShortTermDebt, "Short-Term Debt", 0.15),
            (AccountSubType::LongTermDebt, "Long-Term Debt", 0.15),
            (AccountSubType::DeferredRevenue, "Deferred Revenue", 0.15),
            (AccountSubType::TaxLiabilities, "Tax Liabilities", 0.10),
        ];

        let mut account_num = 200000u32;
        for (sub_type, name_prefix, weight) in sub_types {
            let sub_count = ((count as f64) * weight).round() as usize;
            for i in 0..sub_count.max(1) {
                let account = GLAccount::new(
                    format!("{account_num}"),
                    format!("{} {}", name_prefix, i + 1),
                    AccountType::Liability,
                    sub_type,
                );
                coa.add_account(account);
                account_num += 10;
            }
        }
    }

    fn generate_equity_accounts(&mut self, coa: &mut ChartOfAccounts, count: usize) {
        let sub_types = vec![
            (AccountSubType::CommonStock, "Common Stock", 0.20),
            (AccountSubType::RetainedEarnings, "Retained Earnings", 0.30),
            (AccountSubType::AdditionalPaidInCapital, "APIC", 0.20),
            (AccountSubType::OtherComprehensiveIncome, "OCI", 0.30),
        ];

        let mut account_num = 300000u32;
        for (sub_type, name_prefix, weight) in sub_types {
            let sub_count = ((count as f64) * weight).round() as usize;
            for i in 0..sub_count.max(1) {
                let account = GLAccount::new(
                    format!("{account_num}"),
                    format!("{} {}", name_prefix, i + 1),
                    AccountType::Equity,
                    sub_type,
                );
                coa.add_account(account);
                account_num += 10;
            }
        }
    }

    fn generate_revenue_accounts(&mut self, coa: &mut ChartOfAccounts, count: usize) {
        let sub_types = vec![
            (AccountSubType::ProductRevenue, "Product Revenue", 0.40),
            (AccountSubType::ServiceRevenue, "Service Revenue", 0.30),
            (AccountSubType::InterestIncome, "Interest Income", 0.10),
            (AccountSubType::OtherIncome, "Other Income", 0.20),
        ];

        let mut account_num = 400000u32;
        for (sub_type, name_prefix, weight) in sub_types {
            let sub_count = ((count as f64) * weight).round() as usize;
            for i in 0..sub_count.max(1) {
                let account = GLAccount::new(
                    format!("{account_num}"),
                    format!("{} {}", name_prefix, i + 1),
                    AccountType::Revenue,
                    sub_type,
                );
                coa.add_account(account);
                account_num += 10;
            }
        }
    }

    fn generate_expense_accounts(&mut self, coa: &mut ChartOfAccounts, count: usize) {
        let sub_types = vec![
            (AccountSubType::CostOfGoodsSold, "COGS", 0.20),
            (
                AccountSubType::OperatingExpenses,
                "Operating Expenses",
                0.25,
            ),
            (AccountSubType::SellingExpenses, "Selling Expenses", 0.15),
            (
                AccountSubType::AdministrativeExpenses,
                "Admin Expenses",
                0.15,
            ),
            (AccountSubType::DepreciationExpense, "Depreciation", 0.10),
            (AccountSubType::InterestExpense, "Interest Expense", 0.05),
            (AccountSubType::TaxExpense, "Tax Expense", 0.05),
            (AccountSubType::OtherExpenses, "Other Expenses", 0.05),
        ];

        let mut account_num = 500000u32;
        for (sub_type, name_prefix, weight) in sub_types {
            let sub_count = ((count as f64) * weight).round() as usize;
            for i in 0..sub_count.max(1) {
                let mut account = GLAccount::new(
                    format!("{account_num}"),
                    format!("{} {}", name_prefix, i + 1),
                    AccountType::Expense,
                    sub_type,
                );
                account.requires_cost_center = true;
                coa.add_account(account);
                account_num += 10;
            }
        }
    }

    fn generate_suspense_accounts(&mut self, coa: &mut ChartOfAccounts) {
        let suspense_types = vec![
            (AccountSubType::SuspenseClearing, "Suspense Clearing"),
            (AccountSubType::GoodsReceivedClearing, "GR/IR Clearing"),
            (AccountSubType::BankClearing, "Bank Clearing"),
            (
                AccountSubType::IntercompanyClearing,
                "Intercompany Clearing",
            ),
        ];

        let mut account_num = 199000u32;
        for (sub_type, name) in suspense_types {
            let mut account = GLAccount::new(
                format!("{account_num}"),
                name.to_string(),
                AccountType::Asset,
                sub_type,
            );
            account.is_suspense_account = true;
            coa.add_account(account);
            account_num += 100;
        }
    }
}

impl Generator for ChartOfAccountsGenerator {
    type Item = ChartOfAccounts;
    type Config = (CoAComplexity, IndustrySector);

    fn new(config: Self::Config, seed: u64) -> Self {
        Self::new(config.0, config.1, seed)
    }

    fn generate_one(&mut self) -> Self::Item {
        self.generate()
    }

    fn reset(&mut self) {
        self.rng = seeded_rng(self.seed, 0);
        self.count = 0;
        self.coa_framework = CoAFramework::UsGaap;
    }

    fn count(&self) -> u64 {
        self.count
    }

    fn seed(&self) -> u64 {
        self.seed
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_small_coa() {
        let mut gen =
            ChartOfAccountsGenerator::new(CoAComplexity::Small, IndustrySector::Manufacturing, 42);
        let coa = gen.generate();

        assert!(coa.account_count() >= 50);
        assert!(!coa.get_suspense_accounts().is_empty());
    }

    #[test]
    fn test_generate_pcg_coa() {
        let mut gen =
            ChartOfAccountsGenerator::new(CoAComplexity::Small, IndustrySector::Manufacturing, 42)
                .with_french_pcg(true);
        let coa = gen.generate();

        assert_eq!(coa.country, "FR");
        assert!(coa.name.contains("Plan Comptable") || coa.name.contains("PCG"));
        assert!(coa.account_count() >= 20);
        // PCG accounts are 6-digit (e.g. 411000, 601000)
        let first = coa.accounts.first().expect("has accounts");
        assert_eq!(first.account_number.len(), 6);
    }

    /// Verifies PCG (Plan Comptable Général) account structure: 6-digit format,
    /// all accounts numeric, and coverage of at least two PCG classes (1–8).
    /// Works for both the embedded PCG 2024 loader and the fallback generator.
    #[test]
    fn test_pcg_account_structure() {
        let mut gen =
            ChartOfAccountsGenerator::new(CoAComplexity::Small, IndustrySector::Manufacturing, 42)
                .with_french_pcg(true);
        let coa = gen.generate();

        assert_eq!(
            coa.account_format, "######",
            "PCG uses 6-digit account format"
        );
        assert!(
            coa.account_count() >= 20,
            "PCG CoA has minimum account count"
        );

        let account_numbers: Vec<_> = coa
            .accounts
            .iter()
            .map(|a| a.account_number.as_str())
            .collect();
        for num in &account_numbers {
            assert_eq!(num.len(), 6, "every PCG account is 6 digits: {}", num);
            assert!(
                num.chars().all(|c| c.is_ascii_digit()),
                "PCG account is numeric: {}",
                num
            );
        }

        // All account numbers must belong to a PCG class (first digit 1–8)
        let first_digits: std::collections::HashSet<char> = account_numbers
            .iter()
            .filter_map(|s| s.chars().next())
            .collect();
        let pcg_classes: std::collections::HashSet<_> =
            ['1', '2', '3', '4', '5', '6', '7', '8'].into();
        assert!(
            !first_digits.is_empty() && first_digits.is_subset(&pcg_classes),
            "PCG account numbers must be in classes 1–8, got first digits: {:?}",
            first_digits
        );
    }
}
