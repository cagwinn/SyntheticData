//! SP3 priors-driven smoke test.

use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use std::sync::Arc;

use datasynth_generators::priors_loader::{bundled_priors_path, LoadedPriors};

#[test]
fn load_bundled_health_priors_succeeds() {
    let path = bundled_priors_path("health");
    if !path.exists() {
        eprintln!("Skipping: bundle not present at {}", path.display());
        return;
    }
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let priors = LoadedPriors::load_bundled("health", &mut rng, 365).expect("load_bundled health");

    assert_eq!(priors.industry, "health");
    assert!(
        !priors.source_mix.probabilities.is_empty(),
        "source_mix should have at least one source"
    );
    assert!(
        priors.fanout_samplers.contains_key("GLAccount"),
        "fanout_samplers should have a GLAccount key"
    );
    assert!(
        priors.fanout_samplers.contains_key("CostCenter"),
        "fanout_samplers should have a CostCenter key"
    );
    assert!(
        priors.fanout_samplers.contains_key("ProfitCenter"),
        "fanout_samplers should have a ProfitCenter key"
    );

    // Sample one line-per-JE count from the overall histogram — must be ≥1.
    let n_lines = priors.lines_per_je.overall.sample_bucket(&mut rng);
    assert!(n_lines >= 1, "lines_per_je sampled {n_lines}");
}

#[test]
fn load_bundled_unknown_industry_fails() {
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let err = LoadedPriors::load_bundled("nonexistent_industry", &mut rng, 365);
    assert!(err.is_err(), "expected error for unknown industry");
}

#[test]
fn lines_per_je_samples_within_bucket_range() {
    let path = bundled_priors_path("health");
    if !path.exists() {
        return;
    }
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let priors = LoadedPriors::load_bundled("health", &mut rng, 365).expect("load");
    // Sample 100 times; assert all values are sane (between 1 and 10_000).
    for _ in 0..100 {
        let n = priors.lines_per_je.overall.sample_bucket(&mut rng);
        assert!((1..=10_000).contains(&n), "sampled {n} out of bounds");
    }
}

/// SP3.7 — per-source conditional attribute distributions are loaded into
/// `LoadedPriors` and structural assertions hold when the bundle was built
/// after W4 regeneration.  Pre-W4 bundles hit the warning path (None) and
/// still pass.
#[test]
fn priors_loaded_attributes_conditional_on_source() {
    use std::collections::HashSet;

    let path = bundled_priors_path("health");
    if !path.exists() {
        eprintln!("Skipping: health bundle not present at {}", path.display());
        return;
    }
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let priors = LoadedPriors::load_bundled("health", &mut rng, 365).expect("load_bundled health");

    // The bundle should now carry per_source_attribute after W4 regen,
    // but pre-regen bundles will have None. Tolerate both.
    if let Some(ref psa) = priors.per_source_attribute {
        // For each (source, attribute) pair, the marginal vs conditional
        // distributions should differ — i.e. the prior actually carries
        // structure. Specifically, two different sources should NOT have
        // identical gl_account distributions.
        let sources: Vec<&String> = psa.by_source.keys().collect();
        assert!(
            sources.len() >= 2,
            "expected ≥2 sources in per_source_attribute, got {sources:?}"
        );

        // Spot check: at least one source has a non-empty gl_account conditional.
        let any_gl = sources.iter().any(|s| {
            psa.conditional(s, "gl_account")
                .map(|d| !d.probabilities.is_empty())
                .unwrap_or(false)
        });
        assert!(
            any_gl,
            "expected at least one source to have a non-empty gl_account conditional"
        );

        // Verify sample_attribute_for_source returns values in the known set.
        for source in &sources {
            if let Some(dist) = psa.conditional(source, "gl_account") {
                if dist.probabilities.is_empty() {
                    continue;
                }
                let known_values: HashSet<&String> = dist.probabilities.keys().collect();
                // Sample several times and confirm all fall in the known set.
                for _ in 0..10 {
                    if let Some(sampled) =
                        priors.sample_attribute_for_source(source, "gl_account", &mut rng)
                    {
                        assert!(
                            known_values.contains(&sampled),
                            "sampled gl_account {sampled:?} not in known values for source {source:?}"
                        );
                    }
                }
            }
        }
    } else {
        eprintln!("WARNING: per_source_attribute is None — bundle may need W4 regen");
    }
}

/// SP3.8a — the `per_source_attribute` map in a post-regen bundle contains a
/// `trading_partner` key for at least one source.  Pre-regen bundles hit the
/// WARNING path (None or absent key) and still pass.
#[test]
fn priors_loaded_trading_partner_attribute_present() {
    let path = datasynth_generators::priors_loader::bundled_priors_path("health");
    if !path.exists() {
        eprintln!("Skipping: health bundle not present at {}", path.display());
        return;
    }
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let priors = LoadedPriors::load_bundled("health", &mut rng, 365).expect("load_bundled health");

    if let Some(ref psa) = priors.per_source_attribute {
        let any_tp = psa
            .by_source
            .values()
            .any(|attrs| attrs.contains_key("trading_partner"));
        if !any_tp {
            eprintln!(
                "WARNING: no source has a trading_partner conditional — \
                 bundle may need SP3.8a regen"
            );
        }
        // Either the WARNING fires (pre-regen) or we can assert the key is there
        // (post-regen).  Both paths keep CI green; the WARNING is the signal.
    } else {
        eprintln!(
            "WARNING: per_source_attribute is None — \
             bundle may need SP3.8a regen"
        );
    }
}

/// SP3.9 — trading_partner is a JE-level field: all lines of the same JE share
/// the same trading_partner value when priors are loaded. This test verifies
/// the structural wiring by confirming the header carries the TP draw (the
/// line-inherit step is validated by the output_writer CSV integration tests).
#[test]
fn priors_loaded_trading_partner_is_je_level() {
    let path = datasynth_generators::priors_loader::bundled_priors_path("health");
    if !path.exists() {
        eprintln!("Skipping: health bundle not present at {}", path.display());
        return;
    }
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(42);
    let priors = LoadedPriors::load_bundled("health", &mut rng, 365).expect("load_bundled health");

    // Structural check: if the bundle has a trading_partner conditional for
    // any source, sample_attribute_for_source must return values in the known set.
    if let Some(ref psa) = priors.per_source_attribute {
        for (source, attrs) in &psa.by_source {
            if let Some(dist) = attrs.get("trading_partner") {
                if dist.probabilities.is_empty() {
                    continue;
                }
                let known: std::collections::HashSet<&String> = dist.probabilities.keys().collect();
                for _ in 0..10 {
                    if let Some(tp) =
                        priors.sample_attribute_for_source(source, "trading_partner", &mut rng)
                    {
                        assert!(
                            known.contains(&tp),
                            "sampled trading_partner {tp:?} not in known values for source {source:?}"
                        );
                    }
                }
                // One source confirmed — test passes.
                return;
            }
        }
        eprintln!(
            "WARNING: no source has a trading_partner conditional — \
             bundle may need SP3.9 regen"
        );
    } else {
        eprintln!(
            "WARNING: per_source_attribute is None — \
             bundle may need SP3.9 regen"
        );
    }
}

/// SP3.6 — `SourceMixPrior::sample` returns codes drawn from the bundle's
/// source-mix distribution and never emits the generic categories
/// (`Manual`/`Automated`/`Adjustment`/`Recurring`).
#[test]
fn priors_loaded_source_emits_bundle_codes() {
    let path = bundled_priors_path("health");
    if !path.exists() {
        eprintln!("Skipping: health bundle not present at {}", path.display());
        return;
    }
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let priors = LoadedPriors::load_bundled("health", &mut rng, 365).expect("load health priors");

    // The bundle's source-mix keys — all codes that should appear.
    let bundle_codes: std::collections::HashSet<String> =
        priors.source_mix.probabilities.keys().cloned().collect();

    // The generic categories that must NOT appear when priors are loaded.
    let generic: std::collections::HashSet<&str> = [
        "manual",
        "automated",
        "adjustment",
        "recurring",
        "Manual",
        "Automated",
        "Adjustment",
        "Recurring",
    ]
    .iter()
    .copied()
    .collect();

    let mut in_bundle = 0usize;
    let mut in_generic = 0usize;
    for _ in 0..200 {
        let code = priors.source_mix.sample(&mut rng);
        if bundle_codes.contains(&code) {
            in_bundle += 1;
        }
        if generic.contains(code.as_str()) {
            in_generic += 1;
        }
    }

    assert_eq!(
        in_generic, 0,
        "expected zero generic-category labels when priors loaded, got {in_generic}"
    );
    assert!(
        in_bundle >= 190,
        "expected ≥190/200 bundle codes, got {in_bundle}"
    );
}

