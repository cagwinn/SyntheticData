# Tier A — re-measure the inverse-audit OOD gap (A10 VM runbook)

**Goal.** Stage 2 of the inverse-audit capstone was gated on the OOD problem: real
GLs sit outside the synthetic "normal" manifold, so the SBI posterior collapsed to a
Dirac at the prior floor (FINDINGS §6, degenerate everywhere). Since then the
**ConcentrationPass central abstraction (#143)** landed on `main` (Phase 1 `3ffddcb8` +
Phase 2 `8c808331`) — the post-process layer that closes the §15 "knob bypassed by
other generators" blocker. **Tier A asks: did that widen the manifold enough to start
containing the corpus?** Two deliverables:

- **A1** — corpus OOD probe: train the SBI posterior on a campaign with ConcentrationPass
  OFF vs ON, apply each to the corpus summary-stat vector, and see whether the corpus
  posterior moves off the Dirac floor under ON.
- **A2** — "Stage 1.5": inject labelled anomalies into a ConcentrationPass-realism GL
  and measure unified-detector PR-AUC — a defensible number at corpus-realistic fidelity,
  bridging Stage 1's clean 0.741 and Stage 2's unlabelled hot lists.

Compute: **`gpu_1x_a10` (≈30 vCPU / 200 GB / A10)** — the proven VM #2 class. The GPU
load is a ~760 KB normalizing flow (~15 min); the bottleneck is CPU cores (parallel
campaign generation) + RAM (corpus summarization). A100 is unwarranted until Tier C.
Need **≥50 GB disk** (corpus ~18 GB + campaign outputs + repo + release build).

> Privacy ([[feedback_corpus_vague_reference]]): the corpus dir is referenced only as
> `$DATASYNTH_CORPUS_DIR`; never commit the path, client names, or row content. All
> emitted artifacts are aggregate (x-vectors, PMFs, posteriors, scores).

---

## 0. VM setup

```bash
# repo + release binary (current main has ConcentrationPass)
git clone <repo> ~/SyntheticData && cd ~/SyntheticData    # or: git pull
CARGO_BUILD_JOBS=$(nproc) cargo build --release -p datasynth-cli
export PATH=$HOME/SyntheticData/target/release:$PATH

# python env (no torch on the local box; recreate on the VM)
python3 -m venv ~/mlenv
~/mlenv/bin/pip install --upgrade pip
~/mlenv/bin/pip install torch --index-url https://download.pytorch.org/whl/cu124
~/mlenv/bin/pip install zuko scipy scikit-learn pandas pyarrow numpy pyyaml
~/mlenv/bin/python -c "import torch,zuko;print('cuda',torch.cuda.is_available())"
```

