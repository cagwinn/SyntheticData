//! Manufacturing cost accounting journal entry pipeline.
//!
//! Generates the full multi-stage cost flow JEs for production orders:
//! material issue to WIP, labor absorption, overhead application, FG transfer,
//! scrap recognition, and standard cost variance entries.

use chrono::NaiveDate;
use datasynth_core::accounts::{control_accounts, expense_accounts, manufacturing_accounts};
use datasynth_core::models::documents::Delivery;
use datasynth_core::models::{
    InspectionResult, JournalEntry, JournalEntryLine, ProductionOrder, ProductionOrderStatus,
    QualityInspection,
};
use rust_decimal::Decimal;

/// Generates multi-stage manufacturing cost accounting journal entries.
///
/// Covers the full WIP cost flow from material issue through FG transfer,
/// including scrap write-offs and standard cost variance recognition.
pub struct ManufacturingCostAccounting;

impl ManufacturingCostAccounting {
    /// Generate all manufacturing cost flow JEs for a set of production orders.
    ///
    /// # Arguments
    ///
    /// * `orders` - Production orders to generate JEs for.
    /// * `inspections` - Quality inspections linked to the orders (by `reference_id`).
    /// * `currency` - ISO 4217 transaction currency code.
    ///
    /// # Returns
    ///
    /// A `Vec<JournalEntry>` containing all balanced journal entries.
    pub fn generate_all_jes(
        orders: &[ProductionOrder],
        inspections: &[QualityInspection],
        currency: &str,
    ) -> Vec<JournalEntry> {
        let mut jes = Vec::new();

        for order in orders {
            // Skip orders that have no meaningful activity yet.
            // Released orders haven't started production; InProcess and beyond are costed.
            if matches!(
                order.status,
                ProductionOrderStatus::Planned
                    | ProductionOrderStatus::Cancelled
                    | ProductionOrderStatus::Released
            ) {
                continue;
            }

            let cost = match &order.cost_breakdown {
                Some(c) => c,
                None => continue,
            };

            // Use actual_end if available, else planned_end as the posting date.
            let posting_date = order.actual_end.unwrap_or(order.planned_end);

            // ------------------------------------------------------------------
            // 1. Material issue to WIP
            // ------------------------------------------------------------------
            if cost.material_cost > Decimal::ZERO {
                let mut je = JournalEntry::new_simple(
                    format!("JE-MFG-MAT-{}", order.order_id),
                    order.company_code.clone(),
                    posting_date,
                    format!("material issue to WIP for order {}", order.order_id),
                );
                je.header.currency = currency.to_string();
                let doc_id = je.header.document_id;

                je.add_line(JournalEntryLine {
                    document_id: doc_id,
                    line_number: 1,
                    gl_account: manufacturing_accounts::WIP.to_string(),
                    account_code: manufacturing_accounts::WIP.to_string(),
                    debit_amount: cost.material_cost,
                    local_amount: cost.material_cost,
                    reference: Some(order.order_id.clone()),
                    text: Some(format!("WIP - material: {}", order.material_description)),
                    quantity: Some(order.planned_quantity),
                    unit: Some("EA".to_string()),
                    ..Default::default()
                });
                je.add_line(JournalEntryLine {
                    document_id: doc_id,
                    line_number: 2,
                    gl_account: control_accounts::INVENTORY.to_string(),
                    account_code: control_accounts::INVENTORY.to_string(),
                    credit_amount: cost.material_cost,
                    local_amount: -cost.material_cost,
                    reference: Some(order.order_id.clone()),
                    text: Some(format!("Inventory credit: {}", order.material_description)),
                    quantity: Some(order.planned_quantity),
                    unit: Some("EA".to_string()),
                    ..Default::default()
                });
                jes.push(je);
            }

            // ------------------------------------------------------------------
            // 2. Labor absorption to WIP
            // ------------------------------------------------------------------
            if cost.labor_cost > Decimal::ZERO {
                let labor_qty = Decimal::from_f64_retain(order.labor_hours)
                    .unwrap_or(Decimal::ZERO)
                    .round_dp(2);

                let mut je = JournalEntry::new_simple(
                    format!("JE-MFG-LAB-{}", order.order_id),
                    order.company_code.clone(),
                    posting_date,
                    format!("labor absorption to WIP for order {}", order.order_id),
                );
                je.header.currency = currency.to_string();
                let doc_id = je.header.document_id;

                je.add_line(JournalEntryLine {
                    document_id: doc_id,
                    line_number: 1,
                    gl_account: manufacturing_accounts::WIP.to_string(),
                    account_code: manufacturing_accounts::WIP.to_string(),
                    debit_amount: cost.labor_cost,
                    local_amount: cost.labor_cost,
                    reference: Some(order.order_id.clone()),
                    text: Some("WIP - labor absorption".to_string()),
                    quantity: Some(labor_qty),
                    unit: Some("HR".to_string()),
                    ..Default::default()
                });
                je.add_line(JournalEntryLine {
                    document_id: doc_id,
                    line_number: 2,
                    gl_account: manufacturing_accounts::LABOR_ACCRUAL.to_string(),
                    account_code: manufacturing_accounts::LABOR_ACCRUAL.to_string(),
                    credit_amount: cost.labor_cost,
                    local_amount: -cost.labor_cost,
                    reference: Some(order.order_id.clone()),
                    text: Some("Labor accrual credit".to_string()),
                    quantity: Some(labor_qty),
                    unit: Some("HR".to_string()),
                    ..Default::default()
                });
                jes.push(je);
            }

            // ------------------------------------------------------------------
            // 3. Overhead applied to WIP
            // ------------------------------------------------------------------
            if cost.overhead_cost > Decimal::ZERO {
                let mut je = JournalEntry::new_simple(
                    format!("JE-MFG-OVH-{}", order.order_id),
                    order.company_code.clone(),
                    posting_date,
                    format!("overhead applied to WIP for order {}", order.order_id),
                );
                je.header.currency = currency.to_string();
                let doc_id = je.header.document_id;

                je.add_line(JournalEntryLine {
                    document_id: doc_id,
                    line_number: 1,
                    gl_account: manufacturing_accounts::WIP.to_string(),
                    account_code: manufacturing_accounts::WIP.to_string(),
                    debit_amount: cost.overhead_cost,
                    local_amount: cost.overhead_cost,
                    reference: Some(order.order_id.clone()),
                    text: Some("WIP - overhead applied".to_string()),
                    ..Default::default()
                });
                je.add_line(JournalEntryLine {
                    document_id: doc_id,
                    line_number: 2,
                    gl_account: manufacturing_accounts::OVERHEAD_APPLIED.to_string(),
                    account_code: manufacturing_accounts::OVERHEAD_APPLIED.to_string(),
                    credit_amount: cost.overhead_cost,
                    local_amount: -cost.overhead_cost,
                    reference: Some(order.order_id.clone()),
                    text: Some("Overhead applied credit".to_string()),
                    ..Default::default()
                });
                jes.push(je);
            }

            // ------------------------------------------------------------------
            // 4. FG transfer (Completed / Closed only)
            // ------------------------------------------------------------------
            let is_complete = matches!(
                order.status,
                ProductionOrderStatus::Completed | ProductionOrderStatus::Closed
            );
            let total_actual = cost.total_actual();

            // FG transfer uses standard cost so WIP retains the actual-vs-standard
            // residual, which is then cleared by the variance JEs below.
            let total_standard = cost.total_standard();

            if is_complete && total_standard > Decimal::ZERO {
                let mut je = JournalEntry::new_simple(
                    format!("JE-MFG-FG-{}", order.order_id),
                    order.company_code.clone(),
                    posting_date,
                    format!("FG transfer for order {}", order.order_id),
                );
                je.header.currency = currency.to_string();
                let doc_id = je.header.document_id;

                je.add_line(JournalEntryLine {
                    document_id: doc_id,
                    line_number: 1,
                    gl_account: manufacturing_accounts::FINISHED_GOODS.to_string(),
                    account_code: manufacturing_accounts::FINISHED_GOODS.to_string(),
                    debit_amount: total_standard,
                    local_amount: total_standard,
                    reference: Some(order.order_id.clone()),
                    text: Some(format!("FG receipt: {}", order.material_description)),
                    quantity: Some(order.actual_quantity),
                    unit: Some("EA".to_string()),
                    ..Default::default()
                });
                je.add_line(JournalEntryLine {
                    document_id: doc_id,
                    line_number: 2,
                    gl_account: manufacturing_accounts::WIP.to_string(),
                    account_code: manufacturing_accounts::WIP.to_string(),
                    credit_amount: total_standard,
                    local_amount: -total_standard,
                    reference: Some(order.order_id.clone()),
                    text: Some("WIP clearance to FG at standard cost".to_string()),
                    quantity: Some(order.actual_quantity),
                    unit: Some("EA".to_string()),
                    ..Default::default()
                });
                jes.push(je);
            }

            // ------------------------------------------------------------------
            // 5. Scrap JE (at most ONE per order, regardless of how many
            //    rejected inspections exist for it)
            // ------------------------------------------------------------------
            let first_rejected: Option<&QualityInspection> = inspections
                .iter()
                .find(|insp| {
                    insp.reference_id == order.order_id
                        && matches!(insp.result, InspectionResult::Rejected)
                });

            if let Some(insp) = first_rejected {
                let scrap_value = Self::compute_scrap_value(order, total_actual);
                if scrap_value > Decimal::ZERO {
                    let scrap_posting_date = Self::scrap_posting_date(order, posting_date);

                    let mut je = JournalEntry::new_simple(
                        format!("JE-MFG-SCRAP-{}", order.order_id),
                        order.company_code.clone(),
                        scrap_posting_date,
                        format!(
                            "scrap recognition for order {} inspection {}",
                            order.order_id, insp.inspection_id
                        ),
                    );
                    je.header.currency = currency.to_string();
                    let doc_id = je.header.document_id;

                    // Credit WIP for in-process orders, FG for completed/closed.
                    let credit_account = if is_complete {
                        manufacturing_accounts::FINISHED_GOODS
                    } else {
                        manufacturing_accounts::WIP
                    };

                    je.add_line(JournalEntryLine {
                        document_id: doc_id,
                        line_number: 1,
                        gl_account: manufacturing_accounts::SCRAP_EXPENSE.to_string(),
                        account_code: manufacturing_accounts::SCRAP_EXPENSE.to_string(),
                        debit_amount: scrap_value,
                        local_amount: scrap_value,
                        reference: Some(order.order_id.clone()),
                        text: Some(format!(
                            "Scrap expense: {}",
                            insp.disposition.as_deref().unwrap_or("scrap")
                        )),
                        quantity: Some(order.scrap_quantity),
                        unit: Some("EA".to_string()),
                        ..Default::default()
                    });
                    je.add_line(JournalEntryLine {
                        document_id: doc_id,
                        line_number: 2,
                        gl_account: credit_account.to_string(),
                        account_code: credit_account.to_string(),
                        credit_amount: scrap_value,
                        local_amount: -scrap_value,
                        reference: Some(order.order_id.clone()),
                        text: Some(format!("Scrap relief from {}", credit_account)),
                        quantity: Some(order.scrap_quantity),
                        unit: Some("EA".to_string()),
                        ..Default::default()
                    });
                    jes.push(je);
                }
            }

            // ------------------------------------------------------------------
            // 6. Variance JEs (Completed / Closed only)
            // ------------------------------------------------------------------
            if is_complete {
                // Material price variance
                if let Some(je) = Self::variance_je(
                    order,
                    posting_date,
                    currency,
                    cost.material_cost,
                    cost.standard_material_cost,
                    manufacturing_accounts::MATERIAL_PRICE_VARIANCE,
                    "material price variance",
                ) {
                    jes.push(je);
                }

                // Labor rate variance
                if let Some(je) = Self::variance_je(
                    order,
                    posting_date,
                    currency,
                    cost.labor_cost,
                    cost.standard_labor_cost,
                    manufacturing_accounts::LABOR_RATE_VARIANCE,
                    "labor rate variance",
                ) {
                    jes.push(je);
                }

                // Overhead volume variance
                if let Some(je) = Self::variance_je(
                    order,
                    posting_date,
                    currency,
                    cost.overhead_cost,
                    cost.standard_overhead_cost,
                    manufacturing_accounts::OVERHEAD_VOLUME_VARIANCE,
                    "overhead volume variance",
                ) {
                    jes.push(je);
                }
            }
        }

        jes
    }

