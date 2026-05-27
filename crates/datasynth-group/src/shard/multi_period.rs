//! C2 (#157) Piece 2 — multi-period chain helper for the shard runner.
//!
//! `build_opening_balances_from_prior` reads each entity's closing
//! trial balance from a prior period's shard output and projects it
//! to next-period opening balances via
//! [`datasynth_generators::balance::project_closing_to_opening`].
//! The result feeds straight into
//! [`super::runner::run_shard_with_opening_balances`].
//!
//! See `docs/design/2026-05-27-c2-multi-period-design.md` for the
//! broader C2 plan.

use std::collections::BTreeMap;
use std::path::Path;

use datasynth_core::models::balance::{EntityOpeningBalance, TrialBalance};
use datasynth_generators::balance::project_closing_to_opening;

use crate::errors::{GroupError, GroupResult};
use crate::manifest::builder::GroupManifest;
use crate::shard::runner::{run_shard_with_opening_balances, ShardSummary};

/// Default accounting framework for the closing → opening projection
/// when the caller doesn't pass an explicit one. Matches the
/// orchestrator's default (`AccountingFramework::UsGaap`).
pub const DEFAULT_FRAMEWORK: &str = "us_gaap";

/// Walk every entity in the manifest, read its prior-period closing
/// trial balance from `prior_period_shards_dir/entities/{code}/period_close/trial_balances.json`,
/// project it to next-period opening balances, and assemble the map
/// `run_shard_with_opening_balances` expects.
///
/// Missing per-entity files are a hard error — the manifest contract
/// commits to every entity being present in the prior shard. If a
/// caller wants to tolerate gaps (engagement-level "skip prior data
/// where absent"), they can wrap this and filter the error.
///
/// `framework` selects the Retained-Earnings account code used by
/// the projection (see `project_closing_to_opening` docs). Pass the
/// same framework string the prior period was generated with;
/// mismatches mean net income is absorbed into the wrong account.
pub fn build_opening_balances_from_prior(
    manifest: &GroupManifest,
    prior_period_shards_dir: &Path,
    framework: &str,
) -> GroupResult<BTreeMap<String, Vec<EntityOpeningBalance>>> {
    let mut openings_by_entity: BTreeMap<String, Vec<EntityOpeningBalance>> = BTreeMap::new();

    for entity in &manifest.ownership_graph.entities {
        let tb_path = prior_period_shards_dir
            .join("entities")
            .join(&entity.code)
            .join("period_close")
            .join("trial_balances.json");
        if !tb_path.exists() {
            return Err(GroupError::Shard(format!(
                "build_opening_balances_from_prior: missing prior-period TB for `{}` at `{}` — \
                 the prior-period shard must contain every entity in the manifest",
                entity.code,
                tb_path.display(),
            )));
        }
        let bytes = std::fs::read(&tb_path).map_err(GroupError::Io)?;
        let tbs: Vec<TrialBalance> = serde_json::from_slice(&bytes)?;
        // The orchestrator emits one or more TB rows per entity — opening,
        // interim, closing. Pick the chronologically latest as the
        // closing TB for the close → opening projection.
        let closing_tb = tbs
            .into_iter()
            .max_by_key(|tb| tb.as_of_date)
            .ok_or_else(|| {
                GroupError::Shard(format!(
                    "build_opening_balances_from_prior: prior-period TB file for `{}` was empty",
                    entity.code,
                ))
            })?;
        let openings = project_closing_to_opening(&closing_tb, framework);
        openings_by_entity.insert(entity.code.clone(), openings);
    }

    Ok(openings_by_entity)
}

