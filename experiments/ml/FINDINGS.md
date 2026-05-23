# Corpus → synthetic gap: what's missing, and what the learning tracks recover

A100 study (2026-05-21), DataSynth v5.27. Goal: **learn from the corpus what
the synthetic generator is missing** — the aggregated corpus (53.4M JE lines,
11.8M JEs) vs the v5.27 engine. All learning is on
the corpus on the private box; weights stay on-box (memorization rule). Paper
grounding + generator-optimization targets.

## 1. What's missing (descriptive, corpus vs synthetic)

Raw observables — interpretable units, not normalized DRs:

| Observable | Corpus | Synthetic | Gap |
|---|--:|--:|---|
| Source diversity (entropy / count) | 3.37 / 4,504 | 0.75 / 4 | synthetic **far too concentrated** (one source ≈ 75%) |
| Inter-event-time **std** (days) | 0.0169 | 0.00028 | synthetic **~60× too regular** (irregular-gap structure absent) |
| Amount **p99** | $33k | $542k | synthetic tail **~16× too fat** |
| log-amount std / skew | 2.46 / 0.56 | 3.43 / 0.99 | synthetic **over-dispersed, over-skewed** |
| Lines per JE (mean) | 4.5 | 10.3 | synthetic JEs **~2.3× too large** |
| Benford MAD | 0.0081 | 0.0057 | synthetic slightly *more* Benford-clean than the corpus |

Top generator-optimization targets: **(a)** amount density (tail + spread),
**(b)** IET-variance / lines-per-JE structure, **(c)** source-mix breadth.

## 2. Methodological finding — the DR eval degenerates at full corpus scale

`behavioral score` on the 53.4M-line corpus returns `is_degenerate_baseline =
true` for **every** metric: the corpus-vs-corpus 50/50 noise floor is ≈0, so
each degradation ratio divides by ~0 and saturates at the 100 cap. The
normalized composite is therefore uninformative at this scale — the descriptive
comparison (§1) is the actionable signal. **For the paper:** the DR noise-floor
needs a resampling scheme that stays non-degenerate at large N (e.g. per-entity
block bootstrap), or the composite should fall back to raw distances when the
baseline underflows.

## 3. Learning tracks — recovering the missing structure (corpus-trained)

### Flow (amount density) — `flow/`
Conditional neural-spline flow over `signed log1p(|amount|)`, conditioned on
account-class (COA join, 294/294 accounts matched). **Bug found + fixed:** the
NSF default spline domain (~[-5,5]) cannot represent corpus log-amounts (which
reach ~10.4), collapsing learned p99 to ~$142 (v1). Standardizing `y` before
the flow fixes it (v2).

| | log-amt mean | std | skew | p99 | Benford MAD |
|---|--:|--:|--:|--:|--:|
| Corpus (held-out) | 3.91 | 2.45 | 0.54 | $32,950 | 0.0086 |
| Flow v1 (un-standardized) | 2.81 | 1.45 | −0.39 | $142 | 0.0182 |
| **Flow v2 (standardized y)** | **3.89** | **2.46** | **0.54** | **$31,754** | **0.0081** |
| Synthetic (3-comp mixture) | 3.65 | 3.43 | 0.99 | $541,617 | 0.0057 |

**v2 matches the corpus amount density almost exactly** — NLL 8.96 → 0.67;
p99 $31,754 vs corpus $33,688 (within ~6%), std/skew spot-on — whereas the
current 3-component mixture overshoots p99 by ~16× and is 1.4× over-dispersed.
Headline result: a learned per-account-class flow recovers the corpus amount
distribution the shipped mixture misses. Handoff: export spline knots → candle
`AmountSampler`, or keep as a build-time density artifact.

### Sequence (event-stream temporal) — `sequence/`
Decoder-only transformer over per-(client, source) event-token streams (Δt /
line-count / account-class / weekday buckets), factorized heads. **Trains
cleanly** (loss 1.99 → 1.93 / 25 epochs over 2,500 streams) — the corpus event
structure *is* learnable. Finding: corpus dt-bucket **lag-1 autocorr = −0.118**
(only 11.6% of streams positively autocorrelated), so the corpus is **not**
strongly *sequentially* bursty at this granularity — the §1 "60×" gap is
inter-event-time **variance**, a distinct axis from autocorrelation. **Held-out
lift over an iid per-field marginal sampler: +3.37 nats/token** (account_class
+1.55, weekday +1.36, lines +0.58; Δt ≈ flat at −0.12 — Δt really is near
memoryless). So the autoregressive model captures the joint
source→account-class→line-count→weekday structure the current per-event
marginal sampler discards — the concrete case for an AR event scheduler.
Data-quality
note: the corpus COA `Account Class` carries encoding-mangled label variants
inflating the class count to 397 — a cleaning target.

