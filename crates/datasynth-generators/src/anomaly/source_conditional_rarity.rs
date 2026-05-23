//! SOTA-12 (#140, FINDINGS §13): source-conditional-rarity anomaly tagging.
//!
//! The corpus audit packet showed `source_cond_edge_surprise_max` is the dominant
//! explainer for **100% of top-50 JEs** at production scale. None of the existing
//! synthetic anomaly types target this pattern (`UnusualAccountPair` and
//! `NewCounterparty` operate on global rarity, not source-conditional). This module
//! adds the missing class as a **post-process** over generated JEs — sidestepping the
//! SOTA-8 / SOTA-11 coverage problem because every JE is in the input regardless of
//! which generator produced it.
//!
//! Algorithm (per call):
//!   1. Build per-source empirical PMF `P(account | source)` from the JEs that carry
//!      an SAP source code.
//!   2. Score each JE by Σ over its lines of `-log P(account | source)` — the
//!      source-conditional surprise.
//!   3. Sort descending. The top `rate × n_jes` JEs whose surprise also clears
//!      `min_surprise` are tagged as `RelationalAnomalyType::SourceConditionalRarity`.

use std::collections::HashMap;

use datasynth_core::models::JournalEntry;

/// Configuration for the source-conditional-rarity post-process.
#[derive(Debug, Clone)]
pub struct SourceConditionalRarityConfig {
    /// Fraction of input JEs to tag as anomalous (default 0.01 — matches the
    /// audit-packet hot-list size).
    pub rate: f64,
    /// Minimum `-log P(edge | source)` (summed over JE lines) to consider for tagging.
    /// Guards against tagging mildly-surprising JEs when the rate budget would
    /// otherwise force inclusion. 0.0 disables the floor.
    pub min_surprise: f64,
    /// Per-source line-count floor — sources with fewer lines have unreliable
    /// PMFs and are skipped. Default 5.
    pub min_per_source_lines: u32,
}

impl Default for SourceConditionalRarityConfig {
    fn default() -> Self {
        Self {
            rate: 0.01,
            min_surprise: 5.0,
            min_per_source_lines: 5,
        }
    }
}

const SOURCE_COND_RARITY_LABEL: &str = "SourceConditionalRarity";