Corpus sync from the local box (run locally; path stays off the VM's committed tree):

```bash
# local → VM. JE_*.parquet (45 clients) + the industry map are all corpus_x needs.
rsync -av "$LOCAL_CORPUS/"JE_*.parquet  ubuntu@<vm>:/home/ubuntu/corpus/
rsync -av "$LOCAL_CORPUS/../denylist-build/client-industries.json ubuntu@<vm>:/home/ubuntu/
export DATASYNTH_CORPUS_DIR=/home/ubuntu/corpus
```

## 1. Corpus prep (posterior-independent — compute once)

```bash
cd ~/SyntheticData/experiments/ml
# 1a. corpus summary-stat vector(s): pooled + per-client + per-industry
DATASYNTH_CORPUS_DIR=/home/ubuntu/corpus ~/mlenv/bin/python -m inverse.corpus_x \
    --out runs/A1/corpus_x29.json --per-client \
    --industries /home/ubuntu/client-industries.json

# 1b. per-source account-pair PMF for the +concentration arm's account_pair_substitution
#     (aggregate per-source {debit,credit,p}; needs a synth CSV as the "other side" —
#      any small generated GL works, it's only used for the gap report half).
~/mlenv/bin/python -m inverse_audit.corpus_vs_synth_gap \
    --corpus-parquets /home/ubuntu/corpus/JE_*.parquet \
    --synth-csv <any small synthetic journal_entries.csv> \
    --out runs/A1/gap_report.json \
    --emit-pair-pmf runs/A1/corpus_pair_pmf.json
```

## 2. A1 — corpus OOD probe (OFF vs ON, + archived reference)

**Fix `make_base.py` drift first** (1 line): `init` now emits `distributions.amounts.
components: []`; `setdefault` leaves it empty → bare base fails `validate`. Force-set it
(harmless for the campaign — `simulate.py` overrides components per-θ, but keeps the base
valid). Then build the two base configs (θ prior is identical; only `concentration`
differs):

```bash
~/mlenv/bin/python -m inverse.make_base --out inverse_base.yaml --industry manufacturing
# arm OFF = inverse_base.yaml as-is (ConcentrationPass disabled by default)
# arm ON  = + the concentration block (validated locally 2026-05-29):
```
```yaml
concentration:
  enabled: true
  source_conditional_rarity: { rate: 0.01 }
  trading_partner_pool:       { target_size: 12 }      # corpus ~12 vs synth ~40
  account_pair_substitution:  { pmf_path: runs/A1/corpus_pair_pmf.json,
                                rarity_threshold: 0.005, top_k: 10 }
  source_blanking:            { rate: 0.21 }           # corpus ~21% blank source
  consolidation_outlier:      { rate: 0.001 }
```

**GATE — verify the ON passes actually fire** (validate is lenient on unknown fields;
a mistyped sub-field is silently dropped). Generate ONE small GL and confirm the
orchestrator logs non-zero `entries_modified` per pass + expected effects:

```bash
datasynth-data generate --config inverse_base_concentration.yaml --output /tmp/smoke 2>&1 \
  | grep -i "concentration\|pass\|substitut\|blank\|trading"
# expect: source_blanking ≈21% of JEs nulled; TP pool ≤12 distinct; account-pair
# substitutions > 0; consolidation_outlier expanded a few. If 0 → config didn't take.
```

Then the campaign (≥1500 each), train, apply:

```bash
for arm in off on; do
  cfg=inverse_base.yaml; [ $arm = on ] && cfg=inverse_base_concentration.yaml
  ~/mlenv/bin/python -m inverse.simulate --n 1500 --base $cfg --out runs/A1/pairs_$arm --seed 0
  ~/mlenv/bin/python -m inverse.train --pairs runs/A1/pairs_$arm/pairs.npz --out runs/A1/post_$arm
  ~/mlenv/bin/python -m inverse.apply --posterior runs/A1/post_$arm/posterior.pt \
       --x runs/A1/corpus_x29.json --out runs/A1/recovered_$arm.json
done
# reference: archived PRE-#143 posterior (rsync from
#   ~/DEV/local-artifacts/inverse_audit_stage1/inverse/weights/posterior.pt) applied to the same x.
```

**Read-out (the headline).** For each arm, the recovered θ for the corpus x:
- **Cheap first probe (no training needed):** Mahalanobis distance of corpus x to the
  synthetic x cloud (`pairs_off` vs `pairs_on`) — does ON pull the corpus inside?
- **Posterior:** is `fraud_rate` / `amount_mu` / `amount_sigma` still pinned to the prior
  corner with zero-width CIs (degenerate, as in §6), or does ON give finite-width CIs?
  **ON moving off the Dirac = the fidelity work bought identifiability — measured.**

## 3. A2 — Stage 1.5: labelled anomalies at corpus realism

`generate_mixed.py` needs a `--concentration <pmf_path>` flag that injects the same
concentration block into the `test` (and `normal`) configs via a `with_concentration()`
helper. Then:

```bash
cd ~/SyntheticData/experiments/ml
~/mlenv/bin/python -m inverse_audit.run_capstone --root runs/A2/concentration_on \
    --industry healthcare --complexity medium --fraud-rate 0.04 --anomaly-rate 0.06 --seed 7
# baseline (concentration off) = the FINDINGS §12 numbers (unified vs is_any 0.395/0.654)
```

**Read-out.** unified PR-AUC/ROC vs `is_fraud` / `is_anomaly` / `is_any` at
corpus-realistic fidelity, compared to the §12 in-distribution baseline. Detection
should hold (the manifold is sharper, not noisier); large drops would flag a pass that
distorts the normal manifold.

## Success criteria / decision

- **If the ON corpus posterior moves off the Dirac floor** → fidelity bought
  identifiability; Stage 2 gets real numbers; Tier C (rung-2 OT / latent-flow) becomes
  the priority.
- **If it still collapses** → the residual gap is the *joint/structural* manifold the
  per-marginal passes don't reach; the next fidelity lever (the central-concentration
  refinements) is still the gate, and Tier B (label-free relational depth) is the
  higher-value path.

## Code touch-ups this requires (experiments-only, won't destabilize the engine)

1. `inverse/make_base.py` — force non-empty `distributions.amounts.components`.
2. `inverse_audit/generate_mixed.py` — `--concentration <pmf_path>` + `with_concentration()`.
3. (maybe) `inverse/apply.py` — accept the corpus x29 json directly (confirm interface).

## Artifacts (all aggregate, commit-safe)

`runs/A1/{corpus_x29.json, corpus_pair_pmf.json, recovered_off|on.json}`,
`runs/A2/concentration_on/unified.json`. The OOD read-out → FINDINGS §16.
