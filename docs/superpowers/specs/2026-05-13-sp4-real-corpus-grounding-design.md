# SP4 — corpus Grounded Data — Design Spec

**Date:** 2026-05-13
**Status:** Draft (post-brainstorming, autonomous-execution-approved)
**Scope:** Broaden DataSynth's use of the corpus gem from "behavioral priors only" to "full audit-grade grounded data" — TB anchoring, CoA semantic content, per-(source, account) amount distributions, header/line text vocabulary, user-persona patterns, document-type line-shape conditionals, reference-format conventions.
**Predecessor:** [v5.20 baseline](../../baselines/2026-05-13-v5.20.0/SUMMARY.md) — composite BF 41.5× mean / 16.6× median. SP3 series complete; median crosses ≤25× target line.

## 1. Overview

The SP3 series (SP3.1–SP3.11) brought DataSynth from a row-independent generator (composite BF 59×) to a behaviorally-faithful generator on most measured signals (median 16.6×). What it deliberately did NOT do: leverage the *semantic richness* of the corpus. The bundles ship statistical priors only — distributions and conditionals — but the synthetic output still emits placeholder text ("Manual Journal Entry — Finance"), generic account constants when priors are disabled, and statistically-shaped but semantically-empty descriptive fields.

SP4 closes that gap. The corpus has TB files (per-account balances), CoA files (account name + hierarchy + type), and JE files with rich descriptive text, real user IDs, real reference formats. Mining and threading these into generation makes the synthetic data:

- **Audit-grade**: balance-sheet shapes match a corpus-shaped target (TB anchoring).
- **Semantically faithful**: account descriptions, line texts, reference formats look like corpus documents.
- **Multi-modal**: data is useful for downstream NER/NLP/audit-tool training without further enrichment.

| Item | What | Effort |
| ---- | ---- | ------ |
| SP4.1 | TB anchoring — per-account opening/closing/period-activity targets | ~2 weeks |
| SP4.2 | CoA semantic content — account_description + account_class hierarchy | ~3 days |
| SP4.3 | Per-(source, account_class) amount conditionals — Benford × magnitude per business type | ~1 week |
| SP4.4 | Header/line text vocabulary — sampleable templates per business process | ~4 days |
| SP4.5 | Created_by / user-persona patterns — real user-ID distribution × per-user posting time | ~3 days |
| SP4.6 | Document-type line-shape conditionals — P(lines | doc_type) × P(accounts | doc_type, line_role) | ~1 week |
| SP4.7 | Reference format conventions — per-source reference-string template (e.g. "PO-2024-NNNNNN", "RE-NNNNNN-YYYY") | ~2 days |

Total: ~4-6 weeks using subagent-driven dispatch. SP4.1 (TB anchoring) is the architectural lift; SP4.2–SP4.7 are mostly small, parallelisable additions.

### 1.1 Target outcome

After SP4 ships:

- **Behavioral-fidelity composite stays at v5.20 levels or improves** (no regressions on SP3.x wins).
- **Synthetic balance sheet reconciles to a TB target** within ±5% per account, ±1% in aggregate.
- **Synthetic CoA matches corpus CoA hierarchy** (3-4 levels, ISO 21378 account_class mapped).
- **Synthetic line_text and header_text emit corpus-style templates** (sampled from 200-500 real templates per business process).
- **Synthetic created_by emits real user IDs** drawn from per-industry user-population distribution.
- **Downstream consumers** (audit tools, NER models, fraud-detection ML pipelines) get materially-better training data without additional enrichment.

### 1.2 Non-goals (deferred for separate work)

- **TB-grounded fraud injection** — fraud insertion is currently amount-magnitude-based; making it TB-aware (e.g. ghost employee inflates payroll line but TB total unchanged via offset) is a meaningful audit-realism win but architecturally distinct from SP4.
- **Multi-corpus grounding** — currently we ground against a single industry's clients. Cross-industry grounding (e.g. a manufacturing-flavoured healthcare client) is a future axis.
- **Per-region intraday timing** — the SP3.x ActiveSegmentsPrior handles month-end clustering well enough for now; per-region intraday refinement is out of scope.
- **Hierarchical CoA mining beyond ISO 21378** — country-specific account-class taxonomies (HGB Kontenrahmen, IFRS templates) are useful but out of scope.
- **Differential privacy on text/reference vocabulary** — these mined vocabularies are aggregate by construction, but the DP audit is a separate concern.
- **Tax code distributions** — important but addressed in a separate tax-engine spec.

