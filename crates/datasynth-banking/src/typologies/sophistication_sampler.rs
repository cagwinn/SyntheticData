//! Context-aware sophistication sampler.
//!
//! Replaces the flat multinomial in `TypologyInjector::select_sophistication()`
//! with conditional probabilities based on:
//! - Amount: larger amounts correlate with higher sophistication
//! - Typology: trade-based ML and sanctions evasion skew professional+
//! - Customer type: retail rarely runs state-level schemes
//! - Network size: bigger networks → organized → higher sophistication

use datasynth_core::models::banking::{
    AmlTypology, BankingCustomerType, Sophistication,
};
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;

/// Context for sophistication sampling.
#[derive(Debug, Clone, Copy)]
pub struct SophisticationContext {
    pub amount: f64,
    pub typology: AmlTypology,
    pub customer_type: BankingCustomerType,
    pub network_size: Option<u32>,
}

/// Sample sophistication level conditional on context.
///
/// The base distribution is modulated by shift terms; each contextual factor
/// adjusts the probability mass toward higher or lower sophistication.
pub fn sample_sophistication(rng: &mut ChaCha8Rng, ctx: SophisticationContext) -> Sophistication {
    // Start with base probabilities (roughly matching old flat distribution):
    // Basic 40%, Standard 35%, Professional 20%, Advanced 4%, StateLevel 1%
    let mut weights = [0.40, 0.35, 0.20, 0.04, 0.01];

    // Amount shift — large amounts push mass toward higher sophistication
    let amount_shift = amount_shift_factor(ctx.amount);
    shift_toward_higher(&mut weights, amount_shift);

    // Typology shift — some typologies inherently require more sophistication
    let typology_shift = typology_shift_factor(ctx.typology);
    shift_toward_higher(&mut weights, typology_shift);

    // Customer type shift — retail rarely state-level
    match ctx.customer_type {
        BankingCustomerType::Retail => shift_toward_lower(&mut weights, 0.15),
        BankingCustomerType::Business => shift_toward_higher(&mut weights, 0.05),
        BankingCustomerType::Trust => shift_toward_higher(&mut weights, 0.15),
        BankingCustomerType::FinancialInstitution => shift_toward_higher(&mut weights, 0.20),
        _ => {}
    }

    // Network size shift — organized crime has more resources
    if let Some(size) = ctx.network_size {
        let shift = ((size as f64).log10() / 2.0).clamp(0.0, 0.3);
        shift_toward_higher(&mut weights, shift);
    }

    // Normalize and sample
    let total: f64 = weights.iter().sum();
    for w in weights.iter_mut() {
        *w /= total;
    }

    let roll: f64 = rng.random();
    let mut cumulative = 0.0;
    let variants = [
        Sophistication::Basic,
        Sophistication::Standard,
        Sophistication::Professional,
        Sophistication::Advanced,
        Sophistication::StateLevel,
    ];
    for (i, w) in weights.iter().enumerate() {
        cumulative += w;
        if roll <= cumulative {
            return variants[i];
        }
    }
    Sophistication::Standard
}

/// How much to shift mass toward higher sophistication based on amount.
/// Returns a value 0.0-0.6.
fn amount_shift_factor(amount: f64) -> f64 {
    if amount < 10_000.0 {
        0.0
    } else if amount < 100_000.0 {
        0.10
    } else if amount < 1_000_000.0 {
        0.25
    } else if amount < 10_000_000.0 {
        0.40
    } else {
        0.55
    }
}

/// Typology-specific baseline shift (some are inherently sophisticated).
fn typology_shift_factor(typology: AmlTypology) -> f64 {
    use AmlTypology::*;
    match typology {
        // Simple/low-skill
        Structuring | Smurfing | CuckooSmurfing => 0.0,
        // Moderate skill
        MoneyMule | FunnelAccount | ConcentrationAccount => 0.05,
        Layering | RapidMovement => 0.10,
        // High-skill required
        RoundTripping | TradeBasedML | InvoiceManipulation => 0.20,
        ShellCompany | RealEstateIntegration | CasinoIntegration => 0.15,
        // Specialized/sophisticated
        CryptoIntegration => 0.25,
        SanctionsEvasion => 0.30,
        TerroristFinancing | Corruption => 0.35,
        SyntheticIdentity => 0.15,
        // Social engineering
        RomanceScam | AdvanceFeeFraud | AuthorizedPushPayment => 0.05,
        BusinessEmailCompromise => 0.20,
        AccountTakeover => 0.15,
        _ => 0.0,
    }
}

