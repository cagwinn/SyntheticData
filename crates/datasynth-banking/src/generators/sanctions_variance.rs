//! Context-aware sanctions screening generator.
//!
//! Replaces flat screening rates with distributions that correlate with:
//! - Risk tier (high-risk customers get more thorough screening)
//! - Residence country (sanctioned/high-risk countries elevate match rates)
//! - Industry (MSBs, crypto, precious metals → higher scrutiny)
//! - PEP status (100% enhanced screening with family/associate matches)
//! - Name complexity (transliterated names → more fuzzy matches)

use chrono::NaiveDate;
use datasynth_core::models::banking::RiskTier;
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;

use crate::models::{BankingCustomer, SanctionsScreening, ScreeningResult};

/// Seed offset for the sanctions variance generator.
pub const SANCTIONS_VARIANCE_SEED_OFFSET: u64 = 8400;

/// Countries that trigger elevated screening scrutiny.
const HIGH_RISK_COUNTRIES: &[&str] = &[
    "IR", "KP", "SY", "CU", "VE", "MM", "BY", "RU", "AF", "LY", "YE", "SO",
];

/// Countries often used for transshipment (moderate risk).
const TRANSSHIPMENT_COUNTRIES: &[&str] =
    &["TR", "AE", "SG", "MY", "GE", "AM", "KG", "KZ", "LB", "PK"];

/// Industry codes (NAICS) with elevated AML scrutiny.
const HIGH_RISK_INDUSTRIES: &[&str] = &[
    "522320", // Financial Transactions Processing
    "522390", // Other Activities Related to Credit Intermediation
    "523000", // Securities, Commodity Contracts, and Other Financial Investments
    "523999", // Miscellaneous Financial Investment Activities
    "721120", // Casino Hotels
    "713210", // Casinos (except Casino Hotels)
    "713290", // Other Gambling Industries
    "423940", // Jewelry, Watch, Precious Stone Merchant Wholesalers
    "448310", // Jewelry Stores
];

/// Names with transliterated characters often trigger fuzzy-match screening hits.
fn has_complex_transliteration(name: &str) -> bool {
    // Simple heuristic: names with non-ASCII, apostrophes, multiple spellings
    let complex_indicators: &[&str] = &[
        "Muhammad",
        "Mohammad",
        "Mohammed",
        "Mohamed",
        "Abdullah",
        "Abdelaziz",
        "Abdul",
        "Ahmed",
        "Ahmad",
        "Hussein",
        "Hussain",
        "Husein",
        "'",
        "-", // apostrophes and hyphens in some names
    ];
    complex_indicators.iter().any(|ind| name.contains(ind))
}

/// Context-aware sanctions screening generator.
pub struct SanctionsVarianceGenerator {
    rng: ChaCha8Rng,
}

impl SanctionsVarianceGenerator {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: ChaCha8Rng::seed_from_u64(seed.wrapping_add(SANCTIONS_VARIANCE_SEED_OFFSET)),
        }
    }

    /// Generate context-aware sanctions screening for a customer.
    pub fn generate_for_customer(
        &mut self,
        customer: &BankingCustomer,
        onboarding_date: NaiveDate,
    ) -> SanctionsScreening {
        // Base potential-match rate
        let mut match_probability: f64 = 0.001; // baseline: 0.1%

        // Risk tier multiplier
        match customer.risk_tier {
            RiskTier::Low => {}
            RiskTier::Medium => match_probability *= 3.0,
            RiskTier::High => match_probability *= 10.0,
            RiskTier::VeryHigh => match_probability *= 25.0,
            RiskTier::Prohibited => match_probability *= 100.0,
        }

        // Country risk boost
        if HIGH_RISK_COUNTRIES.contains(&customer.residence_country.as_str()) {
            match_probability *= 20.0;
        } else if TRANSSHIPMENT_COUNTRIES.contains(&customer.residence_country.as_str()) {
            match_probability *= 5.0;
        }

        // PEP boost (always elevated scrutiny)
        if customer.is_pep {
            match_probability *= 15.0;
        }

        // Industry boost (business customers)
        if let Some(ref ind) = customer.industry_code {
            if HIGH_RISK_INDUSTRIES.contains(&ind.as_str()) {
                match_probability *= 8.0;
            }
        }

        // Name complexity (affects fuzzy matching)
        let has_complex_name = has_complex_transliteration(&customer.name.legal_name);
        if has_complex_name {
            match_probability *= 3.0;
        }

        let match_probability = match_probability.min(0.8); // cap

        // Determine screening result based on probability
        let roll: f64 = self.rng.random();
        let (screening_result, match_score, matched_list) = if roll < match_probability * 0.1 {
            // 10% of potential-match probability becomes confirmed match (true positive)
            let score = self.rng.random_range(0.85..=1.0);
            let list = pick_sanctions_list(&mut self.rng);
            (ScreeningResult::ConfirmedMatch, score, Some(list))
        } else if roll < match_probability {
            // Most potential matches are false positives (name similarity)
            let score = self.rng.random_range(0.55..0.85);
            let list = pick_sanctions_list(&mut self.rng);
            (ScreeningResult::PotentialMatch, score, Some(list))
        } else {
            (ScreeningResult::Clear, 0.0, None)
        };

        // Generate name variations for PEPs and complex names (more variations checked)
        let mut name_variations = vec![customer.name.legal_name.clone()];
        if customer.is_pep || has_complex_name {
            name_variations.extend(generate_name_variations(
                &customer.name.legal_name,
                &mut self.rng,
            ));
        }

        SanctionsScreening {
            last_screened: onboarding_date,
            screening_result,
            matched_list,
            match_score,
            name_variations,
            is_true_match: matches!(screening_result, ScreeningResult::ConfirmedMatch),
        }
    }
}

