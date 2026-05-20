//! Profit centre hierarchy generator.
//!
//! Generates a two-level profit centre hierarchy (segments → sub-segments)
//! per company.  The default template produces 4 level-1 segment nodes and
//! 2-3 level-2 sub-units per segment, resulting in 12-16 profit centres
//! per company — sized to mirror typical mid-market segment reporting.

use datasynth_core::models::{ProfitCenter, ProfitCenterCategory};
use datasynth_core::utils::seeded_rng;
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use tracing::debug;

/// Seed discriminator for profit centre generator (avoids UUID collisions).
const SEED_DISCRIMINATOR: u64 = 0x5043_434e; // "PCCN"

/// Template for generating a top-level profit centre and its sub-units.
struct SegmentTemplate {
    code: &'static str,
    name: &'static str,
    category: ProfitCenterCategory,
    /// IFRS 8 reportable segment code that this top-level node rolls up to.
    /// Multiple top-level nodes can share the same segment_code.
    segment_code: &'static str,
    sub_units: &'static [(&'static str, &'static str)],
}

const SEGMENT_TEMPLATES: &[SegmentTemplate] = &[
    SegmentTemplate {
        code: "CONSUMER",
        name: "Consumer Products",
        category: ProfitCenterCategory::Segment,
        segment_code: "SEG-CONSUMER",
        sub_units: &[
            ("FOOD", "Food & Beverage"),
            ("HHC", "Home & Health Care"),
            ("PERS", "Personal Care"),
        ],
    },
    SegmentTemplate {
        code: "INDUSTRIAL",
        name: "Industrial",
        category: ProfitCenterCategory::Segment,
        segment_code: "SEG-INDUSTRIAL",
        sub_units: &[("MFG", "Manufacturing"), ("CHEM", "Specialty Chemicals")],
    },
    SegmentTemplate {
        code: "SERVICES",
        name: "Services",
        category: ProfitCenterCategory::Service,
        segment_code: "SEG-SERVICES",
        sub_units: &[
            ("PROF", "Professional Services"),
            ("LIC", "Licensing & Royalties"),
        ],
    },
    SegmentTemplate {
        code: "CORP",
        name: "Corporate / Unallocated",
        category: ProfitCenterCategory::Corporate,
        segment_code: "SEG-CORP",
        sub_units: &[("ELIM", "Eliminations & Adjustments")],
    },
];

/// Generator for profit centre hierarchies.
pub struct ProfitCenterGenerator {
    rng: ChaCha8Rng,
}

impl ProfitCenterGenerator {
    /// Create a new profit centre generator with the given seed.
    pub fn new(seed: u64) -> Self {
        Self {
            rng: seeded_rng(seed, SEED_DISCRIMINATOR),
        }
    }

    /// Generate all profit centres for a single company.
    ///
    /// Produces level-1 segment nodes and level-2 sub-unit nodes in a
    /// 2-level hierarchy.  Each segment_code is propagated to every
    /// node in the segment subtree so consumers can group by segment.
    /// Each node has a 20 % chance of being assigned a responsible
    /// manager from `employee_ids` (if provided).
    pub fn generate_for_company(
        &mut self,
        company_code: &str,
        employee_ids: &[String],
    ) -> Vec<ProfitCenter> {
        let mut profit_centers: Vec<ProfitCenter> = Vec::with_capacity(16);

        for tmpl in SEGMENT_TEMPLATES {
            let seg_id = format!("PC-{}-{}", company_code, tmpl.code);

            // Level-1: top-level segment / region / product-group node.
            let mut top = ProfitCenter::top_level(
                seg_id.clone(),
                format!("{} — {}", company_code, tmpl.name),
                company_code,
                tmpl.category,
            );
            top.segment_code = Some(tmpl.segment_code.to_string());
            top.responsible_person = self.pick_employee(employee_ids);
            profit_centers.push(top);

            // Level-2: sub-unit nodes.
            for (sub_code, sub_name) in tmpl.sub_units {
                let sub_id = format!("PC-{}-{}-{}", company_code, tmpl.code, sub_code);
                let mut sub = ProfitCenter::sub_unit(
                    sub_id,
                    format!("{} / {}", tmpl.name, sub_name),
                    seg_id.clone(),
                    company_code,
                    tmpl.category,
                );
                sub.segment_code = Some(tmpl.segment_code.to_string());
                sub.responsible_person = self.pick_employee(employee_ids);
                profit_centers.push(sub);
            }
        }

        debug!(
            company_code,
            count = profit_centers.len(),
            "Generated profit centres"
        );
        profit_centers
    }

    /// Randomly pick an employee ID (20 % chance of assignment).
    fn pick_employee(&mut self, employee_ids: &[String]) -> Option<String> {
        if employee_ids.is_empty() || self.rng.random::<f64>() > 0.20 {
            return None;
        }
        let idx = self.rng.random_range(0..employee_ids.len());
        Some(employee_ids[idx].clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deterministic_generation() {
        let mut g1 = ProfitCenterGenerator::new(42);
        let mut g2 = ProfitCenterGenerator::new(42);
        let employees = vec!["EMP-001".to_string(), "EMP-002".to_string()];
        let p1 = g1.generate_for_company("C001", &employees);
        let p2 = g2.generate_for_company("C001", &employees);
        assert_eq!(p1.len(), p2.len());
        for (a, b) in p1.iter().zip(p2.iter()) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.responsible_person, b.responsible_person);
        }
    }

    #[test]
    fn test_hierarchy_shape() {
        let mut pcgen = ProfitCenterGenerator::new(7);
        let pcs = pcgen.generate_for_company("C001", &[]);

        let level1_count = pcs.iter().filter(|p| p.level == 1).count();
        let level2_count = pcs.iter().filter(|p| p.level == 2).count();
        assert_eq!(level1_count, SEGMENT_TEMPLATES.len());
        assert_eq!(
            level2_count,
            SEGMENT_TEMPLATES
                .iter()
                .map(|t| t.sub_units.len())
                .sum::<usize>()
        );

        // Every level-2 has a parent that is one of the level-1 ids.
        let level1_ids: std::collections::HashSet<&String> =
            pcs.iter().filter(|p| p.level == 1).map(|p| &p.id).collect();
        for sub in pcs.iter().filter(|p| p.level == 2) {
            let parent = sub.parent_id.as_ref().expect("level-2 has parent");
            assert!(level1_ids.contains(parent));
        }
    }

    #[test]
    fn test_segment_code_propagation() {
        let mut pcgen = ProfitCenterGenerator::new(7);
        let pcs = pcgen.generate_for_company("C001", &[]);

        // Every node carries a segment_code, and a top-level node + its
        // children share the same segment_code.
        for pc in &pcs {
            assert!(pc.segment_code.is_some(), "{} missing segment_code", pc.id);
        }
        // Pick the first level-1 + its children and check.
        let parent = pcs.iter().find(|p| p.level == 1).unwrap();
        for child in pcs
            .iter()
            .filter(|p| p.parent_id.as_deref() == Some(parent.id.as_str()))
        {
            assert_eq!(parent.segment_code, child.segment_code);
        }
    }
}
