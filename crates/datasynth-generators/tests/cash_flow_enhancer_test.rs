use datasynth_core::models::CashFlowCategory;
use datasynth_generators::period_close::{CashFlowEnhancer, CashFlowSourceData};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

#[test]
fn test_operating_activities() {
    let data = CashFlowSourceData {
        depreciation_total: dec!(50_000),
        provision_movements_net: dec!(10_000),
        delta_ar: dec!(20_000), // AR increased → cash outflow
        delta_ap: dec!(15_000), // AP increased → cash inflow
        delta_inventory: dec!(5_000), // Inventory increased → cash outflow
        capex: Decimal::ZERO,
        debt_issuance: Decimal::ZERO,
        debt_repayment: Decimal::ZERO,
        interest_paid: dec!(12_000),
        tax_paid: dec!(30_000),
        dividends_paid: Decimal::ZERO,
        framework: "US_GAAP".to_string(),
    };
    let items = CashFlowEnhancer::generate(&data);

    // Depreciation should be positive (add-back)
    let dep = items.iter().find(|i| i.item_code == "CF-DEP").unwrap();
    assert_eq!(dep.amount, dec!(50_000));
    assert!(matches!(dep.category, CashFlowCategory::Operating));

    // AR increase should be negative
    let dar = items.iter().find(|i| i.item_code == "CF-DAR").unwrap();
    assert!(dar.amount < Decimal::ZERO);

    // Under US GAAP, interest paid is Operating
    let int = items.iter().find(|i| i.item_code == "CF-INT").unwrap();
    assert!(matches!(int.category, CashFlowCategory::Operating));
}

#[test]
fn test_ifrs_interest_in_financing() {
    let data = CashFlowSourceData {
        depreciation_total: Decimal::ZERO,
        provision_movements_net: Decimal::ZERO,
        delta_ar: Decimal::ZERO,
        delta_ap: Decimal::ZERO,
        delta_inventory: Decimal::ZERO,
        capex: Decimal::ZERO,
        debt_issuance: Decimal::ZERO,
        debt_repayment: Decimal::ZERO,
        interest_paid: dec!(12_000),
        tax_paid: Decimal::ZERO,
        dividends_paid: Decimal::ZERO,
        framework: "IFRS".to_string(),
    };
    let items = CashFlowEnhancer::generate(&data);

    // Under IFRS, interest paid goes to Financing
    let int = items.iter().find(|i| i.item_code == "CF-INT-FIN").unwrap();
    assert!(matches!(int.category, CashFlowCategory::Financing));
    // Should NOT have CF-INT (operating) under IFRS
    assert!(items.iter().all(|i| i.item_code != "CF-INT"));
}

#[test]
fn test_financing_activities() {
    let data = CashFlowSourceData {
        depreciation_total: Decimal::ZERO,
        provision_movements_net: Decimal::ZERO,
        delta_ar: Decimal::ZERO,
        delta_ap: Decimal::ZERO,
        delta_inventory: Decimal::ZERO,
        capex: Decimal::ZERO,
        debt_issuance: dec!(500_000),
        debt_repayment: dec!(100_000),
        interest_paid: Decimal::ZERO,
        tax_paid: Decimal::ZERO,
        dividends_paid: dec!(50_000),
        framework: "US_GAAP".to_string(),
    };
    let items = CashFlowEnhancer::generate(&data);

    let debt_in = items.iter().find(|i| i.item_code == "CF-DEBT-IN").unwrap();
    assert_eq!(debt_in.amount, dec!(500_000));
    assert!(matches!(debt_in.category, CashFlowCategory::Financing));

    let div = items.iter().find(|i| i.item_code == "CF-DIV").unwrap();
    assert!(div.amount < Decimal::ZERO); // outflow
}

#[test]
fn test_zero_amounts_skipped() {
    let data = CashFlowSourceData {
        depreciation_total: Decimal::ZERO,
        provision_movements_net: Decimal::ZERO,
        delta_ar: Decimal::ZERO,
        delta_ap: Decimal::ZERO,
        delta_inventory: Decimal::ZERO,
        capex: Decimal::ZERO,
        debt_issuance: Decimal::ZERO,
        debt_repayment: Decimal::ZERO,
        interest_paid: Decimal::ZERO,
        tax_paid: Decimal::ZERO,
        dividends_paid: Decimal::ZERO,
        framework: "US_GAAP".to_string(),
    };
    let items = CashFlowEnhancer::generate(&data);
    assert!(items.is_empty(), "All-zero input should produce no items");
}