/// Tag the top `rate × n_jes` source-conditionally-rare JEs in `entries`, mutating
/// each tagged JE's **header** (`is_anomaly = true`, `anomaly_type = "Source-
/// ConditionalRarity"`) — same pattern as the existing relational anomaly strategies.
/// Returns the count of JEs actually tagged.
pub fn tag_source_conditional_rarity(
    entries: &mut [JournalEntry],
    cfg: &SourceConditionalRarityConfig,
) -> usize {
    if entries.is_empty() || cfg.rate <= 0.0 {
        return 0;
    }

    // 1. Per-source PMF: source -> (account -> count); source -> total_lines.
    let mut acct_count: HashMap<String, HashMap<String, u32>> = HashMap::new();
    let mut total_per_source: HashMap<String, u32> = HashMap::new();
    for je in entries.iter() {
        let src = match je.header.sap_source_code.as_deref() {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => continue,
        };
        let inner = acct_count.entry(src.clone()).or_default();
        for line in &je.lines {
            *inner.entry(line.gl_account.clone()).or_insert(0) += 1;
            *total_per_source.entry(src.clone()).or_insert(0) += 1;
        }
    }

    // 2. Score each JE by Σ -log P(account | source).
    let mut scores: Vec<(usize, f64)> = Vec::with_capacity(entries.len());
    for (idx, je) in entries.iter().enumerate() {
        let src = match je.header.sap_source_code.as_deref() {
            Some(s) if !s.is_empty() => s,
            _ => continue,
        };
        let total = *total_per_source.get(src).unwrap_or(&0);
        if total < cfg.min_per_source_lines {
            continue;
        }
        let total_f = total as f64;
        let pmf = match acct_count.get(src) {
            Some(m) => m,
            None => continue,
        };
        let mut surprise = 0.0_f64;
        for line in &je.lines {
            let count = *pmf.get(&line.gl_account).unwrap_or(&0);
            // additive smoothing so unseen accounts get a finite (large) surprise
            let p = (count as f64 + 0.5) / (total_f + 0.5);
            surprise += -(p.ln());
        }
        scores.push((idx, surprise));
    }

    // 3. Sort descending; pick the top by rate AND clear the floor.
    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let n_top = ((entries.len() as f64) * cfg.rate).round() as usize;
    let n_top = n_top.min(scores.len()).max(1);
    let mut tagged = 0usize;
    for (idx, surprise) in scores.into_iter().take(n_top) {
        if surprise < cfg.min_surprise {
            break; // sorted descending — any subsequent JE is also below threshold
        }
        let je = &mut entries[idx];
        je.header.is_anomaly = true;
        je.header.anomaly_type = Some(SOURCE_COND_RARITY_LABEL.to_string());
        tagged += 1;
    }
    tagged
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use datasynth_core::models::JournalEntryLine;
    use rust_decimal_macros::dec;

    fn make_je(id: &str, source: &str, debit_acct: &str, credit_acct: &str) -> JournalEntry {
        let mut je = JournalEntry::new_simple(
            id.to_string(),
            "C1".to_string(),
            NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            format!("test {id}"),
        );
        je.header.sap_source_code = Some(source.to_string());
        je.add_line(JournalEntryLine {
            line_number: 1,
            gl_account: debit_acct.to_string(),
            debit_amount: dec!(100),
            ..Default::default()
        });
        je.add_line(JournalEntryLine {
            line_number: 2,
            gl_account: credit_acct.to_string(),
            credit_amount: dec!(100),
            ..Default::default()
        });
        je
    }

    #[test]
    fn tags_the_rare_pair_under_a_common_source() {
        // 99 JEs use the common (1000, 2000) pair under source "SA"; 1 uses (9999, 8888).
        let mut entries: Vec<JournalEntry> = (0..99)
            .map(|i| make_je(&format!("je{i}"), "SA", "1000", "2000"))
            .collect();
        entries.push(make_je("rare", "SA", "9999", "8888"));

        let cfg = SourceConditionalRarityConfig {
            rate: 0.02,
            min_surprise: 0.5,
            ..Default::default()
        };
        let tagged = tag_source_conditional_rarity(&mut entries, &cfg);
        assert!(tagged >= 1, "expected ≥ 1 tag, got {tagged}");
        // The "rare" JE (index 99) must be tagged.
        let rare = &entries[99];
        assert!(rare.header.is_anomaly, "rare JE header not flagged");
        assert_eq!(
            rare.header.anomaly_type.as_deref(),
            Some("SourceConditionalRarity")
        );
        // Common JEs must NOT be tagged (allow at most 1 false positive at the rate budget).
        let common_tagged = entries[..99].iter().filter(|e| e.header.is_anomaly).count();
        assert!(
            common_tagged <= 1,
            "common JEs over-tagged: {common_tagged}"
        );
    }

    #[test]
    fn skips_sources_with_too_few_lines() {
        let mut entries = vec![make_je("solo", "ZZ", "1000", "2000")];
        let cfg = SourceConditionalRarityConfig {
            rate: 1.0,
            min_surprise: 0.0,
            ..Default::default()
        };
        let tagged = tag_source_conditional_rarity(&mut entries, &cfg);
        assert_eq!(tagged, 0, "should skip when per-source data is too sparse");
    }

    #[test]
    fn respects_min_surprise_floor() {
        let mut entries: Vec<JournalEntry> = (0..50)
            .map(|i| make_je(&format!("je{i}"), "SA", "1000", "2000"))
            .collect();
        let cfg = SourceConditionalRarityConfig {
            rate: 0.10,
            min_surprise: 100.0,
            ..Default::default()
        };
        let tagged = tag_source_conditional_rarity(&mut entries, &cfg);
        assert_eq!(
            tagged, 0,
            "unreachable min_surprise should suppress tagging"
        );
    }

    #[test]
    fn no_op_on_empty_input() {
        let mut entries: Vec<JournalEntry> = Vec::new();
        assert_eq!(
            tag_source_conditional_rarity(&mut entries, &SourceConditionalRarityConfig::default()),
            0
        );
    }
}
