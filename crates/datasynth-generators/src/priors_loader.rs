//! Loads an SP2 industry-priors bundle and builds the SP3 sampler state.
//!
//! This module provides [`LoadedPriors`], which wraps the fully-built runtime
//! samplers consumed by `je_generator` and friends.
//!
//! # Architecture note
//!
//! `datasynth-generators` cannot depend on `datasynth-fingerprint` directly
//! (that crate depends on `datasynth-eval` which in turn depends on
//! `datasynth-generators`, creating a package cycle).  Therefore:
//!
//! - `BehavioralPriors` and its sub-types live in
//!   `datasynth_core::distributions::behavioral_priors` and are re-exported
//!   from `datasynth-fingerprint`.
//! - File-loading (`load_bundled` / `load_from_path`) is exposed here via a
//!   thin `zip` + `serde_yaml` reader that is independent of the fingerprint
//!   crate.
//! - Callers that already hold a deserialized [`BehavioralPriors`] (e.g. from
//!   the fingerprint SDK) can use [`LoadedPriors::from_priors`] directly.

use std::collections::HashMap;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};

use rand::Rng;
use thiserror::Error;

use datasynth_core::distributions::behavioral_priors::{
    CoaSemanticPrior, LinesPerJePrior, PerSourceAmountPrior, PerSourceAttributePrior,
    PerSourceRolePrior, PostingLagPrior, ReferenceFormatPrior, SourceMixPrior, TbAnchorPrior,
    UserPersonaPrior,
};
use datasynth_core::distributions::text_taxonomy::TextTaxonomyPrior;
use datasynth_core::distributions::{
    behavioral_priors::BehavioralPriors, BipartiteFanoutSampler, ConditionalIETSampler,
    CrossEntityMotifSampler, MultiSegmentActiveWindow, SourceActiveWindow, SourceIetState,
};

/// Key used inside `.dsf` ZIP archives for the behavioral section.
const BEHAVIORAL_YAML_KEY: &str = "behavioral.yaml";

#[derive(Debug, Error)]
pub enum PriorsLoadError {
    #[error("priors bundle not found at {0}")]
    NotFound(PathBuf),
    #[error("bundle has no behavioral section")]
    MissingBehavioral,
    #[error("bundle industry mismatch: bundle={bundle}, requested={requested}")]
    IndustryMismatch { bundle: String, requested: String },
    #[error("fingerprint read error: {0}")]
    Read(String),
}

/// Conventional resource directory for committed industry-priors bundles.
pub fn bundled_priors_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources")
        .join("priors")
}

/// Resolve the bundled `.dsf` path for an industry slug.
pub fn bundled_priors_path(industry: &str) -> PathBuf {
    bundled_priors_dir().join(format!("industry_priors_{industry}.dsf"))
}

