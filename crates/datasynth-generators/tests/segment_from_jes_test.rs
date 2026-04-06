//! Integration tests for SegmentGenerator::generate_from_journal_entries.

use datasynth_core::models::{JournalEntry, JournalEntryLine};
use datasynth_generators::period_close::SegmentGenerator;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

// ---------------------------------------------------------------------------
// Helper: build a simple two-line JE (debit one account, credit another)
// ---------------------------------------------------------------------------
fn make_je(
    id: &str,
    company: &str,
    debit_account: &str,
    credit_account: &str,
    amount: Decimal,
) -> JournalEntry {
    let date = chrono::NaiveDate::from_ymd_opt(2025, 3, 31).unwrap();
    let mut je = JournalEntry::new_simple(id.into(), company.into(), date, id.into());
    je.add_line(JournalEntryLine {
        line_number: 1,
        gl_account: debit_account.into(),
        debit_amount: amount,
        ..Default::default()
    });
    je.add_line(JournalEntryLine {
        line_number: 2,
        gl_account: credit_account.into(),
        credit_amount: amount,
        ..Default::default()
    });
    je
}

// ---------------------------------------------------------------------------
// Core two-company test from the task spec
// ---------------------------------------------------------------------------

#[test]
fn test_segments_from_two_companies() {
    let mut gen = SegmentGenerator::new(42);

    let mut jes = Vec::new();

    // Company CORP: revenue 100K (1xxx debit / 4xxx credit), COGS 60K (5xxx debit / 2xxx credit)
    jes.push(make_je("JE-1", "CORP", "1010", "4000", dec!(100_000)));
    jes.push(make_je("JE-2", "CORP", "5000", "2000", dec!(60_000)));

    // Company SUB1: revenue 50K, COGS 30K
    jes.push(make_je("JE-3", "SUB1", "1010", "4000", dec!(50_000)));
    jes.push(make_je("JE-4", "SUB1", "5000", "2000", dec!(30_000)));

    let companies = vec![
        ("CORP".to_string(), "Corp HQ".to_string()),
        ("SUB1".to_string(), "Subsidiary 1".to_string()),
    ];

    let (segments, recon) =
        gen.generate_from_journal_entries(&jes, &companies, "2025-Q1", dec!(5_000));

    assert_eq!(segments.len(), 2, "Should produce one segment per company");

    // Revenue totals
    let total_revenue: Decimal = segments.iter().map(|s| s.revenue_external).sum();
    assert_eq!(total_revenue, dec!(150_000), "Total revenue mismatch");

    // Profit totals: (100K - 60K) + (50K - 30K) = 60K
    let total_profit: Decimal = segments.iter().map(|s| s.operating_profit).sum();
    assert_eq!(total_profit, dec!(60_000), "Total profit mismatch");

    // Reconciliation
    assert_eq!(recon.segment_revenue_total, dec!(150_000));
    assert_eq!(recon.intersegment_eliminations, dec!(5_000));
    assert_eq!(recon.consolidated_revenue, dec!(145_000));
}

// ---------------------------------------------------------------------------
// Revenue aggregation: multiple revenue JEs for the same company accumulate
// ---------------------------------------------------------------------------

#[test]
fn test_revenue_accumulates_across_multiple_jes() {
    let mut gen = SegmentGenerator::new(1);
    let jes = vec![
        make_je("JE-A", "C001", "1010", "4000", dec!(20_000)),
        make_je("JE-B", "C001", "1010", "4100", dec!(30_000)),
        make_je("JE-C", "C001", "1010", "4200", dec!(50_000)),
    ];
    let companies = vec![("C001".to_string(), "Company One".to_string())];

    let (segments, _) =
        gen.generate_from_journal_entries(&jes, &companies, "2025-Q1", Decimal::ZERO);

    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].revenue_external, dec!(100_000));
}

// ---------------------------------------------------------------------------
// OpEx (6xxx and 7xxx) reduces operating profit
// ---------------------------------------------------------------------------

#[test]
fn test_opex_reduces_operating_profit() {
    let mut gen = SegmentGenerator::new(2);
    let jes = vec![
        // Revenue
        make_je("JE-1", "C001", "1010", "4000", dec!(100_000)),
        // COGS
        make_je("JE-2", "C001", "5000", "2000", dec!(40_000)),
        // OpEx 6xxx
        make_je("JE-3", "C001", "6000", "1010", dec!(15_000)),
        // OpEx 7xxx
        make_je("JE-4", "C001", "7000", "1010", dec!(5_000)),
    ];
    let companies = vec![("C001".to_string(), "One".to_string())];

    let (segments, _) =
        gen.generate_from_journal_entries(&jes, &companies, "2025-Q1", Decimal::ZERO);

    // operating_profit = 100K - 40K - 15K - 5K = 40K
    assert_eq!(segments[0].operating_profit, dec!(40_000));
}

