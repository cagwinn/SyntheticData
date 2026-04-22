//! Relationship-level label generation.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::{
    BankAccount, BankingCustomer, BeneficialOwner, CustomerRelationship, NetworkRole,
};

/// Relationship type for labeling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RelationshipType {
    /// Family relationship
    Family,
    /// Employer-employee relationship
    Employment,
    /// Business partner
    BusinessPartner,
    /// Vendor relationship
    Vendor,
    /// Customer relationship (B2B)
    Customer,
    /// Beneficial ownership
    BeneficialOwnership,
    /// Transaction counterparty
    TransactionCounterparty,
    /// Money mule link
    MuleLink,
    /// Shell company link
    ShellLink,
    /// Unknown/other
    Unknown,
}

/// Relationship-level labels for ML training.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationshipLabel {
    /// Source entity ID
    pub source_id: Uuid,
    /// Target entity ID
    pub target_id: Uuid,
    /// Relationship type
    pub relationship_type: RelationshipType,
    /// Is this a mule network link?
    pub is_mule_link: bool,
    /// Is this a shell company link?
    pub is_shell_link: bool,
    /// Ownership percentage (for UBO edges)
    pub ownership_percent: Option<f64>,
    /// Number of transactions between entities
    pub transaction_count: u32,
    /// Total transaction volume
    pub transaction_volume: f64,
    /// Relationship strength score (0.0-1.0)
    pub strength: f64,
    /// Associated case ID
    pub case_id: Option<String>,
    /// Confidence score
    pub confidence: f64,
}

impl RelationshipLabel {
    /// Create a new relationship label.
    pub fn new(source_id: Uuid, target_id: Uuid, relationship_type: RelationshipType) -> Self {
        Self {
            source_id,
            target_id,
            relationship_type,
            is_mule_link: false,
            is_shell_link: false,
            ownership_percent: None,
            transaction_count: 0,
            transaction_volume: 0.0,
            strength: 0.5,
            case_id: None,
            confidence: 1.0,
        }
    }

    /// Mark as mule link.
    pub fn as_mule_link(mut self) -> Self {
        self.is_mule_link = true;
        self.relationship_type = RelationshipType::MuleLink;
        self
    }

    /// Mark as shell link.
    pub fn as_shell_link(mut self) -> Self {
        self.is_shell_link = true;
        self.relationship_type = RelationshipType::ShellLink;
        self
    }

    /// Set ownership percentage.
    pub fn with_ownership(mut self, percent: f64) -> Self {
        self.ownership_percent = Some(percent);
        self.relationship_type = RelationshipType::BeneficialOwnership;
        self
    }

    /// Set transaction statistics.
    pub fn with_transactions(mut self, count: u32, volume: f64) -> Self {
        self.transaction_count = count;
        self.transaction_volume = volume;
        // Compute strength based on transaction frequency
        self.strength = (count as f64 / 100.0).min(1.0);
        self
    }

    /// Set case ID.
    pub fn with_case(mut self, case_id: &str) -> Self {
        self.case_id = Some(case_id.to_string());
        self
    }
}

/// Relationship label extractor.
pub struct RelationshipLabelExtractor;

impl RelationshipLabelExtractor {
    /// Extract relationship labels from customers.
    pub fn extract_from_customers(customers: &[BankingCustomer]) -> Vec<RelationshipLabel> {
        let mut labels = Vec::new();

        for customer in customers {
            // Extract explicit relationships
            for rel in &customer.relationships {
                let label = Self::from_customer_relationship(customer.customer_id, rel);
                labels.push(label);
            }

            // Extract beneficial ownership relationships
            for bo in &customer.beneficial_owners {
                let label = Self::from_beneficial_owner(customer.customer_id, bo);
                labels.push(label);
            }
        }

        labels
    }

