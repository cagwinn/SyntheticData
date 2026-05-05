//! 15-scenario synthetic engagement library — AuditMethodology v0.14.
//!
//! Sourced from `src/gam_scraper/synthetic/scenarios/library.py`.
//! Each scenario is a deterministic engagement spec → expected
//! outcome (opinion type / going-concern conclusion / EOM paragraph
//! / etc.) used for testing the audit-FSM engine and as a reference
//! corpus for downstream simulation.
//!
//! # Scenarios
//!
//! | # | id | Standard / Issue |
//! |---|----|------------------|
//! | 1  | `clean_engagement` | Unmodified opinion happy path |
//! | 2  | `qualified_opinion_material_misstatement` | ISA 705 — revenue cut-off |
//! | 3  | `going_concern_material_uncertainty` | ISA 570 — EOM paragraph |
//! | 4  | `going_concern_inappropriate` | ISA 570 — adverse opinion |
//! | 5  | `undisclosed_related_party` | ISA 550 |
//! | 6  | `scope_limitation_component` | ISA 600 — disclaimer |
//! | 7  | `subsequent_event_type_1` | ISA 560 — adjusting event |
//! | 8  | `subsequent_event_type_2` | ISA 560 — disclosure event |
//! | 9  | `equity_method_impairment` | IAS 28 |
//! | 10 | `failed_independence_at_acceptance` | IESBA + ISA 220 |
//! | 11 | `nfp_audit_charity_fund_accountability` | NFP single-entity audit |
//! | 12 | `governmental_audit_intosai_considerations` | ISA 200 + ISSAI 100/200/300/400 |
//! | 13 | `ifrs_first_time_adopter` | IFRS 1 transition |
//! | 14 | `segment_reporting_complexity_ifrs_8` | IFRS 8 segment aggregation |
//! | 15 | `equity_method_significant_influence_loss` | IAS 28 § 22 reclassification |

use serde::{Deserialize, Serialize};

// ── Public types ──────────────────────────────────────────────────────────────

/// Top-level engagement scenario.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngagementScenario {
    /// Unique scenario id (matches the upstream Python `scenario_id`).
    pub scenario_id: String,
    /// Human-readable name.
    pub name: String,
    /// Multi-line description.
    pub description: String,
    /// The expected outcome — what an audit FSM run over this
    /// scenario should produce.
    pub expected: ExpectedOutcome,
}

/// Expected outcome of running an FSM engine over the scenario.
/// Mirrors the upstream Python `ExpectedOutcome` Pydantic model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectedOutcome {
    /// ISA 700 / 705 / 706 opinion type.
    pub opinion_type: OpinionType,
    /// ISA 570 going-concern conclusion.
    pub going_concern: GoingConcern,
    /// ISA 320 / 450 misstatement-aggregation verdict — does the
    /// aggregated misstatement remain below performance materiality?
    /// (Upstream Python field: `evaluation_verdict`.)
    #[serde(default = "default_verdict")]
    pub misstatement_verdict: MisstatementVerdict,
    /// Whether an Emphasis-of-Matter paragraph (ISA 706) is required.
    #[serde(default)]
    pub eom_paragraph: bool,
    /// Whether engagement acceptance was blocked (e.g. independence
    /// failure per IESBA / ISA 220).
    #[serde(default)]
    pub acceptance_blocked: bool,
    /// Whether the scenario carries an undisclosed related-party
    /// transaction (ISA 550).
    #[serde(default)]
    pub has_undisclosed_rpt: bool,
    /// Whether the scenario carries a scope limitation (ISA 600 /
    /// ISA 705).
    #[serde(default)]
    pub has_scope_limitation: bool,
    /// Whether the scenario carries a subsequent event and what type.
    #[serde(default = "default_subseq")]
    pub has_subsequent_event: SubsequentEvent,
    /// Free-form notes.
    #[serde(default)]
    pub notes: String,
}

fn default_verdict() -> MisstatementVerdict {
    MisstatementVerdict::Ok
}
fn default_subseq() -> SubsequentEvent {
    SubsequentEvent::None
}