/// Fully-built runtime priors consumed by `je_generator` and friends.
#[derive(Clone)]
pub struct LoadedPriors {
    pub industry: String,
    pub bundle_path: PathBuf,
    pub source_mix: SourceMixPrior,
    pub iet_sampler: ConditionalIETSampler,
    pub lines_per_je: LinesPerJePrior,
    pub active_window: SourceActiveWindow,
    /// SP3.2 — when `Some`, supersedes `active_window` for `is_active` checks.
    pub multi_segment_window: Option<MultiSegmentActiveWindow>,
    pub fanout_samplers: HashMap<String, BipartiteFanoutSampler>,
    pub posting_lag: Option<PostingLagPrior>,
    /// SP3.3 — cross-entity motif sampler. None when bundle has no entity_clusters.
    pub cross_entity_motifs: Option<CrossEntityMotifSampler>,
    /// SP3.7 — per-source conditional attribute distributions.  When `Some`,
    /// downstream attribute sampling (GL account, cost center, profit center)
    /// is constrained to the values characteristic of the just-drawn source
    /// code rather than the marginal distribution over all sources.
    pub per_source_attribute: Option<PerSourceAttributePrior>,
    /// SP3.12 — TP motif sampler. Built from `tp_entity_clusters`; biases the
    /// TP draw toward cluster-mates of recently-emitted TPs on the same source
    /// to build triangle structure in the TP co-occurrence graph.
    pub tp_motif_sampler: Option<CrossEntityMotifSampler>,
    /// SP4.7 — Per-source reference-string format templates.  When `Some`, the
    /// JE generator calls `sample_reference` to produce a reference string that
    /// matches the corpus format pattern for the current source code.
    pub reference_formats: Option<ReferenceFormatPrior>,
    /// SP4.2 — CoA semantic content extracted from corpus CoA parquet
    /// files.  When `Some`, the CoA generator overwrites generic account
    /// descriptions with corpus names and ISO 21378 hierarchy values.
    pub coa_semantic: Option<CoaSemanticPrior>,
    /// SP4.5 — Per-user behavioral patterns (source mix, hourly density,
    /// weekday density, volume share).  When `Some` and `has_data()` is true,
    /// `je_generator` biases `created_by` and `created_at` toward the
    /// characteristic patterns of each user.
    ///
    /// `None` or empty: generator falls back to the internal user pool.
    pub user_personas: Option<UserPersonaPrior>,
    /// SP4.3 — Per-(source, gl_prefix) log-normal amount parameters.  When
    /// `Some`, `je_generator` draws JE total-amounts from the source-conditional
    /// distribution rather than the global `AmountSampler` marginal.  Fraud
    /// entries bypass this path to preserve fraud-pattern semantics.
    ///
    /// `None` means the bundle was built before SP4.3 or had too few rows.
    pub source_amount_conditionals: Option<PerSourceAmountPrior>,
    /// SP4.6 — Per-(source, line_role) GL account conditional.
    ///
    /// When `Some`, callers use `sample_gl_for_source_role(source, "DR"|"CR")` to
    /// draw a GL account that respects the debit/credit line role for the given SAP
    /// document type.  Falls back to `sample_attribute_for_source` → default when
    /// the pair is missing.
    ///
    /// `None` means the bundle was built before SP4.6 or had too few rows.
    pub source_role_gl: Option<PerSourceRolePrior>,
    /// SP4.1 — Trial-balance anchor prior.
    ///
    /// When `Some` and `has_data()` is true, the `RunningBalanceTracker` can
    /// use these industry-median per-account targets to generate periodic
    /// drift-correction entries that keep the synthetic balance sheet shaped
    /// like a corpus balance sheet.
    ///
    /// `None` means the bundle was built before SP4.1 (current committed
    /// bundles) or had no TB data — the balance tracker operates in its
    /// existing free-drift mode in that case.
    pub tb_anchor: Option<TbAnchorPrior>,
    /// SP6 — corpus text taxonomy. When `Some`, provides
    /// `(source, account-class)` line pools, `source` header pools, and
    /// per-account CoA description templates.
    pub text_taxonomy: Option<TextTaxonomyPrior>,
}

impl LoadedPriors {
    /// Load the bundled `.dsf` for `industry` from the crate's
    /// `resources/priors/` directory.
    pub fn load_bundled<R: Rng>(
        industry: &str,
        rng: &mut R,
        period_days: i64,
    ) -> Result<Self, PriorsLoadError> {
        Self::load_from_path(
            &bundled_priors_path(industry),
            rng,
            period_days,
            Some(industry),
        )
    }

    /// Load a `.dsf` bundle from an arbitrary path.
    pub fn load_from_path<R: Rng>(
        path: &Path,
        rng: &mut R,
        period_days: i64,
        expected_industry: Option<&str>,
    ) -> Result<Self, PriorsLoadError> {
        if !path.exists() {
            return Err(PriorsLoadError::NotFound(path.to_path_buf()));
        }
        let file = std::fs::File::open(path).map_err(|e| PriorsLoadError::Read(e.to_string()))?;
        let bp = read_behavioral_from_dsf(file)
            .map_err(PriorsLoadError::Read)?
            .ok_or(PriorsLoadError::MissingBehavioral)?;
        if let Some(want) = expected_industry {
            if bp.industry != want {
                return Err(PriorsLoadError::IndustryMismatch {
                    bundle: bp.industry,
                    requested: want.to_string(),
                });
            }
        }
        Self::from_priors(bp, path.to_path_buf(), rng, period_days)
    }