    /// Create label from customer relationship.
    fn from_customer_relationship(
        customer_id: Uuid,
        relationship: &CustomerRelationship,
    ) -> RelationshipLabel {
        use crate::models::RelationshipType as CustRelType;

        let rel_type = match relationship.relationship_type {
            CustRelType::Spouse
            | CustRelType::ParentChild
            | CustRelType::Sibling
            | CustRelType::Family => RelationshipType::Family,
            CustRelType::Employment => RelationshipType::Employment,
            CustRelType::BusinessPartner => RelationshipType::BusinessPartner,
            CustRelType::AuthorizedSigner | CustRelType::JointAccountHolder => {
                RelationshipType::Family
            }
            CustRelType::Beneficiary | CustRelType::TrustRelationship => {
                RelationshipType::BeneficialOwnership
            }
            CustRelType::Guarantor | CustRelType::Attorney => RelationshipType::Unknown,
        };

        RelationshipLabel::new(customer_id, relationship.related_customer_id, rel_type)
    }

    /// Create label from beneficial owner.
    ///
    /// Marks the edge as a `ShellLink` when the beneficial-ownership
    /// structure shows shell-company indicators:
    ///
    /// - `is_hidden` — explicitly hidden ownership (the historical v3 trigger).
    /// - `intermediary_entity.is_some()` — indirect ownership via an
    ///   intermediary entity (the historical v3 trigger).
    /// - v4.4.2+ additions: `is_sanctioned`, high-risk-country UBO residence
    ///   (FATF high-risk / AML-deficient jurisdictions), and `is_pep` without
    ///   verified `source_of_wealth` (classic shell-ownership red flags).
    ///
    /// Before v4.4.2, ShellLink was ~0.3% of all relationship edges because
    /// only the two `is_hidden` / intermediary triggers fired. Adding the
    /// three v4.4.2 triggers brings ShellLink coverage into the 5-10% range,
    /// closer to the MuleLink baseline.
    fn from_beneficial_owner(entity_id: Uuid, bo: &BeneficialOwner) -> RelationshipLabel {
        let ownership_pct: f64 = bo.ownership_percentage.try_into().unwrap_or(0.0);
        let mut label =
            RelationshipLabel::new(bo.ubo_id, entity_id, RelationshipType::BeneficialOwnership)
                .with_ownership(ownership_pct);

        let is_shell = bo.is_hidden
            || bo.intermediary_entity.is_some()
            || bo.is_sanctioned
            || is_high_risk_country(&bo.country_of_residence)
            || (bo.is_pep && bo.source_of_wealth.is_none());

        if is_shell {
            label = label.as_shell_link();
        }

        label
    }

    /// Extract transaction-based relationships between accounts and counterparties.
    ///
    /// Aggregates transactions by `(account_id, counterparty_id)` pairs and emits
    /// one [`RelationshipLabel`] per pair with ≥ 2 transactions. Transactions
    /// participating in a coordinated criminal network (via
    /// [`crate::models::NetworkContext`]) mark the edge as mule or shell based
    /// on the role; uncoordinated-but-suspicious transactions still flag
    /// `is_mule_link`.
    pub fn extract_from_transactions(
        transactions: &[crate::models::BankTransaction],
    ) -> Vec<RelationshipLabel> {
        use std::collections::HashMap;

        #[derive(Default)]
        struct PairInfo {
            count: u32,
            volume: f64,
            suspicious: bool,
            mule: bool,
            shell: bool,
            case_id: Option<String>,
            network_id: Option<String>,
        }

        // Group by (source account, counterparty id) pairs. We can only link
        // transactions where the counterparty is an identified entity in our
        // population — external merchants / unknown peers would generate
        // spurious edges if keyed by name, so they're skipped.
        let mut pairs: HashMap<(Uuid, Uuid), PairInfo> = HashMap::new();

        for txn in transactions {
            let Some(cp_id) = txn.counterparty.counterparty_id else {
                continue;
            };
            let key = (txn.account_id, cp_id);
            let entry = pairs.entry(key).or_default();
            entry.count += 1;
            entry.volume += txn.amount.try_into().unwrap_or(0.0);
            if txn.is_suspicious {
                entry.suspicious = true;
            }
            if let Some(ref nc) = txn.network_context {
                entry.network_id = Some(nc.network_id.clone());
                match nc.network_role {
                    NetworkRole::ShellEntity => entry.shell = true,
                    NetworkRole::Smurf
                    | NetworkRole::Middleman
                    | NetworkRole::CashOut
                    | NetworkRole::Recruiter => entry.mule = true,
                    NetworkRole::Coordinator | NetworkRole::Beneficiary => {
                        // Coordinator/beneficiary links are usually mule-adjacent;
                        // mark as mule link to surface them in AML graphs.
                        entry.mule = true;
                    }
                }
            }
            if entry.case_id.is_none() {
                if let Some(ref c) = txn.case_id {
                    entry.case_id = Some(c.clone());
                }
            }
        }

        pairs
            .into_iter()
            .filter(|(_, info)| info.count >= 2)
            .map(|((account_id, cp_id), info)| {
                let mut label = RelationshipLabel::new(
                    account_id,
                    cp_id,
                    RelationshipType::TransactionCounterparty,
                )
                .with_transactions(info.count, info.volume);

                if let Some(ref case) = info.case_id {
                    label = label.with_case(case);
                }
                if info.shell {
                    label = label.as_shell_link();
                } else if info.mule || info.suspicious {
                    label = label.as_mule_link();
                }
                label
            })
            .collect()
    }