/// ISA 700 / 705 / 706 opinion types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpinionType {
    /// Unmodified (clean) opinion per ISA 700.
    Unqualified,
    /// Qualified opinion per ISA 705.
    Qualified,
    /// Adverse opinion per ISA 705.
    Adverse,
    /// Disclaimer of opinion per ISA 705.
    Disclaimer,
    /// No opinion issued (e.g. engagement withdrawn / blocked).
    NoOpinion,
}

/// ISA 570 going-concern conclusions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoingConcern {
    /// No going-concern issue identified.
    NoConcern,
    /// Going concern relevant but mitigated by management's plans.
    MitigatingFactorsDisclosed,
    /// Material uncertainty about going concern → EOM paragraph.
    MaterialUncertainty,
    /// Going-concern basis of preparation is inappropriate → adverse opinion.
    GoingConcernBasisInappropriate,
}

/// ISA 320 / 450 misstatement-aggregation verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MisstatementVerdict {
    /// Aggregate misstatements are below performance materiality.
    Ok,
    /// Aggregate exceeds performance but stays below overall.
    ExceedsPerformance,
    /// Aggregate exceeds overall materiality.
    ExceedsOverall,
}

/// ISA 560 subsequent-event classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubsequentEvent {
    /// No subsequent event.
    None,
    /// Type 1 — provides additional evidence about conditions
    /// existing at the balance-sheet date (adjusting event).
    Type1,
    /// Type 2 — indicative of conditions arising after the balance-
    /// sheet date (disclosure event).
    Type2,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Return all 15 built-in synthetic engagement scenarios in stable
/// declaration order (matches the upstream Python library iteration order).
pub fn builtin_scenarios() -> Vec<EngagementScenario> {
    vec![
        scenario_clean_engagement(),
        scenario_qualified_opinion_material_misstatement(),
        scenario_going_concern_material_uncertainty(),
        scenario_going_concern_inappropriate(),
        scenario_undisclosed_related_party(),
        scenario_scope_limitation_component(),
        scenario_subsequent_event_type_1(),
        scenario_subsequent_event_type_2(),
        scenario_equity_method_impairment(),
        scenario_failed_independence_at_acceptance(),
        scenario_nfp_audit_charity_fund_accountability(),
        scenario_governmental_audit_intosai_considerations(),
        scenario_ifrs_first_time_adopter(),
        scenario_segment_reporting_complexity_ifrs_8(),
        scenario_equity_method_significant_influence_loss(),
    ]
}

/// Lookup a built-in scenario by id.
pub fn lookup_scenario(scenario_id: &str) -> Option<EngagementScenario> {
    builtin_scenarios()
        .into_iter()
        .find(|s| s.scenario_id == scenario_id)
}

// ── Scenario definitions ──────────────────────────────────────────────────────

fn scenario_clean_engagement() -> EngagementScenario {
    EngagementScenario {
        scenario_id: "clean_engagement".to_string(),
        name: "Clean parent + 3 components engagement".to_string(),
        description: "Standard 3-component group audit; all components clean, no \
                      RPT issues, no going-concern concerns, no scope limitations, \
                      aggregate misstatement well below performance materiality."
            .to_string(),
        expected: ExpectedOutcome::default(),
    }
}

fn scenario_qualified_opinion_material_misstatement() -> EngagementScenario {
    EngagementScenario {
        scenario_id: "qualified_opinion_material_misstatement".to_string(),
        name: "Qualified opinion: material revenue cut-off misstatement at C1".to_string(),
        description: "C1 (significant component) has uncorrected revenue cut-off \
                      misstatement aggregating to ~$1.6M; below performance \
                      materiality individually but SAD aggregation crosses the \
                      performance threshold."
            .to_string(),
        expected: ExpectedOutcome {
            opinion_type: OpinionType::Qualified,
            misstatement_verdict: MisstatementVerdict::ExceedsPerformance,
            notes: "C1 revenue cut-off uncorrected, ~$1.6M aggregate misstatement".to_string(),
            ..Default::default()
        },
    }
}