    /// Build `LoadedPriors` from an already-deserialised [`BehavioralPriors`].
    ///
    /// Use this when you already have a `BehavioralPriors` from the
    /// `datasynth-fingerprint` SDK and don't need file I/O here.
    pub fn from_priors<R: Rng>(
        bp: BehavioralPriors,
        bundle_path: PathBuf,
        rng: &mut R,
        period_days: i64,
    ) -> Result<Self, PriorsLoadError> {
        let mut per_source_states: HashMap<String, SourceIetState> = HashMap::new();
        for (src, summ) in &bp.per_source_iet.by_source {
            per_source_states.insert(
                src.clone(),
                SourceIetState {
                    cdf_values: summ.empirical_cdf_days.values.clone(),
                    cdf_probabilities: summ.empirical_cdf_days.probabilities.clone(),
                    lag1_autocorr: summ.lag1_autocorr,
                    last_iet_days: None,
                },
            );
        }
        let fallback = SourceIetState {
            cdf_values: vec![1.0],
            cdf_probabilities: vec![1.0],
            lag1_autocorr: 0.0,
            last_iet_days: None,
        };
        let iet_sampler = ConditionalIETSampler::from_state_map(per_source_states, fallback);

        let lifetime_hist = bp.active_lifetime.overall.clone();
        let sources: Vec<String> = bp.source_mix.probabilities.keys().cloned().collect();
        let active_window = SourceActiveWindow::build(
            &sources,
            period_days,
            |r| lifetime_hist.sample_bucket(r) as i64,
            rng,
        );

        let multi_segment_window = bp.active_segments.as_ref().map(|prior| {
            let lifetime_hist = bp.active_lifetime.overall.clone();
            MultiSegmentActiveWindow::build_from_prior(
                &sources,
                period_days,
                prior,
                |r| lifetime_hist.sample_bucket(r) as i64,
                rng,
            )
        });

        let mut fanout_samplers: HashMap<String, BipartiteFanoutSampler> = HashMap::new();
        for (attr, hist) in &bp.fanout.by_attribute {
            const N_POOL: usize = 256;
            let targets: Vec<u32> = (0..N_POOL).map(|_| hist.sample_bucket(rng)).collect();
            let attr_prefix = attr.clone();
            let sampler = BipartiteFanoutSampler::new_with_targets(targets, move |i| {
                format!("{attr_prefix}-{i:04}")
            });
            fanout_samplers.insert(attr.clone(), sampler);
        }

        let cross_entity_motifs = bp
            .entity_clusters
            .as_ref()
            .map(CrossEntityMotifSampler::from_prior);

        let tp_motif_sampler = bp
            .tp_entity_clusters
            .as_ref()
            .map(CrossEntityMotifSampler::from_prior);

        let per_source_attribute = bp.per_source_attribute.clone();
        let reference_formats = bp.reference_formats.clone();
        let coa_semantic = bp.coa_semantic.clone();
        // SP4.5 — carry user_personas through from the bundle.  When the bundle
        // was built without user-column data (the current corpus), this is either
        // None or Some(empty-stub).  The generator guards with `has_data()`.
        let user_personas = bp.user_personas.clone();
        // SP4.3 — carry source_amount_conditionals through from the bundle.
        // Old bundles (pre-SP4.3) will have None here; generators fall back to
        // the existing AmountSampler.
        let source_amount_conditionals = bp.source_amount_conditionals.clone();
        // SP4.6 — carry source_role_gl_conditionals through from the bundle.
        // Old bundles (pre-SP4.6) will have None here; generators fall back to
        // sample_attribute_for_source then to default GL accounts.
        let source_role_gl = bp.source_role_gl_conditionals.clone();
        // SP4.1 — carry tb_anchor through from the bundle.  Old bundles (pre-SP4.1)
        // will have None here; the balance tracker operates in free-drift mode.
        let tb_anchor = bp.tb_anchor.clone();
        // SP6 — carry text_taxonomy through. Old bundles (pre-SP6) have None;
        // generators fall back to the DescriptionGenerator.
        let text_taxonomy = bp.text_taxonomy.clone();

        Ok(LoadedPriors {
            industry: bp.industry.clone(),
            bundle_path,
            source_mix: bp.source_mix,
            iet_sampler,
            lines_per_je: bp.lines_per_je,
            active_window,
            multi_segment_window,
            fanout_samplers,
            posting_lag: bp.posting_lag,
            cross_entity_motifs,
            per_source_attribute,
            tp_motif_sampler,
            reference_formats,
            coa_semantic,
            user_personas,
            source_amount_conditionals,
            source_role_gl,
            tb_anchor,
            text_taxonomy,
        })
    }
}

