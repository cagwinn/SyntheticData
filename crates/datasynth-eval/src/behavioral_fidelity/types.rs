//! Core types shared across the behavioral-fidelity module.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

/// One JE line, normalised across corpus and synthetic schemas.
///
/// Optional fields tolerate schema variation between real and synthetic
/// (e.g., `created_at` is only available on the synthetic side).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub source: String,
    pub gl_account: String,
    pub cost_center: Option<String>,
    pub profit_center: Option<String>,
    pub trading_partner: Option<String>,
    pub je_number: String,
    pub je_line_number: String,
    pub effective_date: NaiveDate,
    pub entry_date: NaiveDate,
    pub created_at: Option<DateTime<Utc>>,
    pub functional_amount: f64,
    /// SP4.4 W7.3 — Header-level text. Populated from corpus `JE Description`
    /// column (parquet) or `header_text` (CSV). Empty when missing.
    #[serde(default)]
    pub header_text: String,
    /// SP4.4 W7.3 — Line-level text. Populated from corpus `JE Line Description`
    /// column (parquet) or `line_text` (CSV). Empty when missing.
    #[serde(default)]
    pub line_text: String,
}

/// Which entity columns to evaluate and which attributes feed the P3 graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityProfile {
    pub name: String,
    pub primary_entity: String,
    pub secondary_entity: Option<String>,
    pub timestamp_day: String,
    pub timestamp_intra: Option<String>,
    pub attributes_for_p3: Vec<String>,
    pub value_column: String,
    pub burst_thresholds: Vec<i64>,
}

/// Canonical velocity rule set (10 rules for GL).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleSet {
    pub rules: Vec<VelocityRuleSpec>,
}

/// Spec for a single velocity rule. The interpretation lives in `velocity_rules.rs`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VelocityRuleSpec {
    pub id: String, // "R1".."R10"
    pub description: String,
    pub kind: VelocityRuleKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum VelocityRuleKind {
    CountPerEntityPerDay { threshold: u32 },              // R1
    DistinctAccountsPerEntityPerDay { threshold: u32 },   // R2
    SumAmountPerEntityPerDayAbovePercentile { pct: f64 }, // R3 (p90 default)
    DormantAccountActivity { inactivity_days: i64 },      // R4
    DistinctTradingPartnersPerEntityPerDay { threshold: u32 }, // R5
    AmountSpikeRatio { window_days: i64, ratio: f64 },    // R6
    OffHoursPosting,                                      // R7 (weekday only)
    PostClosePosting { tolerance_business_days: i64 },    // R8
    RoundDollarConcentration { share_threshold: f64 },    // R9
    BackdatingDays { gap_days: i64 },                     // R10
}

/// PR-merge / CI gate behaviour for `behavioral score`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GateThresholds {
    pub fail_if_dr_above: f64,
    pub fail_if_composite_above: f64,
}

impl Default for GateThresholds {
    fn default() -> Self {
        Self {
            fail_if_dr_above: 2.0,
            fail_if_composite_above: 1.5,
        }
    }
}

/// Optional period subsetting for the loader.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PeriodFilter {
    pub start: NaiveDate,
    pub end: NaiveDate,
}

/// Top-level config consumed by `compute_report`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BehavioralFidelityConfig {
    pub profile: EntityProfile,
    pub rule_set: RuleSet,
    pub seed: u64,
    pub fail_thresholds: GateThresholds,
    pub period_filter: Option<PeriodFilter>,
    pub client_filter: Option<Vec<String>>,
}

impl BehavioralFidelityConfig {
    /// Default GL profile: Source primary, Trading Partner secondary, EntryDate day, full R1..R10.
    pub fn gl_default() -> Self {
        Self {
            profile: EntityProfile::gl_source_tp_static(),
            rule_set: RuleSet::canonical_gl_rules(),
            seed: 42,
            fail_thresholds: GateThresholds::default(),
            period_filter: None,
            client_filter: None,
        }
    }
}

impl EntityProfile {
    /// Internal builder used by `gl_default` before `entity_profile.rs` lands.
    /// Task 3 promotes this to a public function on `entity_profile.rs`.
    pub(crate) fn gl_source_tp_static() -> Self {
        Self {
            name: "gl-source-tp".to_string(),
            primary_entity: "Source".to_string(),
            secondary_entity: Some("TradingPartner".to_string()),
            timestamp_day: "EntryDate".to_string(),
            timestamp_intra: Some("CreatedAt".to_string()),
            attributes_for_p3: vec![
                "GLAccount".to_string(),
                "CostCenter".to_string(),
                "ProfitCenter".to_string(),
                "TradingPartner".to_string(),
            ],
            value_column: "FunctionalAmount".to_string(),
            burst_thresholds: vec![1, 3, 7],
        }
    }
}