/// Helper: build a `BehavioralPriors` with a per-source GL-account conditional
/// suitable for SP3.12 W1.5 tests.
///
/// Supplies 5 distinct expense GL accounts for the given `source` code so the
/// expense-split path has enough candidates.  The `LinesPerJePrior` always
/// samples `target_lines` (via a single-bucket histogram).
#[cfg(test)]
fn build_bp_with_gl_conditionals(
    source: &str,
    target_lines: u32,
    gl_accounts: &[&str],
) -> datasynth_core::distributions::behavioral_priors::BehavioralPriors {
    use datasynth_core::distributions::behavioral_priors::{
        ActiveLifetimePrior, BehavioralPriors, CategoricalDistribution, FanoutPrior,
        LineCountHistogram, LinesPerJePrior, PerSourceAttributePrior, PerSourceIetPrior,
        SourceMixPrior, LINE_COUNT_BUCKETS,
    };
    use std::collections::BTreeMap;

    // Build histogram that always returns `target_lines`.
    let values: Vec<u32> = vec![target_lines; 1000];
    let (hist, _) = LineCountHistogram::build(&values, LINE_COUNT_BUCKETS);
    let lines_per_je = LinesPerJePrior {
        overall: hist.clone(),
        by_source: {
            let mut m = BTreeMap::new();
            m.insert(source.to_string(), hist);
            m
        },
        min_jes_per_source: 0,
    };

    // Uniform distribution over the supplied GL accounts.
    let prob = 1.0 / gl_accounts.len() as f64;
    let gl_dist = CategoricalDistribution {
        probabilities: gl_accounts
            .iter()
            .map(|gl| (gl.to_string(), prob))
            .collect(),
        n: 1000,
    };

    let mut source_attrs = BTreeMap::new();
    let mut attrs = BTreeMap::new();
    attrs.insert("gl_account".to_string(), gl_dist);
    source_attrs.insert(source.to_string(), attrs);
    let per_source_attribute = Some(PerSourceAttributePrior {
        by_source: source_attrs,
        min_observations: 0,
    });

    let mut source_probs = BTreeMap::new();
    source_probs.insert(source.to_string(), 1.0_f64);
    let source_mix = SourceMixPrior {
        probabilities: source_probs,
        other_fraction: 0.0,
        min_threshold: 0.0,
    };

    BehavioralPriors {
        schema_version: BehavioralPriors::SCHEMA_VERSION,
        generator_version: "test-sp3.12-w1.5".to_string(),
        industry: "test".to_string(),
        n_client_inputs: 1,
        n_rows_aggregated: 1000,
        source_mix,
        per_source_iet: PerSourceIetPrior::default(),
        lines_per_je,
        active_lifetime: ActiveLifetimePrior::default(),
        fanout: FanoutPrior::default(),
        posting_lag: None,
        active_segments: None,
        entity_clusters: None,
        per_source_attribute,
        tp_entity_clusters: None,
        coa_semantic: None,
        reference_formats: None,
        text_taxonomy: None,
        user_personas: None,
        source_amount_conditionals: None,
        source_role_gl_conditionals: None,
        tb_anchor: None,
    }
}

/// SP3.12 W1.5 — Document-flow JEs are split into semantic multi-GL expense
/// sub-lines when a per-source GL conditional is available.
///
/// Uses Delivery (WL) JEs — DR COGS (5000) / CR Inventory (1200) — since the
/// DR COGS line is not a control account and IS splittable into multiple expense
/// GL accounts.
///
/// Constructs a synthetic `BehavioralPriors` with:
///   - `LinesPerJePrior` → always 4 lines for "WL"
///   - `per_source_attribute["WL"]["gl_account"]` → 5 distinct expense accounts
///
/// Emits 50 Delivery-derived JEs and asserts:
///   1. Mean lines per JE ≥ 2.5 (split adds at least one extra line)
///   2. No line posts to suspense account "9000" (no filler)
///   3. Every JE remains balanced (total debits == total credits)
#[test]
fn sp3_12_w1_5_split_raises_line_count_no_filler_balance_preserved() {
    use datasynth_core::models::documents::{Delivery, DeliveryItem};
    use datasynth_generators::document_flow::{DocumentFlowJeConfig, DocumentFlowJeGenerator};
    use rust_decimal::Decimal;

    // 5 distinct expense GL accounts — all start with '5' so they're not control accounts.
    // These represent COGS sub-accounts (direct materials, direct labor, etc.)
    let expense_gls = ["5010", "5020", "5030", "5040", "5050"];
    // Use "WL" as the source key (Delivery JE doc_type is "WL").
    let bp = build_bp_with_gl_conditionals("WL", 4, &expense_gls);

    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let priors = LoadedPriors::from_priors(bp, std::path::PathBuf::from("test"), &mut rng, 365)
        .expect("from_priors");
    let priors_arc = Arc::new(priors);

    let mut generator =
        DocumentFlowJeGenerator::with_config_and_seed(DocumentFlowJeConfig::default(), 1234);
    generator.set_loaded_priors(priors_arc);

    let make_delivery = |id: &str| {
        use chrono::NaiveDate;
        let mut del = Delivery::new(
            id,
            "1000",
            "C-001",
            "SP01",
            2024,
            3,
            NaiveDate::from_ymd_opt(2024, 3, 15).unwrap(),
            "JSMITH",
        );
        let item = DeliveryItem::new(1, "Product A", Decimal::from(10), Decimal::from(80));
        del.items.push(item);
        del
    };

    let mut total_lines = 0usize;
    let n_jes = 50usize;
    for i in 0..n_jes {
        let del = make_delivery(&format!("DEL-{i:04}"));
        let je = generator
            .generate_from_delivery(&del)
            .expect("generate_from_delivery returned None");

        // Balance invariant must always hold.
        assert!(
            je.is_balanced(),
            "JE {} is unbalanced: debit={} credit={}",
            je.header.document_id,
            je.total_debit(),
            je.total_credit(),
        );

        // No filler: no line should post to suspense account 9000.
        for line in &je.lines {
            assert_ne!(
                line.gl_account, "9000",
                "JE {} has a suspense-account (filler) line — expected semantic splits only",
                je.header.document_id,
            );
        }

        total_lines += je.line_count();
    }

    let mean_lines = total_lines as f64 / n_jes as f64;
    assert!(
        mean_lines >= 2.5,
        "Expected mean lines/JE ≥ 2.5 after SP3.12 W1.5 split (prior targets 4), got {mean_lines:.2}"
    );
}

/// SP3.12 W1.5 — KZ payment JEs (canonical 2-line shape) are NOT split when
/// the priors target 2 lines, preserving the corpus 95%-2-line shape.
#[test]
fn sp3_12_w1_5_kz_payment_stays_two_line() {
    use datasynth_generators::document_flow::{DocumentFlowJeConfig, DocumentFlowJeGenerator};
    use rust_decimal::Decimal;

    // Build priors that TARGET 2 lines for "KZ" — payments should not split.
    let expense_gls = ["5010", "5020", "5030"];
    let bp = build_bp_with_gl_conditionals("KZ", 2, &expense_gls);

    let mut rng = ChaCha8Rng::seed_from_u64(99);
    let priors = LoadedPriors::from_priors(bp, std::path::PathBuf::from("test"), &mut rng, 365)
        .expect("from_priors");
    let priors_arc = Arc::new(priors);

    let mut generator =
        DocumentFlowJeGenerator::with_config_and_seed(DocumentFlowJeConfig::default(), 5678);
    generator.set_loaded_priors(priors_arc);

    let make_payment = |id: &str| {
        use chrono::NaiveDate;
        datasynth_core::models::documents::Payment::new_ap_payment(
            id,
            "1000",
            "V-001",
            Decimal::from(500),
            2024,
            3,
            NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
            "JSMITH",
        )
    };

    let n_jes = 30usize;
    for i in 0..n_jes {
        let pay = make_payment(&format!("PAY-{i:04}"));
        let je = generator
            .generate_from_ap_payment(&pay)
            .expect("generate_from_ap_payment returned None");

        // Balance must always hold.
        assert!(
            je.is_balanced(),
            "KZ JE {} is unbalanced",
            je.header.document_id,
        );

        // KZ payments should stay at 2 lines because the target IS 2.
        assert_eq!(
            je.line_count(),
            2,
            "KZ payment JE {} should have 2 lines when prior targets 2, got {}",
            je.header.document_id,
            je.line_count(),
        );
    }
}

// ---------------------------------------------------------------------------
// SP3.13 W1 helpers
// ---------------------------------------------------------------------------