    /// Build customer-to-customer relationship edges from coordinated criminal
    /// networks (e.g., structuring rings, mule chains, shell pyramids).
    ///
    /// Groups transactions by `network_context.network_id`, resolves each
    /// source account to its owning customer, and emits pairwise clique edges
    /// between every pair of participants. This is what closes the "AML
    /// networks are extremely sparse (density 0.0014, zero mule/shell links)"
    /// gap: before this method, `NetworkGenerator` output never made it into
    /// the relationship graph.
    ///
    /// An edge is marked as a shell link if either endpoint played
    /// `ShellEntity`; otherwise as a mule link (since all coordinated criminal
    /// networks are mule-like structures).
    pub fn extract_from_network_contexts(
        transactions: &[crate::models::BankTransaction],
        accounts: &[BankAccount],
    ) -> Vec<RelationshipLabel> {
        use std::collections::{HashMap, HashSet};

        let account_to_customer: HashMap<Uuid, Uuid> = accounts
            .iter()
            .map(|a| (a.account_id, a.primary_owner_id))
            .collect();

        // network_id -> { customer_id -> observed role }
        let mut networks: HashMap<String, HashMap<Uuid, NetworkRole>> = HashMap::new();
        for txn in transactions {
            let Some(ref nc) = txn.network_context else {
                continue;
            };
            let Some(&customer_id) = account_to_customer.get(&txn.account_id) else {
                continue;
            };
            networks
                .entry(nc.network_id.clone())
                .or_default()
                .entry(customer_id)
                .or_insert(nc.network_role);
        }

        let mut labels = Vec::new();
        let mut seen: HashSet<(Uuid, Uuid)> = HashSet::new();
        for (network_id, members) in networks {
            if members.len() < 2 {
                continue;
            }
            let members_vec: Vec<(Uuid, NetworkRole)> = members.into_iter().collect();
            for i in 0..members_vec.len() {
                for j in (i + 1)..members_vec.len() {
                    let (a_id, a_role) = members_vec[i];
                    let (b_id, b_role) = members_vec[j];
                    // Canonicalize pair order so we don't emit both (A,B) and (B,A).
                    let (src, tgt) = if a_id <= b_id {
                        (a_id, b_id)
                    } else {
                        (b_id, a_id)
                    };
                    if !seen.insert((src, tgt)) {
                        continue;
                    }
                    let is_shell = matches!(a_role, NetworkRole::ShellEntity)
                        || matches!(b_role, NetworkRole::ShellEntity);
                    let mut label =
                        RelationshipLabel::new(src, tgt, RelationshipType::TransactionCounterparty)
                            .with_case(&network_id);
                    if is_shell {
                        label = label.as_shell_link();
                    } else {
                        label = label.as_mule_link();
                    }
                    labels.push(label);
                }
            }
        }
        labels
    }
}

