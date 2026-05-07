//! ISO 21378:2019 — Audit Data Collection (ADC) account classification.
//!
//! ISO 21378 standardises a 3-level account-classification hierarchy for
//! audit data exchange:
//!
//! - **Level 1: Account Type** — five categories (`A` Assets, `L`
//!   Liabilities, `E` Equity, `R` Revenue, `X` Expenses).
//! - **Level 2: Account Class** — ~20 broad classes within each type
//!   (e.g. `A.A` Cash, `A.B` Trade Receivables, `A.F` PPE).
//! - **Level 3: Account Sub-Class** — finer-grained categories (e.g.
//!   `A.A.A` Operating Cash, `A.B.A` Trade AR Domestic).
//!
//! This module exposes the ADC hierarchy as Rust enums plus a
//! deterministic mapping from the existing
//! [`crate::models::AccountSubType`] enum to `(AdcClass, AdcSubClass)`,
//! so any chart of accounts produced by DataSynth can be projected onto
//! the ADC surface for audit-exchange exports.
//!
//! The ISO standard does not formally enumerate every Level-3 sub-class
//! — it provides the structure and lets implementers define sub-classes
//! relevant to their own business. The sub-classes here cover every
//! `AccountSubType` variant currently used by the generators, with codes
//! that follow ISO conventions (`<TYPE>.<CLASS>.<SUBCLASS>`).
//!
//! # Stability
//!
//! ADC codes published in this module are part of the public schema:
//! once an `AccountSubType` is mapped to `(AdcClass, AdcSubClass)`, the
//! code remains stable across releases. New codes are appended; existing
//! codes are not renamed.

use serde::{Deserialize, Serialize};

use crate::models::AccountSubType;

// ──────────────────────────────────────────────────────────────────────
// Level 1 — Account Type
// ──────────────────────────────────────────────────────────────────────

/// ISO 21378 Level-1 account type (5 categories).
///
/// One-letter ISO code: `A` Assets, `L` Liabilities, `E` Equity,
/// `R` Revenue, `X` Expenses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdcType {
    /// `A` — Assets
    Asset,
    /// `L` — Liabilities
    Liability,
    /// `E` — Equity
    Equity,
    /// `R` — Revenue
    Revenue,
    /// `X` — Expenses (includes COGS)
    Expense,
}

impl AdcType {
    /// One-letter ISO code (`"A"`, `"L"`, `"E"`, `"R"`, `"X"`).
    pub fn code(&self) -> &'static str {
        match self {
            Self::Asset => "A",
            Self::Liability => "L",
            Self::Equity => "E",
            Self::Revenue => "R",
            Self::Expense => "X",
        }
    }

    /// Human-readable name (`"Assets"`, `"Liabilities"`, …).
    pub fn name(&self) -> &'static str {
        match self {
            Self::Asset => "Assets",
            Self::Liability => "Liabilities",
            Self::Equity => "Equity",
            Self::Revenue => "Revenue",
            Self::Expense => "Expenses",
        }
    }
}

// ──────────────────────────────────────────────────────────────────────
// Level 2 — Account Class
// ──────────────────────────────────────────────────────────────────────