    /// Generate COGS journal entries when goods are delivered/sold.
    ///
    /// For each delivered line item, looks up the average unit cost of the material
    /// from completed/closed production orders and posts:
    ///   DR COGS ("5000"), CR Finished Goods ("1410")
    ///
    /// # Arguments
    ///
    /// * `deliveries` - Outbound delivery documents from the O2C flow.
    /// * `production_orders` - Production orders used to derive average unit cost.
    ///
    /// # Returns
    ///
    /// One balanced `JournalEntry` per delivery that has at least one costed line item.
    pub fn generate_cogs_on_sale(
        deliveries: &[Delivery],
        production_orders: &[ProductionOrder],
    ) -> Vec<JournalEntry> {
        // ------------------------------------------------------------------
        // Build average-unit-cost map: material_id → avg_unit_cost
        // Only Completed / Closed orders with a cost breakdown contribute.
        // ------------------------------------------------------------------
        use std::collections::HashMap;

        // Accumulate (total_cost, total_qty) per material.
        let mut material_cost: HashMap<&str, (Decimal, Decimal)> = HashMap::new();
        for order in production_orders {
            if !matches!(
                order.status,
                ProductionOrderStatus::Completed | ProductionOrderStatus::Closed
            ) {
                continue;
            }
            if order.actual_quantity <= Decimal::ZERO {
                continue;
            }
            let total = match &order.cost_breakdown {
                Some(c) => c.total_actual(),
                None => order.actual_cost,
            };
            if total <= Decimal::ZERO {
                continue;
            }
            let entry = material_cost
                .entry(order.material_id.as_str())
                .or_insert((Decimal::ZERO, Decimal::ZERO));
            entry.0 += total;
            entry.1 += order.actual_quantity;
        }

        // Derive per-material average unit cost.
        let avg_unit_cost: HashMap<&str, Decimal> = material_cost
            .into_iter()
            .filter(|(_, (_, qty))| *qty > Decimal::ZERO)
            .map(|(mat, (cost, qty))| (mat, (cost / qty).round_dp(4)))
            .collect();

        // ------------------------------------------------------------------
        // Generate one JE per delivery (skip if no costed items).
        // ------------------------------------------------------------------
        let mut jes = Vec::new();

        for delivery in deliveries {
            let posting_date = delivery
                .header
                .posting_date
                .unwrap_or(delivery.header.document_date);

            // Accumulate COGS amount across all line items in this delivery.
            let mut cogs_total = Decimal::ZERO;
            let mut total_qty = Decimal::ZERO;

            for item in &delivery.items {
                let Some(ref mat_id) = item.base.material_id else {
                    continue;
                };
                let Some(&unit_cost) = avg_unit_cost.get(mat_id.as_str()) else {
                    continue;
                };
                let qty = item.base.quantity;
                if qty <= Decimal::ZERO || unit_cost <= Decimal::ZERO {
                    continue;
                }
                cogs_total += (unit_cost * qty).round_dp(2);
                total_qty += qty;
            }

            if cogs_total <= Decimal::ZERO {
                continue;
            }

            let mut je = JournalEntry::new_simple(
                format!("JE-COGS-{}", delivery.header.document_id),
                delivery.header.company_code.clone(),
                posting_date,
                format!("COGS on delivery {}", delivery.header.document_id),
            );
            let doc_id = je.header.document_id;

            // DR COGS
            je.add_line(JournalEntryLine {
                document_id: doc_id,
                line_number: 1,
                gl_account: expense_accounts::COGS.to_string(),
                account_code: expense_accounts::COGS.to_string(),
                debit_amount: cogs_total,
                local_amount: cogs_total,
                reference: Some(delivery.header.document_id.clone()),
                text: Some(format!("COGS - delivery {}", delivery.header.document_id)),
                quantity: Some(total_qty),
                unit: Some("EA".to_string()),
                ..Default::default()
            });

            // CR Finished Goods
            je.add_line(JournalEntryLine {
                document_id: doc_id,
                line_number: 2,
                gl_account: manufacturing_accounts::FINISHED_GOODS.to_string(),
                account_code: manufacturing_accounts::FINISHED_GOODS.to_string(),
                credit_amount: cogs_total,
                local_amount: -cogs_total,
                reference: Some(delivery.header.document_id.clone()),
                text: Some(format!(
                    "FG relief - delivery {}",
                    delivery.header.document_id
                )),
                quantity: Some(total_qty),
                unit: Some("EA".to_string()),
                ..Default::default()
            });

            jes.push(je);
        }

        jes
    }