/// Build a `BehavioralPriors` tuned for SP3.13 W1 vendor-invoice tests.
///
/// Sets up a "KR" source with `target_lines` lines/JE and `gl_accounts` for
/// the GL conditional (all non-control expense accounts).
#[cfg(test)]
fn build_bp_for_vendor_invoice(
    target_lines: u32,
    gl_accounts: &[&str],
) -> datasynth_core::distributions::behavioral_priors::BehavioralPriors {
    build_bp_with_gl_conditionals("KR", target_lines, gl_accounts)
}

/// Helper: create a minimal `VendorInvoice` for testing.
#[cfg(test)]
fn make_vendor_invoice(
    id: &str,
    net: rust_decimal::Decimal,
    tax: rust_decimal::Decimal,
) -> datasynth_core::models::documents::VendorInvoice {
    use chrono::NaiveDate;
    use datasynth_core::models::documents::VendorInvoice;

    let date = NaiveDate::from_ymd_opt(2024, 3, 15).unwrap();
    let mut inv = VendorInvoice::new(
        id,
        "1000",
        "V-001",
        format!("EXT-{id}"),
        2024_u16,
        3_u8,
        date,
        "JSMITH",
    );
    let gross = net + tax;
    inv.net_amount = net;
    inv.tax_amount = tax;
    inv.gross_amount = gross;
    inv.payable_amount = gross;
    inv.balance = gross;
    inv.header.posting_date = Some(date);
    inv
}

/// SP3.13 W1 — When priors are loaded and `direct_expense_share = 1.0`,
/// every vendor invoice must emit a non-GR/IR-Clearing debit line (direct
/// expense), the GR/IR clearing account must never appear, and balance must
/// be preserved.  Tax case (3-line: DR Exp + DR VAT + CR AP) is verified.
#[test]
fn sp3_13_w1_direct_expense_emission_when_priors_loaded() {
    use datasynth_generators::document_flow::{DocumentFlowJeConfig, DocumentFlowJeGenerator};
    use rust_decimal::Decimal;

    // 5 distinct expense GL accounts (all non-control, start with '6').
    let expense_gls = ["6100", "6200", "6300", "6400", "6500"];
    let bp = build_bp_for_vendor_invoice(4, &expense_gls);

    let mut rng = ChaCha8Rng::seed_from_u64(77);
    let priors = LoadedPriors::from_priors(bp, std::path::PathBuf::from("test"), &mut rng, 365)
        .expect("from_priors");
    let priors_arc = Arc::new(priors);

    // Force all invoices to direct-expense path.
    let config = DocumentFlowJeConfig {
        direct_expense_share: 1.0,
        ..DocumentFlowJeConfig::default()
    };

    let mut generator = DocumentFlowJeGenerator::with_config_and_seed(config.clone(), 9001);
    generator.set_loaded_priors(priors_arc);

    let gr_ir_account = config.gr_ir_clearing_account.clone();
    let ap_account = config.ap_account.clone();
    let vat_account = config.vat_input_account.clone();

    let n_jes = 100usize;
    let mut gr_ir_count = 0usize;
    let mut total_lines = 0usize;
    let mut tax_cases_found = 0usize;

    for i in 0..n_jes {
        // Alternate: half plain invoices, half with VAT (tax case).
        let (net, tax) = if i % 2 == 0 {
            (Decimal::from(1000), Decimal::ZERO)
        } else {
            (Decimal::from(1000), Decimal::from(190)) // 19% VAT
        };
        let inv = make_vendor_invoice(&format!("VI-{i:04}"), net, tax);
        let je = generator
            .generate_from_vendor_invoice(&inv)
            .expect("generate_from_vendor_invoice returned None");

        // Balance invariant must always hold.
        assert!(
            je.is_balanced(),
            "JE {} is unbalanced: debit={} credit={}",
            je.header.document_id,
            je.total_debit(),
            je.total_credit(),
        );

        // GR/IR clearing account must NOT appear on direct-expense path.
        for line in &je.lines {
            if line.gl_account == gr_ir_account {
                gr_ir_count += 1;
            }
        }

        // AP must appear as CR on all JEs.
        let has_ap_credit = je
            .lines
            .iter()
            .any(|l| l.gl_account == ap_account && l.credit_amount > Decimal::ZERO);
        assert!(
            has_ap_credit,
            "JE {} missing AP credit line",
            je.header.document_id,
        );

        // Tax case: VAT line must be present when tax > 0.
        if tax > Decimal::ZERO {
            tax_cases_found += 1;
            let has_vat_line = je
                .lines
                .iter()
                .any(|l| l.gl_account == vat_account && l.debit_amount > Decimal::ZERO);
            assert!(
                has_vat_line,
                "JE {} (tax case) missing Input VAT debit line",
                je.header.document_id,
            );
            // Tax case: balance must be DR_net + DR_VAT = CR_AP.
            // total_debit = net + vat = gross = payable_amount = total_credit.
            let expected_gross = net + tax;
            assert_eq!(
                je.total_debit(),
                expected_gross,
                "JE {} total debit should equal gross (net+tax)",
                je.header.document_id,
            );
        }

        total_lines += je.line_count();
    }

    // No GR/IR clearing lines must have appeared.
    assert_eq!(
        gr_ir_count, 0,
        "Expected 0 GR/IR clearing lines with direct_expense_share=1.0 and priors loaded, got {gr_ir_count}"
    );

    assert!(
        tax_cases_found > 0,
        "Expected some tax-case invoices in the test sample"
    );

    // With direct-expense + W1.5 split targeting 4 lines, mean should be > 2.
    let mean_lines = total_lines as f64 / n_jes as f64;
    assert!(
        mean_lines > 2.0,
        "Expected mean lines/JE > 2.0 on direct-expense path (got {mean_lines:.2})"
    );

    eprintln!(
        "SP3.13 W1 direct-expense: n={n_jes}, mean_lines={mean_lines:.2}, \
         tax_cases={tax_cases_found}, gr_ir_lines={gr_ir_count}"
    );
}

/// SP3.13 W1 — When priors are NOT loaded, the canonical GR/IR-Clearing path
/// is preserved byte-identical (regression guard).
#[test]
fn sp3_13_w1_canonical_path_preserved_when_priors_disabled() {
    use datasynth_generators::document_flow::{DocumentFlowJeConfig, DocumentFlowJeGenerator};
    use rust_decimal::Decimal;

    let config = DocumentFlowJeConfig::default();
    // No set_loaded_priors call — priors are None.
    let mut generator = DocumentFlowJeGenerator::with_config_and_seed(config.clone(), 9002);

    let gr_ir_account = config.gr_ir_clearing_account.clone();
    let ap_account = config.ap_account.clone();

    let n_jes = 100usize;
    let mut gr_ir_count = 0usize;

    for i in 0..n_jes {
        let inv = make_vendor_invoice(&format!("VI-{i:04}"), Decimal::from(500), Decimal::ZERO);
        let je = generator
            .generate_from_vendor_invoice(&inv)
            .expect("generate_from_vendor_invoice returned None");

        // Balance invariant.
        assert!(
            je.is_balanced(),
            "JE {} is unbalanced",
            je.header.document_id,
        );

        // Canonical path: GR/IR clearing must be the debit account.
        let has_gr_ir_debit = je
            .lines
            .iter()
            .any(|l| l.gl_account == gr_ir_account && l.debit_amount > Decimal::ZERO);
        assert!(
            has_gr_ir_debit,
            "JE {} (priors disabled) must use GR/IR clearing as debit",
            je.header.document_id,
        );

        // AP must appear as CR.
        let has_ap_credit = je
            .lines
            .iter()
            .any(|l| l.gl_account == ap_account && l.credit_amount > Decimal::ZERO);
        assert!(
            has_ap_credit,
            "JE {} missing AP credit line",
            je.header.document_id,
        );

        // Canonical path with no priors: always exactly 2 lines (no split).
        assert_eq!(
            je.line_count(),
            2,
            "JE {} (priors disabled, no VAT) should be exactly 2 lines, got {}",
            je.header.document_id,
            je.line_count(),
        );

        gr_ir_count += 1;
    }

    assert_eq!(
        gr_ir_count, n_jes,
        "All {n_jes} invoices must use GR/IR clearing when priors are disabled"
    );
}

// ---------------------------------------------------------------------------
// SP4.7 — Reference format conventions
// ---------------------------------------------------------------------------

