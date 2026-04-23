//! Expand ownership.generated[] blocks into concrete entity records.

use crate::config::{ConsolidationMethod, GeneratedEntityBlock, OwnershipConfig};
use crate::errors::{GroupError, GroupResult};
use crate::manifest::seeds::derive_manifest_seed;
use chrono::NaiveDate;
use rand::RngExt;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use rust_decimal::Decimal;

/// A fully-expanded entity: either from ownership.entities (explicit) or from
/// ownership.generated (bulk-expanded). Consumers downstream treat both
/// uniformly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandedEntity {
    pub code: String,
    pub name: Option<String>,
    pub country: String,
    pub functional_currency: String,
    pub scoping_profile: String,
    pub consolidation_method: ConsolidationMethod,
    pub ownership_percent: Option<Decimal>,
    pub parent_code: Option<String>,
    pub accounting_framework: Option<String>,
    pub industry: Option<String>,
    /// Source: was this from ownership.entities (Explicit) or ownership.generated (Generated)?
    pub source: EntitySource,
    /// Which generated-block index produced this entity (if Generated).
    pub generated_block_index: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntitySource {
    Explicit,
    Generated,
}

/// Expand the full ownership list: explicit entries first, then every
/// generated block in order, with collision detection.
pub fn expand_ownership(
    ownership: &OwnershipConfig,
    group_seed: u64,
    period_start: NaiveDate,
) -> GroupResult<Vec<ExpandedEntity>> {
    let mut out = Vec::new();
    let mut seen_codes = std::collections::BTreeSet::new();

    // Explicit entities first — preserve YAML order within the slice.
    for e in &ownership.entities {
        if !seen_codes.insert(e.code.clone()) {
            return Err(GroupError::Config(format!(
                "entity code {} appears twice in ownership.entities",
                e.code
            )));
        }
        out.push(ExpandedEntity {
            code: e.code.clone(),
            name: e.name.clone(),
            country: e.country.clone(),
            functional_currency: e.functional_currency.clone(),
            scoping_profile: e.scoping_profile.clone(),
            consolidation_method: e.consolidation_method,
            ownership_percent: e.ownership_percent,
            parent_code: e.parent_code.clone(),
            accounting_framework: e.accounting_framework.clone(),
            industry: e.industry.clone(),
            source: EntitySource::Explicit,
            generated_block_index: None,
        });
    }

    // Each generated block expands into `block.count` entities.
    for (block_idx, block) in ownership.generated.iter().enumerate() {
        expand_block(
            block,
            block_idx,
            group_seed,
            period_start,
            &mut seen_codes,
            &mut out,
        )?;
    }

    Ok(out)
}

fn expand_block(
    block: &GeneratedEntityBlock,
    block_idx: usize,
    group_seed: u64,
    period_start: NaiveDate,
    seen: &mut std::collections::BTreeSet<String>,
    out: &mut Vec<ExpandedEntity>,
) -> GroupResult<()> {
    // Seed the block's local RNG from (manifest_seed ⊕ block_idx). This keeps
    // each block's stream independent of other blocks' counts/content.
    let manifest_seed = derive_manifest_seed(group_seed, period_start);
    let mut block_seed = manifest_seed;
    // XOR-fold block_idx into the first 8 bytes.
    let idx_bytes = (block_idx as u64).to_le_bytes();
    for i in 0..8 {
        block_seed[i] ^= idx_bytes[i];
    }
    let mut rng = ChaCha8Rng::from_seed(block_seed);

    for i in 0..block.count {
        // Zero-padded sequence — use 7 digits for anything up to 9,999,999 entities.
        let code = format!("{}{:07}", block.code_prefix, i + 1);
        if !seen.insert(code.clone()) {
            return Err(GroupError::Config(format!(
                "generated entity code {code} collides with an existing explicit or generated code",
            )));
        }

        // Country: sample from block.country list (or require exactly one).
        let country = if block.country.is_empty() {
            return Err(GroupError::Config(format!(
                "generated block {block_idx} has empty country list — at least one required",
            )));
        } else {
            let idx = rng.random_range(0..block.country.len());
            block.country[idx].clone()
        };

        // Functional currency: explicit block setting takes precedence; otherwise
        // fall back to country → currency default (a conservative table keyed on
        // the common sovereign codes). For anything unmapped, leave as country code
        // and let validation downstream flag it.
        let functional_currency = block
            .functional_currency
            .clone()
            .unwrap_or_else(|| country_to_currency_default(&country));

        // Ownership percent: sample from range or None.
        let ownership_percent = block.ownership_percent_range.map(|[lo, hi]| {
            use rust_decimal::prelude::FromPrimitive;
            let frac: f64 = rng.random();
            let lo_f = lo.to_string().parse::<f64>().unwrap_or(0.0);
            let hi_f = hi.to_string().parse::<f64>().unwrap_or(1.0);
            let sampled = lo_f + frac * (hi_f - lo_f);
            // Round to 4 decimal places to avoid long-tail floats.
            let rounded = (sampled * 10_000.0).round() / 10_000.0;
            Decimal::from_f64(rounded).unwrap_or(lo)
        });

        out.push(ExpandedEntity {
            code,
            name: None,
            country,
            functional_currency,
            scoping_profile: block.scoping_profile.clone(),
            consolidation_method: block.consolidation_method,
            ownership_percent,
            parent_code: block.parent_code.clone(),
            accounting_framework: block.accounting_framework.clone(),
            industry: block.industry.clone(),
            source: EntitySource::Generated,
            generated_block_index: Some(block_idx),
        });
    }
    Ok(())
}

/// Conservative ISO-3166 country → ISO-4217 currency mapping for the ~30
/// jurisdictions a Big-4 group typically touches. Unknown countries return
/// the country code itself (validation downstream will flag invalid currencies).
fn country_to_currency_default(country: &str) -> String {
    match country {
        "CH" => "CHF",
        "US" => "USD",
        "CA" => "CAD",
        "MX" => "MXN",
        "BR" => "BRL",
        "AR" => "ARS",
        "GB" => "GBP",
        "DE" | "FR" | "IT" | "ES" | "NL" | "BE" | "AT" | "IE" | "PT" | "FI" | "LU" => "EUR",
        "PL" => "PLN",
        "SE" => "SEK",
        "NO" => "NOK",
        "DK" => "DKK",
        "CN" => "CNY",
        "JP" => "JPY",
        "KR" => "KRW",
        "IN" => "INR",
        "ID" => "IDR",
        "SG" => "SGD",
        "TH" => "THB",
        "VN" => "VND",
        "AU" => "AUD",
        "NZ" => "NZD",
        "ZA" => "ZAR",
        "AE" => "AED",
        "SA" => "SAR",
        "TR" => "TRY",
        "RU" => "RUB",
        _ => country, // fallback; validation will flag
    }
    .to_string()
}
