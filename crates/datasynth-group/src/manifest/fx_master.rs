//! FX rate master resolution for the manifest (spec §4.1).
//!
//! Validates the user-supplied rate table covers every currency pair the
//! entity set needs, and pre-computes the closing / average / historical
//! rate-lookup structures that aggregate-phase IAS 21 translation will use.

use crate::config::{FxConfig, FxPolicyConfig, FxRateSource};
use crate::errors::{GroupError, GroupResult};
use crate::manifest::expansion::ExpandedEntity;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use std::collections::{BTreeMap, BTreeSet};

/// Resolved FX rate master for an engagement period.
///
/// Provides pre-computed closing and average rates that IAS 21 translation
/// (Chunk 6) consumes without needing access to the raw `FxConfig`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FxRateMaster {
    /// The group presentation currency — equals `cfg.base_currency`.
    pub base_currency: String,

    /// Rate policy copied from `FxConfig` for downstream consumers.
    pub policy: FxPolicyConfig,

    /// Rate table keyed as `"FUNCTIONAL/PRESENTATION"` (canonical direction).
    ///
    /// `rates[pair_key][date] = rate`.  Dates that fell outside
    /// `[period_start, period_end]` are excluded; inverted pairs have been
    /// normalised to the canonical direction.
    pub rates: BTreeMap<String, BTreeMap<NaiveDate, Decimal>>,

    /// Closing rate per required pair (spot at `period_end`, or latest
    /// date at or before `period_end` within the period).
    pub closing_by_pair: BTreeMap<String, Decimal>,

    /// Period-average rate per required pair.
    ///
    /// Computed as the simple arithmetic mean of all rates within
    /// `[period_start, period_end]`.  IAS 21 technically requires a
    /// transaction-weighted average; v5.0 uses this simpler proxy because
    /// the full transaction stream is not yet available at manifest build
    /// time.  Aggregate-phase translation may optionally refine this using
    /// the generated JE amounts.
    pub average_by_pair: BTreeMap<String, Decimal>,
}

/// Build and validate the [`FxRateMaster`] for the engagement period.
///
/// # Errors
///
/// - [`GroupError::Config`] if `rate_source` is `HistoricalSeries` (not
///   supported in v5.0).
/// - [`GroupError::Config`] if `cfg.base_currency` ≠ `presentation_currency`.
/// - [`GroupError::Config`] if any required `FUNCTIONAL/PRESENTATION` pair
///   has no rate in `[period_start, period_end]`.
pub fn build_fx_master(
    cfg: &FxConfig,
    presentation_currency: &str,
    period_start: NaiveDate,
    period_end: NaiveDate,
    expanded_entities: &[ExpandedEntity],
) -> GroupResult<FxRateMaster> {
    // ── 1. Validate rate_source ──────────────────────────────────────────────
    if cfg.rate_source == FxRateSource::HistoricalSeries {
        return Err(GroupError::Config(
            "rate_source: historical_series not supported in v5.0 — use inline or user_supplied"
                .to_string(),
        ));
    }

    // ── 2. Validate base_currency ────────────────────────────────────────────
    if cfg.base_currency != presentation_currency {
        return Err(GroupError::Config(format!(
            "fx.base_currency {} does not match group presentation_currency {}",
            cfg.base_currency, presentation_currency
        )));
    }

    // ── 3. Discover needed currency pairs ────────────────────────────────────
    let mut needed_pairs: BTreeSet<String> = BTreeSet::new();
    for entity in expanded_entities {
        if entity.functional_currency != presentation_currency {
            let pair = format!("{}/{}", entity.functional_currency, presentation_currency);
            needed_pairs.insert(pair);
        }
    }

    // Short-circuit: no cross-currency entities → empty master.
    if needed_pairs.is_empty() {
        return Ok(FxRateMaster {
            base_currency: presentation_currency.to_string(),
            policy: cfg.policy.clone(),
            rates: BTreeMap::new(),
            closing_by_pair: BTreeMap::new(),
            average_by_pair: BTreeMap::new(),
        });
    }

    // ── 4. Validate rate coverage (with auto-inversion) ──────────────────────
    //
    // For each required canonical pair `FUNC/PRES`, look for:
    //   a) `FUNC/PRES` in cfg.rates (canonical — use directly), or
    //   b) `PRES/FUNC` in cfg.rates (inverted — store 1/rate under canonical key).
    //
    // Rates outside [period_start, period_end] are silently dropped.

    let mut resolved_rates: BTreeMap<String, BTreeMap<NaiveDate, Decimal>> = BTreeMap::new();

    for pair in &needed_pairs {
        // pair = "FUNC/PRES", e.g. "USD/CHF"
        let (func, pres) = split_pair(pair)?;
        let inverted_key = format!("{pres}/{func}");

        let period_filtered: BTreeMap<NaiveDate, Decimal>;

        if let Some(table) = cfg.rates.get(pair) {
            // Canonical direction — filter to period.
            period_filtered = table
                .range(period_start..=period_end)
                .map(|(d, r)| (*d, *r))
                .collect();
        } else if let Some(table) = cfg.rates.get(&inverted_key) {
            // Inverted direction — invert each rate and store under canonical key.
            period_filtered = table
                .range(period_start..=period_end)
                .filter_map(|(d, r)| {
                    if r.is_zero() {
                        None // skip zero rates — can't invert
                    } else {
                        Some((*d, Decimal::ONE / r))
                    }
                })
                .collect();
        } else {
            return Err(GroupError::Config(format!(
                "fx.rates missing pair {pair} for period [{period_start}, {period_end}]"
            )));
        }

        if period_filtered.is_empty() {
            return Err(GroupError::Config(format!(
                "fx.rates missing pair {pair} for period [{period_start}, {period_end}]"
            )));
        }

        resolved_rates.insert(pair.clone(), period_filtered);
    }

    // ── 5. Compute closing_by_pair ───────────────────────────────────────────
    let mut closing_by_pair: BTreeMap<String, Decimal> = BTreeMap::new();
    for (pair, table) in &resolved_rates {
        // Latest rate at or before period_end within the resolved table.
        // Because table is already filtered to [start, end], the last entry
        // is the one with the greatest date ≤ period_end.
        let closing =
            table.values().last().copied().ok_or_else(|| {
                GroupError::Config(format!("no closing rate found for pair {pair}"))
            })?;
        closing_by_pair.insert(pair.clone(), closing);
    }

    // ── 6. Compute average_by_pair ───────────────────────────────────────────
    let mut average_by_pair: BTreeMap<String, Decimal> = BTreeMap::new();
    for (pair, table) in &resolved_rates {
        let count = table.len() as u32;
        let sum: Decimal = table.values().copied().sum();
        let average = sum / Decimal::from(count);
        average_by_pair.insert(pair.clone(), average);
    }

    Ok(FxRateMaster {
        base_currency: presentation_currency.to_string(),
        policy: cfg.policy.clone(),
        rates: resolved_rates,
        closing_by_pair,
        average_by_pair,
    })
}

/// Split a `"FUNC/PRES"` pair key into its two components.
fn split_pair(pair: &str) -> GroupResult<(&str, &str)> {
    let mut parts = pair.splitn(2, '/');
    let func = parts.next().ok_or_else(|| {
        GroupError::Config(format!("malformed pair key '{pair}' — expected FUNC/PRES"))
    })?;
    let pres = parts.next().ok_or_else(|| {
        GroupError::Config(format!("malformed pair key '{pair}' — expected FUNC/PRES"))
    })?;
    Ok((func, pres))
}
