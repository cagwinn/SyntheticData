//! Banking fingerprint extractor.
//!
//! Extracts aggregate patterns from banking data for privacy-preserving
//! re-synthesis. Does not retain any individual customer/account/transaction —
//! only distributions, rates, and statistical parameters.

use std::collections::HashMap;

use rust_decimal::prelude::ToPrimitive;

use crate::models::BankingFingerprint;

/// Extractor input — references to banking data slices.
pub struct BankingExtractionInput<'a, Cust, Acct, Txn> {
    pub customers: &'a [Cust],
    pub accounts: &'a [Acct],
    pub transactions: &'a [Txn],
}

/// Banking fingerprint extractor.
pub struct BankingExtractor;

impl BankingExtractor {
    /// Extract a banking fingerprint from the provided customers, accounts, and transactions.
    ///
    /// Uses generic accessor functions (extractor callbacks) to decouple from
    /// the concrete banking crate types, keeping `datasynth-fingerprint` free
    /// of a direct dependency on `datasynth-banking`.
    #[allow(clippy::too_many_arguments)]
    pub fn extract<Cust, Acct, Txn>(
        customers: &[Cust],
        accounts: &[Acct],
        transactions: &[Txn],
        customer_type: impl Fn(&Cust) -> String,
        risk_tier: impl Fn(&Cust) -> String,
        retail_persona: impl Fn(&Cust) -> Option<String>,
        is_pep: impl Fn(&Cust) -> bool,
        is_mule: impl Fn(&Cust) -> bool,
        account_type: impl Fn(&Acct) -> String,
        txn_channel: impl Fn(&Txn) -> String,
        txn_category: impl Fn(&Txn) -> String,
        txn_typology: impl Fn(&Txn) -> Option<String>,
        txn_amount: impl Fn(&Txn) -> f64,
        txn_is_suspicious: impl Fn(&Txn) -> bool,
        txn_is_false_positive: impl Fn(&Txn) -> bool,
        txn_is_bridged: impl Fn(&Txn) -> bool,
        txn_has_network: impl Fn(&Txn) -> bool,
        txn_is_cross_border: impl Fn(&Txn) -> bool,
        txn_is_cash: impl Fn(&Txn) -> bool,
        account_of_txn: impl Fn(&Txn) -> String,
        owner_of_account: impl Fn(&Acct) -> String,
    ) -> BankingFingerprint {
        let mut fp = BankingFingerprint {
            customer_count: customers.len(),
            account_count: accounts.len(),
            transaction_count: transactions.len(),
            ..Default::default()
        };

        // Customer type distribution
        let mut ct_counts: HashMap<String, usize> = HashMap::new();
        let mut rt_counts: HashMap<String, usize> = HashMap::new();
        let mut rp_counts: HashMap<String, usize> = HashMap::new();
        let mut pep_count = 0usize;
        let mut mule_count = 0usize;
        for c in customers {
            *ct_counts.entry(customer_type(c)).or_insert(0) += 1;
            *rt_counts.entry(risk_tier(c)).or_insert(0) += 1;
            if let Some(p) = retail_persona(c) {
                *rp_counts.entry(p).or_insert(0) += 1;
            }
            if is_pep(c) {
                pep_count += 1;
            }
            if is_mule(c) {
                mule_count += 1;
            }
        }
        fp.customer_type_dist = BankingFingerprint::normalize_counts(&ct_counts);
        fp.risk_tier_dist = BankingFingerprint::normalize_counts(&rt_counts);
        fp.retail_persona_dist = BankingFingerprint::normalize_counts(&rp_counts);
        if !customers.is_empty() {
            fp.pep_rate = pep_count as f64 / customers.len() as f64;
            fp.mule_rate = mule_count as f64 / customers.len() as f64;
        }

        // Account type distribution + accounts_per_customer
        let mut at_counts: HashMap<String, usize> = HashMap::new();
        let mut accounts_per_owner: HashMap<String, usize> = HashMap::new();
        for a in accounts {
            *at_counts.entry(account_type(a)).or_insert(0) += 1;
            *accounts_per_owner.entry(owner_of_account(a)).or_insert(0) += 1;
        }
        fp.account_type_dist = BankingFingerprint::normalize_counts(&at_counts);
        if !customers.is_empty() {
            let counts: Vec<f64> = accounts_per_owner.values().map(|v| *v as f64).collect();
            fp.accounts_per_customer_mean = counts.iter().sum::<f64>() / counts.len().max(1) as f64;
            let mean = fp.accounts_per_customer_mean;
            let var =
                counts.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / counts.len().max(1) as f64;
            fp.accounts_per_customer_std = var.sqrt();
        }

        // Transaction distributions
        let mut ch_counts: HashMap<String, usize> = HashMap::new();
        let mut cat_counts: HashMap<String, usize> = HashMap::new();
        let mut typ_counts: HashMap<String, usize> = HashMap::new();
        let mut txn_per_account: HashMap<String, usize> = HashMap::new();
        let mut amounts: Vec<f64> = Vec::with_capacity(transactions.len());
        let mut suspicious_count = 0usize;
        let mut fp_count = 0usize;
        let mut bridged_count = 0usize;
        let mut network_count = 0usize;
        let mut cross_border_count = 0usize;
        let mut cash_count = 0usize;

        for t in transactions {
            *ch_counts.entry(txn_channel(t)).or_insert(0) += 1;
            *cat_counts.entry(txn_category(t)).or_insert(0) += 1;
            if let Some(typ) = txn_typology(t) {
                *typ_counts.entry(typ).or_insert(0) += 1;
            }
            *txn_per_account.entry(account_of_txn(t)).or_insert(0) += 1;
            let amt = txn_amount(t);
            if amt > 0.0 {
                amounts.push(amt);
            }
            if txn_is_suspicious(t) {
                suspicious_count += 1;
            }
            if txn_is_false_positive(t) {
                fp_count += 1;
            }
            if txn_is_bridged(t) {
                bridged_count += 1;
            }
            if txn_has_network(t) {
                network_count += 1;
            }
            if txn_is_cross_border(t) {
                cross_border_count += 1;
            }
            if txn_is_cash(t) {
                cash_count += 1;
            }
        }

        fp.channel_dist = BankingFingerprint::normalize_counts(&ch_counts);
        fp.category_dist = BankingFingerprint::normalize_counts(&cat_counts);
        fp.typology_dist = BankingFingerprint::normalize_counts(&typ_counts);

        if !transactions.is_empty() {
            let n = transactions.len() as f64;
            fp.suspicious_rate = suspicious_count as f64 / n;
            fp.false_positive_rate = fp_count as f64 / n;
            fp.bridged_payment_rate = bridged_count as f64 / n;
            fp.network_rate = network_count as f64 / n;
            fp.cross_border_rate = cross_border_count as f64 / n;
            fp.cash_rate = cash_count as f64 / n;

            // Log-normal fit
            let log_amounts: Vec<f64> = amounts
                .iter()
                .filter(|a| **a > 0.0)
                .map(|a| a.ln())
                .collect();
            if !log_amounts.is_empty() {
                let mu = log_amounts.iter().sum::<f64>() / log_amounts.len() as f64;
                let var = log_amounts.iter().map(|v| (v - mu).powi(2)).sum::<f64>()
                    / log_amounts.len() as f64;
                fp.amount_log_mu = mu;
                fp.amount_log_sigma = var.sqrt();
            }
            fp.amount_min = amounts.iter().cloned().fold(f64::INFINITY, f64::min);
            fp.amount_max = amounts.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            if fp.amount_min.is_infinite() {
                fp.amount_min = 0.0;
            }
            if fp.amount_max.is_infinite() {
                fp.amount_max = 0.0;
            }

            // Txns per account
            let counts: Vec<f64> = txn_per_account.values().map(|v| *v as f64).collect();
            fp.txns_per_account_mean = counts.iter().sum::<f64>() / counts.len().max(1) as f64;
            let mean = fp.txns_per_account_mean;
            let var =
                counts.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / counts.len().max(1) as f64;
            fp.txns_per_account_std = var.sqrt();
        }

        fp
    }
}