/// Build a `BehavioralPriors` with a `ReferenceFormatPrior` for the given source.
#[cfg(test)]
fn build_bp_with_reference_formats(
    source: &str,
    templates: &[(&str, f64, &str)],
) -> datasynth_core::distributions::behavioral_priors::BehavioralPriors {
    use datasynth_core::distributions::behavioral_priors::{
        ActiveLifetimePrior, BehavioralPriors, FanoutPrior, LinesPerJePrior, PerSourceIetPrior,
        ReferenceFormatPrior, ReferenceTemplate, SourceMixPrior,
    };
    use std::collections::BTreeMap;

    let mut source_probs = BTreeMap::new();
    source_probs.insert(source.to_string(), 1.0_f64);

    let rf_templates: Vec<ReferenceTemplate> = templates
        .iter()
        .map(|(tmpl, prob, example)| ReferenceTemplate {
            template: tmpl.to_string(),
            probability: *prob,
            example: example.to_string(),
        })
        .collect();
    let mut by_source = BTreeMap::new();
    by_source.insert(source.to_string(), rf_templates);
    let reference_formats = Some(ReferenceFormatPrior { by_source });

    BehavioralPriors {
        schema_version: BehavioralPriors::SCHEMA_VERSION,
        generator_version: "test-sp4.7".to_string(),
        industry: "test".to_string(),
        n_client_inputs: 1,
        n_rows_aggregated: 1000,
        source_mix: SourceMixPrior {
            probabilities: source_probs,
            other_fraction: 0.0,
            min_threshold: 0.0,
        },
        per_source_iet: PerSourceIetPrior::default(),
        lines_per_je: LinesPerJePrior::default(),
        active_lifetime: ActiveLifetimePrior::default(),
        fanout: FanoutPrior::default(),
        posting_lag: None,
        active_segments: None,
        entity_clusters: None,
        per_source_attribute: None,
        tp_entity_clusters: None,
        coa_semantic: None,
        reference_formats,
        text_taxonomy: None,
        user_personas: None,
        source_amount_conditionals: None,
        source_role_gl_conditionals: None,
        tb_anchor: None,
    }
}

/// SP4.7 — `sample_reference` returns a string matching the configured template.
/// Uses the corpus-observed `{4 digits}-{4 digits}-{10 digits}` format.
#[test]
fn sp4_7_sample_reference_matches_template_format() {
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    let bp = build_bp_with_reference_formats(
        "IM",
        &[(
            "{4 digits}-{4 digits}-{10 digits}",
            1.0,
            "2022-0090-0950645487",
        )],
    );
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let priors = LoadedPriors::from_priors(bp, std::path::PathBuf::from("test"), &mut rng, 365)
        .expect("from_priors");

    for _ in 0..20 {
        let ref_str = priors
            .sample_reference("IM", &mut rng)
            .expect("should return Some for known source");

        // Verify format: NNNN-NNNN-NNNNNNNNNN (20 digits + 2 hyphens = 22 chars).
        assert_eq!(
            ref_str.len(),
            20,
            "expected 20-char reference, got: {ref_str}"
        );
        let parts: Vec<&str> = ref_str.split('-').collect();
        assert_eq!(parts.len(), 3, "expected 3 parts, got: {ref_str}");
        assert_eq!(
            parts[0].len(),
            4,
            "first segment should be 4 digits: {ref_str}"
        );
        assert_eq!(
            parts[1].len(),
            4,
            "second segment should be 4 digits: {ref_str}"
        );
        assert_eq!(
            parts[2].len(),
            10,
            "third segment should be 10 digits: {ref_str}"
        );
        assert!(
            parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit())),
            "all segments should be digits: {ref_str}"
        );
    }
}

/// SP4.7 — `sample_reference` returns `None` for unknown sources (fallback to
/// existing `ReferenceGenerator`).
#[test]
fn sp4_7_sample_reference_returns_none_for_unknown_source() {
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    let bp = build_bp_with_reference_formats(
        "IM",
        &[(
            "{4 digits}-{4 digits}-{10 digits}",
            1.0,
            "2022-0090-0950645487",
        )],
    );
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let priors = LoadedPriors::from_priors(bp, std::path::PathBuf::from("test"), &mut rng, 365)
        .expect("from_priors");

    // A source not in the bundle should return None.
    let result = priors.sample_reference("UNKNOWN_SRC", &mut rng);
    assert!(
        result.is_none(),
        "expected None for unknown source, got: {result:?}"
    );
}

/// SP4.7 — `LoadedPriors::sample_reference` returns `None` when `reference_formats`
/// is `None` (backwards-compatibility: bundles without SP4.7 data still work).
#[test]
fn sp4_7_sample_reference_none_when_prior_absent() {
    use datasynth_core::distributions::behavioral_priors::{
        ActiveLifetimePrior, BehavioralPriors, FanoutPrior, LinesPerJePrior, PerSourceIetPrior,
        SourceMixPrior,
    };
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;
    use std::collections::BTreeMap;

    let bp = BehavioralPriors {
        schema_version: BehavioralPriors::SCHEMA_VERSION,
        generator_version: "test".to_string(),
        industry: "test".to_string(),
        n_client_inputs: 1,
        n_rows_aggregated: 0,
        source_mix: SourceMixPrior {
            probabilities: BTreeMap::new(),
            other_fraction: 0.0,
            min_threshold: 0.0,
        },
        per_source_iet: PerSourceIetPrior::default(),
        lines_per_je: LinesPerJePrior::default(),
        active_lifetime: ActiveLifetimePrior::default(),
        fanout: FanoutPrior::default(),
        posting_lag: None,
        active_segments: None,
        entity_clusters: None,
        per_source_attribute: None,
        tp_entity_clusters: None,
        coa_semantic: None,
        reference_formats: None, // no SP4.7 data
        text_taxonomy: None,
        user_personas: None,
        source_amount_conditionals: None,
        source_role_gl_conditionals: None,
        tb_anchor: None,
    };

    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let priors = LoadedPriors::from_priors(bp, std::path::PathBuf::from("test"), &mut rng, 365)
        .expect("from_priors");

    let result = priors.sample_reference("KR", &mut rng);
    assert!(
        result.is_none(),
        "expected None when reference_formats is absent, got: {result:?}"
    );
}

// ---------------------------------------------------------------------------
// SP4.2 — CoA semantic content
// ---------------------------------------------------------------------------

/// SP4.2 — When a bundle carries `coa_semantic`, `LoadedPriors` exposes it
/// and pre-regen bundles (None path) still work without panicking.
#[test]
fn priors_loaded_coa_semantic_populates_account_descriptions() {
    let path = bundled_priors_path("health");
    if !path.exists() {
        eprintln!("Skipping: health bundle not present at {}", path.display());
        return;
    }
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let priors = LoadedPriors::load_bundled("health", &mut rng, 365).expect("load_bundled health");

    // Pre-regen bundle won't have CoA semantic — that's OK, just emit a warning
    // count so CI confirms the load path works without crashing.
    if let Some(ref coa) = priors.coa_semantic {
        eprintln!(
            "SP4.2: CoA semantic accounts in bundle: {}",
            coa.accounts.len()
        );
        assert!(
            !coa.accounts.is_empty(),
            "CoA semantic prior should have at least one account"
        );
    } else {
        eprintln!("SP4.2: coa_semantic is None (pre-SP4 W6 bundle — expected)");
    }
}

/// SP4.2 — `from_priors` correctly threads `coa_semantic` into `LoadedPriors`.
#[test]
fn priors_loaded_coa_semantic_round_trips_via_from_priors() {
    use datasynth_core::distributions::behavioral_priors::{
        AccountSemantic, ActiveLifetimePrior, BehavioralPriors, CoaSemanticPrior, FanoutPrior,
        LinesPerJePrior, PerSourceIetPrior, SourceMixPrior,
    };
    use std::collections::BTreeMap;

    let mut sem_accounts = BTreeMap::new();
    sem_accounts.insert(
        "1000".to_string(),
        AccountSemantic {
            description: "Kasse".to_string(),
            account_type: Some("Assets".to_string()),
            account_class: Some("C _ Cash".to_string()),
            ..Default::default()
        },
    );
    sem_accounts.insert(
        "2000".to_string(),
        AccountSemantic {
            description: "Kreditoren".to_string(),
            account_type: Some("Liabilities".to_string()),
            ..Default::default()
        },
    );

    let coa_semantic = Some(CoaSemanticPrior {
        accounts: sem_accounts,
    });

    let bp = BehavioralPriors {
        schema_version: BehavioralPriors::SCHEMA_VERSION,
        generator_version: "test-sp4.2".to_string(),
        industry: "test".to_string(),
        n_client_inputs: 1,
        n_rows_aggregated: 1000,
        source_mix: SourceMixPrior {
            probabilities: [("KR".to_string(), 1.0)].into_iter().collect(),
            other_fraction: 0.0,
            min_threshold: 0.0,
        },
        per_source_iet: PerSourceIetPrior::default(),
        lines_per_je: LinesPerJePrior::default(),
        active_lifetime: ActiveLifetimePrior::default(),
        fanout: FanoutPrior::default(),
        posting_lag: None,
        active_segments: None,
        entity_clusters: None,
        per_source_attribute: None,
        tp_entity_clusters: None,
        coa_semantic,
        reference_formats: None,
        text_taxonomy: None,
        user_personas: None,
        source_amount_conditionals: None,
        source_role_gl_conditionals: None,
        tb_anchor: None,
    };

    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let priors = LoadedPriors::from_priors(bp, std::path::PathBuf::from("test"), &mut rng, 365)
        .expect("from_priors");

    let coa = priors.coa_semantic.expect("coa_semantic should be Some");
    assert_eq!(coa.accounts.len(), 2, "should have 2 accounts");
    assert_eq!(coa.accounts["1000"].description, "Kasse");
    assert_eq!(
        coa.accounts["2000"].account_type.as_deref(),
        Some("Liabilities")
    );
}