### 1.3 Renaming note

The original SP4 framing ("showcase release") is superseded. The showcase release becomes a release-cadence milestone after SP4 ships; this spec is about the *content* improvements that make the showcase compelling. The composite BF target (≤25× median, ✓ already hit) doesn't move — SP4 is about *semantic depth* not statistical fidelity.

## 2. Architecture per item

### 2.1 SP4.1 — TB anchoring

**Files:**
- `crates/datasynth-fingerprint/src/extraction/tb_extractor.rs` (new — extracts per-account TB targets from `TB_XXX.parquet`)
- `crates/datasynth-core/src/distributions/behavioral_priors.rs` (add `TbAnchorPrior`)
- `crates/datasynth-fingerprint/src/aggregation/industry_aggregator.rs` (add `aggregate_tb_anchor`)
- `crates/datasynth-generators/src/balance/balance_tracker.rs` (consume target balances)
- `crates/datasynth-generators/src/je_generator.rs` (drift correction)

**Data flow:**

```
TB_XXX.parquet (per-client TB)
  ↓ extract_tb_anchor
TbAnchorPrior {
  per_account: BTreeMap<String, TbTarget>,
  total_assets: f64, total_liabilities: f64, total_equity: f64,
}
  ↓ aggregate_tb_anchor (median per account across clients)
TbAnchorPrior (industry-level)
  ↓ bundled in .dsf
LoadedPriors.tb_anchor
  ↓ balance_tracker consumes targets at start
JE generation aims at target;
periodic drift-correction journal entries close the gap.
```

**Per-account TbTarget structure:**

```rust
pub struct TbTarget {
    pub opening_balance: f64,
    pub closing_balance: f64,
    pub period_activity_debit: f64,
    pub period_activity_credit: f64,
    /// Stdev across contributing clients (smaller = stricter target)
    pub opening_stdev: f64,
    pub closing_stdev: f64,
}
```

The generator's `balance_tracker` is extended with a "target-aware" mode: when `loaded_priors.tb_anchor.is_some()`, the generator monitors running balances per account vs the target. Every N JEs (configurable, default 100), if drift exceeds the target's stdev × 3, the generator emits a small drift-correction JE that nudges the balance back. The drift corrections themselves look like ordinary close-period or revaluation entries (not flagged as artificial).

**Implementation order:**
1. Extract — straightforward parquet read, group by account, aggregate.
2. Generator-side target-aware mode — bigger lift; needs the balance_tracker to expose per-account drift visibility.
3. Drift-correction JE emission — uses existing accrual-style document type; just a new motivation.

**Risk:**
- Drift corrections may collide with existing period-close or accrual generation, producing double-correction. Mitigate with a generator flag that disables anchor-driven corrections during the close-period generation phase.

**Acceptance:**
- New unit test: generator with TB anchor produces final balances within ±5% per account vs target.
- Integration test: 10K-JE generation maintains balance sheet equation (A = L + E) at every period close.
- v5.21 baseline: behavioral-fidelity composite unchanged or improved.

### 2.2 SP4.2 — CoA semantic content

**Files:**
- `crates/datasynth-fingerprint/src/extraction/coa_extractor.rs` (new — extracts CoA from `COA_XXX.parquet`)
- `crates/datasynth-core/src/distributions/behavioral_priors.rs` (add `CoaSemanticPrior`)
- `crates/datasynth-generators/src/coa_generator.rs` (consume the prior)

**Prior shape:**

```rust
pub struct CoaSemanticPrior {
    /// account_number → (description, account_class, account_sub_class, parent_account)
    pub accounts: BTreeMap<String, AccountSemantic>,
    /// Hierarchical structure: parent_account → list of children
    pub hierarchy: BTreeMap<String, Vec<String>>,
}

pub struct AccountSemantic {
    pub description: String,
    pub account_class: Option<String>,        // ISO 21378 Level-2
    pub account_sub_class: Option<String>,    // ISO 21378 Level-3
    pub account_type: Option<AccountType>,    // Asset/Liability/Equity/Income/Expense
}
```

The generator's CoA generator, when priors are loaded, samples real account numbers + descriptions from the bundle instead of synthesizing generic ones. The output CSV's `account_description`, `account_class`, `account_class_name`, `account_sub_class`, `account_sub_class_name` columns get corpus values.