    /// Build a single variance JE, or return `None` if the variance rounds to zero.
    ///
    /// Convention:
    /// - Unfavorable (actual > standard): DR variance account, CR WIP
    /// - Favorable   (actual < standard): DR WIP, CR variance account
    fn variance_je(
        order: &ProductionOrder,
        posting_date: NaiveDate,
        currency: &str,
        actual: Decimal,
        standard: Decimal,
        variance_account: &str,
        label: &str,
    ) -> Option<JournalEntry> {
        let variance = (actual - standard).round_dp(2);
        if variance == Decimal::ZERO {
            return None;
        }

        let abs_variance = variance.abs();
        let mut je = JournalEntry::new_simple(
            format!("JE-MFG-VAR-{}-{}", label.replace(' ', "-"), order.order_id),
            order.company_code.clone(),
            posting_date,
            format!("{} for order {}", label, order.order_id),
        );
        je.header.currency = currency.to_string();
        let doc_id = je.header.document_id;

        let (debit_acct, credit_acct) = if variance > Decimal::ZERO {
            // Unfavorable: DR variance, CR WIP
            (variance_account, manufacturing_accounts::WIP)
        } else {
            // Favorable: DR WIP, CR variance
            (manufacturing_accounts::WIP, variance_account)
        };

        je.add_line(JournalEntryLine {
            document_id: doc_id,
            line_number: 1,
            gl_account: debit_acct.to_string(),
            account_code: debit_acct.to_string(),
            debit_amount: abs_variance,
            local_amount: abs_variance,
            reference: Some(order.order_id.clone()),
            text: Some(format!("{} - debit", label)),
            ..Default::default()
        });
        je.add_line(JournalEntryLine {
            document_id: doc_id,
            line_number: 2,
            gl_account: credit_acct.to_string(),
            account_code: credit_acct.to_string(),
            credit_amount: abs_variance,
            local_amount: -abs_variance,
            reference: Some(order.order_id.clone()),
            text: Some(format!("{} - credit", label)),
            ..Default::default()
        });

        Some(je)
    }