/// FATF high-risk / AML-monitored jurisdictions — 2024 list plus
/// commonly-cited offshore shell-company havens. Used by
/// `from_beneficial_owner` to flag UBO residence in these jurisdictions
/// as a shell-link indicator. Conservative list; a customer residing
/// in a listed country is a signal, not a conviction.
fn is_high_risk_country(country_code: &str) -> bool {
    // FATF grey-list + widely-recognised offshore / shell jurisdictions.
    matches!(
        country_code.to_ascii_uppercase().as_str(),
        // FATF grey-list (Jan 2024 — subset; this is a live list)
        "AL" | "BG" | "BF" | "CM" | "HR" | "CD" | "GI" | "HT" | "JM" | "MZ"
        | "NG" | "PH" | "SN" | "SS" | "SY" | "TZ" | "TR" | "UG" | "AE"
        | "VN" | "YE"
        // Classic shell / tax-haven jurisdictions
        | "VG" | "KY" | "BS" | "BZ" | "BM" | "PA" | "LI" | "MT" | "CY" | "AD"
        | "MC" | "SM" | "LU" | "MU" | "SC" | "VU"
        // DPRK / Iran sanctioned (blanket shell indicator)
        | "KP" | "IR"
    )
}

impl RelationshipLabelExtractor {
    /// Get relationship label summary.
    pub fn summarize(labels: &[RelationshipLabel]) -> RelationshipLabelSummary {
        let total = labels.len();
        let mule_links = labels.iter().filter(|l| l.is_mule_link).count();
        let shell_links = labels.iter().filter(|l| l.is_shell_link).count();
        let ownership_links = labels
            .iter()
            .filter(|l| l.ownership_percent.is_some())
            .count();

        let mut by_type = std::collections::HashMap::new();
        for label in labels {
            *by_type.entry(label.relationship_type).or_insert(0) += 1;
        }

        RelationshipLabelSummary {
            total_relationships: total,
            mule_link_count: mule_links,
            mule_link_rate: mule_links as f64 / total.max(1) as f64,
            shell_link_count: shell_links,
            shell_link_rate: shell_links as f64 / total.max(1) as f64,
            ownership_link_count: ownership_links,
            by_type,
        }
    }
}

/// Relationship label summary.
#[derive(Debug, Clone)]
pub struct RelationshipLabelSummary {
    /// Total relationships
    pub total_relationships: usize,
    /// Number of mule links
    pub mule_link_count: usize,
    /// Mule link rate
    pub mule_link_rate: f64,
    /// Number of shell links
    pub shell_link_count: usize,
    /// Shell link rate
    pub shell_link_rate: f64,
    /// Number of ownership links
    pub ownership_link_count: usize,
    /// Counts by relationship type
    pub by_type: std::collections::HashMap<RelationshipType, usize>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_relationship_label() {
        let source = Uuid::new_v4();
        let target = Uuid::new_v4();

        let label = RelationshipLabel::new(source, target, RelationshipType::Family);

        assert_eq!(label.source_id, source);
        assert_eq!(label.target_id, target);
        assert!(!label.is_mule_link);
    }

    #[test]
    fn test_mule_link() {
        let source = Uuid::new_v4();
        let target = Uuid::new_v4();

        let label =
            RelationshipLabel::new(source, target, RelationshipType::Unknown).as_mule_link();

        assert!(label.is_mule_link);
        assert_eq!(label.relationship_type, RelationshipType::MuleLink);
    }

    #[test]
    fn test_ownership_label() {
        let owner = Uuid::new_v4();
        let entity = Uuid::new_v4();

        let label =
            RelationshipLabel::new(owner, entity, RelationshipType::Unknown).with_ownership(25.0);

        assert_eq!(label.ownership_percent, Some(25.0));
        assert_eq!(
            label.relationship_type,
            RelationshipType::BeneficialOwnership
        );
    }

    mod extract_from_transactions {
        use super::*;
        use crate::models::{
            BankTransaction, CounterpartyRef, CounterpartyType, NetworkContext, NetworkRole,
        };
        use chrono::Utc;
        use datasynth_core::banking::{Direction, TransactionCategory, TransactionChannel};
        use rust_decimal::Decimal;

        fn mk_txn(account_id: Uuid, cp_id: Option<Uuid>, amount: i64) -> BankTransaction {
            let counterparty = CounterpartyRef {
                counterparty_type: CounterpartyType::Peer,
                counterparty_id: cp_id,
                name: "Peer".into(),
                account_identifier: None,
                bank_identifier: None,
                country: None,
            };
            BankTransaction::new(
                Uuid::new_v4(),
                account_id,
                Decimal::from(amount),
                "USD",
                Direction::Outbound,
                TransactionChannel::Wire,
                TransactionCategory::TransferOut,
                counterparty,
                "ref",
                Utc::now(),
            )
        }