**Acceptance:**
- Synthetic `account_description` distribution overlaps ≥80% with corpus distinct descriptions for the dominant industry.
- ISO 21378 `account_class` populated for ≥95% of synthetic rows when priors enabled.

### 2.3 SP4.3 — Per-(source, account_class) amount conditionals

**Why:** Each business-source-and-account-type pair has a characteristic amount magnitude. KR (vendor invoice) hitting AP control (account_class A.B): typically €500–€50,000. SA (manual journal voucher) hitting depreciation expense: typically €1,000–€10,000. RV (customer invoice) hitting revenue: highly variable. The current `AmountSampler` uses log-normal mix over the entire amount space, which produces statistically-shaped but semantically-flat amount distributions.

**Files:**
- `crates/datasynth-fingerprint/src/extraction/behavioral_extractor.rs` (extend `extract_per_source_attribute` with a new attribute `"amount_class"` — buckets amounts into log-magnitude bins)
- OR add a new prior: `PerSourceAmountPrior { by_source_and_account_class: BTreeMap<(String, String), AmountDistribution> }`
- `crates/datasynth-generators/src/je_generator.rs` (consult per-(source, account_class) when sampling amounts)

**Approach decision:** Going with the new prior (`PerSourceAmountPrior`) rather than extending `per_source_attribute`, because amount-class is a continuous attribute and the existing categorical conditional doesn't quite fit. The new prior carries a 2D conditional `P(amount | source, account_class)`.

**Acceptance:**
- v5.21 baseline: P4 MeanGap on Source drops from 3.97 → ≤2.0.
- Per-account-class Benford compliance MAD < 0.02 (currently global MAD < 0.015).

### 2.4 SP4.4 — Header/line text vocabulary

**Files:**
- `crates/datasynth-fingerprint/src/extraction/text_extractor.rs` (new — mines header_text + line_text patterns from JE files)
- `crates/datasynth-core/src/distributions/behavioral_priors.rs` (add `TextTemplatePrior`)
- `crates/datasynth-generators/src/je_generator.rs` (sample template at JE-emit time)

**Prior shape:**

```rust
pub struct TextTemplatePrior {
    /// Templates by business_process or source-code key.  Stored as templates,
    /// not raw text — so generation can interpolate values (e.g. "Sales Commission Accrual - {quarter}").
    pub header_templates: BTreeMap<String, Vec<TextTemplate>>,
    pub line_templates: BTreeMap<String, Vec<TextTemplate>>,
}

pub struct TextTemplate {
    pub template: String,       // e.g. "Cash withdrawal", "Sales Commission Accrual - {quarter}"
    pub probability: f64,
    pub placeholders: Vec<Placeholder>,  // describes {quarter}, {month}, {employee}, etc.
}
```

**Placeholder filling:**

The generator already has access to fiscal context (year, quarter, month) and employee/vendor lists. Placeholder values are sampled from those existing pools. New placeholders that aren't easily filled get left as-is (still better than empty).

**Mining the templates:**

For each source / business_process bucket, find the top-100 most frequent header_texts. Run a lightweight template extractor:
1. Normalise digit sequences to `{digits}` placeholders.
2. Normalise date-like patterns to `{date}` / `{month}` / `{quarter}`.
3. Normalise capitalised single-word entities (likely names) to `{entity}`.
4. Keep punctuation + literal words.

Top-N (configurable, default 50) templates per bucket are kept.

**Privacy:**
- Text templates are aggregate by construction — only mass-frequency templates survive (≥10 occurrences). Single-occurrence specific entries don't get retained. The DP audit on text is mostly automatic.

**Acceptance:**
- Synthetic `header_text` distinct count matches real distribution within ±20%.
- Top-20 most-frequent synthetic header_texts include ≥10 corpus phrases.
- Manual spot-check: synthetic line_texts look "obviously realistic" to a domain reviewer.

### 2.5 SP4.5 — Created_by / user-persona patterns

**Files:**
- `crates/datasynth-fingerprint/src/extraction/user_extractor.rs` (new)
- `crates/datasynth-core/src/distributions/behavioral_priors.rs` (add `UserPersonaPrior`)
- `crates/datasynth-generators/src/user_generator.rs` (existing — extend to consume prior)

**Prior shape:**