impl LoadedPriors {
    /// SP3.7 — Try to sample `attribute` value conditional on the given
    /// `source` code.  Returns `None` when either the prior is absent,
    /// the source isn't represented in the prior, the attribute isn't
    /// present for that source, or the conditional distribution is empty.
    ///
    /// Caller falls back to the marginal sampler in that case.
    pub fn sample_attribute_for_source<R: rand::Rng>(
        &self,
        source: &str,
        attribute: &str,
        rng: &mut R,
    ) -> Option<String> {
        self.per_source_attribute
            .as_ref()?
            .conditional(source, attribute)?
            .sample(rng)
    }

    /// SP4.5 — Sample a user ID likely to post the given `source` code from the
    /// user-persona prior.
    ///
    /// Returns `None` when:
    /// - No `user_personas` prior was loaded, OR
    /// - The prior is empty (no user-column data; typical for current corpus), OR
    /// - No user has a non-zero weight for `source`.
    ///
    /// Callers fall back to the internal user pool (`select_user`) in all of these
    /// cases — the prior is purely additive.
    pub fn sample_user_for_source<R: rand::Rng>(
        &self,
        source: &str,
        rng: &mut R,
    ) -> Option<String> {
        self.user_personas
            .as_ref()
            .filter(|up| up.has_data())
            .and_then(|up| up.sample_user_for_source(source, rng))
    }

    /// SP4.5 — Given a `user_id` from the prior, sample an `(hour, weekday)` pair
    /// from the user's characteristic density.
    ///
    /// Returns `None` when the prior is absent, empty, or the user ID is unknown.
    /// `hour` ∈ 0..24, `weekday` ∈ 0..7 (Monday = 0).
    pub fn sample_timestamp_for_user<R: rand::Rng>(
        &self,
        user_id: &str,
        rng: &mut R,
    ) -> Option<(u32, u32)> {
        self.user_personas
            .as_ref()
            .filter(|up| up.has_data())
            .and_then(|up| up.sample_timestamp_for_user(user_id, rng))
    }

    /// SP4.3 — Sample a JE total-amount magnitude for `source` (and optionally
    /// `gl_prefix`) from the per-(source, gl_prefix) log-normal prior.
    ///
    /// Lookup strategy:
    /// 1. Try `(source, gl_prefix)` when `gl_prefix` is non-empty.
    /// 2. Fall back to the source-marginal.
    /// 3. Return `None` when the prior is absent or the source isn't represented.
    ///
    /// Callers fall back to the existing `AmountSampler` when `None` is returned.
    /// Fraud entries should bypass this helper entirely — the caller is responsible
    /// for that guard.
    pub fn sample_amount_for_source<R: rand::Rng>(
        &self,
        source: &str,
        gl_prefix: &str,
        rng: &mut R,
    ) -> Option<f64> {
        let p = self.source_amount_conditionals.as_ref()?;
        // Try (source, gl_prefix) first.
        if !gl_prefix.is_empty() {
            if let Some(per_class) = p.by_source_and_class.get(source) {
                if let Some(params) = per_class.get(gl_prefix) {
                    return Some(params.sample(rng));
                }
            }
        }
        // Fall back to source-marginal.
        p.by_source.get(source).map(|params| params.sample(rng))
    }

    /// SP4.6 — Sample a GL account conditioned on `(source, role)` where `role`
    /// is `"DR"` or `"CR"`.
    ///
    /// Returns `None` when:
    /// - No `source_role_gl` prior was loaded (old bundle), OR
    /// - The `(source, role)` pair isn't represented (sparse corpus), OR
    /// - The distribution is empty.
    ///
    /// Callers must fall back to `sample_attribute_for_source(source, "gl_account", ...)`
    /// and ultimately to a hard-coded default GL when `None` is returned.
    pub fn sample_gl_for_source_role<R: rand::Rng>(
        &self,
        source: &str,
        role: &str,
        rng: &mut R,
    ) -> Option<String> {
        self.source_role_gl
            .as_ref()?
            .conditional(source, role)?
            .sample(rng)
    }

    /// SP4.7 — Sample a reference string for `source` from the reference-format
    /// prior.  Returns `None` when the prior is absent or the source has no
    /// templates (caller falls back to the existing `format!(...)` template).
    pub fn sample_reference<R: rand::Rng>(&self, source: &str, rng: &mut R) -> Option<String> {
        let rf = self.reference_formats.as_ref()?;
        let templates = rf.by_source.get(source)?;
        if templates.is_empty() {
            return None;
        }
        // Weighted sample by probability mass.
        let total: f64 = templates.iter().map(|t| t.probability).sum();
        if total <= 0.0 {
            return None;
        }
        use rand::RngExt;
        let r: f64 = rng.random_range(0.0..total);
        let mut cum = 0.0;
        for t in templates {
            cum += t.probability;
            if r <= cum {
                return Some(fill_reference_template(&t.template, rng));
            }
        }
        // Floating-point rounding: return last template.
        templates
            .last()
            .map(|t| fill_reference_template(&t.template, rng))
    }

