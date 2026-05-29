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
exact and detection is strong. On a corpus GL the forward-fidelity gap (the OOD problem,
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

Reproduce: `python -m inverse_audit.run_capstone` runs the full pipeline (mixed-GL
generation → density + relational scoring → unified + observability + graph export) and
writes `{root}/unified.json` plus a substrate-ingestable graph JSON
(`{root}/account_flow_graph.json`, node/edge/per-JE schema, decoupled — no DataSynth →
external dependency). On the mixed GL: **362 accounts / 20 878 directed edges / 42 nodes
in test-only cycles / 4 622 new-in-test edges** — these are the audit hot list a graph DB
or living-graph substrate can route to scored JEs.

## 13. Stage 2 — corpus residual fingerprints, per industry (2026-05-23)

Stage 1's manifold residual transfers cleanly to corpus data once the canonical column
map is in place. `inverse_audit.corpus_runner` ingests a corpus parquet → canonicalises
to the synthetic schema (corpus column names → DataSynth canonical; signed
`Functional Amount` split into debit/credit) → fits the relational manifold
**half-split** (deterministic shuffle by `document_id`, fit on first half, score the
second) so the new-in-test signals (`cycle_novelty`, `tp_account_novelty`) remain
meaningful. Privacy: corpus paths/clients never appear in committed artifacts; the
batch runner uses SHA-tagged output dirs and emits aggregate statistics only.

**Cross-industry sweep** — 6 industries, smallest GL parquet per industry,
half-split deterministic seed, total wall-clock ~20 min on a CPU box:

| industry | tag | JEs (test half) | edges (normal) | rs p50 | rs p99 | rs max | dorm p99 | cyc max |
|---|---|--:|--:|--:|--:|--:|--:|--:|
| Health                          | `5a491735` |   2,151 |  5,248 |  1.45 | 10.37 | 14.66 | 8.68 | 5 |
| Professional Firms & Services   | `b59326eb` |   9,325 |  2,647 |  0.18 | 11.66 | 19.52 | 8.22 | 4 |
| Technology                      | `8809d54d` |  26,423 |  1,692 |  0.52 | 17.16 | 32.82 | 8.73 | 2 |
| Life Sciences                   | `6cfad1f4` | 141,712 |  1,542 | −0.73 | 33.71 | 82.17 | 9.11 | 4 |
| Power & Utilities               | `9fe8cd50` | 124,936 | 11,749 |  0.36 | 12.43 | 26.89 | 9.95 | 2 |
| Government and Public Sector    | `16966e5f` | 176,963 |  2,233 |  1.75 | 18.02 | 38.89 | 7.94 | 3 |

(Pharmaceutical's smallest parquet lacked the `Functional Amount` column → registry-like;
runner now raises a clear ValueError + recommends a per-industry rescan. Hospitality &
Leisure was excluded — its smallest file is 162 MB and would dominate the sweep budget.)

**Observations.**

- **The residual approach generalises**: every industry has a heavy-tailed
  `relational_score` distribution (p99 / max well above p50 in every row). The hot list
  is non-empty and well-separated for all six.
- **Dormancy is the *most stable* signal across industries** — p99 `account_dormancy_max`
  sits in a tight 7.9–9.95 band even as p99/max of the overall score range by 3.3×
  (10.4 → 33.7). The IDF dormancy probe behaves as a near-universal feature.
- **Tail magnitude tracks manifold sparsity, not data size.** Life Sciences (141k JEs,
  1542 edges → ~92 JEs/edge) shows the most extreme tail (max 82); Power & Utilities
  (similar JE count, but 11 749 edges → 11 JEs/edge) shows max 26.9. When the
  reconstructed graph has few edges, more test edges are unseen → `edge_surprise`
  saturates at the cap (`_LOG_EPS=30`) for many lines → the z-summed score inflates.
  An audit interpretation reads this as "this client's accounting topology is
  concentrated; small numbers of edges carry most of the value" — which is itself a
  fingerprint, but the inflated tail should be interpreted with that caveat.
- **`cycle_novelty` fires modestly everywhere (max 2–5)** — the half-split brings the
  feature alive but cycles in production GLs are rare relative to the total JE count.
- **`p50` shifts vary**: Government and Health have positive median residual (entire
  distribution shifted right vs the half-fitted reference) → many test-half JEs touch
  edges/accounts that the normal-half didn't represent well. The corpus shape is uneven
  by JE-time (likely period or business-cycle drift) — a half-split by JE_id catches it.

These are corpus-wide *aggregate* signatures only. The audit-actionable next step is
expert review of the **top-1% per-industry JE ID lists** (written by `corpus_runner`)
against the original GLs — done by an auditor, not the model.

**Cap-artifact check — what dominates the heavy tails?** A `_LOG_EPS=30` cap on
`-log P(edge)` ensures finite scores when an edge is unseen in the normal half; the
Life Sciences max-82 tail raised the worry that sparse-manifold cases were just
cap-firing on `edge_surprise`. Decomposing the top-1% per industry — what fraction of
top JEs sit at the cap, and how many features (of 6) are simultaneously above the
non-top p90:

| industry | top@cap (`edge_surprise_max`) | features active per top JE |
|---|--:|--:|
| Life Sciences                  | 0.8 % | **4.08** |
| Power & Utilities              | 1.0 % | 4.02 |
| Government and Public Sector   | 1.0 % | 3.03 |
| Technology                     | 0.0 % | 3.73 |
| Professional Firms & Services  | 10.6 % | 3.66 |
| Health                         |  9.1 % | 3.32 |

The cap-artifact hypothesis is **rejected for the large-sample industries**: Life
Sciences' top-1% activates **4+ features simultaneously** on average — a genuine
multi-feature anomaly. The cap *does* fire more in the small-sample files (Health,
Professional Firms — single-digit thousand JEs), where the manifold has too few edges
to discriminate cleanly. So the practical guidance is *not* "lift `_LOG_EPS`" but
*"distrust the score on tiny GLs"* — for files <~5 k JEs, treat the top-1% as a
suggested-look list rather than ranked by raw score.

**Intra-industry variance — Health (5 stratified-by-size files of 20 in registry).**
Picking the smallest, 25 %, 50 %, 75 %, and largest Health parquets <30 MB and running
the half-split scorer on each. The story is **mostly stable + one extreme outlier whose
mechanism turned out to be a numerical degeneracy in the z-standardiser**:

| tag | size | JEs (test) | edges | nodes | rs p50 | rs p99 | rs max | dorm p99 | cyc max |
|---|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| `5a491735` |  1.2 MB |   2 151 |  5 248 | 280 |  1.45 | 10.37 | 14.66 | 8.68 | 5 |
| `4fe1a883` |  4.0 MB |  56 655 |  1 109 | 152 |  0.00 | 17.38 | 35.44 | 8.14 | 2 |
| `65aff36e` |  5.9 MB |   6 173 |  6 476 | 312 |  0.74 | 10.45 | 17.10 | 9.30 | 4 |
| `a6c1e115` |  8.1 MB |  36 070 |    447 | 123 | −0.67 | **37.22** | **43.00** | 8.67 | 1 |
| `d15bbf6a` | 29.4 MB | 173 936 | 13 124 | 550 |  0.34 | 24.46 | 42.26 | 9.38 | 6 |

(Numbers shown are *post* the z-clip fix described below. Without it, `a6c1e115`
showed p99 = 1 284, max = 1 972 — three orders of magnitude beyond its peers.)

**Diagnosis of the outlier.** `a6c1e115` has an unusually concentrated normal half:
its `source_cond_edge_surprise_max` feature has median 2.064 and **MAD = 0.00881**
(eight thousandths). The client's source-conditional edge usage is so regimented that
the prior on `P(edge | source)` is razor-tight; off-pattern test JEs then produce
z = (test − median) / MAD ≈ 1 700 on that single feature, dwarfing everything else in
the score sum. It's *not* a true cap-firing event (0 % of top-1 % JEs sit at the
`_LOG_EPS=30` cap) — purely a divide-by-tiny-MAD numerical pathology.