/// ISO 21378 Level-2 account class (~24 categories).
///
/// Two-segment ISO code (`<type>.<class>`, e.g. `A.B`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdcClass {
    // Assets
    /// `A.A` — Cash & Cash Equivalents
    AssetCash,
    /// `A.B` — Trade Receivables
    AssetReceivables,
    /// `A.C` — Inventory
    AssetInventory,
    /// `A.D` — Prepaid Expenses & Other Current Assets
    AssetPrepaid,
    /// `A.E` — Long-term Investments
    AssetInvestments,
    /// `A.F` — Property, Plant & Equipment
    AssetPpe,
    /// `A.G` — Intangible Assets
    AssetIntangible,
    /// `A.H` — Other Long-term Assets
    AssetOtherLongTerm,
    /// `A.X` — Suspense / Clearing (non-standard but conventional)
    AssetSuspense,

    // Liabilities
    /// `L.A` — Trade Payables
    LiabilityPayables,
    /// `L.B` — Accrued Liabilities
    LiabilityAccrued,
    /// `L.C` — Short-term Debt
    LiabilityShortTermDebt,
    /// `L.D` — Tax Liabilities
    LiabilityTax,
    /// `L.E` — Long-term Debt
    LiabilityLongTermDebt,
    /// `L.F` — Other Long-term Liabilities (incl. pension)
    LiabilityOther,

    // Equity
    /// `E.A` — Contributed Capital
    EquityContributed,
    /// `E.B` — Retained Earnings
    EquityRetained,
    /// `E.C` — Other Comprehensive Income
    EquityOci,
    /// `E.D` — Treasury Stock
    EquityTreasury,

    // Revenue
    /// `R.A` — Operating Revenue
    RevenueOperating,
    /// `R.B` — Other Income
    RevenueOther,

    // Expenses
    /// `X.A` — Cost of Goods Sold / Cost of Revenue
    ExpenseCogs,
    /// `X.B` — Selling, General & Administrative
    ExpenseSga,
    /// `X.D` — Depreciation & Amortisation
    ExpenseDepAmort,
    /// `X.E` — Interest Expense
    ExpenseInterest,
    /// `X.F` — Tax Expense
    ExpenseTax,
    /// `X.G` — Other Expenses
    ExpenseOther,
}

impl AdcClass {
    /// Two-segment ISO code (`"A.B"`).
    pub fn code(&self) -> &'static str {
        match self {
            Self::AssetCash => "A.A",
            Self::AssetReceivables => "A.B",
            Self::AssetInventory => "A.C",
            Self::AssetPrepaid => "A.D",
            Self::AssetInvestments => "A.E",
            Self::AssetPpe => "A.F",
            Self::AssetIntangible => "A.G",
            Self::AssetOtherLongTerm => "A.H",
            Self::AssetSuspense => "A.X",
            Self::LiabilityPayables => "L.A",
            Self::LiabilityAccrued => "L.B",
            Self::LiabilityShortTermDebt => "L.C",
            Self::LiabilityTax => "L.D",
            Self::LiabilityLongTermDebt => "L.E",
            Self::LiabilityOther => "L.F",
            Self::EquityContributed => "E.A",
            Self::EquityRetained => "E.B",
            Self::EquityOci => "E.C",
            Self::EquityTreasury => "E.D",
            Self::RevenueOperating => "R.A",
            Self::RevenueOther => "R.B",
            Self::ExpenseCogs => "X.A",
            Self::ExpenseSga => "X.B",
            Self::ExpenseDepAmort => "X.D",
            Self::ExpenseInterest => "X.E",
            Self::ExpenseTax => "X.F",
            Self::ExpenseOther => "X.G",
        }
    }

    /// Human-readable class name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::AssetCash => "Cash & Cash Equivalents",
            Self::AssetReceivables => "Trade Receivables",
            Self::AssetInventory => "Inventory",
            Self::AssetPrepaid => "Prepaid Expenses & Other Current Assets",
            Self::AssetInvestments => "Long-term Investments",
            Self::AssetPpe => "Property, Plant & Equipment",
            Self::AssetIntangible => "Intangible Assets",
            Self::AssetOtherLongTerm => "Other Long-term Assets",
            Self::AssetSuspense => "Suspense & Clearing (Asset side)",
            Self::LiabilityPayables => "Trade Payables",
            Self::LiabilityAccrued => "Accrued Liabilities",
            Self::LiabilityShortTermDebt => "Short-term Debt",
            Self::LiabilityTax => "Tax Liabilities",
            Self::LiabilityLongTermDebt => "Long-term Debt",
            Self::LiabilityOther => "Other Long-term Liabilities",
            Self::EquityContributed => "Contributed Capital",
            Self::EquityRetained => "Retained Earnings",
            Self::EquityOci => "Other Comprehensive Income",
            Self::EquityTreasury => "Treasury Stock",
            Self::RevenueOperating => "Operating Revenue",
            Self::RevenueOther => "Other Income",
            Self::ExpenseCogs => "Cost of Goods Sold",
            Self::ExpenseSga => "Selling, General & Administrative",
            Self::ExpenseDepAmort => "Depreciation & Amortisation",
            Self::ExpenseInterest => "Interest Expense",
            Self::ExpenseTax => "Tax Expense",
            Self::ExpenseOther => "Other Expenses",
        }
    }

    /// Parent ADC type.
    pub fn adc_type(&self) -> AdcType {
        match self {
            Self::AssetCash
            | Self::AssetReceivables
            | Self::AssetInventory
            | Self::AssetPrepaid
            | Self::AssetInvestments
            | Self::AssetPpe
            | Self::AssetIntangible
            | Self::AssetOtherLongTerm
            | Self::AssetSuspense => AdcType::Asset,
            Self::LiabilityPayables
            | Self::LiabilityAccrued
            | Self::LiabilityShortTermDebt
            | Self::LiabilityTax
            | Self::LiabilityLongTermDebt
            | Self::LiabilityOther => AdcType::Liability,
            Self::EquityContributed
            | Self::EquityRetained
            | Self::EquityOci
            | Self::EquityTreasury => AdcType::Equity,
            Self::RevenueOperating | Self::RevenueOther => AdcType::Revenue,
            Self::ExpenseCogs
            | Self::ExpenseSga
            | Self::ExpenseDepAmort
            | Self::ExpenseInterest
            | Self::ExpenseTax
            | Self::ExpenseOther => AdcType::Expense,
        }
    }
}

