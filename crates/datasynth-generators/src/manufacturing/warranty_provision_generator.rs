//! Warranty provision generator — IAS 37 / ASC 450.
//!
//! Examines quality inspection failure rates from completed production orders
//! and creates Provision records (warranty type), ProvisionMovement roll-forwards,
//! and balanced journal entries (DR Warranty Expense 5400 / CR Warranty Provision 2410).
//!
//! A provision is only recognised when the defect rate meets the 1% materiality
//! threshold, consistent with the probable-outflow criterion in IAS 37.14 and
//! ASC 450-20-25-2.

use chrono::{Datelike, NaiveDate};
use datasynth_core::accounts::manufacturing_accounts;
use datasynth_core::models::{
    InspectionResult, JournalEntry, JournalEntryLine, ProductionOrder, ProductionOrderStatus,
    Provision, ProvisionMovement, ProvisionType, QualityInspection,
};
use datasynth_core::utils::seeded_rng;
use datasynth_core::uuid_factory::{DeterministicUuidFactory, GeneratorType};
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use rust_decimal::Decimal;
use tracing::debug;

// ---------------------------------------------------------------------------
// Threshold
// ---------------------------------------------------------------------------

/// Minimum defect rate (1%) required to recognise a warranty provision.
const DEFECT_RATE_THRESHOLD: f64 = 0.01;

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

/// Result of warranty provision generation.
#[derive(Debug, Default)]
pub struct WarrantyProvisionResult {
    /// Recognised warranty provisions (IAS 37 / ASC 450).
    pub provisions: Vec<Provision>,
    /// Roll-forward movements for each provision.
    pub movements: Vec<ProvisionMovement>,
    /// Balanced journal entries (DR Warranty Expense / CR Warranty Provision).
    pub journal_entries: Vec<JournalEntry>,
}

// ---------------------------------------------------------------------------
// Generator
// ---------------------------------------------------------------------------

/// Generates warranty provisions driven by quality inspection failure rates.
pub struct WarrantyProvisionGenerator {
    rng: ChaCha8Rng,
    uuid_factory: DeterministicUuidFactory,
}

impl WarrantyProvisionGenerator {
    /// Create a new generator with the given seed.
    pub fn new(seed: u64) -> Self {
        Self {
            rng: seeded_rng(seed, 0),
            uuid_factory: DeterministicUuidFactory::new(seed, GeneratorType::Provision),
        }
    }