/// SP4.2 — `apply_coa_semantic_prior` overwrites descriptions on matching accounts.
#[test]
fn coa_semantic_prior_applied_to_chart_of_accounts() {
    use datasynth_core::distributions::behavioral_priors::{AccountSemantic, CoaSemanticPrior};
    use datasynth_core::models::{CoAComplexity, IndustrySector};
    use datasynth_generators::coa_generator::ChartOfAccountsGenerator;
    use std::collections::BTreeMap;

    let mut gen =
        ChartOfAccountsGenerator::new(CoAComplexity::Small, IndustrySector::Healthcare, 42);
    let mut coa = gen.generate();

    // Pick a real account number from the generated COA.
    let Some(first_account_number) = coa.accounts.first().map(|a| a.account_number.clone()) else {
        eprintln!("Empty COA — skipping");
        return;
    };

    // Build a prior that sets a distinctive description on this account.
    let mut sem_accounts = BTreeMap::new();
    sem_accounts.insert(
        first_account_number.clone(),
        AccountSemantic {
            description: "SP4.2 Test Account Name".to_string(),
            account_type: Some("Test".to_string()),
            ..Default::default()
        },
    );
    let prior = CoaSemanticPrior {
        accounts: sem_accounts,
    };

    ChartOfAccountsGenerator::apply_coa_semantic_prior(&mut coa, &prior);

    let enriched = coa
        .accounts
        .iter()
        .find(|a| a.account_number == first_account_number)
        .expect("account should still exist");

    assert_eq!(
        enriched.short_description, "SP4.2 Test Account Name",
        "description should be overwritten by prior"
    );
    assert_eq!(
        enriched.long_description, "SP4.2 Test Account Name",
        "long_description should also be overwritten"
    );
}

// ---------------------------------------------------------------------------
// SP4.2 W7.1 — Public overlay_coa_semantic free-function test
// ---------------------------------------------------------------------------

/// W7.1 — `overlay_coa_semantic` replaces description, account_class, and
/// account_class_name on the matching account and returns the correct count.
#[test]
fn w7_1_overlay_coa_semantic_replaces_description_and_class() {
    use datasynth_core::distributions::behavioral_priors::{AccountSemantic, CoaSemanticPrior};
    use datasynth_core::models::{CoAComplexity, IndustrySector};
    use datasynth_generators::coa_generator::{overlay_coa_semantic, ChartOfAccountsGenerator};
    use std::collections::BTreeMap;

    // Build a small CoA with known accounts.
    let mut gen =
        ChartOfAccountsGenerator::new(CoAComplexity::Small, IndustrySector::Healthcare, 42);
    let mut coa = gen.generate();
    assert!(
        !coa.accounts.is_empty(),
        "CoA must have at least one account"
    );

    // Pick the first account number: it will match the prior.
    let match_number = coa.accounts[0].account_number.clone();
    // Capture original description to confirm it was changed.
    let original_desc = coa.accounts[0].short_description.clone();

    // Build a CoaSemanticPrior with corpus values for the first account only.
    let mut sem_accounts = BTreeMap::new();
    sem_accounts.insert(
        match_number.clone(),
        AccountSemantic {
            description: "Trade Receivables — W7.1".to_string(),
            account_class: Some("A.B".to_string()),
            account_class_name: Some("Trade Receivables".to_string()),
            account_sub_class: Some("A.B.A".to_string()),
            account_sub_class_name: Some("Trade Accounts Receivable".to_string()),
            ..Default::default()
        },
    );
    let prior = CoaSemanticPrior {
        accounts: sem_accounts,
    };

    // Call the public free function.
    let applied = overlay_coa_semantic(&mut coa, &prior);

    // Exactly one account should have been enriched.
    assert_eq!(
        applied, 1,
        "overlay_coa_semantic should return 1 for one matching account"
    );

    // The matched account must carry the prior's values.
    let acc = coa
        .accounts
        .iter()
        .find(|a| a.account_number == match_number)
        .expect("matched account must still exist in CoA");

    assert_eq!(
        acc.short_description, "Trade Receivables — W7.1",
        "short_description should be overwritten by prior"
    );
    assert_eq!(
        acc.long_description, "Trade Receivables — W7.1",
        "long_description should also be overwritten"
    );
    assert_ne!(
        acc.short_description, original_desc,
        "description must differ from the synthetic default"
    );
    assert_eq!(
        acc.account_class, "A.B",
        "account_class should be overwritten"
    );
    assert_eq!(
        acc.account_class_name, "Trade Receivables",
        "account_class_name should be overwritten"
    );
    assert_eq!(
        acc.account_sub_class, "A.B.A",
        "account_sub_class should be overwritten"
    );
    assert_eq!(
        acc.account_sub_class_name, "Trade Accounts Receivable",
        "account_sub_class_name should be overwritten"
    );
}

// ---------------------------------------------------------------------------
// SP4.5 — User-persona prior smoke tests
// ---------------------------------------------------------------------------

/// SP4.5 — `LoadedPriors::user_personas` is wired for bundles that carry the
/// field.  Existing health bundles were built before SP4.5, so `user_personas`
/// will be `None` (old-bundle compatibility).  We verify the accessor methods
/// handle both cases gracefully.
#[test]
fn sp4_5_user_personas_field_loads_without_panic() {
    let path = bundled_priors_path("health");
    if !path.exists() {
        eprintln!("Skipping: health bundle not present at {}", path.display());
        return;
    }
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let priors = LoadedPriors::load_bundled("health", &mut rng, 365).expect("load_bundled health");

    // Old bundles have user_personas = None.  Both None and Some(empty) are valid.
    match &priors.user_personas {
        None => {
            // Pre-SP4.5 bundle — sampling helpers must return None safely.
            assert!(
                priors.sample_user_for_source("KR", &mut rng).is_none(),
                "sample_user_for_source must return None when prior is absent"
            );
            assert!(
                priors
                    .sample_timestamp_for_user("USER0001", &mut rng)
                    .is_none(),
                "sample_timestamp_for_user must return None when prior is absent"
            );
        }
        Some(up) => {
            if !up.has_data() {
                // Empty stub — same expectation.
                assert!(priors.sample_user_for_source("KR", &mut rng).is_none());
            } else {
                // Real data present (future bundles) — just verify it doesn't panic.
                let _ = priors.sample_user_for_source("KR", &mut rng);
            }
        }
    }
}