**Fix — clip per-feature z at ±10.** A small, rank-preserving guard in
`relational/graph_scorer.py::z_of`: after dividing by MAD, clip to [−10, 10] so no
single feature's contribution dominates by orders of magnitude. Verified:

- Outlier `a6c1e115`: max 1 972 → **43** (in band with peers); top-1 % JE ranking
  preserved — *who* the anomalies are doesn't change, only the score's numerical scale.
- 3 small Health files (`5a491735`, `4fe1a883`, `65aff36e`) re-run: clip is a
  **no-op** (post-fix == pre-fix to 0.01). Their MADs were healthy.
- §12 synthetic capstone result (mixed-GL unified vs `is_any`): pre-clip 0.395/0.654 →
  post-clip 0.398/0.657 — within ordinary re-run jitter; the clip is rank-preserving
  on healthy features, so PR-AUC/ROC are unaffected.

**Intra-industry takeaway.** Within one vertical (Health), the residual fingerprint is
**substantially client-specific**: even after the fix, the post-fix `rs_max` ranges
14.7 → 43 (~3×) across same-vertical clients with similar size class. The manifold
features capture client-typical accounting structure more than industry-typical;
expert review can't lean on an "industry average tail" — the threshold has to be set
per-client. Dormancy stays the most stable cross-row signal (p99 8.1 – 9.4 across
all 5 Health files, same band as the cross-industry sweep).