// ──────────────────────────────────────────────────────────────────────
// Level 3 — Account Sub-Class
// ──────────────────────────────────────────────────────────────────────

/// ISO 21378 Level-3 account sub-class.
///
/// Three-segment ISO code (`<type>.<class>.<subclass>`, e.g. `A.B.A`).
/// Codes are stable; new variants are appended, existing variants are
/// not renamed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdcSubClass {
    // Assets
    /// `A.A.A` — Operating cash
    AssetCashOperating,
    /// `A.A.B` — Bank clearing accounts
    AssetCashBankClearing,
    /// `A.B.A` — Trade Accounts Receivable
    AssetTradeAr,
    /// `A.B.B` — Other Receivables
    AssetOtherReceivables,
    /// `A.C.A` — Inventory (general)
    AssetInventoryGeneral,
    /// `A.D.A` — Prepaid Expenses
    AssetPrepaidExpenses,
    /// `A.E.A` — Long-term Investments
    AssetInvestmentsLongTerm,
    /// `A.F.A` — Fixed Assets (gross)
    AssetFixedAssets,
    /// `A.F.B` — Accumulated Depreciation (contra)
    AssetAccumulatedDepreciation,
    /// `A.G.A` — Intangible Assets
    AssetIntangibleAssets,
    /// `A.H.A` — Other Assets
    AssetOtherAssets,
    /// `A.X.A` — Suspense Clearing (Asset side)
    AssetSuspenseClearing,
    /// `A.X.B` — Intercompany Clearing (Asset side)
    AssetIntercompanyClearing,

    // Liabilities
    /// `L.A.A` — Trade Accounts Payable
    LiabilityTradeAp,
    /// `L.B.A` — Accrued Expenses
    LiabilityAccruedExpenses,
    /// `L.B.B` — Deferred Revenue
    LiabilityDeferredRevenue,
    /// `L.B.C` — Goods Received / Invoice Received Clearing
    LiabilityGoodsReceivedClearing,
    /// `L.C.A` — Short-term Debt
    LiabilityShortTermDebt,
    /// `L.D.A` — Tax Liabilities
    LiabilityTaxLiabilities,
    /// `L.E.A` — Long-term Debt
    LiabilityLongTermDebt,
    /// `L.F.A` — Pension Liabilities
    LiabilityPensionLiabilities,
    /// `L.F.B` — Other Liabilities
    LiabilityOtherLiabilities,

    // Equity
    /// `E.A.A` — Common Stock
    EquityCommonStock,
    /// `E.A.B` — Additional Paid-In Capital
    EquityAdditionalPaidIn,
    /// `E.B.A` — Retained Earnings
    EquityRetainedEarnings,
    /// `E.B.B` — Net Income (current period)
    EquityNetIncome,
    /// `E.C.A` — Other Comprehensive Income
    EquityOtherComprehensiveIncome,
    /// `E.D.A` — Treasury Stock
    EquityTreasuryStock,

    // Revenue
    /// `R.A.A` — Product Revenue
    RevenueProduct,
    /// `R.A.B` — Service Revenue
    RevenueService,
    /// `R.B.A` — Interest Income
    RevenueInterestIncome,
    /// `R.B.B` — Dividend Income
    RevenueDividendIncome,
    /// `R.B.C` — Gain on Sale of Assets
    RevenueGainOnSale,
    /// `R.B.D` — Other Income
    RevenueOtherIncome,

    // Expenses
    /// `X.A.A` — Cost of Goods Sold
    ExpenseCostOfGoodsSold,
    /// `X.B.A` — Operating Expenses
    ExpenseOperatingExpenses,
    /// `X.B.B` — Selling Expenses
    ExpenseSellingExpenses,
    /// `X.B.C` — Administrative Expenses
    ExpenseAdministrativeExpenses,
    /// `X.D.A` — Depreciation Expense
    ExpenseDepreciationExpense,
    /// `X.D.B` — Amortisation Expense
    ExpenseAmortisationExpense,
    /// `X.E.A` — Interest Expense
    ExpenseInterestExpense,
    /// `X.F.A` — Tax Expense
    ExpenseTaxExpense,
    /// `X.G.A` — Foreign-Exchange Loss
    ExpenseForeignExchangeLoss,
    /// `X.G.B` — Loss on Sale of Assets
    ExpenseLossOnSale,
    /// `X.G.C` — Other Expenses
    ExpenseOtherExpenses,
}

