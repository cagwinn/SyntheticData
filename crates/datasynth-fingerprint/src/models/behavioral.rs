//! Behavioral priors mined from corpus GL data.
//!
//! All types are now defined in `datasynth_core::distributions::behavioral_priors`
//! and re-exported here to preserve the existing public API surface.
//!
//! Spec: `docs/superpowers/specs/2026-05-12-sp2-real-world-prior-extraction-design.md`

pub use datasynth_core::distributions::behavioral_priors::{
    AccountSemantic, ActiveLifetimePrior, ActiveSegmentsPrior, BehavioralPriors,
    CategoricalDistribution, CoaSemanticPrior, EntityCluster, EntityClustersPrior, FanoutPrior,
    IetSummary, LagSummary, LineCountHistogram, LinesPerJePrior, LognormalAmount, LognormalParams,
    PerSourceAmountPrior, PerSourceAttributePrior, PerSourceIetPrior, PerSourceRolePrior,
    PostingLagPrior, ReferenceFormatPrior, ReferenceTemplate, SourceMixPrior, SourceSegmentSummary,
    TbAnchorPrior, TbTarget, UserBehavior, UserPersonaPrior, ACTIVE_LIFETIME_DAY_BUCKETS,
    FANOUT_BUCKETS, LINE_COUNT_BUCKETS, SEGMENT_COUNT_BUCKETS, SEGMENT_GAP_BUCKETS,
};
// SP6 — text-taxonomy types live in a sibling distributions module; re-exported
// here so SDK consumers can reach them from the same fingerprint surface as the
// rest of `BehavioralPriors`'s child types.
pub use datasynth_core::distributions::text_taxonomy::{
    PiiHit, PiiPlaceholderKind, PlaceholderGrammar, PlaceholderResolver, SyntheticExampleResolver,
    TaxonomyMeta, TemplateEntry, TemplatePool, TextTaxonomyPrior,
};