fn scenario_going_concern_material_uncertainty() -> EngagementScenario {
    EngagementScenario {
        scenario_id: "going_concern_material_uncertainty".to_string(),
        name: "Material uncertainty about going concern (C2 covenant breach)".to_string(),
        description: "C2 has Q3 covenant breach + 2-year recurring loss pattern. \
                      Mitigating factors disclosed but material uncertainty remains. \
                      Group conclusion is MATERIAL_UNCERTAINTY → EOM paragraph."
            .to_string(),
        expected: ExpectedOutcome {
            going_concern: GoingConcern::MaterialUncertainty,
            eom_paragraph: true,
            ..Default::default()
        },
    }
}

fn scenario_going_concern_inappropriate() -> EngagementScenario {
    EngagementScenario {
        scenario_id: "going_concern_inappropriate".to_string(),
        name: "Adverse opinion: going-concern basis inappropriate (parent liquidation)".to_string(),
        description: "Parent in formal liquidation proceedings; going-concern basis \
                      of preparation is inappropriate.  Adverse opinion under ISA 570."
            .to_string(),
        expected: ExpectedOutcome {
            opinion_type: OpinionType::Adverse,
            going_concern: GoingConcern::GoingConcernBasisInappropriate,
            ..Default::default()
        },
    }
}

fn scenario_undisclosed_related_party() -> EngagementScenario {
    EngagementScenario {
        scenario_id: "undisclosed_related_party".to_string(),
        name: "Undisclosed RPT identified at C1".to_string(),
        description: "C1 has a material RPT with controlling shareholder that is \
                      not disclosed in the financial statements.  ISA 550 finding."
            .to_string(),
        expected: ExpectedOutcome {
            opinion_type: OpinionType::Qualified,
            has_undisclosed_rpt: true,
            ..Default::default()
        },
    }
}

fn scenario_scope_limitation_component() -> EngagementScenario {
    EngagementScenario {
        scenario_id: "scope_limitation_component".to_string(),
        name: "Scope limitation at significant component C2".to_string(),
        description: "Group auditor cannot obtain sufficient appropriate audit \
                      evidence regarding C2 (component auditor access denied).  \
                      ISA 600 → disclaimer of opinion."
            .to_string(),
        expected: ExpectedOutcome {
            opinion_type: OpinionType::Disclaimer,
            has_scope_limitation: true,
            ..Default::default()
        },
    }
}

fn scenario_subsequent_event_type_1() -> EngagementScenario {
    EngagementScenario {
        scenario_id: "subsequent_event_type_1".to_string(),
        name: "Subsequent event Type 1: customer bankruptcy provides AR adjustment".to_string(),
        description: "Major customer files for bankruptcy after balance-sheet date \
                      providing evidence of AR collectibility issues that existed at \
                      year-end.  ISA 560 Type-1 adjusting event."
            .to_string(),
        expected: ExpectedOutcome {
            has_subsequent_event: SubsequentEvent::Type1,
            ..Default::default()
        },
    }
}

fn scenario_subsequent_event_type_2() -> EngagementScenario {
    EngagementScenario {
        scenario_id: "subsequent_event_type_2".to_string(),
        name: "Subsequent event Type 2: post-period-end fire requires disclosure".to_string(),
        description: "Major manufacturing facility fire after balance-sheet date.  \
                      ISA 560 Type-2 event — material loss but doesn't relate to \
                      conditions at year-end → disclosure only."
            .to_string(),
        expected: ExpectedOutcome {
            has_subsequent_event: SubsequentEvent::Type2,
            ..Default::default()
        },
    }
}

fn scenario_equity_method_impairment() -> EngagementScenario {
    EngagementScenario {
        scenario_id: "equity_method_impairment".to_string(),
        name: "Equity-method investee impairment (C5_eq)".to_string(),
        description: "Equity-method investee C5_eq shows objective evidence of \
                      impairment under IAS 28 / IAS 36.  Recoverable amount < \
                      carrying.  Impairment loss recognised."
            .to_string(),
        expected: ExpectedOutcome::default(),
    }
}

fn scenario_failed_independence_at_acceptance() -> EngagementScenario {
    EngagementScenario {
        scenario_id: "failed_independence_at_acceptance".to_string(),
        name: "Engagement blocked at acceptance: network-firm conflict".to_string(),
        description: "Network-firm independence conflict identified at acceptance \
                      (significant non-audit service to a non-audit affiliate).  \
                      IESBA / ISA 220 acceptance gate fails."
            .to_string(),
        expected: ExpectedOutcome {
            opinion_type: OpinionType::NoOpinion,
            acceptance_blocked: true,
            ..Default::default()
        },
    }
}