impl AdcSubClass {
    /// Three-segment ISO code (`"A.B.A"`).
    pub fn code(&self) -> &'static str {
        match self {
            Self::AssetCashOperating => "A.A.A",
            Self::AssetCashBankClearing => "A.A.B",
            Self::AssetTradeAr => "A.B.A",
            Self::AssetOtherReceivables => "A.B.B",
            Self::AssetInventoryGeneral => "A.C.A",
            Self::AssetPrepaidExpenses => "A.D.A",
            Self::AssetInvestmentsLongTerm => "A.E.A",
            Self::AssetFixedAssets => "A.F.A",
            Self::AssetAccumulatedDepreciation => "A.F.B",
            Self::AssetIntangibleAssets => "A.G.A",
            Self::AssetOtherAssets => "A.H.A",
            Self::AssetSuspenseClearing => "A.X.A",
            Self::AssetIntercompanyClearing => "A.X.B",
            Self::LiabilityTradeAp => "L.A.A",
            Self::LiabilityAccruedExpenses => "L.B.A",
            Self::LiabilityDeferredRevenue => "L.B.B",
            Self::LiabilityGoodsReceivedClearing => "L.B.C",
            Self::LiabilityShortTermDebt => "L.C.A",
            Self::LiabilityTaxLiabilities => "L.D.A",
            Self::LiabilityLongTermDebt => "L.E.A",
            Self::LiabilityPensionLiabilities => "L.F.A",
            Self::LiabilityOtherLiabilities => "L.F.B",
            Self::EquityCommonStock => "E.A.A",
            Self::EquityAdditionalPaidIn => "E.A.B",
            Self::EquityRetainedEarnings => "E.B.A",
            Self::EquityNetIncome => "E.B.B",
            Self::EquityOtherComprehensiveIncome => "E.C.A",
            Self::EquityTreasuryStock => "E.D.A",
            Self::RevenueProduct => "R.A.A",
            Self::RevenueService => "R.A.B",
            Self::RevenueInterestIncome => "R.B.A",
            Self::RevenueDividendIncome => "R.B.B",
            Self::RevenueGainOnSale => "R.B.C",
            Self::RevenueOtherIncome => "R.B.D",
            Self::ExpenseCostOfGoodsSold => "X.A.A",
            Self::ExpenseOperatingExpenses => "X.B.A",
            Self::ExpenseSellingExpenses => "X.B.B",
            Self::ExpenseAdministrativeExpenses => "X.B.C",
            Self::ExpenseDepreciationExpense => "X.D.A",
            Self::ExpenseAmortisationExpense => "X.D.B",
            Self::ExpenseInterestExpense => "X.E.A",
            Self::ExpenseTaxExpense => "X.F.A",
            Self::ExpenseForeignExchangeLoss => "X.G.A",
            Self::ExpenseLossOnSale => "X.G.B",
            Self::ExpenseOtherExpenses => "X.G.C",
        }
    }

    /// Human-readable sub-class name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::AssetCashOperating => "Operating Cash",
            Self::AssetCashBankClearing => "Bank Clearing",
            Self::AssetTradeAr => "Trade Accounts Receivable",
            Self::AssetOtherReceivables => "Other Receivables",
            Self::AssetInventoryGeneral => "Inventory",
            Self::AssetPrepaidExpenses => "Prepaid Expenses",
            Self::AssetInvestmentsLongTerm => "Long-term Investments",
            Self::AssetFixedAssets => "Fixed Assets",
            Self::AssetAccumulatedDepreciation => "Accumulated Depreciation",
            Self::AssetIntangibleAssets => "Intangible Assets",
            Self::AssetOtherAssets => "Other Assets",
            Self::AssetSuspenseClearing => "Suspense Clearing",
            Self::AssetIntercompanyClearing => "Intercompany Clearing",
            Self::LiabilityTradeAp => "Trade Accounts Payable",
            Self::LiabilityAccruedExpenses => "Accrued Expenses",
            Self::LiabilityDeferredRevenue => "Deferred Revenue",
            Self::LiabilityGoodsReceivedClearing => "GR/IR Clearing",
            Self::LiabilityShortTermDebt => "Short-term Debt",
            Self::LiabilityTaxLiabilities => "Tax Liabilities",
            Self::LiabilityLongTermDebt => "Long-term Debt",
            Self::LiabilityPensionLiabilities => "Pension Liabilities",
            Self::LiabilityOtherLiabilities => "Other Liabilities",
            Self::EquityCommonStock => "Common Stock",
            Self::EquityAdditionalPaidIn => "Additional Paid-In Capital",
            Self::EquityRetainedEarnings => "Retained Earnings",
            Self::EquityNetIncome => "Net Income",
            Self::EquityOtherComprehensiveIncome => "Other Comprehensive Income",
            Self::EquityTreasuryStock => "Treasury Stock",
            Self::RevenueProduct => "Product Revenue",
            Self::RevenueService => "Service Revenue",
            Self::RevenueInterestIncome => "Interest Income",
            Self::RevenueDividendIncome => "Dividend Income",
            Self::RevenueGainOnSale => "Gain on Sale of Assets",
            Self::RevenueOtherIncome => "Other Income",
            Self::ExpenseCostOfGoodsSold => "Cost of Goods Sold",
            Self::ExpenseOperatingExpenses => "Operating Expenses",
            Self::ExpenseSellingExpenses => "Selling Expenses",
            Self::ExpenseAdministrativeExpenses => "Administrative Expenses",
            Self::ExpenseDepreciationExpense => "Depreciation Expense",
            Self::ExpenseAmortisationExpense => "Amortisation Expense",
            Self::ExpenseInterestExpense => "Interest Expense",
            Self::ExpenseTaxExpense => "Tax Expense",
            Self::ExpenseForeignExchangeLoss => "Foreign-Exchange Loss",
            Self::ExpenseLossOnSale => "Loss on Sale of Assets",
            Self::ExpenseOtherExpenses => "Other Expenses",
        }
    }

    /// Parent ADC class (Level 2).
    pub fn adc_class(&self) -> AdcClass {
        match self {
            Self::AssetCashOperating | Self::AssetCashBankClearing => AdcClass::AssetCash,
            Self::AssetTradeAr | Self::AssetOtherReceivables => AdcClass::AssetReceivables,
            Self::AssetInventoryGeneral => AdcClass::AssetInventory,
            Self::AssetPrepaidExpenses => AdcClass::AssetPrepaid,
            Self::AssetInvestmentsLongTerm => AdcClass::AssetInvestments,
            Self::AssetFixedAssets | Self::AssetAccumulatedDepreciation => AdcClass::AssetPpe,
            Self::AssetIntangibleAssets => AdcClass::AssetIntangible,
            Self::AssetOtherAssets => AdcClass::AssetOtherLongTerm,
            Self::AssetSuspenseClearing | Self::AssetIntercompanyClearing => {
                AdcClass::AssetSuspense
            }
            Self::LiabilityTradeAp => AdcClass::LiabilityPayables,
            Self::LiabilityAccruedExpenses
            | Self::LiabilityDeferredRevenue
            | Self::LiabilityGoodsReceivedClearing => AdcClass::LiabilityAccrued,
            Self::LiabilityShortTermDebt => AdcClass::LiabilityShortTermDebt,
            Self::LiabilityTaxLiabilities => AdcClass::LiabilityTax,
            Self::LiabilityLongTermDebt => AdcClass::LiabilityLongTermDebt,
            Self::LiabilityPensionLiabilities | Self::LiabilityOtherLiabilities => {
                AdcClass::LiabilityOther
            }
            Self::EquityCommonStock | Self::EquityAdditionalPaidIn => AdcClass::EquityContributed,
            Self::EquityRetainedEarnings | Self::EquityNetIncome => AdcClass::EquityRetained,
            Self::EquityOtherComprehensiveIncome => AdcClass::EquityOci,
            Self::EquityTreasuryStock => AdcClass::EquityTreasury,
            Self::RevenueProduct | Self::RevenueService => AdcClass::RevenueOperating,
            Self::RevenueInterestIncome
            | Self::RevenueDividendIncome
            | Self::RevenueGainOnSale
            | Self::RevenueOtherIncome => AdcClass::RevenueOther,
            Self::ExpenseCostOfGoodsSold => AdcClass::ExpenseCogs,
            Self::ExpenseOperatingExpenses
            | Self::ExpenseSellingExpenses
            | Self::ExpenseAdministrativeExpenses => AdcClass::ExpenseSga,
            Self::ExpenseDepreciationExpense | Self::ExpenseAmortisationExpense => {
                AdcClass::ExpenseDepAmort
            }
            Self::ExpenseInterestExpense => AdcClass::ExpenseInterest,
            Self::ExpenseTaxExpense => AdcClass::ExpenseTax,
            Self::ExpenseForeignExchangeLoss
            | Self::ExpenseLossOnSale
            | Self::ExpenseOtherExpenses => AdcClass::ExpenseOther,
        }
    }
}