    /// Generate warranty provisions from production orders and quality inspections.
    ///
    /// # Arguments
    ///
    /// * `company_code` - Company / entity code.
    /// * `orders` - Production orders to analyse.
    /// * `inspections` - Quality inspections linked to the orders.
    /// * `currency` - ISO 4217 reporting currency (e.g. "USD").
    /// * `framework` - Accounting framework: `"IFRS"` or `"US_GAAP"`.
    ///
    /// # Returns
    ///
    /// A [`WarrantyProvisionResult`] containing provisions, movements, and journal entries.
    /// Returns an empty result when the defect rate falls below the 1% threshold.
    pub fn generate(
        &mut self,
        company_code: &str,
        orders: &[ProductionOrder],
        inspections: &[QualityInspection],
        currency: &str,
        framework: &str,
    ) -> WarrantyProvisionResult {
        // ------------------------------------------------------------------
        // 1. Compute defect rate from inspections
        // ------------------------------------------------------------------
        let total_inspections = inspections.len();
        if total_inspections == 0 {
            debug!(company_code, "No inspections — skipping warranty provision");
            return WarrantyProvisionResult::default();
        }

        let rejection_count = inspections
            .iter()
            .filter(|i| i.result == InspectionResult::Rejected)
            .count();

        let defect_rate = rejection_count as f64 / total_inspections as f64;

        debug!(
            company_code,
            total_inspections,
            rejection_count,
            defect_rate,
            threshold = DEFECT_RATE_THRESHOLD,
            "Computed defect rate for warranty provision"
        );

        // ------------------------------------------------------------------
        // 2. Materiality gate — below threshold → no provision
        // ------------------------------------------------------------------
        if defect_rate < DEFECT_RATE_THRESHOLD {
            debug!(
                company_code,
                defect_rate, "Defect rate below 1% threshold — no warranty provision recognised"
            );
            return WarrantyProvisionResult::default();
        }

        // ------------------------------------------------------------------
        // 3. Sum actual_cost of completed / closed orders
        // ------------------------------------------------------------------
        let total_production_value: Decimal = orders
            .iter()
            .filter(|o| {
                matches!(
                    o.status,
                    ProductionOrderStatus::Completed | ProductionOrderStatus::Closed
                )
            })
            .map(|o| o.actual_cost)
            .sum();

        if total_production_value <= Decimal::ZERO {
            debug!(
                company_code,
                "No completed orders with cost — no warranty provision"
            );
            return WarrantyProvisionResult::default();
        }

        // ------------------------------------------------------------------
        // 4. Estimate provision amount
        // ------------------------------------------------------------------
        let warranty_cost_factor: f64 = self.rng.random_range(1.2_f64..=2.0_f64);

        let defect_rate_dec = Decimal::from_f64_retain(defect_rate).unwrap_or(Decimal::ZERO);
        let factor_dec = Decimal::from_f64_retain(warranty_cost_factor).unwrap_or(Decimal::from(2));

        let best_estimate = (total_production_value * defect_rate_dec * factor_dec)
            .round_dp(2)
            .max(Decimal::ZERO);

        let range_low = (best_estimate
            * Decimal::from_f64_retain(0.7).unwrap_or(Decimal::new(7, 1)))
        .round_dp(2);

        let range_high = (best_estimate
            * Decimal::from_f64_retain(1.5).unwrap_or(Decimal::new(15, 1)))
        .round_dp(2);

        // ------------------------------------------------------------------
        // 5. Determine posting date (latest actual_end among completed orders,
        //    falling back to planned_end)
        // ------------------------------------------------------------------
        let posting_date: NaiveDate = orders
            .iter()
            .filter(|o| {
                matches!(
                    o.status,
                    ProductionOrderStatus::Completed | ProductionOrderStatus::Closed
                )
            })
            .filter_map(|o| o.actual_end.or(Some(o.planned_end)))
            .max()
            .unwrap_or_else(|| {
                orders
                    .iter()
                    .map(|o| o.planned_end)
                    .max()
                    .unwrap_or_else(|| NaiveDate::from_ymd_opt(2024, 12, 31).unwrap_or_default())
            });

        let quarter = (posting_date.month() - 1) / 3 + 1;
        let period = format!("{}-Q{}", posting_date.year(), quarter);

        // Expected utilization date: one year after posting
        let expected_utilization_date = NaiveDate::from_ymd_opt(
            posting_date.year() + 1,
            posting_date.month(),
            posting_date.day(),
        )
        .unwrap_or(posting_date);

        // ------------------------------------------------------------------
        // 6. Create Provision
        // ------------------------------------------------------------------
        let provision_id = self.uuid_factory.next().to_string();

        let provision = Provision {
            id: provision_id.clone(),
            entity_code: company_code.to_string(),
            provision_type: ProvisionType::Warranty,
            description: format!(
                "Product warranty provision — {} (defect rate {:.1}%)",
                period,
                defect_rate * 100.0
            ),
            best_estimate,
            range_low,
            range_high,
            discount_rate: None, // Short-term warranty, no discounting
            expected_utilization_date,
            framework: framework.to_string(),
            currency: currency.to_string(),
        };

        // ------------------------------------------------------------------
        // 7. Create ProvisionMovement (initial recognition)
        // ------------------------------------------------------------------
        let movement = ProvisionMovement {
            provision_id: provision_id.clone(),
            period: period.clone(),
            opening: Decimal::ZERO,
            additions: best_estimate,
            utilizations: Decimal::ZERO,
            reversals: Decimal::ZERO,
            unwinding_of_discount: Decimal::ZERO,
            closing: best_estimate,
        };

        // ------------------------------------------------------------------
        // 8. Create balanced journal entry
        //    DR Warranty Expense (5400)   best_estimate
        //    CR Warranty Provision (2410) best_estimate
        // ------------------------------------------------------------------
        let mut je = JournalEntry::new_simple(
            format!("JE-WP-{}", provision_id),
            company_code.to_string(),
            posting_date,
            format!(
                "Warranty provision recognition — {} ({} defect rate {:.1}%)",
                period,
                company_code,
                defect_rate * 100.0
            ),
        );
        je.header.currency = currency.to_string();

        let doc_id = je.header.document_id;

        je.add_line(JournalEntryLine::debit(
            doc_id,
            1,
            manufacturing_accounts::WARRANTY_EXPENSE.to_string(),
            best_estimate,
        ));

        je.add_line(JournalEntryLine::credit(
            doc_id,
            2,
            manufacturing_accounts::WARRANTY_PROVISION.to_string(),
            best_estimate,
        ));

        debug_assert!(
            je.is_balanced(),
            "Warranty provision JE must be balanced — debit {best_estimate} ≠ credit {best_estimate}"
        );

        WarrantyProvisionResult {
            provisions: vec![provision],
            movements: vec![movement],
            journal_entries: vec![je],
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use datasynth_core::models::{InspectionResult, ProductionOrderStatus};
    use rust_decimal::Decimal;

    fn make_orders() -> Vec<ProductionOrder> {
        // Minimal completed orders with non-zero actual_cost
        let base_date = NaiveDate::from_ymd_opt(2025, 3, 31).unwrap();
        (0..5)
            .map(|i| ProductionOrder {
                order_id: format!("ORD-{i:03}"),
                company_code: "C001".to_string(),
                material_id: "MAT-001".to_string(),
                material_description: "Widget".to_string(),
                order_type: datasynth_core::models::ProductionOrderType::Standard,
                status: ProductionOrderStatus::Completed,
                planned_quantity: Decimal::from(100),
                actual_quantity: Decimal::from(95),
                scrap_quantity: Decimal::from(5),
                planned_start: NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
                planned_end: base_date,
                actual_start: Some(NaiveDate::from_ymd_opt(2025, 1, 2).unwrap()),
                actual_end: Some(base_date),
                work_center: "WC-100".to_string(),
                routing_id: None,
                planned_cost: Decimal::from(10_000),
                actual_cost: Decimal::from(10_500),
                cost_breakdown: None,
                labor_hours: 40.0,
                machine_hours: 28.0,
                yield_rate: 0.95,
                batch_number: None,
                operations: vec![],
            })
            .collect()
    }

    fn make_inspections(result: InspectionResult) -> Vec<QualityInspection> {
        use datasynth_core::models::{InspectionType, QualityInspection};
        let date = NaiveDate::from_ymd_opt(2025, 3, 31).unwrap();
        (0..10)
            .map(|i| QualityInspection {
                inspection_id: format!("INS-{i:03}"),
                company_code: "C001".to_string(),
                reference_type: "production_order".to_string(),
                reference_id: format!("ORD-{:03}", i % 5),
                material_id: "MAT-001".to_string(),
                material_description: "Widget".to_string(),
                inspection_type: InspectionType::Final,
                inspection_date: date,
                inspector_id: Some("QC-01".to_string()),
                lot_size: Decimal::from(100),
                sample_size: Decimal::from(20),
                defect_count: 0,
                defect_rate: 0.0,
                result,
                characteristics: smallvec::smallvec![],
                disposition: None,
                notes: None,
            })
            .collect()
    }

    #[test]
    fn test_provision_when_above_threshold() {
        let orders = make_orders();
        let mut inspections = make_inspections(InspectionResult::Accepted);
        // Force ~20% rejection to exceed 1% threshold
        for (i, insp) in inspections.iter_mut().enumerate() {
            if i % 5 == 0 {
                insp.result = InspectionResult::Rejected;
            }
        }
        let mut gen = WarrantyProvisionGenerator::new(42);
        let result = gen.generate("C001", &orders, &inspections, "USD", "IFRS");

        assert!(
            !result.provisions.is_empty(),
            "Should create provision above threshold"
        );
        let prov = &result.provisions[0];
        assert_eq!(prov.provision_type, ProvisionType::Warranty);
        assert!(prov.best_estimate > Decimal::ZERO);
        assert!(prov.range_low <= prov.best_estimate);
        assert!(prov.range_high >= prov.best_estimate);
        assert_eq!(prov.entity_code, "C001");
        assert_eq!(prov.framework, "IFRS");
        assert_eq!(prov.currency, "USD");
    }

    #[test]
    fn test_no_provision_below_threshold() {
        let orders = make_orders();
        let inspections = make_inspections(InspectionResult::Accepted);

        let mut gen = WarrantyProvisionGenerator::new(42);
        let result = gen.generate("C001", &orders, &inspections, "USD", "US_GAAP");

        assert!(
            result.provisions.is_empty(),
            "All-passed inspections → no provision"
        );
        assert!(result.journal_entries.is_empty());
        assert!(result.movements.is_empty());
    }

    #[test]
    fn test_je_is_balanced() {
        let orders = make_orders();
        let mut inspections = make_inspections(InspectionResult::Accepted);
        // Force 50% rejection
        for (i, insp) in inspections.iter_mut().enumerate() {
            if i % 2 == 0 {
                insp.result = InspectionResult::Rejected;
            }
        }
        let mut gen = WarrantyProvisionGenerator::new(99);
        let result = gen.generate("C001", &orders, &inspections, "EUR", "IFRS");

        assert!(!result.journal_entries.is_empty());
        for je in &result.journal_entries {
            assert!(je.is_balanced(), "JE must be balanced");
        }
    }

    #[test]
    fn test_movement_identity() {
        let orders = make_orders();
        let mut inspections = make_inspections(InspectionResult::Accepted);
        for insp in inspections.iter_mut() {
            insp.result = InspectionResult::Rejected;
        }
        let mut gen = WarrantyProvisionGenerator::new(7);
        let result = gen.generate("C001", &orders, &inspections, "USD", "IFRS");

        assert!(!result.movements.is_empty());
        for mvmt in &result.movements {
            // opening + additions - utilizations - reversals + unwinding = closing
            let computed = mvmt.opening + mvmt.additions - mvmt.utilizations - mvmt.reversals
                + mvmt.unwinding_of_discount;
            assert_eq!(
                computed, mvmt.closing,
                "ProvisionMovement identity must hold"
            );
        }
    }

    #[test]
    fn test_je_accounts() {
        let orders = make_orders();
        let mut inspections = make_inspections(InspectionResult::Accepted);
        for (i, insp) in inspections.iter_mut().enumerate() {
            if i % 3 == 0 {
                insp.result = InspectionResult::Rejected;
            }
        }
        let mut gen = WarrantyProvisionGenerator::new(13);
        let result = gen.generate("C001", &orders, &inspections, "USD", "IFRS");

        assert!(!result.journal_entries.is_empty());
        let je = &result.journal_entries[0];
        let debit_line = je
            .lines
            .iter()
            .find(|l| l.debit_amount > Decimal::ZERO)
            .unwrap();
        let credit_line = je
            .lines
            .iter()
            .find(|l| l.credit_amount > Decimal::ZERO)
            .unwrap();
        assert_eq!(
            debit_line.gl_account,
            manufacturing_accounts::WARRANTY_EXPENSE
        );
        assert_eq!(
            credit_line.gl_account,
            manufacturing_accounts::WARRANTY_PROVISION
        );
    }
}