```rust
pub struct UserPersonaPrior {
    /// User-ID → (per-source mix, per-intraday-hour density, total volume share)
    pub users: BTreeMap<String, UserBehavior>,
    /// Number of distinct users per industry
    pub user_count_distribution: LineCountHistogram,
}

pub struct UserBehavior {
    /// Which sources this user posts (e.g. "AP clerks" mostly post KR/KZ)
    pub source_mix: BTreeMap<String, f64>,
    /// Hour-of-day posting density (24 buckets)
    pub hourly_density: [f64; 24],
    /// Day-of-week density (7 buckets)
    pub weekday_density: [f64; 7],
    /// What fraction of total volume this user accounts for
    pub volume_share: f64,
}
```

**Generator-side:**

At each JE emission, after the source code is chosen, sample a user_id from users that post that source with probability proportional to their `source_mix[source]`. Use the user's `hourly_density` + `weekday_density` to set `created_at`.

**Privacy:**
- User IDs are anonymised (corpus has them as numeric IDs already, e.g. `USER0010`). No PII concern.
- Per-user volume_share is published at industry-aggregate level. Single-client per-user data is aggregated/medianed.

**Acceptance:**
- Synthetic `created_by` distribution matches corpus distinct user count within ±30%.
- Per-user × per-source matrix correlation between synthetic and real ≥ 0.7.

### 2.6 SP4.6 — Document-type line-shape conditionals

**Why:** Each SAP document type has a typical line structure. KR (vendor invoice) typically has 2 lines: AP control credit + expense debit. RV has 2: AR control debit + revenue credit. WE (goods receipt) has 2: inventory debit + GR/IR credit. SA (manual journal voucher) has variable lines. Current `LinesPerJePrior` captures the count distribution per source but not the *role structure* per source.

**Files:**
- `crates/datasynth-fingerprint/src/extraction/behavioral_extractor.rs` (extend `extract_per_source_attribute` with new attribute `"line_role"`: account_class-of-the-debit-line and account_class-of-the-credit-line)
- `crates/datasynth-generators/src/je_generator.rs` (use the line-role structure when building JE lines)

**Approach:** Use the existing `PerSourceAttributePrior` infrastructure. Add a synthesized attribute "line_role_pattern" with values like `"A.B|R.A"` (debit AP, credit revenue) for KR documents. The generator, after picking a source, samples a line-role pattern then samples accounts from each role's pool.

**Acceptance:**
- Synthetic KR documents post to AP control + expense accounts ≥90% of the time.
- Synthetic RV documents post to AR control + revenue accounts ≥90% of the time.

### 2.7 SP4.7 — Reference format conventions

**Files:**
- `crates/datasynth-fingerprint/src/extraction/reference_extractor.rs` (new — mines reference string patterns)
- `crates/datasynth-core/src/distributions/behavioral_priors.rs` (add `ReferenceFormatPrior`)
- `crates/datasynth-generators/src/je_generator.rs` (consume at reference-emission time)

**Prior shape:**

```rust
pub struct ReferenceFormatPrior {
    /// Per-source reference format string templates.
    /// Examples: "PO-2024-{6digits}", "RE-{8digits}", "{Y}-{6digits}-{2alpha}"
    pub by_source: BTreeMap<String, Vec<ReferenceTemplate>>,
}
```

**Mining:** For each source, find the top-10 reference string patterns by frequency. Use a similar template-extraction approach as SP4.4 (digits → `{N digits}`, alpha runs → `{N alpha}`, fixed punctuation kept).

**Generator-side:** When emitting a JE reference, look up the source's templates and sample one, then fill placeholders with random digits/alpha. Falls back to existing default when source isn't in prior.

**Acceptance:**
- Synthetic reference format distribution matches corpus per-source pattern distribution within ±10% on top-3 templates.

## 3. Testing

### 3.1 Per-task unit tests

Each SP4.X gets:
- Extraction unit test (input synthetic Record vector → prior with known shape)
- Aggregation unit test where applicable (N per-client priors → aggregated prior; check pooling)
- Generator consumption unit test (priors loaded → emission shape verified)

### 3.2 Integration smoke

A `crates/datasynth-runtime/tests/v5_21_sp4_smoke.rs` integration test runs a small SP4-enabled generation (~1000 JEs) and asserts:
- `account_description` populated for ≥95% of lines
- `created_by` distinct count ≥ 10
- `header_text` distinct count ≥ 50
- Reference format follows corpus pattern for top-5 sources
- TB closing balance for cash account within ±5% of target

### 3.3 Baseline measurement