// ──────────────────────────────────────────────────────────────────────
// Mapping from `AccountSubType` to `(AdcClass, AdcSubClass)`
// ──────────────────────────────────────────────────────────────────────

/// Map a [`AccountSubType`] to the equivalent ISO 21378 Level-3
/// sub-class (and, transitively, the Level-2 class via
/// [`AdcSubClass::adc_class`]).
///
/// Total: every variant of `AccountSubType` is mapped — the function
/// is total, never panics, and never returns a default. New
/// `AccountSubType` variants must be added to the match arm or the
/// crate will fail to compile.
pub fn from_account_sub_type(sub_type: AccountSubType) -> AdcSubClass {
    use AccountSubType as S;
    match sub_type {
        S::Cash => AdcSubClass::AssetCashOperating,
        S::AccountsReceivable => AdcSubClass::AssetTradeAr,
        S::OtherReceivables => AdcSubClass::AssetOtherReceivables,
        S::Inventory => AdcSubClass::AssetInventoryGeneral,
        S::PrepaidExpenses => AdcSubClass::AssetPrepaidExpenses,
        S::FixedAssets => AdcSubClass::AssetFixedAssets,
        S::AccumulatedDepreciation => AdcSubClass::AssetAccumulatedDepreciation,
        S::Investments => AdcSubClass::AssetInvestmentsLongTerm,
        S::IntangibleAssets => AdcSubClass::AssetIntangibleAssets,
        S::OtherAssets => AdcSubClass::AssetOtherAssets,
        S::AccountsPayable => AdcSubClass::LiabilityTradeAp,
        S::AccruedLiabilities => AdcSubClass::LiabilityAccruedExpenses,
        S::ShortTermDebt => AdcSubClass::LiabilityShortTermDebt,
        S::LongTermDebt => AdcSubClass::LiabilityLongTermDebt,
        S::DeferredRevenue => AdcSubClass::LiabilityDeferredRevenue,
        S::TaxLiabilities => AdcSubClass::LiabilityTaxLiabilities,
        S::PensionLiabilities => AdcSubClass::LiabilityPensionLiabilities,
        S::OtherLiabilities => AdcSubClass::LiabilityOtherLiabilities,
        S::CommonStock => AdcSubClass::EquityCommonStock,
        S::RetainedEarnings => AdcSubClass::EquityRetainedEarnings,
        S::AdditionalPaidInCapital => AdcSubClass::EquityAdditionalPaidIn,
        S::TreasuryStock => AdcSubClass::EquityTreasuryStock,
        S::OtherComprehensiveIncome => AdcSubClass::EquityOtherComprehensiveIncome,
        S::NetIncome => AdcSubClass::EquityNetIncome,
        S::ProductRevenue => AdcSubClass::RevenueProduct,
        S::ServiceRevenue => AdcSubClass::RevenueService,
        S::InterestIncome => AdcSubClass::RevenueInterestIncome,
        S::DividendIncome => AdcSubClass::RevenueDividendIncome,
        S::GainOnSale => AdcSubClass::RevenueGainOnSale,
        S::OtherIncome => AdcSubClass::RevenueOtherIncome,
        S::CostOfGoodsSold => AdcSubClass::ExpenseCostOfGoodsSold,
        S::OperatingExpenses => AdcSubClass::ExpenseOperatingExpenses,
        S::SellingExpenses => AdcSubClass::ExpenseSellingExpenses,
        S::AdministrativeExpenses => AdcSubClass::ExpenseAdministrativeExpenses,
        S::DepreciationExpense => AdcSubClass::ExpenseDepreciationExpense,
        S::AmortizationExpense => AdcSubClass::ExpenseAmortisationExpense,
        S::InterestExpense => AdcSubClass::ExpenseInterestExpense,
        S::TaxExpense => AdcSubClass::ExpenseTaxExpense,
        S::ForeignExchangeLoss => AdcSubClass::ExpenseForeignExchangeLoss,
        S::LossOnSale => AdcSubClass::ExpenseLossOnSale,
        S::OtherExpenses => AdcSubClass::ExpenseOtherExpenses,
        S::SuspenseClearing => AdcSubClass::AssetSuspenseClearing,
        S::GoodsReceivedClearing => AdcSubClass::LiabilityGoodsReceivedClearing,
        S::BankClearing => AdcSubClass::AssetCashBankClearing,
        S::IntercompanyClearing => AdcSubClass::AssetIntercompanyClearing,
    }
}