    /// SP6 — Sample a line-text string for `(source, account_class)` from the
    /// text-taxonomy prior, filling placeholders via `resolver`.
    ///
    /// Lookup cascade:
    /// 1. `line_pools["SOURCE|CLASS"]`
    /// 2. `line_pools["SOURCE|_unknown_"]`
    /// 3. `header_pools["SOURCE"]` (last resort — source-level vocabulary)
    ///
    /// Returns `None` only when the prior is absent or the source has no pools
    /// at any cascade tier — the caller then falls back to the `DescriptionGenerator`.
    pub fn sample_line_template<R: rand::Rng>(
        &self,
        source: &str,
        account_class: &str,
        resolver: &mut dyn datasynth_core::distributions::text_taxonomy::PlaceholderResolver,
        rng: &mut R,
    ) -> Option<String> {
        let tx = self.text_taxonomy.as_ref()?;
        let class_key = TextTaxonomyPrior::line_key(source, account_class);
        let unknown_key = TextTaxonomyPrior::line_key(source, TextTaxonomyPrior::UNKNOWN_CLASS);
        let pool = tx
            .line_pools
            .get(&class_key)
            .or_else(|| tx.line_pools.get(&unknown_key))
            .or_else(|| tx.header_pools.get(source))?;
        sample_pool_filled(pool, resolver, rng)
    }

    /// SP6 — Sample a header-text string for `source` from the text-taxonomy
    /// prior. Returns `None` when absent / no pool for the source.
    pub fn sample_header_template<R: rand::Rng>(
        &self,
        source: &str,
        resolver: &mut dyn datasynth_core::distributions::text_taxonomy::PlaceholderResolver,
        rng: &mut R,
    ) -> Option<String> {
        let tx = self.text_taxonomy.as_ref()?;
        let pool = tx.header_pools.get(source)?;
        sample_pool_filled(pool, resolver, rng)
    }

    /// SP6 — Fill the CoA description template for `account_no`. Returns `None`
    /// when the prior is absent or the account has no template.
    pub fn sample_coa_description<R: rand::Rng>(
        &self,
        account_no: &str,
        resolver: &mut dyn datasynth_core::distributions::text_taxonomy::PlaceholderResolver,
        rng: &mut R,
    ) -> Option<String> {
        let tx = self.text_taxonomy.as_ref()?;
        let entry = tx.coa_pools.get(account_no)?;
        Some(
            datasynth_core::distributions::text_taxonomy::PlaceholderGrammar::fill(
                &entry.template,
                resolver,
                rng,
            ),
        )
    }
}

/// SP6 — Weighted-pick a `TemplateEntry` from a `TemplatePool` and fill it.
fn sample_pool_filled<R: rand::Rng>(
    pool: &datasynth_core::distributions::text_taxonomy::TemplatePool,
    resolver: &mut dyn datasynth_core::distributions::text_taxonomy::PlaceholderResolver,
    rng: &mut R,
) -> Option<String> {
    use datasynth_core::distributions::text_taxonomy::PlaceholderGrammar;
    use rand::RngExt;
    if pool.templates.is_empty() {
        return None;
    }
    let total: f64 = pool.templates.iter().map(|t| t.probability).sum();
    if total <= 0.0 {
        return None;
    }
    let r: f64 = rng.random_range(0.0..total);
    let mut cum = 0.0;
    for t in &pool.templates {
        cum += t.probability;
        if r <= cum {
            return Some(PlaceholderGrammar::fill(&t.template, resolver, rng));
        }
    }
    pool.templates
        .last()
        .map(|t| PlaceholderGrammar::fill(&t.template, resolver, rng))
}