    /// Compute the scrap value proportional to the order's scrap quantity.
    ///
    /// `scrap_value = total_actual * (scrap_quantity / planned_quantity)`
    fn compute_scrap_value(order: &ProductionOrder, total_actual: Decimal) -> Decimal {
        if order.planned_quantity <= Decimal::ZERO || order.scrap_quantity <= Decimal::ZERO {
            return Decimal::ZERO;
        }
        let fraction = order.scrap_quantity / order.planned_quantity;
        (total_actual * fraction).round_dp(2)
    }

    /// Determine the posting date for a scrap entry.
    ///
    /// Uses `actual_end` when available, otherwise falls back to the order's `posting_date`.
    fn scrap_posting_date(order: &ProductionOrder, fallback: NaiveDate) -> NaiveDate {
        order.actual_end.unwrap_or(fallback)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use datasynth_config::schema::{
        ManufacturingCostingConfig, ProductionOrderConfig, RoutingConfig,
    };
    use datasynth_core::models::ProductionOrderStatus;

    use crate::manufacturing::ProductionOrderGenerator;

    fn make_orders(status: ProductionOrderStatus) -> Vec<ProductionOrder> {
        let mut gen = ProductionOrderGenerator::new(42);
        let config = ProductionOrderConfig::default();
        let costing = ManufacturingCostingConfig::default();
        let routing = RoutingConfig::default();
        let materials = vec![("MAT-001".to_string(), "Widget".to_string())];
        let start = chrono::NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
        let end = chrono::NaiveDate::from_ymd_opt(2025, 1, 31).unwrap();
        let mut orders = gen.generate("C001", &materials, start, end, &config, &costing, &routing);
        for o in &mut orders {
            o.status = status;
            if matches!(
                status,
                ProductionOrderStatus::Completed | ProductionOrderStatus::Closed
            ) {
                o.actual_end = Some(end);
            }
        }
        orders
    }

    #[test]
    fn test_in_process_generates_wip_jes() {
        let orders = make_orders(ProductionOrderStatus::InProcess);
        let jes = ManufacturingCostAccounting::generate_all_jes(&orders, &[], "USD");
        assert!(!jes.is_empty());
        for je in &jes {
            assert!(
                je.is_balanced(),
                "JE '{}' is unbalanced",
                je.description().unwrap_or("")
            );
        }
    }

    #[test]
    fn test_completed_generates_fg_and_variances() {
        let orders = make_orders(ProductionOrderStatus::Completed);
        let jes = ManufacturingCostAccounting::generate_all_jes(&orders, &[], "USD");

        let fg: Vec<_> = jes
            .iter()
            .filter(|je| {
                je.description()
                    .map_or(false, |d| d.contains("FG transfer"))
            })
            .collect();
        assert!(!fg.is_empty(), "Should generate FG transfer JEs");

        let var: Vec<_> = jes
            .iter()
            .filter(|je| je.description().map_or(false, |d| d.contains("variance")))
            .collect();
        assert!(!var.is_empty(), "Should generate variance JEs");

        for je in &jes {
            assert!(
                je.is_balanced(),
                "JE '{}' is unbalanced",
                je.description().unwrap_or("")
            );
        }
    }

    #[test]
    fn test_planned_produces_no_jes() {
        let orders = make_orders(ProductionOrderStatus::Planned);
        let jes = ManufacturingCostAccounting::generate_all_jes(&orders, &[], "USD");
        assert!(jes.is_empty(), "Planned orders should produce no JEs");
    }

    #[test]
    fn test_cancelled_produces_no_jes() {
        let orders = make_orders(ProductionOrderStatus::Cancelled);
        let jes = ManufacturingCostAccounting::generate_all_jes(&orders, &[], "USD");
        assert!(jes.is_empty(), "Cancelled orders should produce no JEs");
    }
}