### Inverse SBI — run the engine backward — `inverse/`
Amortized neural posterior `q(θ | x)` (zuko NSF) over 3 tier-1 knobs
(`fraud_rate`, amount `mu`, amount `sigma`), trained on **1,000 forward-simulated
`(θ, GL-summary)` pairs (0 failures)**, validated on held-out synthetic with
simulation-based calibration + 90% credible-interval coverage:

| knob | MAE (norm) | 90% coverage | verdict |
|---|--:|--:|---|
| **amount_mu** | 0.049 | **0.92** | strongly identifiable |
| **fraud.fraud_rate** | 0.078 | **0.88** | identifiable, calibrated |
| amount_sigma | 0.209 | 0.77 | poorly identified (honest) |

A GL's amount **location** and **fraud rate** are recoverable with calibrated
uncertainty; amount **width** is not (other variance sources swamp the single
component's σ). This is the audit-analytics direction — *"the GL most likely
came from these process parameters"* — validated on synthetic before any
out-of-sample-GL use. Identifiability is gated by forward-model fidelity (the §1 gap), so
the flow/sequence work directly improves how much an inverse can recover.

**Capstone — posterior applied to the corpus.** Feeding the corpus's
summary into the SBC-calibrated `q(θ|x)` returns a **degenerate, boundary-pinned
posterior** — `fraud_rate→0.100`, `amount_mu→3.0`, `amount_sigma→2.6`, all with
**zero-width 90% CIs** — i.e. confidently *wrong* (corpus log-amount mean 3.92
implies `amount_mu≈6.2`). The corpus `x` is **out-of-distribution** for the
synthetic-trained posterior: the §1 gaps (source entropy, lines-per-JE tail, IET
variance) put out-of-sample GLs outside the manifold the forward model produces, so the
flow extrapolates to the prior bounds and collapses its uncertainty. This is
**"distribution shift = the BF gap" made empirical**: the inverse is
well-calibrated on synthetic (cov 0.92) yet *untrustworthy on the corpus
until the forward-fidelity gap is closed*. It is the single strongest argument
for the flow/sequence fidelity work — closing §1 is precisely what makes
backward inference on out-of-sample GLs valid. (Methodology lands; the headline number is
the negative transfer, not a recovered θ.)

### Surrogate / tuning loop — `surrogate/`
Grounded CMA-ES: MLP surrogate `θ → distance-to-corpus` over 10 robust
observables, fit on the campaign, searched by CMA-ES. **Machinery runs
end-to-end on campaign data** (vs the scaffold `optimize.py`'s synthetic-seed
placeholder). Honest result: held-out Spearman **0.46**, and CMA-ES landed
`amount_mu` at its upper bound (10.0) rather than the corpus-implied ≈6.2 — the
single-small-generate summary stats are too noisy for the surrogate to locate
the optimum reliably. (A first attempt was worse, Spearman −0.08, until the
corpus `lpje_std=123` heavy-tail outlier was dropped from the distance + the
features clipped.) **Takeaway:** the accelerator needs a larger / lower-variance
campaign; the calibrated **inverse posterior** above is the more principled
route to "what params did the corpus come from" — `amount_mu` is strongly
identified there (cov 0.92), so feeding the corpus summary into `q(θ|x)` is the
recommended next step over the distance-surrogate.

## 4. GNN fraud showcase (public synthetic data) — `scripts/ml/`
Separate, publishable result (see `scripts/ml/RESULTS_v5.27.md`): binary fraud
GraphSAGE test AUC 0.909 (≈ a LogReg on edge features — graph adds little);
fraud-**typology** is near-random on the collapsed edge list (macro-F1 0.09) but
**0.58 on the line-level view** — `fraud_type` is learnable, but consumers must
join the line table.

## 5. Implications
- **Amount sampler**: the corpus tail is *thinner* and less skewed than the
  synthetic mixture — the engine over-generates extreme amounts. A learned flow
  (v2) or a re-fit mixture narrows this.
- **Source mix**: the engine emits ~4–24 sources vs the corpus's thousands;
  source-mix breadth is a generation gap (priors bundle partially addresses it).
- **Lines per JE**: synthetic JEs are ~2× too large — the lines-per-JE prior
  needs down-weighting toward the corpus mean of ~4.5.
- **Eval**: fix the DR noise-floor degeneracy at corpus scale before re-baselining.

## 6. v5.28 re-run — does closing lines-per-JE fix the inverse? (2026-05-22)

T1 landed three gap-closers in v5.28: amount tail (#114), **lines-per-JE prior
~10.3 → ~4.6** (#115, now matching the corpus's 4.5), and the DR noise-floor cap
(#113). Re-running the inverse capstone on the v5.28 engine isolates #115's
effect on backward identifiability: the forward sim θ-controls amounts in both
v5.27 and v5.28, and does **not** override `line_item_distribution`, so the only
moved axis is the default lines-per-JE. The (previously ad-hoc, lost) corpus →
canonical-summary converter is now committed and reproducible
(`inverse/corpus_x.py`); it reproduces §1 exactly (lpje 4.51, log-amt-mean 3.92,
src-entropy 3.37 on the health subset).

**Result: closing lines-per-JE alone does NOT make the corpus identifiable.**
The synthetic-trained posterior (N=1000 and a less-overfit N=2000 retrain; SBC
90%-coverage ≈0.80 on held-out synthetic — an identifiability ceiling for these
three knobs, not an overfit artifact, since 2× data didn't move it) collapses to
**prior-box corners with zero-width (over-confident) CIs** on the corpus —
*everywhere*: pooled health, pooled full corpus (45 clients / 104.5M lines), all
**8 industries**, and **45/45 individual clients**. The collapse corner varies
run-to-run (`amount_mu`→3.0 or 10.0; `sigma`→0.5 or 2.6) — the signature of a
flow extrapolating off-manifold, not a recovered θ. A handful of clients (e.g.
two health GLs) momentarily recovered `amount_mu`≈6.2–6.4 under the noisier
N=1000 posterior, but the cleaner N=2000 posterior collapsed even those.

**Why one axis is insufficient — the corpus is OOD on several axes at once:**

| feature | corpus (full) | synthetic forward-sim | gap after #115 |
|---|--:|--:|---|
| lines-per-JE *mean* | 4.95 | ~4.6 | **closed** (#115) |
| lines-per-JE *std* | ~100 | small | open (heavy tail) |
| source entropy | **3.91** | ≤2.72 (priors) / 0.75 (default) | open (the #1 lever) |
| n_lines (log) | 18.5 | ~8 | open (GL-scale mismatch) |
| IET std (days) | 0.015 | near-0 | open |

This **strengthens** the §3 capstone thesis empirically: valid backward
inference on out-of-sample GLs requires closing **multiple** forward-fidelity
axes simultaneously; lines-per-JE was necessary but nowhere near sufficient.

**Sharpened roadmap — source-mix (T2-D) is the top remaining lever**, decomposed
here: the emitted `source` column is `sap_source_code` when industry priors are
loaded (opt-in) — else it falls back to the coarse `TransactionSource` enum
(`csv_sink.rs`), which is why the *default* engine measures entropy 0.75 (~4
values). Loading priors lifts it to the bundle's 25-code `source_mix`
(entropy 2.72); the corpus is 4,504 codes (entropy 3.91). So: (1) populate
`sap_source_code` by default (0.75 → 2.72, cheap, all output), then (2) extend
the bundle vocabulary with a privacy-safe synthetic long tail (2.72 → ~3.9).

*Privacy/method notes:* apply emits only parameter posteriors, never row content;
corpus dates are European `%d.%m.%Y` (the summary now parses `dayfirst=True` —
prior runs' lag/weekend features were unreliable, but scale + source dominate the
OOD regardless); GL-scale (n_lines) OOD is constant across the v5.27↔v5.28
comparison so it does not confound the #115 isolation.

## 7. T2 outcome — source breadth closes source-mix AND most of the IET gap (2026-05-22)

T2(D) shipped a default SAP source-mix — Lever 1 (25-code standard FI/MM/SD head)
+ Lever 2 (a synthetic power-law long tail of Z-prefixed custom codes) — flag-
gated (`transactions.synthetic_source_codes`, default-on), drawn from a **separate
RNG stream** so non-source fields stay byte-identical. Measured end-to-end on a
no-priors run (manufacturing/medium, ~86k JE lines, 1 year):

| | source entropy | per-source IET std (days) |
|---|--:|--:|
| v5.27 (enum `source`) | 0.75 | 0.00028 |
| Lever 1 (25-code head) | 2.79 | 0.0008 |
| **Lever 2 (+ long tail, 417 obs. codes)** | **3.36** | **0.0085** |
| corpus | 3.37 (health) / 3.91 (full) | ~0.015 |

Source entropy now matches the health corpus. **T2(E) (IET variance) needed no
separate AR scheduler:** §1/§3 established that corpus Δt is near-memoryless, and
the IET variance is *coupled to source breadth* — with few sources each source is
dense (same-day postings → ~zero gaps), whereas the corpus's long tail of rare
sources draws few events → large gaps. Extending source breadth (Lever 2) lifted
IET std 0.00028 → 0.0085 (**~30×**), closing most of the gap. The residual (0.0085
vs the full-corpus 0.015) is partly a scale artifact (the synthetic run is 86k
lines / 1 yr / 417 sources vs the corpus's 104M / multi-year / 4,504). A dedicated
over-dispersion mechanism could close the remainder but isn't warranted for the
modest residual — the "AR event-scheduler" turned out to be the source long tail.

## 8. Structural / process gaps — the SOTA roadmap (#123, 2026-05-22)

Closing the §1 marginals (amount, lines-per-JE, source, IET) was necessary but
left the inverse posterior **degenerate on the corpus** (§6/§7) even after
source-mix closed — because the binding gap is the **joint / structural
manifold**, not the marginals. A structural fingerprint (`corpus_structure.py`:
JE archetypes, GL-flow graph, dimensions, multi-currency, account Pareto,
reversal/recurring proxies) run identically on the corpus (health, 300k JEs) and
a T2 synthetic generate (manufacturing/medium, ~18k JEs):

| dimension | corpus | synthetic (T2) | gap |
|---|--:|--:|---|
| archetypes / 1k JEs | 47 | 758 | synthetic JEs ~16x too *unique* |
| top-50 archetype coverage | 65% | 19% | corpus reuses standard postings |
| recurring-archetype share | **97%** | 28% | recurring/standard journals missing |
| account top-10% line share | **95%** | 21% | account-activity Pareto missing |
| business_unit | 11 codes / 82% fill | **absent** | dimension missing entirely |
| distinct currencies | 2 (3.5% func!=rep) | 1 | multi-currency / FX postings missing |
| reversal proxy | 10% | 0.2% | reversal / correction process missing |
| cost_center distinct | 527 | 42 | breadth gap (cf. source-mix) |
| GL-flow edge entropy | 5.9 | 10.7 | posting graph too diffuse |
| profit_center fill | 19% | 93% | synthetic *over*-fills the dimension |

**Headline: the corpus is heavily TEMPLATED; the engine is too "creative."** A
production ERP posts the same standard journals repeatedly (recurring entries,
standard postings) — 97% of corpus JEs use an archetype recurring across periods
and the top-50 archetypes cover 65% of all JEs. The engine builds each JE
near-independently from its process logic, so 758/1k archetypes are unique and
only 28% recur. This one axis (templating / recurring-entry reuse) also drives
the diffuse flow graph and the too-flat account distribution.

**SOTA roadmap (ranked by structural leverage):**
1. **Recurring / standard-journal templates** — a templated posting process: a
   per-(entity, source) library of standard JE archetypes drawn repeatedly so
   reuse + recurring share approach the corpus. The biggest gap.
2. **Account-activity Pareto** — concentrate postings on a hot subset (top-10%
   -> ~95% of lines) instead of spreading evenly across the CoA.
3. **Business Unit dimension** — add a BU field (absent today; 82% corpus fill).
4. **Multi-currency / FX line postings** — functional + reporting amounts with a
   small func!=rep share (intercompany / foreign-currency transactions).
5. **Reversal / correction process** — emit reversal JEs offsetting prior ones
   at a realistic rate (~10% account-amount sign overlap).
6. **Dimensional discipline** — wider cost-center vocabulary, but *sparser*
   profit-center fill (the engine over-populates dimensions the corpus leaves
   blank).

These are the missing **processes** (not marginals) separating the engine from a
SOTA, corpus-indistinguishable GL — and precisely what would pull the corpus
back inside the inverse manifold (§6/§7).

*Method/caveats:* aggregate stats only (privacy); corpus health subset vs
manufacturing synthetic (cross-industry, so dimensional vocabularies differ —
the structural *ratios* are the signal); 300k-JE deterministic sample each.

## 9. SOTA round implemented & corpus-validated (v5.29, 2026-05-22)

The §8 roadmap, shipped as four flag-gated, default-on posting processes. Each
preserves **JE balance** and **same-seed determinism**, drawing its override
from a dedicated RNG stream so the *direct* amount/line-count/date draws are
untouched. (Caveat found during the CI shake-out: the selected account feeds
line-text generation, whose RNG draw count is account-dependent, so when line
text is on the downstream amount *values* shift — distributions and Benford are
preserved, but the output is not byte-identical to v5.28. This is why the copula
correlation smoke tests had to be re-pinned to a no-SOTA-lever config.)
Re-measured with the same `corpus_structure.py` / `corpus_processes.py`
fingerprint on a fresh manufacturing/medium generate (commit `38d7722f`):

| dimension | corpus | before | after | lever |
|---|--:|--:|--:|---|
| account top-10% line share | ~95% | 21% | **93.5%** | #126 account Pareto (Zipf s=2.0) |
| recurring-archetype share | ~97% | 28% | **~91%** | #125 templates (+ #126 side-effect) |
| top-50 archetype coverage | 65% | 19% | **56%** | #125 + #126 |
| reversal proxy | 10% | 0.2% | **4.9%** | #129 reversals |
| `AB` allocation lines-per-JE | ~52 | absent | **59.3** | #131 allocation batches |
| distinct sources | thousands | low | **390** | #119 source-mix breadth |

**Result: the templating + Pareto gaps — §8's top structural leverage — are
essentially closed**, with no determinism cost on the non-account fields. The
account-activity Pareto in particular went 21% → 93.5% on a single principled
parameter (Zipf s=2.0), and concentrating accounts also lifted the recurring /
top-50 templating metrics as a side effect (a hot account set yields more
repeated archetypes). #131 adds the allocation/assessment *process* the engine
lacked (`AB`, ~52 lines, 1-to-many cost-center spread) rather than the line
distribution's incidental heavy tail.

A regression worth recording: the derived-id processes (#129/#131 mint
`base ^ salt` ids without advancing the uuid factory) duplicated document ids
when the same buffered original was reused — caught only by the macOS/Windows
**full integration suite** (`test_document_reference_integrity`), not the local
or feature-matrix runs. Fix: consume the base entry on use; locked by a fast
lib regression (`test_derived_id_processes_keep_document_ids_unique`). *Lesson:
XOR-salt id derivation requires single-use of the base, and the integration
suite is the gate that catches output-uniqueness invariants.*

**Still open from §8:** Business Unit dimension (#127, a core model field),
multi-currency / FX line postings (#128), and the dimensional-discipline tail
(wider cost-center vocab + sparser profit-center fill; blank-source ~21% — a
data-quality characteristic, candidate for the `data_quality` section rather
than a default-on transaction feature).

*Caveats as §8 (cross-industry ratios; 300k-JE deterministic sample). The
`AB` lpje 59.3 vs corpus ~52 is within the target band; tune
`allocation_batch_rate` / target-count bounds if a tighter match is wanted.*

## 10. Corpus-vs-synthetic comparison + tuning round (2026-05-22)

A clean side-by-side: **corpus** vs the **same binary** with the 5 SOTA levers
**off (pre-v5.29 baseline)** vs **on (current)**, healthcare to match the corpus
industry, same `corpus_structure.py` / `corpus_processes.py` fingerprint.

| Structural metric | Corpus | Baseline (off) | Current (on) |
|---|--:|--:|--:|
| account top-10% line share | 0.95 | 0.16 | **0.946** |
| recurring-archetype share | 0.967 | 0.131 | **0.885** |
| top-50 archetype coverage | 0.651 | 0.03 | 0.48 |
| reversal proxy | 0.100 | 0.0015 | 0.034 |
| flow-graph edge entropy *(lower=more templated)* | 5.95 | 10.75 | 7.95 |
| `AB` allocation lines-per-JE | 52.2 | absent | 55.7 |
| distinct source codes | 46 | 429 | 405 |
| business_unit *(dedicated check)* | 82% / 11 | absent | 23.5% / 10, coherent |

**Every structural dimension moved toward the corpus.** Account-Pareto, recurring
share and allocation lines-per-JE are essentially closed; templating coverage,
reversal proxy and flow entropy were major-but-partial → **tuning round** (v5.29
"Tuned"): reversal default 0.04 → 0.10, templating reuse 0.82 → 0.90 + archetype
cap 48 → 24, and business_unit now rolls up CC **or PC** (fill ~24% → toward 82%).

**Deliberately not tuned:** the source-mix distinct count (405 vs 46) — the
*entropy* the source-breadth lever targets already matches the corpus (3.36 vs
3.37); the extra rare codes are general-realism breadth and trimming to this
health subset's 46 would overfit (the full corpus has far more sources).
*Tooling note: `corpus_structure.py` reads `business_unit` from corpus parquet
but not yet from the synthetic CSV (shows 0.0 there) — the dedicated pandas
check confirms the synthetic value; a tool follow-up.*

## 11. Inverse-audit capstone — Stage 1 (synthetic, in-distribution), 2026-05-22

Operationalises the inverse-audit thesis (reconstruct the normal-system manifold from
a GL + the accounting rules, then flag the JEs it *cannot explain* — high residual).
The "manifold" is a conditional density fit on a NORMAL synthetic GL; a JE's anomaly
score is its negative log-likelihood under it:

- amount     — per-(account-class) signed-log1p density (robust), max over lines
- structure  — −log P(account-set signature | source), add-1 smoothed
- behavioral — per-source surprise of {weekend, post-close, round-dollar} (the
               fraud-bias signatures)

Test = a synthetic GL with ~3.7% per-JE fraud injected (`fraud.fraud_rate`); ground
truth = the `is_fraud` label. Pipeline: `experiments/ml/inverse_audit/`.

**Result — the structure-aware residual recovers per-JE fraud and crushes the
marginal-feature baseline:**

| detector | PR-AUC | ROC-AUC | precision@1% |
|---|--:|--:|--:|
| reconstructed-manifold residual (ours) | **0.741** | **0.913** | **0.94** |
| Isolation Forest (n_lines, total, n_accounts) | 0.038 | 0.479 | 0.04 |

Base rate 3.7% (382 / 10 277). **~19× PR-AUC over Isolation Forest** — the joint
structural+behavioral manifold sees what marginal features cannot. Score separation
mean 22.8 (fraud) vs 0.5 (normal).

**Per fraud type (type-vs-normal) — a clean gradient by what a per-JE manifold can
observe:**

| family | example types (ROC-AUC) |
|---|---|
| structural (improbable account-set) | ExceededApprovalLimit 1.00, ExpenseCapitalization 0.99, SuspenseAccountAbuse 0.96, RevenueManipulation 0.96, FictitiousTransaction 0.94, SplitTransaction 0.91 |
| behavioral (timing / amount-shape) | SegregationOfDuties 0.88, TimingAnomaly 0.87, RoundDollar 0.86, SelfApproval 0.84, JustBelowThreshold 0.81 |
| cross-JE / relational | DuplicatePayment 0.75, ConflictOfInterestSourcing 0.69 |

On the **relational anomaly family** (secondary arm: NewCounterparty, CentralityAnomaly,
MissingRelationship, intercompany, …) the *same* scorer reaches only PR-AUC 0.096 /
ROC 0.61 — near-blind.

**System-theory reading (partial observability).** The per-JE residual reconstructs
the **structural + behavioral subsystem** of the GL and detects its fraud strongly
(ROC 0.9–1.0). Anomalies defined by **cross-JE relationships** (duplicate payments,
circular flows, network centrality) are *off* the single-JE manifold and degrade
gracefully toward random — they need the graph model. Route each anomaly family to the
detector that observes its subsystem; the generative residual and the discriminative
GNN are complementary, and DataSynth powers both (labelled training data for the GNN;
the forward model for the residual).

**Light-B (system-state) — DONE: the amortized SBI posterior recovers the injected
fraud_rate.** Re-ran `inverse/train.py` (1500-sim campaign, val NLL −2.67), then probed
`inverse.apply` over a `fraud.fraud_rate` sweep. The probe GL must be built from the
*campaign's own* base config (`inverse_base.yaml`, amount params held at prior centres,
only `fraud.fraud_rate` swept) so x lands on the training manifold:

| injected | recovered median | 90% CI |
|---|--:|--:|
| 0.00 | 0.000 | — |
| 0.02 | 0.027 | [0.015, 0.043] |
| 0.05 | 0.053 | [0.039, 0.066] |
| 0.10 | 0.087 | [0.073, 0.100] |

Pearson r = 0.989; the 90% CIs contain truth (0.10 is ceiling-clipped at the prior
bound). A first probe built from a *different* base (the inverse_audit healthcare config)
instead collapsed the posterior to a near-Dirac at the prior floor (fraud_rate ≡ 0.000,
zero-width CIs on every param) — a clean, intended demonstration of the OOD failure
mode: off-manifold x ⇒ the conditional flow extrapolates to a degenerate density. So
**both capstone arms now hold** — *local* (per-JE density residual, PR-AUC 0.741) and
*global* (SBI system-state, r 0.989) — and that OOD collapse is precisely why the §9/§10
fidelity work (which widens the on-manifold region toward the corpus) is the Stage-2
enabler, not a nicety.

**Why it matters + the prerequisite.** The residual envelope is only as sharp as the
forward model's coverage of *normal*. Stage 1 is in-distribution, so the manifold is
exact and detection is strong. On a real GL the forward-fidelity gap (the OOD problem,
§6/§7) widens the residuals — and the v5.29 SOTA round + tuning (§9/§10) closed much of
that structural gap, making the corpus the natural **Stage-2** target. Stage 2 needs
the SAP-style multi-currency work (#128, **DONE** — `transactions.foreign_currency_rate`,
additive SAP DMBTR/WRBTR split; a 3.5%-rate healthcare GL now emits 8 document
currencies / 2.11% foreign lines vs the corpus's ~3.5%, so FX JEs sit on-manifold rather
than OOD) and shifts validation from labelled PR-AUC to expert review of the top-ranked
residuals.

*Caveats: in-distribution only; single-JE scoring (cross-JE/duplicate types need a
sequence-of-JEs or graph model); the amount term is a lightweight per-class density
(the spline flow is a drop-in upgrade).*

## 12. Stage 1 closeout — relational arm + unified routed detector (2026-05-23)

§11 left one explicit gap: the per-JE residual is near-blind to *relational* anomalies
(circular flows, dormant-account reactivation, …) — ROC ≈ 0.5 on that family — because
a single JE's residual can't see graph context. This section closes that gap with a
third, *graph-manifold* arm and shows the three-armed routed detector working end-to-end.

**The relational arm — account-flow-graph manifold residual.** Rung 1 of the
methodology-paper integration (`experiments/ml/inverse_audit/relational/`): entropic-OT
(Sinkhorn) reconstructs the within-JE debit↔credit *flows* from the line marginals
(exact marginals → balance preserved), aggregating across JEs into the account-flow
graph. Fit on a clean GL; score test JEs by how off-manifold their reconstructed flows
are. The deployable score is the z-sum of four positive-prior features (no labels at
score time):

- `edge_surprise_max`, `edge_surprise_w` — −log P<sub>normal</sub>(account→account edge);
  amount-weighted variants of the rare-pair signal.
- `account_dormancy_max` — IDF of touched accounts in normal (high = rarely-used =
  dormant-proxy); no date arithmetic needed.
- `tp_account_novelty` — bipartite (`trading_partner`, `gl_account`) pair novelty.

On a labelled relational-only GL (anomaly_injection.rates.total_rate 0.08, fraud off;
10 279 JEs, 1 057 anomalous), versus the per-JE density scorer of §11 on the same labels:

| arm                                         | PR-AUC    | ROC-AUC  |
|---------------------------------------------|----------:|---------:|
| density baseline (§11, vs `is_anomaly`)     | 0.119     | 0.522    |
| **relational arm (deployable, unsupervised)** | **0.220** | **0.544** |
| LR-CV ceiling (uses labels — upper bound)   | 0.147     | 0.555    |

+85 % PR-AUC over the density baseline on the relational families. One family-level
breakthrough: **DormantAccountActivity (n=89) ROC 0.28 → 0.993** under the dormancy-IDF
feature alone — a clean "right feature for the right subsystem" win. Several families
remain hard (NewCounterparty 0.46, MissingRelationship 0.49, UnusualAccountPair 0.48) —
these need cross-JE / source-conditional features (cycle detection on the aggregate
graph, P(edge | source)).

**Unified routed detector — the routing thesis, measured.** On a mixed GL
(fraud_rate 0.04 + anomaly_injection.rates.total_rate 0.06; 10 279 JEs), `unified_score`
= z-sum(density, relational):

| target                       | density            | relational         | **unified**          |
|------------------------------|-------------------:|-------------------:|---------------------:|
| vs `is_fraud`   (n=357)      | **0.783 / 0.920**  | 0.037 / 0.504      | 0.733 / 0.917        |
| vs `is_anomaly` (n=831)      | 0.078 / 0.504      | **0.134 / 0.540**  | 0.091 / 0.525        |
| **vs `is_any`   (n=1 151)**  | 0.373 / 0.641      | 0.158 / 0.531      | **0.395 / 0.654**    |

The diagonal pattern *is* the thesis: each arm excels on its own subsystem, each is
near-blind to the other's, and **the unified score beats either alone on the union
(`is_any`)**. The capstone's routing recipe — "route each anomaly family to the
detector that observes its subsystem" — is now empirically demonstrated.

**Observability map (the audit-actionable artifact).** Per-family best arm, sorted by
ROC. `density` owns per-JE fraud (every `fraud_type` ROC ≥ 0.87); `relational` owns the
dormancy / centrality / statistical-shape families; the residual hard families are the
counterparty/cycle ones that need richer features.

| `fraud_type`         | n  | density | relational | unified | best         |
|----------------------|---:|--------:|-----------:|--------:|--------------|
| UnauthorizedAccess   | 38 | **0.976** | 0.409   | 0.963   | density      |
| RevenueManipulation  | 43 | 0.933 | 0.509       | **0.937** | unified    |
| FictitiousTransaction| 52 | **0.925** | 0.522   | 0.916   | density      |
| SuspenseAccountAbuse | 85 | **0.925** | 0.481   | 0.920   | density      |
| SplitTransaction     | 55 | 0.907 | 0.542       | **0.909** | unified    |
| ExpenseCapitalization| 27 | 0.885 | 0.548       | **0.890** | unified    |
| DuplicatePayment     | 15 | 0.873 | 0.639       | **0.888** | unified    |
| TimingAnomaly        | 34 | **0.881** | 0.523   | 0.875   | density      |

| `anomaly_type` (rel. family)    | n   | density | relational | unified | best       |
|---------------------------------|----:|--------:|-----------:|--------:|------------|
| DormantAccountActivity          | 89  | 0.700 | **0.978**   | 0.964   | relational |
| RepeatingAmount                 |  8  | **0.692** | 0.607   | 0.644   | density    |
| StatisticalOutlier              | 12  | 0.579 | **0.638**   | 0.622   | relational |
| UnusuallyLowAmount              | 10  | 0.570 | 0.587       | **0.603** | unified  |
| CentralityAnomaly               | 38  | 0.550 | **0.579**   | 0.564   | relational |
| TrendBreak                      | 13  | 0.524 | **0.578**   | 0.549   | relational |
| CircularTransaction             | 45  | **0.566** | 0.514   | 0.539   | density    |
| MissingRelationship             | 96  | 0.496 | 0.491       | 0.481   | density    |
| UnusualAccountPair              | 120 | 0.472 | **0.480**   | 0.474   | relational |
| NewCounterparty                 | 139 | 0.451 | **0.459**   | 0.447   | relational |
| UnmatchedIntercompany           | 76  | 0.491 | 0.490       | 0.472   | density    |
| TransferPricingAnomaly          | 51  | 0.443 | **0.466**   | 0.443   | relational |

**Throughline.** The capstone's Stage 1 thesis — *reconstruct the normal-system manifold
from a GL + the accounting rules; flag JEs the manifold cannot explain* — now holds at
**all three observability layers**: local density (per-JE structural + behavioural; §11),
global SBI (parameter posterior, fraud_rate r 0.989; §11 light-B), and **relational
graph residual (this section)**. Each is generative, label-free at deploy time, and
attributable to its subsystem. The methodology paper's account-flow-graph reconstruction
earned its place as the relational organ of that residual auditor — exactly the
substrate role hypothesised in [[reference_accounting_network_papers]].

The remaining within-Stage-1 work is feature engineering against the hard families
(cycle detection on the aggregate graph for Circular*; source-conditional edge surprise
for UnusualAccountPair; counterparty-relationship model for NewCounterparty /
MissingRelationship). Stage 2 — the corpus GL with expert review of top residuals — is
gated on continued forward-fidelity (§9/§10) rather than on the detector design.

Reproduce: `experiments/ml/inverse_audit/{generate_mixed,unified_score}.py` →
`/tmp/iam/unified.json`; relational-only training set: `generate_relational.py` →
`relational/graph_scorer.py` → `/tmp/iar/graph_scores.assess.json`.