/// Fill a reference format template by replacing `{N digits}` and `{N alpha}`
/// placeholders with random strings of the indicated length and character class.
/// Fixed characters in the template are reproduced verbatim.
fn fill_reference_template<R: rand::Rng>(template: &str, rng: &mut R) -> String {
    use rand::RngExt;
    if template.is_empty() {
        return String::new();
    }
    let mut result = String::with_capacity(template.len() * 2);
    let mut chars = template.char_indices().peekable();

    while let Some((i, ch)) = chars.next() {
        if ch == '{' {
            // Find the closing '}'
            let rest = &template[i..];
            if let Some(close_offset) = rest.find('}') {
                let inner = &rest[1..close_offset];
                // Advance the iterator past the closing '}'
                let end_byte = i + close_offset + 1;
                // Consume chars up to end_byte
                while chars.peek().map(|(j, _)| *j < end_byte).unwrap_or(false) {
                    chars.next();
                }
                if let Some((count, kind)) = parse_ref_placeholder(inner) {
                    match kind {
                        RefPlaceholderKind::Digits => {
                            for _ in 0..count {
                                result.push(char::from(b'0' + rng.random_range(0u8..10)));
                            }
                        }
                        RefPlaceholderKind::Alpha => {
                            for _ in 0..count {
                                result.push(char::from(b'A' + rng.random_range(0u8..26)));
                            }
                        }
                    }
                } else {
                    result.push('{');
                    result.push_str(inner);
                    result.push('}');
                }
            } else {
                result.push(ch);
            }
        } else {
            result.push(ch);
        }
    }
    result
}

enum RefPlaceholderKind {
    Digits,
    Alpha,
}