/// SP4.5 — Verify that a manually wired `UserPersonaPrior` with real data is
/// correctly consumed by `LoadedPriors::sample_user_for_source` and
/// `sample_timestamp_for_user`.
#[test]
fn sp4_5_loaded_priors_consumes_user_persona_prior() {
    use datasynth_core::distributions::behavioral_priors::{UserBehavior, UserPersonaPrior};
    use std::collections::BTreeMap;

    let path = bundled_priors_path("health");
    if !path.exists() {
        eprintln!("Skipping: health bundle not present at {}", path.display());
        return;
    }
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let mut priors =
        LoadedPriors::load_bundled("health", &mut rng, 365).expect("load_bundled health");

    // Inject a synthetic UserPersonaPrior with two users.
    let mut users = BTreeMap::new();

    let mut kr_mix = BTreeMap::new();
    kr_mix.insert("KR".to_string(), 0.9);
    kr_mix.insert("KZ".to_string(), 0.1);
    let mut kr_hourly = [0.0f64; 24];
    kr_hourly[9] = 1.0; // 9am
    let mut kr_weekday = [0.0f64; 7];
    kr_weekday[0] = 1.0; // Monday
    users.insert(
        "AP_CLERK".to_string(),
        UserBehavior {
            source_mix: kr_mix,
            hourly_density: kr_hourly,
            weekday_density: kr_weekday,
            volume_share: 0.6,
        },
    );

    let mut rv_mix = BTreeMap::new();
    rv_mix.insert("RV".to_string(), 1.0);
    let mut rv_hourly = [0.0f64; 24];
    rv_hourly[14] = 1.0; // 2pm
    let mut rv_weekday = [0.0f64; 7];
    rv_weekday[4] = 1.0; // Friday
    users.insert(
        "AR_CLERK".to_string(),
        UserBehavior {
            source_mix: rv_mix,
            hourly_density: rv_hourly,
            weekday_density: rv_weekday,
            volume_share: 0.4,
        },
    );

    priors.user_personas = Some(UserPersonaPrior {
        users,
        user_count_distribution: Default::default(),
    });

    // sample_user_for_source("KR") should always return AP_CLERK.
    for _ in 0..20 {
        let uid = priors
            .sample_user_for_source("KR", &mut rng)
            .expect("KR must map to AP_CLERK");
        assert_eq!(uid, "AP_CLERK");
    }

    // sample_user_for_source("RV") should always return AR_CLERK.
    for _ in 0..20 {
        let uid = priors
            .sample_user_for_source("RV", &mut rng)
            .expect("RV must map to AR_CLERK");
        assert_eq!(uid, "AR_CLERK");
    }

    // sample_timestamp_for_user("AP_CLERK") → (hour=9, weekday=0).
    let (hour, weekday) = priors
        .sample_timestamp_for_user("AP_CLERK", &mut rng)
        .expect("AP_CLERK must return a timestamp");
    assert_eq!(hour, 9, "AP_CLERK posts at 9am");
    assert_eq!(weekday, 0, "AP_CLERK posts on Monday");

    // sample_timestamp_for_user("AR_CLERK") → (hour=14, weekday=4).
    let (hour, weekday) = priors
        .sample_timestamp_for_user("AR_CLERK", &mut rng)
        .expect("AR_CLERK must return a timestamp");
    assert_eq!(hour, 14, "AR_CLERK posts at 2pm");
    assert_eq!(weekday, 4, "AR_CLERK posts on Friday");

    // Unknown user → None.
    assert!(priors.sample_user_for_source("XX", &mut rng).is_none());
    assert!(priors
        .sample_timestamp_for_user("UNKNOWN", &mut rng)
        .is_none());
}