impl RuleSet {
    pub fn canonical_gl_rules() -> Self {
        Self {
            rules: vec![
                VelocityRuleSpec {
                    id: "R1".into(),
                    description: ">5 JEs / Source / business day".into(),
                    kind: VelocityRuleKind::CountPerEntityPerDay { threshold: 5 },
                },
                VelocityRuleSpec {
                    id: "R2".into(),
                    description: ">10 distinct GL accounts / Source / day".into(),
                    kind: VelocityRuleKind::DistinctAccountsPerEntityPerDay { threshold: 10 },
                },
                VelocityRuleSpec {
                    id: "R3".into(),
                    description: "Sum |amount| / Source / day > p90".into(),
                    kind: VelocityRuleKind::SumAmountPerEntityPerDayAbovePercentile { pct: 0.90 },
                },
                VelocityRuleSpec {
                    id: "R4".into(),
                    description: "Posting to account dormant >=180 days".into(),
                    kind: VelocityRuleKind::DormantAccountActivity {
                        inactivity_days: 180,
                    },
                },
                VelocityRuleSpec {
                    id: "R5".into(),
                    description: ">3 distinct Trading Partners / Source / day".into(),
                    kind: VelocityRuleKind::DistinctTradingPartnersPerEntityPerDay { threshold: 3 },
                },
                VelocityRuleSpec {
                    id: "R6".into(),
                    description: "max/median amount per Source in 30d > 3.0".into(),
                    kind: VelocityRuleKind::AmountSpikeRatio {
                        window_days: 30,
                        ratio: 3.0,
                    },
                },
                VelocityRuleSpec {
                    id: "R7".into(),
                    description: "Off-hours posting (EntryDate weekday in Sat/Sun)".into(),
                    kind: VelocityRuleKind::OffHoursPosting,
                },
                VelocityRuleSpec {
                    id: "R8".into(),
                    description: "Post-close posting (EntryDate > period_end + 5bd)".into(),
                    kind: VelocityRuleKind::PostClosePosting {
                        tolerance_business_days: 5,
                    },
                },
                VelocityRuleSpec {
                    id: "R9".into(),
                    description: "Round-dollar share (|amt| mod 1000 == 0) > 10%".into(),
                    kind: VelocityRuleKind::RoundDollarConcentration {
                        share_threshold: 0.10,
                    },
                },
                VelocityRuleSpec {
                    id: "R10".into(),
                    description: "Backdating (EffectiveDate − EntryDate > 30d)".into(),
                    kind: VelocityRuleKind::BackdatingDays { gap_days: 30 },
                },
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn record_construction_roundtrips() {
        let r = Record {
            source: "KR".to_string(),
            gl_account: "1100".to_string(),
            cost_center: Some("CC100".to_string()),
            profit_center: Some("PC100".to_string()),
            trading_partner: Some("TP1".to_string()),
            je_number: "2022-0090-001".to_string(),
            je_line_number: "001".to_string(),
            effective_date: NaiveDate::from_ymd_opt(2022, 4, 25).unwrap(),
            entry_date: NaiveDate::from_ymd_opt(2022, 4, 14).unwrap(),
            created_at: None,
            functional_amount: 761.65,
            header_text: String::new(),
            line_text: String::new(),
        };
        assert_eq!(r.source, "KR");
        assert_eq!(r.je_line_number, "001");
    }

    #[test]
    fn gl_default_config_is_source_tp_profile() {
        let cfg = BehavioralFidelityConfig::gl_default();
        assert_eq!(cfg.profile.name, "gl-source-tp");
        assert_eq!(cfg.profile.primary_entity, "Source");
        assert_eq!(
            cfg.profile.secondary_entity.as_deref(),
            Some("TradingPartner")
        );
        assert_eq!(cfg.profile.timestamp_day, "EntryDate");
        assert_eq!(cfg.profile.value_column, "FunctionalAmount");
        assert_eq!(cfg.profile.burst_thresholds, vec![1, 3, 7]);
        assert_eq!(cfg.seed, 42);
        assert!((cfg.fail_thresholds.fail_if_dr_above - 2.0).abs() < 1e-9);
        assert!((cfg.fail_thresholds.fail_if_composite_above - 1.5).abs() < 1e-9);
    }
}