        #[test]
        fn uses_real_counterparty_id_not_new_v4() {
            let src = Uuid::new_v4();
            let cp = Uuid::new_v4();
            let txns = vec![mk_txn(src, Some(cp), 100), mk_txn(src, Some(cp), 200)];
            let labels = RelationshipLabelExtractor::extract_from_transactions(&txns);
            assert_eq!(labels.len(), 1);
            assert_eq!(labels[0].source_id, src);
            assert_eq!(labels[0].target_id, cp); // not a random v4
            assert_eq!(labels[0].transaction_count, 2);
        }

        #[test]
        fn skips_counterparties_without_id() {
            let src = Uuid::new_v4();
            let txns = vec![mk_txn(src, None, 100), mk_txn(src, None, 200)];
            let labels = RelationshipLabelExtractor::extract_from_transactions(&txns);
            assert!(labels.is_empty());
        }

        #[test]
        fn suspicious_transaction_marks_mule_link() {
            let src = Uuid::new_v4();
            let cp = Uuid::new_v4();
            let mut t1 = mk_txn(src, Some(cp), 100);
            t1.is_suspicious = true;
            let t2 = mk_txn(src, Some(cp), 200);
            let labels = RelationshipLabelExtractor::extract_from_transactions(&[t1, t2]);
            assert_eq!(labels.len(), 1);
            assert!(labels[0].is_mule_link);
            assert!(!labels[0].is_shell_link);
        }

        #[test]
        fn shell_entity_role_marks_shell_link() {
            let src = Uuid::new_v4();
            let cp = Uuid::new_v4();
            let mut t1 = mk_txn(src, Some(cp), 1_000);
            t1.network_context = Some(NetworkContext {
                network_id: "net-1".into(),
                network_role: NetworkRole::ShellEntity,
                co_occurring_typologies: vec![],
                network_size: 5,
            });
            let t2 = mk_txn(src, Some(cp), 1_000);
            let labels = RelationshipLabelExtractor::extract_from_transactions(&[t1, t2]);
            assert_eq!(labels.len(), 1);
            assert!(labels[0].is_shell_link);
            assert_eq!(labels[0].relationship_type, RelationshipType::ShellLink);
        }

        #[test]
        fn smurf_role_marks_mule_link() {
            let src = Uuid::new_v4();
            let cp = Uuid::new_v4();
            let mut t1 = mk_txn(src, Some(cp), 500);
            t1.network_context = Some(NetworkContext {
                network_id: "net-2".into(),
                network_role: NetworkRole::Smurf,
                co_occurring_typologies: vec![],
                network_size: 3,
            });
            let t2 = mk_txn(src, Some(cp), 500);
            let labels = RelationshipLabelExtractor::extract_from_transactions(&[t1, t2]);
            assert_eq!(labels.len(), 1);
            assert!(labels[0].is_mule_link);
            assert!(!labels[0].is_shell_link);
        }

        #[test]
        fn single_transaction_pair_is_filtered_out() {
            let src = Uuid::new_v4();
            let cp = Uuid::new_v4();
            let labels = RelationshipLabelExtractor::extract_from_transactions(&[mk_txn(
                src,
                Some(cp),
                100,
            )]);
            assert!(labels.is_empty());
        }
    }

    mod extract_from_network_contexts {
        use super::*;
        use crate::models::{
            BankAccount, BankTransaction, CounterpartyRef, CounterpartyType, NetworkContext,
            NetworkRole,
        };
        use chrono::{NaiveDate, Utc};
        use datasynth_core::banking::{
            BankAccountType, Direction, TransactionCategory, TransactionChannel,
        };
        use rust_decimal::Decimal;

        fn mk_account(owner: Uuid) -> BankAccount {
            BankAccount::new(
                Uuid::new_v4(),
                "ACC-001".to_string(),
                BankAccountType::Checking,
                owner,
                "USD",
                NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date"),
            )
        }

