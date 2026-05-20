//! Entity profile preset + column alias maps for SP1.
//!
//! `gl_source_tp()` is the canonical Source + Trading Partner profile.
//! Alias maps translate canonical column names (`Source`, `GLAccount`, …)
//! into the corpus and synthetic-output column names.

use crate::behavioral_fidelity::types::EntityProfile;

/// Source + Trading Partner profile, per spec §1.1.
pub fn gl_source_tp() -> EntityProfile {
    EntityProfile::gl_source_tp_static()
}

/// Column-alias map from canonical name -> corpus column name.
/// Synthetic-side mapping is identical for canonical names; the loader
/// applies real-only renames first.
pub fn real_corpus_aliases() -> [(&'static str, &'static str); 13] {
    [
        ("Source", "Source"),
        ("GLAccount", "GL Account Number"),
        ("CostCenter", "Cost Center"),
        ("ProfitCenter", "Profit Center"),
        ("TradingPartner", "Tarding Partner"), // sic — real-data typo
        ("JENumber", "JE Number"),
        ("JELineNumber", "JE Line Number"),
        ("EffectiveDate", "Effective Date"),
        ("EntryDate", "Entry Date"),
        ("FunctionalAmount", "Functional Amount"),
        ("ReportingAmount", "Reporting Amount"),
        // SP4.4 W7.3 — JE description columns for text template extraction.
        ("HeaderText", "JE Description"),
        ("LineText", "JE Line Description"),
    ]
}

/// Synthetic-side column aliases (DataSynth journal_entries output).
pub fn synthetic_aliases() -> [(&'static str, &'static str); 13] {
    [
        ("Source", "source"),
        ("GLAccount", "gl_account"),
        ("CostCenter", "cost_center"),
        ("ProfitCenter", "profit_center"),
        // No trading_partner column in DataSynth's standard journal_entries; loader returns None.
        ("TradingPartner", "trading_partner"),
        ("JENumber", "document_id"),
        ("JELineNumber", "line_number"),
        ("EffectiveDate", "posting_date"),
        // DataSynth has document_date (document creation) as the closest analog to "entry date".
        ("EntryDate", "document_date"),
        ("CreatedAt", "created_at"),
        // local_amount is the signed functional-currency amount in DataSynth output.
        ("FunctionalAmount", "local_amount"),
        // SP4.4 W7.3 — header/line text fields in synthetic output.
        ("HeaderText", "header_text"),
        ("LineText", "line_text"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gl_source_tp_profile_shape() {
        let p = gl_source_tp();
        assert_eq!(p.name, "gl-source-tp");
        assert_eq!(p.primary_entity, "Source");
        assert_eq!(p.secondary_entity.as_deref(), Some("TradingPartner"));
        assert_eq!(p.attributes_for_p3.len(), 4);
    }

    #[test]
    fn real_corpus_aliases_match_observed_columns() {
        let aliases = real_corpus_aliases();
        let by_canon: std::collections::HashMap<_, _> = aliases.into_iter().collect();
        // corpus typo preserved
        assert_eq!(by_canon.get("TradingPartner"), Some(&"Tarding Partner"));
        assert_eq!(by_canon.get("Source"), Some(&"Source"));
        assert_eq!(by_canon.get("EntryDate"), Some(&"Entry Date"));
    }
}