## 14. Round 0 — corpus-vs-synthetic realism gap (2026-05-23)

The Stage 2 corpus work revealed five realism levers that would *both* (a) make
synthetic GLs better reflect production and (b) tighten the inverse-audit normal
manifold (sharper residuals, less OOD collapse on corpus probes). Round 0 quantifies
where each lever sits today, before the engine rounds. Measured against the canonical
synthetic mixed GL (`ia_canonical_v5/test`, 10 277 JEs / 23.6 MB) and 6 sample corpus
parquets (one per industry, 9 576 – 353 925 JEs, 1.2 – 24.5 MB):

| metric | corpus median | synth | corpus / synth | lever |
|---|--:|--:|--:|---|
| **edges / je** (manifold density) | 0.008 | 0.062 | **0.12×** | **SOTA-9** — synth is 8.75× too diffuse |
| **jes / edge** | 140 | 16 | 8.75× | mirror of above |
| **top-10 % edge concentration** | 0.93 | 0.80 | 1.16× | corpus's hot-account-pair concentration |
| **src-cond entropy median** | 0.684 | 0.966 | 0.71× | **SOTA-8** — synth too uniform per source |
| **src-cond entropy p10 (tightest)** | 0.47 | 0.79 | 0.60× | corpus has tight sources synth never reaches |
| **accts / source median** | 23.5 | 5 | 4.70× | synth has too *few* accounts per source (then uses them uniformly) |
| **tp_set_size** | 10.5 | 35 | **0.30×** | **SOTA-11** — synth has 3.3× too many trading partners |
| **lines / je p99** | 18 | 66 | **0.28×** | **SOTA-10 redirected** — synth's *mid-tail* is too fat |
| **lines / je p99.9** | 99 | 98 | 1.01× | the extreme tail already matches |
| **lines / je max** | 924 | 8 804 | **0.10×** | synth produces occasional 8 800-line monster JEs (cap the outlier) |

Five concrete findings + corrections that fed back into the SOTA backlog:

1. **SOTA-8 (source-conditional Dirichlet) is correctly the #1 lever.** Synth is too
   uniform (entropy 0.97 vs corpus 0.68) *and* has too few accounts per source (5 vs
   23.5). The lever needs **both**: a larger per-source account pool *and* a
   concentrated Dirichlet (low α). Not just one.
2. **SOTA-9 (manifold density) has a stronger gap than I sketched.** Synth is 8.75×
   too diffuse on `edges/je`; the corpus runs 140 JEs/edge while synth only 16.
3. **SOTA-10's original direction was wrong.** Synth's *extreme* tail (p99.9 = 98)
   already matches corpus (99). What synth gets wrong is the *mid-tail* (p99 = 66 vs
   corpus 18 — 3.6× too high) and an occasional **monster outlier** (max = 8 804 vs
   corpus 924). #131's allocation generator probably over-fires; the fix is to *tame*
   the mid-tail and cap the extreme, not add more big JEs.
4. **SOTA-11 (TP pool size) is well-justified.** Synth has 3.3× too many TPs; corpus
   includes single-counterparty consolidated entities that synth can't currently
   express.
5. **Edge concentration matches direction but trails.** Corpus's top-10% of edges
   carry 93 % of flow; synth's only 80 %. Tightening manifold density (SOTA-9) will
   close this as a side-effect.

Stage 2 doesn't need anything *more* from Round 0 — every gap above is now numerically
quantified and each subsequent engine round will close part of it (re-running this
script after each is the validation pattern).

## 15. SOTA-N realism round — scorecard + central-abstraction motivation (2026-05-23)

Five planned SOTA-N levers, all landed or with documented blockers. Two delivered
measurable corpus-gap closure end-to-end; two hit the same multi-generator coverage
blocker; one shipped as a post-process module that sidesteps the blocker by design.

**Per-lever scorecard:**

| #   | lever                                  | outcome  | what moved                                                                |
|-----|----------------------------------------|----------|----------------------------------------------------------------------------|
| 8   | source-conditional Dirichlet sampler   | blocked  | sampler + integration ship; can't reach doc-flow / allocation / period-close generators → #141 |
| 9   | archetype reuse @ 0.97                 | **shipped** | edges/je 0.062 → 0.054 (−15%); lines/je max 8 804 → 2 133 (4× reduction) |
| 10  | lines_per_je_cap = 100                 | **shipped** | mean 5.88 → **4.30** ≈ corpus 4.27; p99.9 → 98 ≈ corpus 99; max → 104; accts/source p99 → 100.8 |
| 11  | TP pool size + consolidated preset     | blocked  | master_data.vendor.count doesn't reach doc-flow generators → #142          |
| 12  | source-conditional rarity injector     | **shipped** (module) | post-process tagger + 4 tests; ~30 LOC orchestrator wire-up remaining |

