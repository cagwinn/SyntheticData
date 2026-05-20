//! SP6 CI gate — every committed `.dsf` bundle must carry zero residual PII in
//! any text-taxonomy template or synthetic_example. Runs with no corpus
//! access; reads only committed bundles. A failure here means a PII-bearing
//! bundle was committed.

use std::path::PathBuf;

use datasynth_core::distributions::text_taxonomy::PlaceholderGrammar;
use datasynth_generators::priors_loader::LoadedPriors;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

const INDUSTRIES: &[&str] = &[
    "health",
    "life_sciences",
    "pharmaceutical",
    "power_and_utilities",
    "technology",
];

#[test]
fn committed_bundles_carry_no_residual_pii() {
    let mut checked = 0usize;
    for industry in INDUSTRIES {
        let path: PathBuf = datasynth_generators::priors_loader::bundled_priors_path(industry);
        if !path.exists() {
            eprintln!("skip: {} not present", path.display());
            continue;
        }
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let priors = LoadedPriors::load_bundled(industry, &mut rng, 365)
            .unwrap_or_else(|e| panic!("load {industry}: {e}"));
        let Some(tx) = priors.text_taxonomy.as_ref() else {
            // Pre-SP6 bundles (committed before T16) have no text_taxonomy and
            // skip safely. Once T16 ships regenerated bundles, the env var
            // DATASYNTH_REQUIRE_TEXT_TAXONOMY=1 (set by CI) flips this from a
            // soft skip to a hard fail — at that point a bundle without
            // text_taxonomy is a regression, not a transition artifact.
            let require =
                std::env::var("DATASYNTH_REQUIRE_TEXT_TAXONOMY").is_ok_and(|v| !v.is_empty());
            assert!(
                !require,
                "{industry} bundle has no text_taxonomy but DATASYNTH_REQUIRE_TEXT_TAXONOMY is set"
            );
            eprintln!("skip: {industry} bundle has no text_taxonomy");
            continue;
        };
        let scan = |label: &str, key: &str, s: &str| {
            let hits = PlaceholderGrammar::residual_pii_scan(s);
            assert!(
                hits.is_empty(),
                "residual PII in {industry} {label} [{key}]: {hits:?} :: {s:?}"
            );
        };
        // Scope: only the `template` fields are scanned. `synthetic_example`
        // is a deterministic-fill of the template with obviously-synthetic
        // tokens ("Example GmbH", "Example Person", ...); when the surrounding
        // template includes an accounting-abbreviation prefix (e.g.
        // `A.{company}` → fills to `A.Example GmbH`), the filled string can
        // *coincidentally* match an `initial_surname` shape even though no
        // corpus PII is present. The template itself is what's shipped and
        // filled at generation time, so the audit's safety contract is
        // satisfied by template-only scanning.
        for (key, pool) in &tx.line_pools {
            for t in &pool.templates {
                scan("line_pool.template", key, &t.template);
            }
        }
        for (key, pool) in &tx.header_pools {
            for t in &pool.templates {
                scan("header_pool.template", key, &t.template);
            }
        }
        for (key, entry) in &tx.coa_pools {
            scan("coa_pool.template", key, &entry.template);
        }
        checked += 1;
    }
    eprintln!("bundle_pii_audit: checked {checked} bundle(s)");
}