/// One-shot multi-period shard runner: load the prior period's
/// closing TBs, project them to openings, run the new period.
///
/// Equivalent to:
///
/// ```ignore
/// let openings = build_opening_balances_from_prior(manifest, prior_dir, framework)?;
/// run_shard_with_opening_balances(manifest, shard_id, out_dir, &openings)
/// ```
///
/// Lifted into its own function so the CLI handler can call it
/// directly instead of stitching the two pieces together inline.
pub fn run_shard_chained(
    manifest: &GroupManifest,
    shard_id: &str,
    out_dir: &Path,
    prior_period_shards_dir: &Path,
    framework: &str,
) -> GroupResult<ShardSummary> {
    let openings = build_opening_balances_from_prior(manifest, prior_period_shards_dir, framework)?;
    run_shard_with_opening_balances(manifest, shard_id, out_dir, &openings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, NaiveDateTime};
    use datasynth_core::models::balance::{
        AccountCategory, AccountType, TrialBalanceLine, TrialBalanceStatus, TrialBalanceType,
    };
    use rust_decimal_macros::dec;
    use std::collections::HashMap;
    use std::fs;
    use tempfile::TempDir;

    fn make_tb(
        company_code: &str,
        as_of_date: NaiveDate,
        lines: Vec<TrialBalanceLine>,
    ) -> TrialBalance {
        let total_debits: rust_decimal::Decimal = lines.iter().map(|l| l.debit_balance).sum();
        let total_credits: rust_decimal::Decimal = lines.iter().map(|l| l.credit_balance).sum();
        TrialBalance {
            trial_balance_id: format!("TB-{company_code}-{as_of_date}"),
            company_code: company_code.to_string(),
            company_name: None,
            as_of_date,
            fiscal_year: as_of_date.format("%Y").to_string().parse().unwrap_or(2026),
            fiscal_period: 12,
            currency: "USD".into(),
            balance_type: TrialBalanceType::PostClosing,
            lines,
            total_debits,
            total_credits,
            is_balanced: total_debits == total_credits,
            out_of_balance: total_debits - total_credits,
            is_equation_valid: true,
            equation_difference: rust_decimal::Decimal::ZERO,
            category_summary: HashMap::new(),
            created_at: NaiveDateTime::default(),
            created_by: "test".into(),
            approved_by: None,
            approved_at: None,
            status: TrialBalanceStatus::Final,
        }
    }

    fn make_line(
        code: &str,
        at: AccountType,
        cat: AccountCategory,
        debit: rust_decimal::Decimal,
        credit: rust_decimal::Decimal,
    ) -> TrialBalanceLine {
        TrialBalanceLine {
            account_code: code.into(),
            account_description: format!("Test {code}"),
            category: cat,
            account_type: at,
            opening_balance: rust_decimal::Decimal::ZERO,
            period_debits: rust_decimal::Decimal::ZERO,
            period_credits: rust_decimal::Decimal::ZERO,
            closing_balance: debit - credit,
            debit_balance: debit,
            credit_balance: credit,
            cost_center: None,
            profit_center: None,
        }
    }

    /// Stage a fake prior-period shard output for `entity_code`:
    /// writes a minimal `trial_balances.json` containing a single
    /// closing TB with one asset, one liability, and one equity row.
    fn stage_prior_shard(
        root: &Path,
        entity_code: &str,
        as_of: NaiveDate,
        assets: rust_decimal::Decimal,
        liab: rust_decimal::Decimal,
        equity: rust_decimal::Decimal,
    ) {
        let pc_dir = root.join("entities").join(entity_code).join("period_close");
        fs::create_dir_all(&pc_dir).unwrap();
        let lines = vec![
            make_line(
                "1000",
                AccountType::Asset,
                AccountCategory::CurrentAssets,
                assets,
                rust_decimal::Decimal::ZERO,
            ),
            make_line(
                "2000",
                AccountType::Liability,
                AccountCategory::CurrentLiabilities,
                rust_decimal::Decimal::ZERO,
                liab,
            ),
            make_line(
                "3000",
                AccountType::Equity,
                AccountCategory::Equity,
                rust_decimal::Decimal::ZERO,
                equity,
            ),
        ];
        let tbs = vec![make_tb(entity_code, as_of, lines)];
        let json = serde_json::to_string(&tbs).unwrap();
        fs::write(pc_dir.join("trial_balances.json"), json).unwrap();
    }

    /// Build a real manifest via the shipped mini_acme.yaml fixture.
    /// We can't realistically hand-roll a `GroupManifest` JSON — the
    /// struct has 18 required fields including a full
    /// `ChartOfAccountsMaster` — so we drive `build_manifest` from a
    /// known-good fixture and then return the resulting manifest.
    /// `entity_codes` is a subset filter so each test can scope down
    /// to the entity codes it stages prior-period TBs for.
    fn build_test_manifest(entity_codes: &[&str]) -> crate::manifest::builder::GroupManifest {
        let yaml = include_str!("../../tests/fixtures/mini_acme.yaml");
        let cfg: crate::config::GroupConfig =
            serde_yaml::from_str(yaml).expect("mini_acme.yaml must parse");
        let mut manifest = crate::manifest::builder::build_manifest(&cfg)
            .expect("build_manifest must succeed for mini_acme");
        // Filter the entities list to just the codes this test cares
        // about so missing-entity errors point at the test's intent
        // rather than at some other mini_acme entity.
        if !entity_codes.is_empty() {
            let want: std::collections::BTreeSet<&str> = entity_codes.iter().copied().collect();
            manifest
                .ownership_graph
                .entities
                .retain(|e| want.contains(e.code.as_str()));
        }
        manifest
    }

    #[test]
    fn build_opening_balances_reads_each_entity_tb() {
        let tmp = TempDir::new().unwrap();
        let prior = tmp.path();
        stage_prior_shard(
            prior,
            "ACME_USA",
            NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
            dec!(10_000),
            dec!(4_000),
            dec!(6_000),
        );
        stage_prior_shard(
            prior,
            "ACME_DE",
            NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
            dec!(20_000),
            dec!(7_000),
            dec!(13_000),
        );

        let manifest = build_test_manifest(&["ACME_USA", "ACME_DE"]);

        let openings = build_opening_balances_from_prior(&manifest, prior, DEFAULT_FRAMEWORK)
            .expect("walk should succeed");

        assert_eq!(openings.len(), 2);
        // Each entity's openings should carry its 3 BS accounts
        // (1000 + 2000 + 3000) — no P&L lines existed in the fixture,
        // so net income is zero and no synthetic RE row is added.
        let usa = openings.get("ACME_USA").expect("ACME_USA missing");
        assert_eq!(
            usa.len(),
            3,
            "expected 3 BS opening rows for ACME_USA, got {}",
            usa.len()
        );
        let eu = openings.get("ACME_DE").expect("ACME_EU missing");
        assert_eq!(eu.len(), 3);
    }

    #[test]
    fn missing_prior_entity_is_a_hard_error() {
        let tmp = TempDir::new().unwrap();
        let prior = tmp.path();
        // Only stage ACME_USA, not ACME_EU — manifest expects both.
        stage_prior_shard(
            prior,
            "ACME_USA",
            NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
            dec!(10_000),
            dec!(4_000),
            dec!(6_000),
        );
        let manifest = build_test_manifest(&["ACME_USA", "ACME_DE"]);

        let err = build_opening_balances_from_prior(&manifest, prior, DEFAULT_FRAMEWORK)
            .expect_err("missing entity must error");
        let msg = format!("{err}");
        assert!(
            msg.contains("ACME_DE") && msg.contains("missing prior-period TB"),
            "error should name the missing entity and be specific: {msg}",
        );
    }

    #[test]
    fn latest_tb_wins_when_multiple_periods_present() {
        let tmp = TempDir::new().unwrap();
        let prior = tmp.path();
        let pc_dir = prior.join("entities").join("ACME_USA").join("period_close");
        fs::create_dir_all(&pc_dir).unwrap();

        // Two TBs: older (Q3) + newer (Q4). The walk should pick the newer.
        let early = make_tb(
            "ACME_USA",
            NaiveDate::from_ymd_opt(2026, 9, 30).unwrap(),
            vec![make_line(
                "1000",
                AccountType::Asset,
                AccountCategory::CurrentAssets,
                dec!(5_000), // smaller — should NOT win
                rust_decimal::Decimal::ZERO,
            )],
        );
        let late = make_tb(
            "ACME_USA",
            NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
            vec![make_line(
                "1000",
                AccountType::Asset,
                AccountCategory::CurrentAssets,
                dec!(10_000), // larger — should win
                rust_decimal::Decimal::ZERO,
            )],
        );
        let tbs = vec![early, late];
        fs::write(
            pc_dir.join("trial_balances.json"),
            serde_json::to_string(&tbs).unwrap(),
        )
        .unwrap();

        let manifest = build_test_manifest(&["ACME_USA"]);
        let openings = build_opening_balances_from_prior(&manifest, prior, DEFAULT_FRAMEWORK)
            .expect("walk should succeed");
        let usa = openings.get("ACME_USA").unwrap();
        assert_eq!(usa.len(), 1);
        assert_eq!(
            usa[0].debit,
            dec!(10_000),
            "should have picked the Q4 TB (10_000), not the Q3 TB (5_000)"
        );
    }
}