**Round 0 corpus-vs-synth movement** (baseline → after-shipped-levers, all values
on the same `corpus_vs_synth_gap.py` config):

| metric                       | corpus | baseline | after | corpus / after | status                  |
|------------------------------|-------:|---------:|------:|---------------:|-------------------------|
| `lines/je mean`              |  4.27  | 5.88     | 4.30  | **0.99×**      | corpus-match (within 1%) |
| `lines/je p99.9`             |  99    | 98       | 98    | **1.01×**      | corpus-match            |
| `lines/je max`               |  924   | 8 804    | 104   | 8.88×          | overcorrected (cap fires) |
| `accts/source p99`           |  147   | 121      | 100.8 | **1.46×**      | best yet, closer than corpus baseline |
| `edges/je` (manifold density)| 0.008  | 0.062    | 0.054 | 0.14×          | still 7× off (SOTA-9 only ~15%) |
| `lines/je p99`               |  18    | 66       | 62    | 0.30×          | still 3.4× off (mid-tail) |
| `tp_set_size`                | 10.5   | 35       | 35    | 0.30×          | unmoved (SOTA-11 blocked) |
| `src-cond entropy median`    | 0.68   | 0.97     | 0.97  | 0.71×          | unmoved (SOTA-8 blocked) |
| `accts/source median`        | 23.5   | 5        | 5     | 4.70×          | unmoved (SOTA-8 blocked) |

**The blocker pattern.** Three of the five planned levers (SOTA-8, SOTA-11, the
`edges/je` long tail) ran into the same architectural issue: the synthetic engine has
**many independent generator modules** — `je_generator` (the main path), document-flow
generators (p2p, o2c), allocation/balance/period-close, subledger, intercompany. Each
has its own account / TP / line-count selection logic. A config knob added to one
generator is bypassed by all the others. Round 0 directly confirmed this: small
`master_data.vendor.count` doesn't move `tp_set_size`; SOTA-8 precedence over SP3/SP4
priors doesn't move `src-cond entropy`; manifold density only partially closes because
allocation/document-flow JEs feed their own edges.

**The deliberate counter-example: SOTA-12.** Designed up-front as a **post-process**
over the generated JE batch — single integration point, covers every JE regardless
of producer. Module shipped + tested + clippy clean; ~30 LOC orchestrator wire-up
remaining. Confirms the architectural lesson by inversion: the lever that *doesn't*
need to be threaded through all generators ships cleanly.

**Forward path — central concentration abstraction.** Both `edges/je` (synth 7× too
diffuse) and `tp_set_size` (synth 3.3× too high) are *distributional concentration*
problems. Closing them with the per-lever pattern would need SOTA-8.1 + SOTA-11.1 +
likely SOTA-9.1 + future similar refactors. Each one touches every generator. The
honest read of the round is that the architectural cost is now greater than the
single-time cost of building the **shared concentration abstraction** the next round
is designed around (spec: `docs/superpowers/specs/2026-05-23-central-abstraction-
proposal.md`).

## 16. Tier A1 — re-measuring the OOD gap after ConcentrationPass (2026-05-29)

The #143 ConcentrationPass central abstraction (source-conditional rarity, trading-partner
pool, account-pair substitution vs a corpus PMF, source blanking, consolidation outlier)
landed on `main` after the §6 "closing lines-per-JE alone doesn't make the corpus
identifiable" result. Tier A1 asks: does the wider manifold now contain the corpus?

Setup (A10 VM): two SBI campaigns sharing one θ prior (`fraud.fraud_rate`, `amount_mu`,
`amount_sigma`), 1500 sims each, identical except `concentration.enabled` — OFF vs ON
(all six passes; `account_pair_substitution` against a 584-source / 66k-pair corpus PMF
built by the vectorized `emit_pmf_fast.py`). Train the amortized posterior on each, apply
to the pooled-corpus 29-dim summary x (45 clients / ~21M JEs) + a training-free
Mahalanobis probe of corpus-x → synth-cloud.

**Result — the corpus posterior stays a degenerate Dirac under BOTH arms** (collapse to
prior corners, zero-width CIs): fraud_rate 0.000, amount_mu 3.000, amount_sigma 2.600 (OFF)
/ 2.594 (ON). ConcentrationPass does not move the corpus onto the SBI manifold.