/// Shift probability mass toward higher sophistication indices.
/// `amount` is 0.0-1.0 — fraction of mass to shift from lower to higher slots.
fn shift_toward_higher(weights: &mut [f64; 5], amount: f64) {
    let shift = amount.clamp(0.0, 0.9);
    // Take from index 0 and 1, give to 2, 3, 4
    let from_basic = weights[0] * shift * 0.7;
    let from_standard = weights[1] * shift * 0.5;
    weights[0] -= from_basic;
    weights[1] -= from_standard;
    let total_moved = from_basic + from_standard;
    // Distribute: 40% to Professional, 35% to Advanced, 25% to StateLevel
    weights[2] += total_moved * 0.40;
    weights[3] += total_moved * 0.35;
    weights[4] += total_moved * 0.25;
}

/// Shift probability mass toward lower sophistication indices.
fn shift_toward_lower(weights: &mut [f64; 5], amount: f64) {
    let shift = amount.clamp(0.0, 0.9);
    let from_statelevel = weights[4] * shift * 0.9;
    let from_advanced = weights[3] * shift * 0.7;
    let from_professional = weights[2] * shift * 0.4;
    weights[4] -= from_statelevel;
    weights[3] -= from_advanced;
    weights[2] -= from_professional;
    let total_moved = from_statelevel + from_advanced + from_professional;
    weights[0] += total_moved * 0.6;
    weights[1] += total_moved * 0.4;
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_small_amount_retail_skews_basic() {
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let ctx = SophisticationContext {
            amount: 5_000.0,
            typology: AmlTypology::Structuring,
            customer_type: BankingCustomerType::Retail,
            network_size: None,
        };
        let mut counts = [0; 5];
        for _ in 0..1000 {
            let s = sample_sophistication(&mut rng, ctx);
            let idx = match s {
                Sophistication::Basic => 0,
                Sophistication::Standard => 1,
                Sophistication::Professional => 2,
                Sophistication::Advanced => 3,
                Sophistication::StateLevel => 4,
            };
            counts[idx] += 1;
        }
        // Should be heavily basic/standard
        assert!(counts[0] + counts[1] > 700, "Small-retail should be basic/standard: {counts:?}");
    }

    #[test]
    fn test_large_amount_business_skews_higher() {
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let ctx = SophisticationContext {
            amount: 50_000_000.0,
            typology: AmlTypology::TradeBasedML,
            customer_type: BankingCustomerType::Business,
            network_size: Some(20),
        };
        let mut counts = [0; 5];
        for _ in 0..1000 {
            let s = sample_sophistication(&mut rng, ctx);
            let idx = match s {
                Sophistication::Basic => 0,
                Sophistication::Standard => 1,
                Sophistication::Professional => 2,
                Sophistication::Advanced => 3,
                Sophistication::StateLevel => 4,
            };
            counts[idx] += 1;
        }
        // Large amount + trade-based + network → should skew to professional+
        let higher = counts[2] + counts[3] + counts[4];
        assert!(higher > 600, "Large/trade-based should skew higher: {counts:?}");
    }

    #[test]
    fn test_sanctions_evasion_rarely_basic() {
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let ctx = SophisticationContext {
            amount: 100_000.0,
            typology: AmlTypology::SanctionsEvasion,
            customer_type: BankingCustomerType::Business,
            network_size: None,
        };
        let mut basic_count = 0;
        for _ in 0..1000 {
            if matches!(sample_sophistication(&mut rng, ctx), Sophistication::Basic) {
                basic_count += 1;
            }
        }
        // Sanctions evasion should skew away from Basic (baseline 40% → should be substantially lower)
        assert!(basic_count < 300, "Sanctions should rarely be Basic: {basic_count}/1000 (baseline 40%)");
    }
}