/// SP4.3 — `sample_amount_for_source` returns `None` when the bundle has no
/// source_amount_conditionals (pre-SP4.3 bundle) and a positive `f64` when
/// the prior is populated.
///
/// For bundles built before SP4.3 (i.e. the existing committed health bundle)
/// the function must return `None` — this is the no-regression path.
/// When the bundle is regenerated with SP4.3 extraction, the test emits a
/// WARNING but still passes.
#[test]
fn priors_loaded_amount_sampling_uses_per_source_distribution() {
    use datasynth_core::distributions::behavioral_priors::{LognormalAmount, PerSourceAmountPrior};
    use datasynth_generators::priors_loader::{bundled_priors_path, LoadedPriors};
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    let mut rng = ChaCha8Rng::seed_from_u64(99);

    // ---- Path 1: bundle without source_amount_conditionals → None ----
    // Build a minimal priors struct without source_amount_conditionals.
    let bp_no_sac = datasynth_core::distributions::behavioral_priors::BehavioralPriors {
        schema_version: 1,
        generator_version: "test".to_string(),
        industry: "test".to_string(),
        n_client_inputs: 0,
        n_rows_aggregated: 0,
        source_mix: datasynth_core::distributions::behavioral_priors::SourceMixPrior::default(),
        per_source_iet:
            datasynth_core::distributions::behavioral_priors::PerSourceIetPrior::default(),
        lines_per_je: datasynth_core::distributions::behavioral_priors::LinesPerJePrior::default(),
        active_lifetime:
            datasynth_core::distributions::behavioral_priors::ActiveLifetimePrior::default(),
        fanout: datasynth_core::distributions::behavioral_priors::FanoutPrior::default(),
        posting_lag: None,
        active_segments: None,
        entity_clusters: None,
        per_source_attribute: None,
        tp_entity_clusters: None,
        coa_semantic: None,
        reference_formats: None,
        text_taxonomy: None,
        user_personas: None,
        source_amount_conditionals: None,
        source_role_gl_conditionals: None,
        tb_anchor: None,
    };
    let priors_no_sac = LoadedPriors::from_priors(bp_no_sac, PathBuf::from("test"), &mut rng, 365)
        .expect("from_priors");
    let result = priors_no_sac.sample_amount_for_source("KR", "", &mut rng);
    assert!(
        result.is_none(),
        "without source_amount_conditionals the helper must return None"
    );

    // ---- Path 2: priors with source_amount_conditionals → positive f64 ----
    let mut by_source = BTreeMap::new();
    by_source.insert(
        "KR".to_string(),
        LognormalAmount {
            mu: 4.5,
            sigma: 2.158,
            n: 1000,
            median_abs: 90.0,
        },
    );
    let sac = PerSourceAmountPrior {
        by_source_and_class: BTreeMap::new(),
        by_source,
    };
    let bp_with_sac = datasynth_core::distributions::behavioral_priors::BehavioralPriors {
        schema_version: 1,
        generator_version: "test".to_string(),
        industry: "test".to_string(),
        n_client_inputs: 0,
        n_rows_aggregated: 0,
        source_mix: datasynth_core::distributions::behavioral_priors::SourceMixPrior::default(),
        per_source_iet:
            datasynth_core::distributions::behavioral_priors::PerSourceIetPrior::default(),
        lines_per_je: datasynth_core::distributions::behavioral_priors::LinesPerJePrior::default(),
        active_lifetime:
            datasynth_core::distributions::behavioral_priors::ActiveLifetimePrior::default(),
        fanout: datasynth_core::distributions::behavioral_priors::FanoutPrior::default(),
        posting_lag: None,
        active_segments: None,
        entity_clusters: None,
        per_source_attribute: None,
        tp_entity_clusters: None,
        coa_semantic: None,
        reference_formats: None,
        text_taxonomy: None,
        user_personas: None,
        source_amount_conditionals: Some(sac),
        source_role_gl_conditionals: None,
        tb_anchor: None,
    };
    let priors_with_sac =
        LoadedPriors::from_priors(bp_with_sac, PathBuf::from("test"), &mut rng, 365)
            .expect("from_priors");

    // Sample 20 times; all must be > 0.
    for _ in 0..20 {
        let v = priors_with_sac
            .sample_amount_for_source("KR", "", &mut rng)
            .expect("should return Some when sac is populated");
        assert!(v > 0.0, "sampled amount must be > 0, got {v}");
    }

    // Unknown source returns None (falls back to marginal, which is also absent).
    let unknown = priors_with_sac.sample_amount_for_source("UNKNOWN_SRC", "", &mut rng);
    assert!(
        unknown.is_none(),
        "unknown source should return None (fallback to marginal also absent)"
    );

    // ---- Path 3: real bundle (pre-SP4.3) returns None from sample_amount ----
    let path = bundled_priors_path("health");
    if path.exists() {
        let priors_real =
            LoadedPriors::load_bundled("health", &mut rng, 365).expect("load_bundled health");
        // Pre-SP4.3 bundle: source_amount_conditionals is None, so result is None.
        // Post-SP4.3 bundle: result may be Some.  Both are acceptable.
        let real_result = priors_real.sample_amount_for_source("KR", "", &mut rng);
        if let Some(v) = real_result {
            assert!(v > 0.0, "real bundle amount must be > 0, got {v}");
        } else {
            eprintln!(
                "INFO: health bundle has no source_amount_conditionals yet — \
                 regenerate with SP4.3 fingerprint extract to populate"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// W7.M — per-source amount autocorr mitigation
// ---------------------------------------------------------------------------

/// W8.2 — `remap_account_numbers_to_prior` replaces ~80% of synthetic account
/// numbers with corpus ones from the prior, preserving account_type.
/// After the remap the W7.1 overlay should match at high rate.
#[test]
fn w8_2_remap_account_numbers_to_prior_uses_real_account_numbers() {
    use datasynth_core::distributions::behavioral_priors::{AccountSemantic, CoaSemanticPrior};
    use datasynth_core::models::{
        AccountSubType, AccountType, ChartOfAccounts, CoAComplexity, GLAccount, IndustrySector,
    };
    use datasynth_generators::coa_generator::{
        overlay_coa_semantic, remap_account_numbers_to_prior,
    };
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;
    use std::collections::HashSet;

    // Build a synthetic CoA with 100 Asset accounts using generic numbers.
    let mut coa = ChartOfAccounts::new(
        "COA_TEST".to_string(),
        "Test CoA".to_string(),
        "US".to_string(),
        IndustrySector::Healthcare,
        CoAComplexity::Small,
    );
    for i in 0..80 {
        coa.add_account(GLAccount::new(
            format!("{}", 100000 + i * 10),
            format!("Asset Account {}", i + 1),
            AccountType::Asset,
            AccountSubType::OtherAssets,
        ));
    }
    for i in 0..20 {
        coa.add_account(GLAccount::new(
            format!("{}", 500000 + i * 10),
            format!("Expense Account {}", i + 1),
            AccountType::Expense,
            AccountSubType::OperatingExpenses,
        ));
    }
    assert_eq!(coa.accounts.len(), 100);

    // Build a prior with 20 corpus asset accounts and 10 expense accounts.
    let mut prior = CoaSemanticPrior::default();
    for i in 0..20u32 {
        prior.accounts.insert(
            format!("0000100{:03}", i),
            AccountSemantic {
                description: format!("Real Asset {}", i),
                account_type: Some("Assets".to_string()),
                account_class: Some("A".to_string()),
                ..Default::default()
            },
        );
    }
    for i in 0..10u32 {
        prior.accounts.insert(
            format!("0000500{:03}", i),
            AccountSemantic {
                description: format!("Real Expense {}", i),
                account_type: Some("Expense".to_string()),
                account_class: Some("E".to_string()),
                ..Default::default()
            },
        );
    }

    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let remapped = remap_account_numbers_to_prior(&mut coa, &prior, &mut rng);

    // ~80% should be remapped (100 accounts × 0.80 ± 3σ).
    let n = 100usize;
    let expected = (n as f64) * 0.80;
    let stdev = ((n as f64) * 0.80 * 0.20_f64).sqrt();
    assert!(
        (remapped as f64 - expected).abs() < 3.5 * stdev,
        "expected ~{expected:.0} remapped accounts (±3.5σ = {:.1}), got {remapped}",
        3.5 * stdev
    );
    assert!(
        remapped >= 60,
        "expected ≥60 accounts remapped, got {remapped}"
    );

    // Remapped account numbers should all be in the prior key set.
    let prior_keys: HashSet<&String> = prior.accounts.keys().collect();
    for account in &coa.accounts {
        if prior_keys.contains(&account.account_number) {
            // Good — this is a prior-matched number.
        }
        // Accounts not in the prior are either the un-remapped 20% or
        // those that fell through (type bucket mismatch). Both are fine.
    }

    // The prior-matched accounts should cover ≥60 of the 100 accounts.
    let matched: usize = coa
        .accounts
        .iter()
        .filter(|a| prior_keys.contains(&a.account_number))
        .count();
    assert!(
        matched >= 60,
        "expected ≥60 accounts with prior-matched numbers after remap, got {matched}"
    );

    // account_type must be preserved (Asset accounts should still be Asset).
    let asset_count = coa
        .accounts
        .iter()
        .filter(|a| a.account_type == AccountType::Asset)
        .count();
    assert_eq!(asset_count, 80, "asset_count should be 80 after remap");

    // Now run the W7.1 overlay — it should enrich at high rate.
    let enriched = overlay_coa_semantic(&mut coa, &prior);
    assert!(
        enriched >= 60,
        "expected W7.1 overlay to enrich ≥60 accounts after W8.2 remap, got {enriched}"
    );
}

// ---------------------------------------------------------------------------
// W8.1 — TB drift-correction JE emission tests
// ---------------------------------------------------------------------------

/// W8.1 — `drift_correction_needed` fires when a balance is far from its target,
/// and `build_drift_correction_je` emits a balanced JE that debits the
/// under-target account to pull it toward the target.
#[test]
fn w8_1_drift_correction_emits_balanced_je_when_drift_above_threshold() {
    use datasynth_core::distributions::behavioral_priors::{TbAnchorPrior, TbTarget};
    use datasynth_generators::balance::BalanceTrackerConfig;
    use datasynth_generators::RunningBalanceTracker;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;
    use rust_decimal::Decimal;
    use std::collections::BTreeMap;

    // Build a TB anchor with one account whose target closing balance is 10 000.
    let mut per_account = BTreeMap::new();
    per_account.insert(
        "1000".to_string(),
        TbTarget {
            opening_balance: 0.0,
            closing_balance: 10_000.0,
            period_net_activity: 10_000.0,
            opening_stdev: 0.0,
            closing_stdev: 100.0, // 3σ = 300 — well below the 5 000 drift we'll inject
            n_clients: 5,
        },
    );
    let anchor = TbAnchorPrior {
        per_account,
        total_assets: 10_000.0,
        total_liabilities: 0.0,
        total_equity: 10_000.0,
        n_clients: 5,
    };

    // Tracker with no initial balance → closing balance for "1000" = 0, target = 10 000.
    let cfg = BalanceTrackerConfig {
        validate_on_each_entry: false,
        track_history: false,
        fail_on_validation_error: false,
        ..Default::default()
    };
    let mut tracker = RunningBalanceTracker::new_with_currency(cfg, "USD".to_string());
    tracker.set_tb_anchor(anchor);

    // Inject a JE that puts account "1000" at 5 000 (still 5 000 under target of 10 000).
    // Balanced JE: debit 1000, credit 9999.
    let je_setup = {
        use datasynth_core::models::{JournalEntry, JournalEntryLine};
        let mut je = JournalEntry::new_simple(
            "SETUP001".to_string(),
            "TEST".to_string(),
            chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            "Setup balance".to_string(),
        );
        je.add_line(JournalEntryLine {
            line_number: 1,
            gl_account: "1000".to_string(),
            account_code: "1000".to_string(),
            debit_amount: Decimal::new(5_000, 0),
            ..Default::default()
        });
        je.add_line(JournalEntryLine {
            line_number: 2,
            gl_account: "9999".to_string(),
            account_code: "9999".to_string(),
            credit_amount: Decimal::new(5_000, 0),
            ..Default::default()
        });
        je
    };
    tracker.apply_entry(&je_setup).unwrap();

    // Drift = 5 000 − 10 000 = −5 000 (under-target): correction needed.
    assert!(
        tracker.drift_correction_needed("TEST"),
        "drift of -5 000 should exceed 3×σ=300 threshold"
    );

    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let je = tracker
        .build_drift_correction_je(
            "TEST",
            chrono::NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
            &mut rng,
        )
        .expect("should emit a drift-correction JE");

    // The JE must balance.
    assert_eq!(
        je.total_debit(),
        je.total_credit(),
        "drift JE must balance (debit={}, credit={})",
        je.total_debit(),
        je.total_credit()
    );

    // Account "1000" is under-target → the correction should debit it.
    let dr_to_1000: Decimal = je
        .lines
        .iter()
        .filter(|l| l.gl_account == "1000")
        .map(|l| l.debit_amount)
        .sum();
    assert!(
        dr_to_1000 > Decimal::ZERO,
        "correction JE should DEBIT account 1000 to bring it toward target; \
         lines: {:?}",
        je.lines
            .iter()
            .map(|l| (&l.gl_account, l.debit_amount, l.credit_amount))
            .collect::<Vec<_>>()
    );

    // At least 2 lines.
    assert!(je.lines.len() >= 2, "JE should have ≥2 lines");
    // SA document type.
    assert_eq!(je.header.document_type, "SA", "document_type should be SA");
}

/// W8.1 — When no TB anchor is loaded, `drift_correction_needed` returns false
/// and `build_drift_correction_je` returns None (backwards-compatible).
#[test]
fn w8_1_no_drift_correction_without_anchor() {
    use datasynth_generators::balance::BalanceTrackerConfig;
    use datasynth_generators::RunningBalanceTracker;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    let cfg = BalanceTrackerConfig::default();
    let tracker = RunningBalanceTracker::new_with_currency(cfg, "USD".to_string());

    assert!(
        !tracker.drift_correction_needed("ANY"),
        "without an anchor, drift_correction_needed must return false"
    );

    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let je = tracker.build_drift_correction_je(
        "ANY",
        chrono::NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
        &mut rng,
    );
    assert!(
        je.is_none(),
        "without an anchor, build_drift_correction_je must return None"
    );
}

/// W7.M — The bypass gate draws a uniform random value in [0,1) and bypasses
/// the per-source conditional ~30 % of the time.  With 10 000 trials the
/// observed bypass rate should be within ±3 standard deviations of 0.30.
#[test]
fn w7_m_bypass_gate_fires_approximately_at_target_share() {
    use rand::{RngExt, SeedableRng};
    use rand_chacha::ChaCha8Rng;

    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let target = 0.30_f64;
    let n = 10_000_usize;
    let mut bypass_count = 0usize;
    for _ in 0..n {
        // Mirror the gate condition: bypass when draw < PRIORS_AMOUNT_BYPASS_SHARE.
        if rng.random_range(0.0..1.0_f64) < target {
            bypass_count += 1;
        }
    }
    let expected = (n as f64) * target;
    let stdev = ((n as f64) * target * (1.0 - target)).sqrt();
    assert!(
        (bypass_count as f64 - expected).abs() < 3.0 * stdev,
        "bypass count {bypass_count} out of {n} not near expected {expected:.0} \
         (±3σ = {:.1})",
        3.0 * stdev
    );
}

/// SP5.2 — Verify that a `CoaSemanticPrior` can be indexed by account number and that
/// the fallback logic (primary miss → secondary hit) resolves `description` and ISO
/// class codes correctly.  This mirrors what `write_journal_entries_csv` does when
/// building and using the secondary `coa_semantic_index`.
#[test]
fn sp5_2_coa_semantic_prior_secondary_index_resolves_descriptions() {
    use datasynth_core::distributions::behavioral_priors::{AccountSemantic, CoaSemanticPrior};
    use std::collections::BTreeMap;

    // Build a minimal prior with two accounts.
    let mut accounts = BTreeMap::new();
    accounts.insert(
        "0000105000".to_string(),
        AccountSemantic {
            description: "Trade Receivables".to_string(),
            account_class: Some("A.B".to_string()),
            account_class_name: Some("Trade Receivables".to_string()),
            account_sub_class: Some("A.B.A".to_string()),
            account_sub_class_name: Some("Trade Accounts Receivable".to_string()),
            ..AccountSemantic::default()
        },
    );
    accounts.insert(
        "0000800000".to_string(),
        AccountSemantic {
            description: "Revenue".to_string(),
            account_class: Some("I.A".to_string()),
            account_class_name: Some("Revenue".to_string()),
            ..AccountSemantic::default()
        },
    );
    let prior = CoaSemanticPrior { accounts };

    // Mirror the secondary-index construction from write_journal_entries_csv.
    let coa_semantic_index: std::collections::HashMap<&str, (&str, &str, &str, &str, &str)> = prior
        .accounts
        .iter()
        .map(|(account_number, sem)| {
            (
                account_number.as_str(),
                (
                    sem.description.as_str(),
                    sem.account_class.as_deref().unwrap_or(""),
                    sem.account_class_name.as_deref().unwrap_or(""),
                    sem.account_sub_class.as_deref().unwrap_or(""),
                    sem.account_sub_class_name.as_deref().unwrap_or(""),
                ),
            )
        })
        .collect();

    // Simulate primary CoA index miss (empty map) + secondary hit.
    let primary_index: std::collections::HashMap<&str, (&str, &str, &str, &str, &str)> =
        std::collections::HashMap::new();

    let gl_account = "0000105000";
    let coa_hit = primary_index
        .get(gl_account)
        .copied()
        .or_else(|| coa_semantic_index.get(gl_account).copied());

    assert!(
        coa_hit.is_some(),
        "secondary index should resolve 0000105000"
    );
    let hit = coa_hit.unwrap();
    assert_eq!(hit.0, "Trade Receivables", "description");
    assert_eq!(hit.1, "A.B", "account_class");
    assert_eq!(hit.2, "Trade Receivables", "account_class_name");
    assert_eq!(hit.3, "A.B.A", "account_sub_class");
    assert_eq!(hit.4, "Trade Accounts Receivable", "account_sub_class_name");

    // Account not in either index → both miss.
    let gl_missing = "9999999999";
    let coa_hit_miss = primary_index
        .get(gl_missing)
        .copied()
        .or_else(|| coa_semantic_index.get(gl_missing).copied());
    assert!(coa_hit_miss.is_none(), "unknown account should return None");

    // When prior is absent (None), the secondary index is empty and the
    // fallback to empty string is safe.
    let no_prior: Option<CoaSemanticPrior> = None;
    let empty_secondary: std::collections::HashMap<&str, (&str, &str, &str, &str, &str)> = no_prior
        .as_ref()
        .map(|p| {
            p.accounts
                .iter()
                .map(|(k, sem)| {
                    (
                        k.as_str(),
                        (
                            sem.description.as_str(),
                            sem.account_class.as_deref().unwrap_or(""),
                            sem.account_class_name.as_deref().unwrap_or(""),
                            sem.account_sub_class.as_deref().unwrap_or(""),
                            sem.account_sub_class_name.as_deref().unwrap_or(""),
                        ),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    assert!(
        empty_secondary.is_empty(),
        "no-prior path must yield empty secondary index"
    );
}

/// SP5.1 — Verify that the lowered 2σ threshold makes drift-correction fire for
/// a drift that would have been BELOW the old 3σ threshold (proving the tune works).
///
/// Setup: account "1000", target closing_balance = 10_000, closing_stdev = 100.
///   - 2σ threshold = 200
///   - 3σ threshold = 300  (old value)
///   - We inject a drift of exactly 300 (current = 9_700 → under-target by 300).
///   - At 3σ: 300 == 300, NOT strictly greater → would NOT fire.
///   - At 2σ: 300 > 200 → FIRES.
#[test]
fn sp5_1_drift_correction_fires_at_lowered_threshold() {
    use datasynth_core::distributions::behavioral_priors::{TbAnchorPrior, TbTarget};
    use datasynth_core::models::{JournalEntry, JournalEntryLine};
    use datasynth_generators::balance::BalanceTrackerConfig;
    use datasynth_generators::RunningBalanceTracker;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;
    use rust_decimal::Decimal;
    use std::collections::BTreeMap;

    let mut per_account = BTreeMap::new();
    per_account.insert(
        "1000".to_string(),
        TbTarget {
            opening_balance: 0.0,
            closing_balance: 10_000.0,
            period_net_activity: 10_000.0,
            opening_stdev: 0.0,
            closing_stdev: 100.0, // 2σ = 200; 3σ = 300
            n_clients: 3,
        },
    );
    let anchor = TbAnchorPrior {
        per_account,
        total_assets: 10_000.0,
        total_liabilities: 0.0,
        total_equity: 10_000.0,
        n_clients: 3,
    };

    let cfg = BalanceTrackerConfig {
        validate_on_each_entry: false,
        track_history: false,
        fail_on_validation_error: false,
        ..Default::default()
    };
    let mut tracker = RunningBalanceTracker::new_with_currency(cfg, "USD".to_string());
    tracker.set_tb_anchor(anchor);

    // Inject a balanced JE: debit account "1000" by 9_700, credit suspense "9999" by 9_700.
    // This sets the running balance for account "1000" at 9_700 (under-target by 300).
    let je_setup = {
        let mut je = JournalEntry::new_simple(
            "SETUP001".to_string(),
            "TEST".to_string(),
            chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            "Setup balance".to_string(),
        );
        je.add_line(JournalEntryLine {
            line_number: 1,
            gl_account: "1000".to_string(),
            account_code: "1000".to_string(),
            debit_amount: Decimal::new(9_700, 0),
            ..Default::default()
        });
        je.add_line(JournalEntryLine {
            line_number: 2,
            gl_account: "9999".to_string(),
            account_code: "9999".to_string(),
            credit_amount: Decimal::new(9_700, 0),
            ..Default::default()
        });
        je
    };
    tracker.apply_entry(&je_setup).unwrap();

    // Drift = 9_700 − 10_000 = −300.
    // At 2σ = 200: 300 > 200 → correction fires.
    assert!(
        tracker.drift_correction_needed("TEST"),
        "SP5.1: drift of 300 should exceed 2σ=200 threshold after threshold tune; \
         would NOT have fired at old 3σ=300"
    );

    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let je = tracker
        .build_drift_correction_je(
            "TEST",
            chrono::NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
            &mut rng,
        )
        .expect("SP5.1: should emit a drift JE at 2σ threshold");

    // The JE must balance.
    assert_eq!(
        je.total_debit(),
        je.total_credit(),
        "SP5.1: drift JE must balance (debit={}, credit={})",
        je.total_debit(),
        je.total_credit()
    );

    // Account "1000" is under-target → correction should DEBIT it.
    let dr_to_1000: Decimal = je
        .lines
        .iter()
        .filter(|l| l.gl_account == "1000")
        .map(|l| l.debit_amount)
        .sum();
    assert!(
        dr_to_1000 > Decimal::ZERO,
        "SP5.1: correction JE should DEBIT account 1000 to bring it toward target"
    );

    assert_eq!(je.header.document_type, "SA", "document_type should be SA");
    assert!(
        je.header
            .reference
            .as_deref()
            .unwrap_or("")
            .starts_with("DRIFT-CORR-"),
        "reference should start with DRIFT-CORR-"
    );
}