**Decomposition — where the OOD lives:**
- Full-vector Mahalanobis (OFF 14383 / ON 15447) is dominated by a **posting-lag artifact**:
  corpus lag_std ≈ 8353 days, lag_mean ≈ −1620 days (z ≈ 1e4) vs synth ~1.4 / 0.1 —
  sentinel/placeholder effective-dates or genuine multi-year accrual gaps. Concentration-
  irrelevant; clip/exclude for any SBI-on-corpus probe (data semantics, not a fidelity lever).
- On the **concentration-relevant subspace** (source/gl/lpje) ON is still not closer
  (maha 86 → 108), but per-feature it is informative:
  - `account_pair_substitution` moved GL-account structure toward corpus — gl_n_log
    4.80→5.62, gl_top5_share 0.564→0.490, gl_entropy 3.16→3.71 (all correct direction).
  - `source_blanking` pushed source entropy the WRONG way (2.85→2.79; corpus 3.91 is
    higher) by concentrating mass on the blank category.
  - `consolidation_outlier` @0.001 was far too rare to touch corpus lpje_std=99.6 (synth 35).
- Irreducible gaps: **scale** (gl_n_log ⇒ ~122 synth accounts vs ~15,600 corpus; line-count
  tail) and **data semantics** (lag) — neither addressed by #143.
- Both flows overfit (val_nll diverged to +5.8/+3.8; the saved model is the early-epoch
  best), but the corpus collapse is an OOD-input effect independent of that.

**Conclusion.** The global SBI posterior is the wrong tool at corpus scale: an amortized
posterior trained on small/feasible campaign GLs collapses on scale + semantics when
applied to enterprise GLs (15k accounts, millions of lines), before structural fidelity
registers. This is precisely why the **relational fit-on-self residual (Stage 2, §13)
already works** — it fits on the corpus's own scale and sidesteps the SBI OOD entirely.
Tier-A verdict: stop investing in the global-SBI arm for corpus use; the label-free
relational residual + detector depth (Tier B) is the productive path. Aggregate artifacts
(`ood_probe.json`, `recovered_{off,on}.json`) archived at
`~/DEV/local-artifacts/inverse_audit_tierA/`.

## 17. Tier A2 — does corpus-realism (ConcentrationPass) preserve detection? (2026-05-29)