fn pick_sanctions_list(rng: &mut ChaCha8Rng) -> String {
    const LISTS: &[&str] = &[
        "OFAC SDN",
        "OFAC Consolidated",
        "EU Consolidated",
        "UN Security Council",
        "UK HMT",
    ];
    LISTS[rng.random_range(0..LISTS.len())].to_string()
}

fn generate_name_variations(name: &str, rng: &mut ChaCha8Rng) -> Vec<String> {
    let mut out = Vec::new();
    let parts: Vec<&str> = name.split_whitespace().collect();

    // Reversed order (Last First)
    if parts.len() >= 2 {
        if let (Some(first), Some(last)) = (parts.first(), parts.last()) {
            out.push(format!("{last} {first}"));
        }
    }

    // Transliteration alternates
    let translits: &[(&str, &str)] = &[
        ("Muhammad", "Mohamed"),
        ("Mohammad", "Mohammed"),
        ("Ahmed", "Ahmad"),
        ("Hussein", "Husein"),
    ];
    for (a, b) in translits {
        if name.contains(a) {
            out.push(name.replace(a, b));
        } else if name.contains(b) {
            out.push(name.replace(b, a));
        }
    }

    // Limit to 4 variations
    out.truncate(4);
    let _ = rng; // reserved for future randomization
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn mk_customer(country: &str, tier: RiskTier, pep: bool) -> BankingCustomer {
        let mut c = BankingCustomer::new_retail(
            Uuid::new_v4(),
            "John",
            "Doe",
            country,
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        );
        c.risk_tier = tier;
        c.is_pep = pep;
        c
    }

    #[test]
    fn test_low_risk_us_customer_usually_clear() {
        let mut gen = SanctionsVarianceGenerator::new(42);
        let customer = mk_customer("US", RiskTier::Low, false);
        // Run many to get statistics
        let mut clear_count = 0;
        for _ in 0..1000 {
            let s =
                gen.generate_for_customer(&customer, NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
            if matches!(s.screening_result, ScreeningResult::Clear) {
                clear_count += 1;
            }
        }
        // >99% should be clear
        assert!(
            clear_count > 990,
            "Low-risk US should be clear: {clear_count}/1000"
        );
    }

    #[test]
    fn test_high_risk_sanctioned_country_elevated_matches() {
        let mut gen = SanctionsVarianceGenerator::new(42);
        let customer = mk_customer("IR", RiskTier::VeryHigh, false);
        let mut match_count = 0;
        for _ in 0..1000 {
            let s =
                gen.generate_for_customer(&customer, NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
            if !matches!(s.screening_result, ScreeningResult::Clear) {
                match_count += 1;
            }
        }
        // Should have many more matches than low-risk
        assert!(
            match_count > 100,
            "High-risk country should have elevated matches: {match_count}/1000"
        );
    }

    #[test]
    fn test_pep_gets_name_variations() {
        let mut gen = SanctionsVarianceGenerator::new(42);
        let mut customer = mk_customer("US", RiskTier::Medium, true);
        customer.name.legal_name = "Muhammad Ali Khan".to_string();
        let s = gen.generate_for_customer(&customer, NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
        assert!(
            s.name_variations.len() > 1,
            "PEP should get name variations"
        );
    }
}