        fn mk_ctx_txn(account_id: Uuid, network_id: &str, role: NetworkRole) -> BankTransaction {
            let cp = CounterpartyRef {
                counterparty_type: CounterpartyType::Peer,
                counterparty_id: Some(Uuid::new_v4()),
                name: "Peer".into(),
                account_identifier: None,
                bank_identifier: None,
                country: None,
            };
            let mut txn = BankTransaction::new(
                Uuid::new_v4(),
                account_id,
                Decimal::from(1000),
                "USD",
                Direction::Outbound,
                TransactionChannel::Wire,
                TransactionCategory::TransferOut,
                cp,
                "ref",
                Utc::now(),
            );
            txn.network_context = Some(NetworkContext {
                network_id: network_id.to_string(),
                network_role: role,
                co_occurring_typologies: vec![],
                network_size: 3,
            });
            txn
        }

        #[test]
        fn ring_produces_clique_edges() {
            // 3-participant ring → C(3,2) = 3 edges
            let c1 = Uuid::new_v4();
            let c2 = Uuid::new_v4();
            let c3 = Uuid::new_v4();
            let a1 = mk_account(c1);
            let a2 = mk_account(c2);
            let a3 = mk_account(c3);
            let accounts = vec![a1.clone(), a2.clone(), a3.clone()];
            let txns = vec![
                mk_ctx_txn(a1.account_id, "NET-1", NetworkRole::Coordinator),
                mk_ctx_txn(a2.account_id, "NET-1", NetworkRole::Smurf),
                mk_ctx_txn(a3.account_id, "NET-1", NetworkRole::Smurf),
            ];
            let labels =
                RelationshipLabelExtractor::extract_from_network_contexts(&txns, &accounts);
            assert_eq!(labels.len(), 3);
            assert!(labels.iter().all(|l| l.is_mule_link));
            assert!(labels.iter().all(|l| l.case_id.as_deref() == Some("NET-1")));
        }

        #[test]
        fn shell_role_marks_shell_link() {
            let c1 = Uuid::new_v4();
            let c2 = Uuid::new_v4();
            let a1 = mk_account(c1);
            let a2 = mk_account(c2);
            let accounts = vec![a1.clone(), a2.clone()];
            let txns = vec![
                mk_ctx_txn(a1.account_id, "NET-2", NetworkRole::Coordinator),
                mk_ctx_txn(a2.account_id, "NET-2", NetworkRole::ShellEntity),
            ];
            let labels =
                RelationshipLabelExtractor::extract_from_network_contexts(&txns, &accounts);
            assert_eq!(labels.len(), 1);
            assert!(labels[0].is_shell_link);
            assert!(!labels[0].is_mule_link);
        }

        #[test]
        fn distinct_networks_isolated() {
            let c1 = Uuid::new_v4();
            let c2 = Uuid::new_v4();
            let c3 = Uuid::new_v4();
            let a1 = mk_account(c1);
            let a2 = mk_account(c2);
            let a3 = mk_account(c3);
            let accounts = vec![a1.clone(), a2.clone(), a3.clone()];
            let txns = vec![
                mk_ctx_txn(a1.account_id, "NET-A", NetworkRole::Smurf),
                mk_ctx_txn(a2.account_id, "NET-A", NetworkRole::Smurf),
                mk_ctx_txn(a3.account_id, "NET-B", NetworkRole::CashOut),
            ];
            let labels =
                RelationshipLabelExtractor::extract_from_network_contexts(&txns, &accounts);
            // Only NET-A has ≥ 2 members; NET-B is skipped.
            assert_eq!(labels.len(), 1);
            assert_eq!(labels[0].case_id.as_deref(), Some("NET-A"));
        }

        #[test]
        fn no_duplicate_edges_for_repeat_transactions() {
            let c1 = Uuid::new_v4();
            let c2 = Uuid::new_v4();
            let a1 = mk_account(c1);
            let a2 = mk_account(c2);
            let accounts = vec![a1.clone(), a2.clone()];
            let txns = vec![
                mk_ctx_txn(a1.account_id, "NET-3", NetworkRole::Coordinator),
                mk_ctx_txn(a1.account_id, "NET-3", NetworkRole::Coordinator),
                mk_ctx_txn(a2.account_id, "NET-3", NetworkRole::Smurf),
                mk_ctx_txn(a2.account_id, "NET-3", NetworkRole::Smurf),
            ];
            let labels =
                RelationshipLabelExtractor::extract_from_network_contexts(&txns, &accounts);
            assert_eq!(labels.len(), 1);
        }
    }
}