A1 showed the global SBI arm can't reach corpus scale. A2 asks the complementary question
for the *detector*: if we sharpen the synthetic normal manifold toward corpus realism (the
six #143 passes), does the unified routed detector's labelled-anomaly PR-AUC hold? Same
labelled mixed GL (healthcare/medium, fraud_rate 0.04 + anomaly 0.06, seed 7) generated with
concentration OFF (= §12 baseline) vs ON, scored by the three-arm unified detector. OFF
reproduces §12 (vs is_any unified 0.375/0.655; vs is_fraud density 0.697/0.912).

| target | arm | OFF (PR-AUC/ROC) | ON (PR-AUC/ROC) |
|---|---|---|---|
| is_fraud   | density    | 0.697 / 0.912 | **0.697 / 0.907** |
| is_fraud   | unified    | 0.668 / 0.916 | 0.688 / 0.907 |
| is_anomaly | relational | 0.137 / 0.544 | **0.102 / 0.532** |
| is_any     | unified    | 0.375 / 0.655 | **0.343 / 0.633** |

**Findings:**
- **Per-JE fraud detection (density) is robust to corpus realism** — PR-AUC identical (0.697),
  ROC flat (0.912→0.907). Sharpening the manifold toward corpus marginals does not degrade
  the strongest arm; realism ≠ noise for the density residual.
- **The relational arm degrades under ConcentrationPass** — is_anomaly relational 0.137→0.102,
  is_any unified 0.375→0.343 — and the drop is *understated* because the ON GL has more
  labelled positives (is_any n_pos 1151→1238, which raises the PR-AUC base rate). Mechanism:
  the passes inject structure the account-flow residual reads as unusual —
  `account_pair_substitution` writes out-of-CoA corpus accounts (165-account WARN),
  `consolidation_outlier` adds bridge-account JEs — raising the residual floor and diluting
  separation from the injected relational anomalies.

**Conclusion + Tier-A synthesis.** Concentration realism is not a free lunch for the
relational detector: the levers that improve marginal fidelity (A1: gl features toward corpus)
bolt structural oddities onto an otherwise-clean base, which the relational residual conflates
with anomalies. This aligns with A1: **for the relational detector, fit-on-self on the
corpus (Stage 2, §13) is the right deployment** — the corpus carries these structures natively
and consistently, so the residual calibrates against them instead of treating bolted-on
synthetic versions as anomalies. Tier A overall: (i) the global SBI posterior is the wrong
tool at corpus scale; (ii) concentration-realism preserves per-JE fraud but degrades the
relational arm ⇒ invest in the **label-free relational fit-on-self residual + Tier-B detector
depth** (close the hard families, break the z-sum plateau), validated on the corpus directly,
not on synthetic-with-concentration. Artifacts at `~/DEV/local-artifacts/inverse_audit_tierA/`.

## 18. Tier B — closing the hard relational families + breaking the z-sum plateau (2026-05-29)

§12 left the relational arm plateaued: the unsupervised z-sum reaches PR-AUC ~0.21 /
ROC ~0.55, with the hard families (NewCounterparty, MissingRelationship, UnusualAccountPair,
Circular*, TransferPricing) stuck near random. Tier B attacks this two ways.

**(a) Hand-engineered features — NULL.** Added `centrality_delta_max` (test-graph PageRank −
normal PageRank; for CentralityAnomaly) and `tp_account_source_novelty` ((tp,account,source)
triple novelty; for NewCounterparty/MissingRelationship). Both failed: the triple novelty
NEVER fires (the injector reuses existing tp/account/source combos in a single-company GL →
test triples ⊆ normal), and `centrality_delta_max` scores 0.533 on CentralityAnomaly — below
the incumbent `centrality_max` (0.586). The LR-CV ceiling was unchanged (0.214/0.598). The
hard families' signal is not in any single new per-JE graph observable.

**(b) The plateau IS breakable — by nonlinear supervision, not new features.** A RandomForest
over the EXISTING relational features (trainable because DataSynth supplies the labels) reaches
RF-CV 0.252/0.623 — above the linear LR ceiling (0.214/0.598) and the unsupervised sum
(0.211/0.547) — lifting every hard family above random. The lift is driven by
`source_cond_edge_surprise_max` (RF importance 0.41) and `coupling_entropy` (0.17) — the latter
anti-correlated as a single feature (ROC 0.468, hence excluded from the unsupervised sum) but
valuable in nonlinear interaction. The hand-features are dead weight (tri-novelty 0.00,
centrality_delta 0.02).

**Cross-GL transfer (deployability) + the hybrid.** Train the RF on one labelled relational GL
(seed 7), apply to an unseen one (seed 23). The RF transfers and beats the unsupervised sum on
every hard family; the unsupervised sum still wins on dormancy (which it nails at 0.999). The
deployable detector is the HYBRID = rank-z-sum(unsupervised residual, RF):

| metric | unsup | RF | hybrid |
|---|---|---|---|
| overall PR-AUC / ROC    | 0.234 / 0.566 | 0.152 / 0.624 | **0.288 / 0.631** |
| DormantAccountActivity  | 0.999 | 0.904 | **0.999** |
| MissingRelationship     | 0.480 | 0.619 | 0.588 |
| CentralityAnomaly       | 0.474 | 0.619 | 0.580 |
| TransferPricingAnomaly  | 0.519 | 0.614 | 0.608 |
| NewCounterparty         | 0.533 | 0.562 | **0.584** |
| UnusualAccountPair      | 0.508 | 0.565 | 0.574 |
| CircularTransaction     | 0.439 | 0.586 | 0.523 |

The hybrid keeps dormancy perfect, lifts every hard family above the unsupervised baseline,
and improves overall PR-AUC +23%. It deploys WITHOUT test labels (train the RF once on
synthetic labelled relational GLs; the per-GL z-normalized features generalize cross-GL),
which sidesteps the A1 SBI-OOD problem (fit-on-self features, not a scale-bound global posterior).

**Conclusion.** The §12 plateau was an artifact of the unsupervised additive combination, not a
signal ceiling. The hard families are weakly learnable by a DataSynth-label-trained nonlinear
model that transfers across GLs — the discriminative complement to the generative residual,
exactly the "DataSynth powers both" thesis, now demonstrated for the relational families. None
are "solved" (0.55–0.62, vs dormancy's 0.999); the residual gap needs cross-JE / temporal
observability (a sequence/state-space model over the JE stream) — Tier C. Reproducible:
`inverse_audit.relational.rf_arm` (model `rf_arm.joblib`, result `rf_arm_transfer.json`).

## 19. Tier C-1 — productized hybrid on the corpus (2026-05-29)

Tier B's hybrid (unsupervised residual + DataSynth-label-trained RF) was validated cross-GL
on synthetic data. C-1 productizes it (`corpus_runner --rf-model`) and runs it on corpus
GL data (fit-on-self half-split) — the actual Stage-2 deployment, and the synthetic→corpus
transfer test the A1 global-SBI arm failed.

On a health corpus client (113,309 JEs, half-split → 56,655 scored, manifold 1,109 edges):
- **The synthetic-trained RF transfers** — `rf_score` is non-degenerate and discriminative
  (median 0.187, p99 0.494, max 0.833), NOT the Dirac collapse the global SBI posterior showed
  (§16). The per-GL z-normalized relational features generalize synthetic→corpus precisely
  because they're deviations-from-own-normal, not scale-bound absolutes.
- **Hybrid hot list** (top-1% = 566 JE IDs) overlaps the unsupervised top-1% at 0.84 — the RF
  reorders 16%, surfacing hard-family signatures the unsupervised residual ranks lower. (Corpus
  is unlabelled → expert-review delta; the RF's value was label-validated cross-GL in §18.)
- The unsupervised residual stays heavy-tailed (p50 0 / p99 17.4 / max 35.4) — a clean hot
  list; dormancy + edge-surprise carry it (this consolidated client has tp_set=1, so the tp
  features are inert, as §13 noted for single-counterparty entities).

The full inverse-audit detector now runs end-to-end on corpus data — density (per-JE fraud) +
unsupervised relational residual (dormancy/edge/cycle) + DataSynth-trained RF (hard families),
hybridized — and it transfers without the OOD collapse that sinks the global SBI arm.
Reproducible: `corpus_runner --mode half-split --rf-model rf_arm.joblib`.

## 20. Tier C-3 — rung-2 supervised OT cost from ground-truth flows (2026-05-29)

ot_flow rung-1 reconstructs within-JE debit↔credit pairings with a uniform (max-entropy)
cost. Rung-2 learns the cost from the flows DataSynth reveals: a 2-line JE (1 debit, 1 credit)
is an unambiguous ground-truth credit→debit edge — supervision the corpus can't give. On the
relational GL (10,279 JEs: 6,619 two-line / 3,660 multi-line; 6,733 ground-truth edges,
369 distinct pairs):
- **The learned cost is meaningful** — held-out 2-line edge ranking AUC 0.601 (cost(true edge)
  < cost(random pair); 0.5 = no signal). The trivially-paired JEs carry a genuine, learnable
  account-pairing cost.
- **It sharpens multi-line reconstruction** — mean coupling entropy on multi-line JEs drops
  9.7% (rung-1 uniform 0.798 → rung-2 learned 0.721): the learned cost resolves the
  transportation polytope more confidently than the uniform prior.

This implements the methodology paper's rung-2 (cost learned from known flows) — the
forward-model contribution only DataSynth enables. The effect is modest (the 369 common
double-entry pairs dominate, so the global edge_p the detector already uses is little-changed);
detector integration via the `cost_fn` hook (already present in `reconstruct_per_je`) is the
wired next step, expected marginal. Reproducible: `inverse_audit.relational.ot_cost`.

## 21. Tier C-2 — first cross-JE temporal observable (2026-05-29)

Tier B's ceiling said the hard families' residual signal needs cross-JE observability. C-2
adds the first temporal observable over the JE stream G(t): per-JE max-over-touched-accounts
of the account's weekly-activity burst z-score + trend (level shift), from the 412-day /
55-week GL. Measured via the Tier-B RF-CV harness (with vs without the temporal features):

| family | base | +temporal |
|---|---|---|
| overall PR-AUC / ROC | 0.252 / 0.623 | 0.270 / 0.642 |
| TransactionBurst    | 0.546 | 0.628 (+0.08) |
| UnusualTiming       | 0.556 | 0.641 (+0.085) |
| UnusualFrequency    | 0.587 | 0.638 (+0.05) |
| CircularTransaction | 0.554 | 0.591 (+0.04) |
| CentralityAnomaly   | 0.641 | 0.658 (+0.02) |
| TrendBreak          | 0.618 | 0.493 (−0.13, n=9) |

The burst observable lifts every temporal/cyclic family it targets (TransactionBurst,
UnusualTiming, UnusualFrequency, Circular, Centrality) and overall PR-AUC +0.018 — confirming
Tier B's diagnosis that the remaining signal lives in cross-JE temporal structure, not more
per-JE features. TrendBreak regressed (n=9, tiny; the simple latter-vs-earlier trend feature
doesn't match its injector mechanism). This is the tractable first rung; the full grey-box
state-space (Kalman innovations / CUSUM over G(t), process-FSM-structured) — where TrendBreak
and the relationship families would be modelled properly — is the larger follow-on the
methodology papers (b27/b28) point to. Reproducible: `inverse_audit.relational.temporal_features`.

---

### Tier A→C synthesis (2026-05-29)

The inverse-audit detector, end to end: **density** (per-JE fraud, PR-AUC 0.70/ROC 0.91) +
**unsupervised relational residual** (dormancy 0.99, edge-rarity) + **DataSynth-trained RF**
(the hard families, +23% PR-AUC, cross-GL transfer) + **temporal observable** (the burst/
cyclic families), all **hybridized** and **running on the corpus** (fit-on-self, no OOD
collapse). The global SBI arm is retired for corpus use (scale-bound); the forward model's
role is as the **label source for the discriminative arms** and the **fit-on-self
manifold** the residuals deviate from. Remaining frontier: the grey-box state-space over G(t)
for the relationship/trend families (C-2 follow-on) and detector-level rung-2 OT integration.

## 22. Research log — CoA reconstruction from the JE cube, no CoA mapping (2026-05-29, autoloop)

First increment of the autonomous hourly research loop. Goal: infer account type from JE flow
behaviour alone, validated against the account-number convention (1=asset / 2=liability /
3=equity / 4=revenue / 5,6=expense). On the relational GL (319 typeable accounts):
- **debit/credit-nature: 94.7% accuracy** — flow direction (`debit_frac`) cleanly reconstructs
  whether an account is debit-nature (asset/expense) vs credit-nature (liability/equity/revenue).
  The cube reveals account nature without the CoA file.
- 5-class (KMeans on debit_frac / net_frac / log-activity): purity 0.530 / ARI 0.273 —
  liability 0.91, expense 0.67, but asset 0.43 (confused with expense; both debit-nature) and
  revenue 0.11 / equity 0.00 (all credit-nature, indistinguishable on these features). Next
  increment: add a balance-sheet-vs-P&L signal (account-balance persistence / period-reset) to
  split within each nature group — the missing axis for full 5-class type reconstruction.
Reproducible: `inverse_audit.coa_reconstruct`.

## 23. Research log — CoA reconstruction, 5-class + supervised ceiling (2026-05-29, autoloop)

Increment 2 on #8. Added a balance-sheet-vs-P&L temporal signal (months-active fraction,
monthly activity CV, last-period share) + a supervised RF-CV ceiling. On the relational GL
(319 accounts):
- nature 94.7% (unchanged — robust).
- 5-class KMeans purity 0.498 / ARI 0.259; **supervised RF-CV ceiling only 0.367** — even a
  label-trained model can't separate the 5 types from flow+activity features. Per-type
  (supervised recall): asset 0.47, liability 0.41, expense 0.40, revenue 0.18, equity 0.00 (n=3).
- The activity-temporal features did NOT crack the within-nature confusions (asset↔expense,
  revenue↔equity↔liability); KMeans purity even dipped vs flow-only (0.53→0.50).

**Finding: the JE cube's flow + activity features carry strong NATURE signal (debit vs credit,
94.7%) but weak fine-TYPE signal (5-class ceiling 0.37).** The missing axis — balance-sheet
(persistent running balance) vs P&L (period flows closed at year-end) — is not in activity
aggregates; it needs explicit per-account running-balance dynamics (cumulative balance
persistence ratio, year-end closing-entry detection over fiscal_period). That's the next
increment for #8. Reproducible: `inverse_audit.coa_reconstruct`.

## 24. Research log — CoA reconstruction: running-balance dynamics + #8 conclusion (2026-05-29, autoloop)

Increment 3 on #8. Added running-balance-dynamics features (balance persistence = |final|/max|run|,
balance-to-flow, running-balance sign-changes) — the balance-sheet-vs-P&L axis. On the relational
GL (319 accounts): nature 94.7% (unchanged); supervised RF-CV ceiling 0.367 → 0.395 (marginal —
asset recall 0.47→0.54, revenue 0.18→0.26); unsupervised KMeans purity dipped (0.50→0.42, extra
features add noise to k-means).

**#8 conclusion: the JE cube robustly reconstructs account NATURE (debit/credit, 94.7%) but the
fine 5-class type is only weakly identifiable — supervised ceiling ~0.40 even with balance
dynamics.** On this GL the account types are not strongly behaviourally distinct beyond nature on
aggregate flow/balance features (likely no clean year-end P&L closing in the window, and the
generator's type→behaviour mapping isn't sharply separable). The one untried lever is RELATIONAL —
an account's type from its flow-graph NEIGHBOURS (revenue pairs with AR, expense with AP, …) rather
than aggregate stats (methodology-paper account-flow-graph → classification); a possible future
return. Deliverable: nature-level CoA reconstruction (94.7%) + an honest fine-type identifiability
bound. Pivoting the loop to #9. Reproducible: `inverse_audit.coa_reconstruct`.