fn parse_ref_placeholder(inner: &str) -> Option<(usize, RefPlaceholderKind)> {
    let inner = inner.trim();
    if let Some(rest) = inner.strip_suffix("digits") {
        let n: usize = rest.trim().parse().ok()?;
        Some((n, RefPlaceholderKind::Digits))
    } else if let Some(rest) = inner.strip_suffix("alpha") {
        let n: usize = rest.trim().parse().ok()?;
        Some((n, RefPlaceholderKind::Alpha))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Internal: minimal .dsf reader (ZIP + YAML)
// ---------------------------------------------------------------------------

/// Read and deserialize the `behavioral.yaml` component from a `.dsf` archive.
/// Returns `None` if the archive does not contain a behavioral section.
fn read_behavioral_from_dsf<R: Read + Seek>(reader: R) -> Result<Option<BehavioralPriors>, String> {
    let mut archive = zip::ZipArchive::new(reader).map_err(|e| format!("zip open: {e}"))?;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("zip entry {i}: {e}"))?;
        if entry.name() == BEHAVIORAL_YAML_KEY {
            let mut buf = String::new();
            entry
                .read_to_string(&mut buf)
                .map_err(|e| format!("read {BEHAVIORAL_YAML_KEY}: {e}"))?;
            let bp: BehavioralPriors = serde_yaml::from_str(&buf)
                .map_err(|e| format!("deserialize behavioral.yaml: {e}"))?;
            return Ok(Some(bp));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn bundled_priors_path_known() {
        let p = bundled_priors_path("health");
        assert!(p.ends_with("industry_priors_health.dsf"));
    }

    #[test]
    fn load_bundled_health_actually_works() {
        let p = bundled_priors_path("health");
        if !p.exists() {
            eprintln!("skipping: {} not present", p.display());
            return;
        }
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let priors =
            LoadedPriors::load_bundled("health", &mut rng, 365).expect("load_bundled health");
        assert_eq!(priors.industry, "health");
        assert!(!priors.source_mix.probabilities.is_empty());
        assert!(priors.fanout_samplers.contains_key("GLAccount"));
    }

    #[test]
    fn load_from_path_not_found() {
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let result =
            LoadedPriors::load_from_path(Path::new("/nonexistent.dsf"), &mut rng, 365, None);
        assert!(result.is_err());
        assert!(matches!(
            result.err().expect("expected err"),
            PriorsLoadError::NotFound(_)
        ));
    }

    // ---- SP4.6 tests -------------------------------------------------------

    use datasynth_core::distributions::behavioral_priors::{
        BehavioralPriors, CategoricalDistribution, PerSourceRolePrior,
    };
    use std::collections::BTreeMap;

    fn make_kr_role_prior() -> PerSourceRolePrior {
        let mut dr_counts = BTreeMap::new();
        dr_counts.insert("6000".to_string(), 100usize);
        dr_counts.insert("6100".to_string(), 50usize);
        let mut cr_counts = BTreeMap::new();
        cr_counts.insert("2000".to_string(), 150usize);

        let mut role_map = BTreeMap::new();
        role_map.insert(
            "DR".to_string(),
            CategoricalDistribution::from_counts(dr_counts),
        );
        role_map.insert(
            "CR".to_string(),
            CategoricalDistribution::from_counts(cr_counts),
        );

        let mut by_source_and_role = BTreeMap::new();
        by_source_and_role.insert("KR".to_string(), role_map);
        PerSourceRolePrior { by_source_and_role }
    }

    fn minimal_bp_with_role_prior(role_prior: PerSourceRolePrior) -> BehavioralPriors {
        use datasynth_core::distributions::behavioral_priors::*;
        BehavioralPriors {
            schema_version: BehavioralPriors::SCHEMA_VERSION,
            generator_version: "test".to_string(),
            industry: "test".to_string(),
            n_client_inputs: 1,
            n_rows_aggregated: 1000,
            source_mix: SourceMixPrior::default(),
            per_source_iet: PerSourceIetPrior::default(),
            lines_per_je: LinesPerJePrior::default(),
            active_lifetime: ActiveLifetimePrior::default(),
            fanout: FanoutPrior::default(),
            posting_lag: None,
            active_segments: None,
            entity_clusters: None,
            per_source_attribute: None,
            tp_entity_clusters: None,
            coa_semantic: None,
            reference_formats: None,
            text_taxonomy: None,
            user_personas: None,
            source_amount_conditionals: None,
            source_role_gl_conditionals: Some(role_prior),
            tb_anchor: None,
        }
    }

    /// `sample_gl_for_source_role` returns only DR-class accounts when role="DR".
    #[test]
    fn sp4_6_sample_gl_for_source_role_dr_returns_expense_accounts() {
        let bp = minimal_bp_with_role_prior(make_kr_role_prior());
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let priors = LoadedPriors::from_priors(bp, std::path::PathBuf::from("test"), &mut rng, 365)
            .expect("from_priors");

        let mut rng2 = ChaCha8Rng::seed_from_u64(77);
        for _ in 0..100 {
            let v = priors.sample_gl_for_source_role("KR", "DR", &mut rng2);
            assert!(v.is_some(), "should return Some for KR/DR");
            let v = v.unwrap();
            assert!(
                v == "6000" || v == "6100",
                "DR draw must be expense account, got {v}"
            );
        }
    }

    /// `sample_gl_for_source_role` returns `None` when the prior is absent.
    #[test]
    fn sp4_6_sample_gl_for_source_role_returns_none_when_prior_absent() {
        use datasynth_core::distributions::behavioral_priors::*;
        let bp = BehavioralPriors {
            schema_version: BehavioralPriors::SCHEMA_VERSION,
            generator_version: "test".to_string(),
            industry: "test".to_string(),
            n_client_inputs: 0,
            n_rows_aggregated: 0,
            source_mix: SourceMixPrior::default(),
            per_source_iet: PerSourceIetPrior::default(),
            lines_per_je: LinesPerJePrior::default(),
            active_lifetime: ActiveLifetimePrior::default(),
            fanout: FanoutPrior::default(),
            posting_lag: None,
            active_segments: None,
            entity_clusters: None,
            per_source_attribute: None,
            tp_entity_clusters: None,
            coa_semantic: None,
            reference_formats: None,
            text_taxonomy: None,
            user_personas: None,
            source_amount_conditionals: None,
            source_role_gl_conditionals: None, // no role prior
            tb_anchor: None,
        };
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let priors = LoadedPriors::from_priors(bp, std::path::PathBuf::from("test"), &mut rng, 365)
            .expect("from_priors");

        let result = priors.sample_gl_for_source_role("KR", "DR", &mut rng);
        assert!(
            result.is_none(),
            "must return None when source_role_gl is absent"
        );
    }

    // ---- SP6 tests -----------------------------------------------------------

    use datasynth_core::distributions::text_taxonomy::{
        TemplateEntry, TemplatePool, TextTaxonomyPrior,
    };

    fn bp_with_text_taxonomy() -> BehavioralPriors {
        let mut tx = TextTaxonomyPrior::default();
        tx.line_pools.insert(
            "KR|A.B".to_string(),
            TemplatePool {
                templates: vec![TemplateEntry {
                    template: "Rechnung Eingang".to_string(),
                    probability: 1.0,
                    synthetic_example: "Rechnung Eingang".to_string(),
                }],
                n: 50,
            },
        );
        tx.line_pools.insert(
            "KR|_unknown_".to_string(),
            TemplatePool {
                templates: vec![TemplateEntry {
                    template: "Diverse".to_string(),
                    probability: 1.0,
                    synthetic_example: "Diverse".to_string(),
                }],
                n: 20,
            },
        );
        tx.header_pools.insert(
            "KR".to_string(),
            TemplatePool {
                templates: vec![TemplateEntry {
                    template: "Monatsabschluss".to_string(),
                    probability: 1.0,
                    synthetic_example: "Monatsabschluss".to_string(),
                }],
                n: 30,
            },
        );
        tx.coa_pools.insert(
            "0000204000".to_string(),
            TemplateEntry {
                template: "Kreditoren".to_string(),
                probability: 1.0,
                synthetic_example: "Kreditoren".to_string(),
            },
        );
        let mut bp = minimal_bp_with_role_prior(make_kr_role_prior());
        bp.text_taxonomy = Some(tx);
        bp
    }

    #[test]
    fn sample_line_template_keyed_on_source_and_class() {
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        let priors =
            LoadedPriors::from_priors(bp_with_text_taxonomy(), PathBuf::from("t"), &mut rng, 365)
                .expect("from_priors");
        let mut resolver = datasynth_core::distributions::text_taxonomy::SyntheticExampleResolver;
        let mut r2 = ChaCha8Rng::seed_from_u64(2);
        // exact (source,class) hit
        let v = priors.sample_line_template("KR", "A.B", &mut resolver, &mut r2);
        assert_eq!(v, Some("Rechnung Eingang".to_string()));
        // unknown class -> cascade to KR|_unknown_
        let v = priors.sample_line_template("KR", "Z.Z", &mut resolver, &mut r2);
        assert_eq!(v, Some("Diverse".to_string()));
        // unknown source -> None (caller falls back)
        let v = priors.sample_line_template("ZZ", "A.B", &mut resolver, &mut r2);
        assert_eq!(v, None);
    }

    /// Cascade tier 3: when a source has NO `_unknown_` line pool but DOES
    /// have a header pool, `sample_line_template` must fall through to the
    /// header vocabulary. Uses a fixture without `RV|_unknown_` to force
    /// tier 2 to miss.
    #[test]
    fn sample_line_template_falls_through_to_header_pool_at_tier_3() {
        use datasynth_core::distributions::text_taxonomy::{
            TemplateEntry, TemplatePool, TextTaxonomyPrior,
        };
        let mut tx = TextTaxonomyPrior::default();
        // RV has only a header pool — no line pools at all.
        tx.header_pools.insert(
            "RV".to_string(),
            TemplatePool {
                templates: vec![TemplateEntry {
                    template: "Header-only fallback".to_string(),
                    probability: 1.0,
                    synthetic_example: "Header-only fallback".to_string(),
                }],
                n: 5,
            },
        );
        let mut bp = minimal_bp_with_role_prior(make_kr_role_prior());
        bp.text_taxonomy = Some(tx);

        let mut rng = ChaCha8Rng::seed_from_u64(1);
        let priors =
            LoadedPriors::from_priors(bp, PathBuf::from("t"), &mut rng, 365).expect("from_priors");
        let mut resolver = datasynth_core::distributions::text_taxonomy::SyntheticExampleResolver;
        let mut r2 = ChaCha8Rng::seed_from_u64(2);
        // tier 1 miss (no RV|A.B), tier 2 miss (no RV|_unknown_),
        // tier 3 hits (header_pools["RV"])
        let v = priors.sample_line_template("RV", "A.B", &mut resolver, &mut r2);
        assert_eq!(v, Some("Header-only fallback".to_string()));
    }

    #[test]
    fn sample_coa_description_hits_account() {
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        let priors =
            LoadedPriors::from_priors(bp_with_text_taxonomy(), PathBuf::from("t"), &mut rng, 365)
                .expect("from_priors");
        let mut resolver = datasynth_core::distributions::text_taxonomy::SyntheticExampleResolver;
        let mut r2 = ChaCha8Rng::seed_from_u64(2);
        assert_eq!(
            priors.sample_coa_description("0000204000", &mut resolver, &mut r2),
            Some("Kreditoren".to_string())
        );
        assert_eq!(
            priors.sample_coa_description("9999999999", &mut resolver, &mut r2),
            None
        );
    }
}