v5.21 baseline (post-SP4) re-runs the behavioral-fidelity scorer. Acceptance:
- Composite BF mean ≤ 41.5 (no regression)
- Composite BF median ≤ 16.6 (no regression)
- New: balance-sheet reconciliation gate (assets = liabilities + equity within ±1% at every period close)

## 4. Acceptance criteria summary

| Criterion | Source | Pass condition |
| --- | --- | --- |
| Behavioral fidelity preserved | v5.21 baseline | composite ≤ v5.20 |
| TB reconciliation | new gate | balance sheet equation holds; per-account drift ≤ 5% |
| CoA semantic content | manual + auto | ≥80% description overlap with real |
| Amount realism | P4 MeanGap | ≤2.0 (was 3.97) |
| Text vocabulary | distinct count | within ±20% of real |
| User personas | user count + matrix correlation | within ±30%; correlation ≥0.7 |
| Document line shape | KR/RV/WE compliance | ≥90% expected role pattern |
| Reference formats | top-3 pattern match | within ±10% |

## 5. Execution

Subagent-driven dispatch. Wave-by-wave (each wave = one task = one commit + tests):

- **Wave 1**: SP4.2 CoA semantic content + SP4.7 reference formats (smallest, parallel-safe — dispatch both at once)
- **Wave 2**: SP4.4 text vocabulary + SP4.5 user personas (medium, parallel-safe)
- **Wave 3**: SP4.3 per-(source, account_class) amount conditionals
- **Wave 4**: SP4.6 document-type line-shape conditionals
- **Wave 5**: SP4.1 TB anchoring (architectural lift, last)
- **Wave 6**: Regen bundles, v5.21 baseline, CHANGELOG entry, commit

Waves 1-2 can be dispatched in parallel. Waves 3-5 sequential because they each touch je_generator. Wave 5 is the largest (~2 weeks); waves 1-4 are each ~3-7 days.

## 6. Risks and mitigations

| Risk | Mitigation |
| --- | --- |
| TB anchoring drift-correction collides with period-close generation | Disable anchor-correction during close-period; document the boundary |
| Text templates leak private/identifiable content | Frequency-threshold ≥10; manual review of top-100 per industry before commit |
| Generated synthetic data tied too tightly to the dominant industry | Multi-industry bundles regenerable; the hard dependence is only on the existence of TB/CoA data |
| Bundle size growth | Text + user-persona priors add ~500KB-2MB per industry; acceptable. Compression-on-write already handled by .dsf |
| corpus contains PII that survives aggregation | Privacy audit pass before each new prior type lands |

## 7. Self-review

| Spec section | Task |
| --- | --- |
| §2.1 SP4.1 TB anchoring | Wave 5 |
| §2.2 SP4.2 CoA semantic content | Wave 1 |
| §2.3 SP4.3 per-(source, account_class) amount conditionals | Wave 3 |
| §2.4 SP4.4 text vocabulary | Wave 2 |
| §2.5 SP4.5 user-persona patterns | Wave 2 |
| §2.6 SP4.6 document-type line-shape | Wave 4 |
| §2.7 SP4.7 reference formats | Wave 1 |
| §3 Testing | each task |
| §4 Acceptance | per-task gates + v5.21 baseline |

All seven items covered with explicit architecture, files, acceptance criteria, and execution wave. The privacy/audit posture is preserved (frequency thresholds, aggregate-only mining). The behavioral-fidelity composite is held as a hard regression gate so SP4's content additions don't accidentally degrade the statistical fidelity that SP3 won.

---

## 8. Open questions for confirmation before kickoff

1. **Bundle naming**: introduce a new `industry_priors_corpus_v2_X.dsf` format vs extend the existing `industry_priors_X.dsf`? Recommend the latter (additive — old fields stay, new fields land as `Option<...>` with `#[serde(default)]`).

2. **Sequencing**: parallel waves 1+2 are aggressive but tractable. Or do strictly serial for safer dispatch and less context-pressure on the main session? Recommend parallel for waves 1+2 (small, independent); serial for waves 3-5.

3. **Privacy review cadence**: do we run a one-shot privacy review at SP4 end, or one per text/reference/user-persona prior as it lands? Recommend per-prior — catches issues early.

4. **TB anchoring target stiffness**: the spec sets ±5% per account, ±1% aggregate. Are those stringent enough for audit-grade or too loose? May need calibration after the v5.21 baseline data lands.