// ---------------------------------------------------------------------------
// Asset and liability aggregation
// ---------------------------------------------------------------------------

#[test]
fn test_asset_and_liability_aggregation() {
    let mut gen = SegmentGenerator::new(3);
    let jes = vec![
        // Asset: debit 1xxx
        make_je("JE-1", "C001", "1010", "2000", dec!(200_000)),
        // Liability: credit 2xxx (debit is to asset, credit is to liability)
        // The 2xxx accounts get credit_amount, making liabilities positive
    ];
    let companies = vec![("C001".to_string(), "One".to_string())];

    let (segments, _) =
        gen.generate_from_journal_entries(&jes, &companies, "2025-Q1", Decimal::ZERO);

    assert_eq!(segments.len(), 1);
    // Assets: debit 1010 = 200K → net assets for 1xxx = debit - credit = 200K
    assert_eq!(segments[0].total_assets, dec!(200_000));
    // Liabilities: credit 2000 = 200K → net liabilities for 2xxx = credit - debit = 200K
    assert_eq!(segments[0].total_liabilities, dec!(200_000));
}

// ---------------------------------------------------------------------------
// Company filtering: JEs from other companies are excluded
// ---------------------------------------------------------------------------

#[test]
fn test_company_filtering_isolates_data() {
    let mut gen = SegmentGenerator::new(4);
    let jes = vec![
        make_je("JE-1", "CORP", "1010", "4000", dec!(100_000)),
        make_je("JE-2", "OTHER", "1010", "4000", dec!(999_999)), // must not bleed into CORP
    ];
    let companies = vec![("CORP".to_string(), "Corp".to_string())];

    let (segments, _) =
        gen.generate_from_journal_entries(&jes, &companies, "2025-Q1", Decimal::ZERO);

    assert_eq!(segments.len(), 1);
    assert_eq!(
        segments[0].revenue_external,
        dec!(100_000),
        "OTHER company data must not bleed into CORP segment"
    );
}

// ---------------------------------------------------------------------------
// Reconciliation math: consolidated = segment_total - eliminations
// ---------------------------------------------------------------------------

#[test]
fn test_reconciliation_math() {
    let mut gen = SegmentGenerator::new(5);
    let jes = vec![
        make_je("JE-1", "A", "1010", "4000", dec!(80_000)),
        make_je("JE-2", "B", "1010", "4000", dec!(60_000)),
    ];
    let companies = vec![
        ("A".to_string(), "Alpha".to_string()),
        ("B".to_string(), "Beta".to_string()),
    ];
    let elimination = dec!(10_000);

    let (segments, recon) =
        gen.generate_from_journal_entries(&jes, &companies, "2025-Q1", elimination);

    let expected_total: Decimal = segments.iter().map(|s| s.revenue_external).sum();
    assert_eq!(recon.segment_revenue_total, expected_total);
    assert_eq!(
        recon.consolidated_revenue,
        recon.segment_revenue_total - elimination
    );
    assert_eq!(
        recon.consolidated_profit,
        recon.segment_profit_total + recon.corporate_overhead
    );
    assert_eq!(
        recon.consolidated_assets,
        recon.segment_assets_total + recon.unallocated_assets
    );
}

// ---------------------------------------------------------------------------
// Empty JE slice: segments have zero amounts, reconciliation sums are zero
// ---------------------------------------------------------------------------

#[test]
fn test_empty_jes_produces_zero_segments() {
    let mut gen = SegmentGenerator::new(6);
    let companies = vec![("C001".to_string(), "One".to_string())];

    let (segments, recon) = gen.generate_from_journal_entries(&[], &companies, "2025-Q1", Decimal::ZERO);

    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].revenue_external, Decimal::ZERO);
    assert_eq!(segments[0].operating_profit, Decimal::ZERO);
    assert_eq!(recon.segment_revenue_total, Decimal::ZERO);
    assert_eq!(recon.consolidated_revenue, Decimal::ZERO);
}

// ---------------------------------------------------------------------------
// Period and segment_type are set correctly on outputs
// ---------------------------------------------------------------------------

#[test]
fn test_period_and_segment_type_propagated() {
    use datasynth_core::models::SegmentType;

    let mut gen = SegmentGenerator::new(7);
    let jes = vec![make_je("JE-1", "C001", "1010", "4000", dec!(1_000))];
    let companies = vec![("C001".to_string(), "One".to_string())];

    let (segments, recon) =
        gen.generate_from_journal_entries(&jes, &companies, "2025-Q2", Decimal::ZERO);

    assert_eq!(recon.period, "2025-Q2");
    for seg in &segments {
        assert_eq!(seg.period, "2025-Q2");
        assert_eq!(seg.segment_type, SegmentType::LegalEntity);
    }
}