fn scenario_nfp_audit_charity_fund_accountability() -> EngagementScenario {
    EngagementScenario {
        scenario_id: "nfp_audit_charity_fund_accountability".to_string(),
        name: "NFP single-entity audit with donor-fund accountability".to_string(),
        description: "Not-for-profit single-entity audit with restricted vs \
                      unrestricted donor funds.  Expense-driven materiality (revenue \
                      benchmark inappropriate); EOM paragraph for fund-accounting \
                      disclosure."
            .to_string(),
        expected: ExpectedOutcome {
            eom_paragraph: true,
            ..Default::default()
        },
    }
}

fn scenario_governmental_audit_intosai_considerations() -> EngagementScenario {
    EngagementScenario {
        scenario_id: "governmental_audit_intosai_considerations".to_string(),
        name: "Governmental audit under ISA 200 + ISSAI 100/200/300/400".to_string(),
        description: "Governmental audit under ISA 200 with ISSAI 100/200/300/400 \
                      considerations (compliance + performance + financial \
                      dimensions).  No issues, but ISSAI-aligned reporting."
            .to_string(),
        expected: ExpectedOutcome::default(),
    }
}

fn scenario_ifrs_first_time_adopter() -> EngagementScenario {
    EngagementScenario {
        scenario_id: "ifrs_first_time_adopter".to_string(),
        name: "IFRS 1 first-time adoption (transition from local GAAP)".to_string(),
        description: "Entity transitions from local GAAP to IFRS under IFRS 1.  \
                      Reconciliation disclosures are heavy; EOM paragraph for \
                      transition-disclosure quality."
            .to_string(),
        expected: ExpectedOutcome {
            eom_paragraph: true,
            ..Default::default()
        },
    }
}

fn scenario_segment_reporting_complexity_ifrs_8() -> EngagementScenario {
    EngagementScenario {
        scenario_id: "segment_reporting_complexity_ifrs_8".to_string(),
        name: "Listed entity with complex IFRS 8 segment-aggregation judgment".to_string(),
        description: "Listed entity has 7 operating segments aggregated into 3 \
                      reportable segments under IFRS 8.  Aggregation judgment is \
                      KAM consideration."
            .to_string(),
        expected: ExpectedOutcome::default(),
    }
}

fn scenario_equity_method_significant_influence_loss() -> EngagementScenario {
    EngagementScenario {
        scenario_id: "equity_method_significant_influence_loss".to_string(),
        name: "IAS 28 § 22 reclassification at loss of significant influence".to_string(),
        description: "Investor's equity-method investee dilutes its stake below the \
                      significant-influence threshold (20 %).  IAS 28 § 22 requires \
                      reclassification with fair-value remeasurement; one-off P&L \
                      impact."
            .to_string(),
        expected: ExpectedOutcome::default(),
    }
}