// ──────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::AccountSubType;

    /// Every AccountSubType variant must map to a valid ISO sub-class.
    /// This is a compile-time check (the match in `from_account_sub_type`
    /// is exhaustive) plus a runtime sanity check that the produced
    /// codes are well-formed.
    #[test]
    fn every_account_sub_type_maps_to_valid_iso_codes() {
        for &sub in &[
            AccountSubType::Cash,
            AccountSubType::AccountsReceivable,
            AccountSubType::OtherReceivables,
            AccountSubType::Inventory,
            AccountSubType::PrepaidExpenses,
            AccountSubType::FixedAssets,
            AccountSubType::AccumulatedDepreciation,
            AccountSubType::Investments,
            AccountSubType::IntangibleAssets,
            AccountSubType::OtherAssets,
            AccountSubType::AccountsPayable,
            AccountSubType::AccruedLiabilities,
            AccountSubType::ShortTermDebt,
            AccountSubType::LongTermDebt,
            AccountSubType::DeferredRevenue,
            AccountSubType::TaxLiabilities,
            AccountSubType::PensionLiabilities,
            AccountSubType::OtherLiabilities,
            AccountSubType::CommonStock,
            AccountSubType::RetainedEarnings,
            AccountSubType::AdditionalPaidInCapital,
            AccountSubType::TreasuryStock,
            AccountSubType::OtherComprehensiveIncome,
            AccountSubType::NetIncome,
            AccountSubType::ProductRevenue,
            AccountSubType::ServiceRevenue,
            AccountSubType::InterestIncome,
            AccountSubType::DividendIncome,
            AccountSubType::GainOnSale,
            AccountSubType::OtherIncome,
            AccountSubType::CostOfGoodsSold,
            AccountSubType::OperatingExpenses,
            AccountSubType::SellingExpenses,
            AccountSubType::AdministrativeExpenses,
            AccountSubType::DepreciationExpense,
            AccountSubType::AmortizationExpense,
            AccountSubType::InterestExpense,
            AccountSubType::TaxExpense,
            AccountSubType::ForeignExchangeLoss,
            AccountSubType::LossOnSale,
            AccountSubType::OtherExpenses,
            AccountSubType::SuspenseClearing,
            AccountSubType::GoodsReceivedClearing,
            AccountSubType::BankClearing,
            AccountSubType::IntercompanyClearing,
        ] {
            let sub_class = from_account_sub_type(sub);
            let class = sub_class.adc_class();
            let ty = class.adc_type();

            // Sub-class code must be `<type>.<class>.<x>` (5 chars,
            // dotted) and consistent with class / type codes.
            let sub_code = sub_class.code();
            let class_code = class.code();
            let type_code = ty.code();
            assert_eq!(
                sub_code.len(),
                5,
                "sub-class code should be 5 chars: {sub_code}"
            );
            assert!(
                sub_code.starts_with(class_code),
                "sub-class code {sub_code} should start with class {class_code}"
            );
            assert!(
                class_code.starts_with(type_code),
                "class code {class_code} should start with type {type_code}"
            );
            assert!(
                !sub_class.name().is_empty(),
                "sub-class name should not be empty for {sub:?}"
            );
        }
    }

    #[test]
    fn isocode_examples_are_correct() {
        assert_eq!(AdcType::Asset.code(), "A");
        assert_eq!(AdcClass::AssetReceivables.code(), "A.B");
        assert_eq!(AdcSubClass::AssetTradeAr.code(), "A.B.A");
        assert_eq!(
            from_account_sub_type(AccountSubType::AccountsReceivable).code(),
            "A.B.A"
        );
        assert_eq!(
            from_account_sub_type(AccountSubType::CostOfGoodsSold).code(),
            "X.A.A"
        );
        assert_eq!(
            from_account_sub_type(AccountSubType::CommonStock).code(),
            "E.A.A"
        );
    }

    #[test]
    fn parent_relationships_are_consistent() {
        // Every sub-class's parent class must report the same type code.
        for sub_class in [
            AdcSubClass::AssetTradeAr,
            AdcSubClass::LiabilityTradeAp,
            AdcSubClass::EquityRetainedEarnings,
            AdcSubClass::RevenueProduct,
            AdcSubClass::ExpenseCostOfGoodsSold,
        ] {
            let class = sub_class.adc_class();
            let ty = class.adc_type();
            // Sub-class code starts with class code; class code starts with type code.
            assert!(sub_class.code().starts_with(class.code()));
            assert!(class.code().starts_with(ty.code()));
        }
    }
}
