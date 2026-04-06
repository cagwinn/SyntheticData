//! Integration tests for the enhanced financial statement notes (Task 7).
//!
//! Verifies that `NotesGenerator::generate_enhanced_notes` produces 4 notes
//! backed by inventory, debt, hedge, and provision data.

use datasynth_generators::period_close::notes_generator::{EnhancedNotesContext, NotesGenerator};
use rust_decimal::Decimal;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn default_enhanced_context() -> EnhancedNotesContext {
    EnhancedNotesContext {
        entity_code: "C001".to_string(),
        period: "FY2024".to_string(),
        currency: "USD".to_string(),

        // Inventory
        finished_goods_value: Decimal::new(1_200_000, 0),
        wip_value: Decimal::new(400_000, 0),
        raw_materials_value: Decimal::new(600_000, 0),

        // Debt — 3 instruments
        debt_instruments: vec![
            (
                "Senior Secured Loan".to_string(),
                Decimal::new(5_000_000, 0),
                "2027-06-30".to_string(),
            ),
            (
                "Revolving Credit Facility".to_string(),
                Decimal::new(2_000_000, 0),
                "2026-12-31".to_string(),
            ),
            (
                "Subordinated Notes".to_string(),
                Decimal::new(1_500_000, 0),
                "2029-03-31".to_string(),
            ),
        ],

        // Hedges
        hedge_count: 10,
        effective_hedges: 8,
        total_notional: Decimal::new(20_000_000, 0),
        total_fair_value: Decimal::new(350_000, 0),

        // Provisions
        provision_movements: vec![
            (
                "Warranty Provisions".to_string(),
                Decimal::new(500_000, 0),
                Decimal::new(200_000, 0),
                Decimal::new(650_000, 0),
            ),
            (
                "ECL Provision".to_string(),
                Decimal::new(300_000, 0),
                Decimal::new(100_000, 0),
                Decimal::new(380_000, 0),
            ),
        ],
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_generate_enhanced_notes_returns_four_notes() {
    let mut gen = NotesGenerator::new(42);
    let ctx = default_enhanced_context();
    let notes = gen.generate_enhanced_notes(&ctx, 9);
    assert_eq!(notes.len(), 4, "Expected exactly 4 enhanced notes");
}

#[test]
fn test_note_numbers_start_from_given_offset() {
    let mut gen = NotesGenerator::new(42);
    let ctx = default_enhanced_context();
    let notes = gen.generate_enhanced_notes(&ctx, 9);
    for (i, note) in notes.iter().enumerate() {
        let expected = 9 + i as u32;
        assert_eq!(
            note.note_number, expected,
            "Note at index {i} should be numbered {expected}, got {}",
            note.note_number
        );
    }
}

// --- Inventory note ---

#[test]
fn test_inventory_note_correct_total() {
    let mut gen = NotesGenerator::new(42);
    let ctx = default_enhanced_context();
    let notes = gen.generate_enhanced_notes(&ctx, 9);

    // Note index 0 is the inventory note
    let inv_note = &notes[0];
    assert!(
        inv_note.title.to_lowercase().contains("inventor"),
        "First enhanced note should be the inventory note, got '{}'",
        inv_note.title
    );

    // Find the table and locate the Total row
    let table = &inv_note.content_sections[0].tables[0];
    let total_row = table
        .rows
        .iter()
        .find(|row| matches!(&row[0], datasynth_core::models::NoteTableValue::Text(t) if t == "Total"))
        .expect("Inventory table must have a Total row");

    let expected_total = Decimal::new(1_200_000 + 400_000 + 600_000, 0);
    match &total_row[1] {
        datasynth_core::models::NoteTableValue::Amount(amt) => {
            assert_eq!(
                *amt, expected_total,
                "Inventory total should be {expected_total}, got {amt}"
            );
        }
        other => panic!("Expected Amount cell for inventory total, got {other:?}"),
    }
}

#[test]
fn test_inventory_note_has_three_category_rows_plus_total() {
    let mut gen = NotesGenerator::new(42);
    let ctx = default_enhanced_context();
    let notes = gen.generate_enhanced_notes(&ctx, 9);
    let table = &notes[0].content_sections[0].tables[0];
    // Finished Goods + WIP + Raw Materials + Total = 4 rows
    assert_eq!(
        table.rows.len(),
        4,
        "Inventory table should have 4 rows (3 categories + total)"
    );
}

// --- Debt / borrowings note ---

#[test]
fn test_debt_note_has_one_row_per_instrument() {
    let mut gen = NotesGenerator::new(42);
    let ctx = default_enhanced_context();
    let notes = gen.generate_enhanced_notes(&ctx, 9);

    let debt_note = &notes[1];
    assert!(
        debt_note.title.to_lowercase().contains("borrowing")
            || debt_note.title.to_lowercase().contains("debt"),
        "Second enhanced note should be the debt/borrowings note, got '{}'",
        debt_note.title
    );

    let table = &debt_note.content_sections[0].tables[0];
    assert_eq!(
        table.rows.len(),
        ctx.debt_instruments.len(),
        "Debt table should have one row per instrument ({}), got {}",
        ctx.debt_instruments.len(),
        table.rows.len()
    );
}

#[test]
fn test_debt_note_row_count_matches_context() {
    let mut gen = NotesGenerator::new(42);
    let mut ctx = default_enhanced_context();
    // Override with 5 instruments
    ctx.debt_instruments = (1u32..=5)
        .map(|i| {
            (
                format!("Debt {i}"),
                Decimal::new(i as i64 * 1_000_000, 0),
                format!("203{i}-01-01"),
            )
        })
        .collect();

    let notes = gen.generate_enhanced_notes(&ctx, 9);
    let table = &notes[1].content_sections[0].tables[0];
    assert_eq!(table.rows.len(), 5);
}

// --- Hedge accounting note ---

#[test]
fn test_hedge_note_shows_effectiveness_rate() {
    let mut gen = NotesGenerator::new(42);
    let ctx = default_enhanced_context();
    let notes = gen.generate_enhanced_notes(&ctx, 9);

    let hedge_note = &notes[2];
    assert!(
        hedge_note.title.to_lowercase().contains("hedge"),
        "Third enhanced note should be the hedge accounting note, got '{}'",
        hedge_note.title
    );

    let table = &hedge_note.content_sections[0].tables[0];

    // Find the effectiveness rate row (last key-value pair)
    let effectiveness_row = table
        .rows
        .iter()
        .find(|row| {
            matches!(&row[0], datasynth_core::models::NoteTableValue::Text(t)
                if t.to_lowercase().contains("effectiveness"))
        })
        .expect("Hedge table must contain an effectiveness rate row");

    // The value cell should be "80.0%" (8 out of 10 = 80%)
    match &effectiveness_row[1] {
        datasynth_core::models::NoteTableValue::Text(rate_str) => {
            assert!(
                rate_str.contains("80"),
                "Effectiveness rate should display 80.0%, got '{rate_str}'"
            );
        }
        other => panic!("Expected Text cell for effectiveness rate, got {other:?}"),
    }
}

#[test]
fn test_hedge_note_perfect_effectiveness_when_all_effective() {
    let mut gen = NotesGenerator::new(42);
    let mut ctx = default_enhanced_context();
    ctx.hedge_count = 5;
    ctx.effective_hedges = 5;

    let notes = gen.generate_enhanced_notes(&ctx, 9);
    let table = &notes[2].content_sections[0].tables[0];

    let effectiveness_row = table
        .rows
        .iter()
        .find(|row| {
            matches!(&row[0], datasynth_core::models::NoteTableValue::Text(t)
                if t.to_lowercase().contains("effectiveness"))
        })
        .expect("Hedge table must contain an effectiveness rate row");

    match &effectiveness_row[1] {
        datasynth_core::models::NoteTableValue::Text(rate_str) => {
            assert!(
                rate_str.contains("100"),
                "Effectiveness rate should display 100.0%, got '{rate_str}'"
            );
        }
        other => panic!("Expected Text cell for effectiveness rate, got {other:?}"),
    }
}

// --- Provisions rollforward note ---

#[test]
fn test_provisions_note_has_correct_rollforward() {
    let mut gen = NotesGenerator::new(42);
    let ctx = default_enhanced_context();
    let notes = gen.generate_enhanced_notes(&ctx, 9);

    let prov_note = &notes[3];
    assert!(
        prov_note.title.to_lowercase().contains("provision"),
        "Fourth enhanced note should be the provisions rollforward note, got '{}'",
        prov_note.title
    );

    let table = &prov_note.content_sections[0].tables[0];

    // Should have one row per provision type + one Total row
    let expected_rows = ctx.provision_movements.len() + 1;
    assert_eq!(
        table.rows.len(),
        expected_rows,
        "Provisions table should have {expected_rows} rows, got {}",
        table.rows.len()
    );

    // Verify the Total row sums correctly
    let total_row = table
        .rows
        .last()
        .expect("Provisions table should have a Total row");

    let expected_opening: Decimal = ctx.provision_movements.iter().map(|(_, o, _, _)| *o).sum();
    let expected_additions: Decimal = ctx.provision_movements.iter().map(|(_, _, a, _)| *a).sum();
    let expected_closing: Decimal = ctx.provision_movements.iter().map(|(_, _, _, c)| *c).sum();

    match (&total_row[1], &total_row[2], &total_row[3]) {
        (
            datasynth_core::models::NoteTableValue::Amount(opening),
            datasynth_core::models::NoteTableValue::Amount(additions),
            datasynth_core::models::NoteTableValue::Amount(closing),
        ) => {
            assert_eq!(*opening, expected_opening, "Total opening mismatch");
            assert_eq!(*additions, expected_additions, "Total additions mismatch");
            assert_eq!(*closing, expected_closing, "Total closing mismatch");
        }
        other => panic!("Expected Amount cells in the Total row, got {other:?}"),
    }
}

#[test]
fn test_provisions_note_has_four_columns() {
    let mut gen = NotesGenerator::new(42);
    let ctx = default_enhanced_context();
    let notes = gen.generate_enhanced_notes(&ctx, 9);
    let table = &notes[3].content_sections[0].tables[0];
    assert_eq!(
        table.headers.len(),
        4,
        "Provisions table should have 4 columns (type, opening, additions, closing)"
    );
}

// --- General properties ---

#[test]
fn test_all_enhanced_notes_have_titles_and_content() {
    let mut gen = NotesGenerator::new(42);
    let ctx = default_enhanced_context();
    let notes = gen.generate_enhanced_notes(&ctx, 9);
    for note in &notes {
        assert!(
            !note.title.is_empty(),
            "Note {} has empty title",
            note.note_number
        );
        assert!(
            !note.content_sections.is_empty(),
            "Note '{}' has no content sections",
            note.title
        );
        assert!(
            !note.content_sections[0].tables.is_empty(),
            "Note '{}' has no tables in first section",
            note.title
        );
    }
}

#[test]
fn test_enhanced_notes_deterministic() {
    let ctx = default_enhanced_context();
    let notes1 = NotesGenerator::new(99).generate_enhanced_notes(&ctx, 9);
    let notes2 = NotesGenerator::new(99).generate_enhanced_notes(&ctx, 9);
    assert_eq!(notes1.len(), notes2.len());
    for (a, b) in notes1.iter().zip(notes2.iter()) {
        assert_eq!(a.note_number, b.note_number);
        assert_eq!(a.title, b.title);
    }
}

#[test]
fn test_enhanced_notes_integrate_with_existing_notes() {
    use chrono::NaiveDate;
    use datasynth_generators::period_close::notes_generator::NotesGeneratorContext;

    let mut gen = NotesGenerator::new(42);

    let base_ctx = NotesGeneratorContext {
        entity_code: "C001".to_string(),
        framework: "IFRS".to_string(),
        period: "FY2024".to_string(),
        period_end: NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
        currency: "USD".to_string(),
        revenue_contract_count: 10,
        revenue_amount: Some(Decimal::new(5_000_000, 0)),
        ..NotesGeneratorContext::default()
    };

    let base_notes = gen.generate(&base_ctx);
    let base_count = base_notes.len();

    let enhanced_ctx = default_enhanced_context();
    let enhanced_notes = gen.generate_enhanced_notes(&enhanced_ctx, base_count as u32 + 1);

    // Verify no note number collisions
    let all_numbers: Vec<u32> = base_notes
        .iter()
        .chain(enhanced_notes.iter())
        .map(|n| n.note_number)
        .collect();

    let unique_count = {
        let mut sorted = all_numbers.clone();
        sorted.sort();
        sorted.dedup();
        sorted.len()
    };

    assert_eq!(
        unique_count,
        all_numbers.len(),
        "Combined note numbers must be unique — found duplicates"
    );
}