impl Default for ExpectedOutcome {
    fn default() -> Self {
        Self {
            opinion_type: OpinionType::Unqualified,
            going_concern: GoingConcern::NoConcern,
            misstatement_verdict: MisstatementVerdict::Ok,
            eom_paragraph: false,
            acceptance_blocked: false,
            has_undisclosed_rpt: false,
            has_scope_limitation: false,
            has_subsequent_event: SubsequentEvent::None,
            notes: String::new(),
        }
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_scenarios_returns_exactly_fifteen() {
        let scenarios = builtin_scenarios();
        assert_eq!(scenarios.len(), 15);
    }

    #[test]
    fn scenario_ids_match_methodology_v014() {
        let scenarios = builtin_scenarios();
        let ids: Vec<&str> = scenarios.iter().map(|s| s.scenario_id.as_str()).collect();
        let expected = [
            "clean_engagement",
            "qualified_opinion_material_misstatement",
            "going_concern_material_uncertainty",
            "going_concern_inappropriate",
            "undisclosed_related_party",
            "scope_limitation_component",
            "subsequent_event_type_1",
            "subsequent_event_type_2",
            "equity_method_impairment",
            "failed_independence_at_acceptance",
            "nfp_audit_charity_fund_accountability",
            "governmental_audit_intosai_considerations",
            "ifrs_first_time_adopter",
            "segment_reporting_complexity_ifrs_8",
            "equity_method_significant_influence_loss",
        ];
        assert_eq!(ids, expected);
    }

    #[test]
    fn clean_engagement_yields_unqualified_no_concern() {
        let s = lookup_scenario("clean_engagement").unwrap();
        assert_eq!(s.expected.opinion_type, OpinionType::Unqualified);
        assert_eq!(s.expected.going_concern, GoingConcern::NoConcern);
        assert!(!s.expected.eom_paragraph);
    }

    #[test]
    fn qualified_misstatement_yields_qualified_exceeds_performance() {
        let s = lookup_scenario("qualified_opinion_material_misstatement").unwrap();
        assert_eq!(s.expected.opinion_type, OpinionType::Qualified);
        assert_eq!(
            s.expected.misstatement_verdict,
            MisstatementVerdict::ExceedsPerformance
        );
    }

    #[test]
    fn going_concern_material_uncertainty_triggers_eom() {
        let s = lookup_scenario("going_concern_material_uncertainty").unwrap();
        assert_eq!(s.expected.going_concern, GoingConcern::MaterialUncertainty);
        assert!(s.expected.eom_paragraph);
    }

    #[test]
    fn going_concern_inappropriate_yields_adverse() {
        let s = lookup_scenario("going_concern_inappropriate").unwrap();
        assert_eq!(s.expected.opinion_type, OpinionType::Adverse);
        assert_eq!(
            s.expected.going_concern,
            GoingConcern::GoingConcernBasisInappropriate
        );
    }

    #[test]
    fn scope_limitation_yields_disclaimer() {
        let s = lookup_scenario("scope_limitation_component").unwrap();
        assert_eq!(s.expected.opinion_type, OpinionType::Disclaimer);
        assert!(s.expected.has_scope_limitation);
    }

    #[test]
    fn subsequent_event_types_classified_correctly() {
        let s1 = lookup_scenario("subsequent_event_type_1").unwrap();
        assert_eq!(s1.expected.has_subsequent_event, SubsequentEvent::Type1);
        let s2 = lookup_scenario("subsequent_event_type_2").unwrap();
        assert_eq!(s2.expected.has_subsequent_event, SubsequentEvent::Type2);
    }

    #[test]
    fn failed_independence_blocks_acceptance_no_opinion() {
        let s = lookup_scenario("failed_independence_at_acceptance").unwrap();
        assert!(s.expected.acceptance_blocked);
        assert_eq!(s.expected.opinion_type, OpinionType::NoOpinion);
    }

    #[test]
    fn undisclosed_rpt_flagged_with_qualified() {
        let s = lookup_scenario("undisclosed_related_party").unwrap();
        assert!(s.expected.has_undisclosed_rpt);
        assert_eq!(s.expected.opinion_type, OpinionType::Qualified);
    }

    #[test]
    fn nfp_audit_eom_for_fund_accounting() {
        let s = lookup_scenario("nfp_audit_charity_fund_accountability").unwrap();
        assert!(s.expected.eom_paragraph);
    }

    #[test]
    fn ifrs_first_time_adopter_eom_for_transition() {
        let s = lookup_scenario("ifrs_first_time_adopter").unwrap();
        assert!(s.expected.eom_paragraph);
    }

    #[test]
    fn lookup_unknown_returns_none() {
        assert!(lookup_scenario("does_not_exist").is_none());
    }

    #[test]
    fn json_round_trips_each_scenario() {
        for s in builtin_scenarios() {
            let json = serde_json::to_string(&s).unwrap();
            let back: EngagementScenario = serde_json::from_str(&json).unwrap();
            assert_eq!(s, back);
        }
    }

    #[test]
    fn opinion_type_serde_uses_snake_case() {
        let s = lookup_scenario("clean_engagement").unwrap();
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"unqualified\""));
        assert!(json.contains("\"no_concern\""));
    }
}