/// Convenience wrapper for when the caller has `Decimal` amounts.
pub fn decimal_to_f64(d: rust_decimal::Decimal) -> f64 {
    d.to_f64().unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct FakeCust {
        ctype: String,
        tier: String,
        pep: bool,
        mule: bool,
    }
    #[derive(Clone)]
    struct FakeAcct {
        atype: String,
        owner: String,
    }
    #[derive(Clone)]
    struct FakeTxn {
        channel: String,
        category: String,
        typology: Option<String>,
        amount: f64,
        susp: bool,
        fp: bool,
        bridged: bool,
        network: bool,
        cross_border: bool,
        cash: bool,
        account: String,
    }

    #[test]
    fn test_extract_basic() {
        let customers = vec![
            FakeCust {
                ctype: "retail".into(),
                tier: "low".into(),
                pep: false,
                mule: false,
            },
            FakeCust {
                ctype: "business".into(),
                tier: "high".into(),
                pep: true,
                mule: false,
            },
            FakeCust {
                ctype: "retail".into(),
                tier: "medium".into(),
                pep: false,
                mule: true,
            },
        ];
        let accounts = vec![
            FakeAcct {
                atype: "checking".into(),
                owner: "c1".into(),
            },
            FakeAcct {
                atype: "savings".into(),
                owner: "c1".into(),
            },
            FakeAcct {
                atype: "business_operating".into(),
                owner: "c2".into(),
            },
        ];
        let transactions = vec![
            FakeTxn {
                channel: "ach".into(),
                category: "salary".into(),
                typology: None,
                amount: 1000.0,
                susp: false,
                fp: false,
                bridged: true,
                network: false,
                cross_border: false,
                cash: false,
                account: "a1".into(),
            },
            FakeTxn {
                channel: "wire".into(),
                category: "transfer_out".into(),
                typology: Some("structuring".into()),
                amount: 9500.0,
                susp: true,
                fp: false,
                bridged: false,
                network: false,
                cross_border: true,
                cash: false,
                account: "a2".into(),
            },
            FakeTxn {
                channel: "cash".into(),
                category: "cash_deposit".into(),
                typology: None,
                amount: 500.0,
                susp: false,
                fp: true,
                bridged: false,
                network: false,
                cross_border: false,
                cash: true,
                account: "a1".into(),
            },
        ];

        let fp = BankingExtractor::extract(
            &customers,
            &accounts,
            &transactions,
            |c| c.ctype.clone(),
            |c| c.tier.clone(),
            |_| None,
            |c| c.pep,
            |c| c.mule,
            |a| a.atype.clone(),
            |t| t.channel.clone(),
            |t| t.category.clone(),
            |t| t.typology.clone(),
            |t| t.amount,
            |t| t.susp,
            |t| t.fp,
            |t| t.bridged,
            |t| t.network,
            |t| t.cross_border,
            |t| t.cash,
            |t| t.account.clone(),
            |a| a.owner.clone(),
        );

        assert_eq!(fp.customer_count, 3);
        assert_eq!(fp.account_count, 3);
        assert_eq!(fp.transaction_count, 3);
        assert!((fp.customer_type_dist["retail"] - 2.0 / 3.0).abs() < 0.001);
        assert!((fp.pep_rate - 1.0 / 3.0).abs() < 0.001);
        assert!((fp.mule_rate - 1.0 / 3.0).abs() < 0.001);
        assert!((fp.suspicious_rate - 1.0 / 3.0).abs() < 0.001);
        assert!((fp.false_positive_rate - 1.0 / 3.0).abs() < 0.001);
        assert!((fp.bridged_payment_rate - 1.0 / 3.0).abs() < 0.001);
        assert!(fp.amount_log_mu > 0.0);
        assert!(fp.accounts_per_customer_mean > 0.0);
    }
}
