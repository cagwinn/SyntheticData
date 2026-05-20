# Group audit v5.0 — XXL Azure VM verification plan

**Context:** The current workstation killed twice while running `cargo test --workspace` — the `datasynth-cli::saft_export_smoke` test spawns three concurrent `datasynth-data generate` subprocesses that each materialize ~1.8 GB of data, which blows past ~30 GB RAM with the compiler + test harnesses already resident. v5.0 development on-host therefore runs **per-crate only** (`cargo test -p <crate> -- --test-threads=2`) and defers the full-stack smoke to a dedicated VM.

This document describes what that dedicated VM run looks like: machine, layout, commands, success criteria, and rollback. Run it **after Chunk 12 closes** and before we ship v5.0.

---

## 1. VM sizing

Azure SKU: **Standard_NC40ads_H100_v5** (West Europe)
- 40 vCPU (AMD EPYC Genoa)
- **320 GiB RAM**
- 1× NVIDIA H100 GPU (unused by v5.0 — reserved for future RustGraph / RustCompute workloads in the same subscription)
- 128 GiB local NVMe (`/mnt/resource`)
- Accelerated networking
- Single-zone is fine — the v5.0 verification run is a couple of hours.

**Why this SKU:** the VynFi subscription has a pre-approved 80-vCPU quota
on the `Standard NCadsH100v5 Family` (intended for GPU-accelerated
frameworks coming later). All other CPU/memory families in this
subscription are capped at 10 vCPU per family, which would block any
of the Standard_E32ds_v5 / E48ds_v5 / E64ds_v5 alternatives without a
support-ticket quota increase. NC40ads_H100_v5 over-provisions the
v5.0 workload (320 GiB RAM vs. the planned 256 GiB) so it carries
even more headroom than the original sizing target. The H100 is
idle during this run; treat it as a sunk cost amortised across
later GPU work.

Rationale for 320 GiB:
- `cargo test --workspace --release` peaks at ~40 GiB RSS across rustc + linker + test binaries.
- `saft_export_smoke` alone holds ~6 GiB peak per subprocess (×3 parallel) ≈ 18 GiB.
- Mini-Acme Chunk 11 golden fixture regen is ~5 GiB peak per shard; with `parallel_shards: true` and 5 entities at ~17 GiB each, a worst-case parallel run could touch ~85 GiB. 320 GiB leaves ~3.7× headroom.
- The hypothetical `enterprise-2000` scale smoke (2 000 entities × ~500 rows each, ~80 GiB peak) is **out of scope for v5.0**; if added later, this SKU still has 4× memory headroom for it.

Region: **West Europe** (only region in the VynFi subscription with sufficient pre-approved CPU/GPU quota — Switzerland North caps at 10 total regional vCPUs).

Storage:
- OS disk: 128 GiB Premium SSD (Ubuntu 24.04 LTS).
- Local NVMe: 128 GiB at `/mnt/resource` — fits cargo `target/` (~5 GiB built) + Mini-Acme output (~50 MB) + workspace test artefacts (≪ 1 GiB) with comfortable margin.
- **If `enterprise-2000` is later added:** attach a 2-TiB Premium SSD as a data disk; 128 GiB ephemeral is too small for that workload (~1 TiB output).
- Mount `/mnt/resource` with `discard` and `noatime` for speed.

Cost envelope:
- Standard_NC40ads_H100_v5 Spot in `westeurope` ≈ USD 1.30/hr (pay-as-you-go ≈ USD 6.50/hr — the H100 dominates the price even though we don't use it).
- 4 h budget (provisioning 15 min + full run ~3 h + teardown 15 min) ≈ USD **5–25 per pass** (Spot vs PAYG).
- For a one-off v5.0 verification: spot is fine — eviction risk during a 3 h run is low and the deployment is idempotent.

---

## 2. One-shot provisioning

```bash
# VynFi Production subscription — confirm before running
az account show --query name -o tsv  # expect: VynFi Production

az group create --name ds-v5-verify --location westeurope

az vm create \
  --resource-group ds-v5-verify \
  --name ds-v5-xxl \
  --image Ubuntu2404 \
  --size Standard_NC40ads_H100_v5 \
  --priority Spot \
  --max-price -1 \
  --eviction-policy Delete \
  --admin-username michael \
  --ssh-key-values ~/.ssh/id_ed25519.pub \
  --public-ip-sku Standard \
  --accept-term

az vm open-port --resource-group ds-v5-verify --name ds-v5-xxl --port 22

# The local NVMe ships unformatted on the NC* series — mount it before the run.
# Wait until ssh is reachable, then:
PUBLIC_IP=$(az vm show -d -g ds-v5-verify -n ds-v5-xxl --query publicIps -o tsv)
ssh "michael@${PUBLIC_IP}" '
  sudo mkfs.ext4 -F /dev/disk/azure/resource &&
  sudo mkdir -p /mnt/work &&
  sudo mount -o discard,noatime /dev/disk/azure/resource /mnt/work &&
  sudo chown $USER:$USER /mnt/work
'
```

Bootstrap script (`scripts/xxl-verify-bootstrap.sh` — add to repo when running this plan):

```bash
#!/usr/bin/env bash
set -euxo pipefail

# Toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
source "$HOME/.cargo/env"

# Build deps
sudo apt-get update -q
sudo apt-get install -y build-essential pkg-config libssl-dev cmake protobuf-compiler git jq

# Clone the branch
git clone --depth=1 --branch feat/group-audit-v5.0 \
  git@github.com:<org>/RustSyntheticData.git
cd RustSyntheticData/SyntheticData

# Route target/ and generated output to the ephemeral disk
mkdir -p /mnt/work/target /mnt/work/output
ln -s /mnt/work/target target
```

---

## 3. Verification matrix (what to run, what good looks like)

Run everything from the repo root, in this order. Each step has a **pass criterion** and a **time budget**.

### 3.1 Full workspace build

```bash
cargo build --workspace --release 2>&1 | tee /mnt/work/output/build.log
```

- **Pass:** Exit 0. No `error[` lines. Warnings okay.
- **Budget:** ~6 min on 32 vCPU (first build). Incremental rebuilds inside the VM can skip this.
- **If it fails:** Read the log, fix, rebuild. Don't move on until this is green.

### 3.2 Full workspace test

```bash
cargo test --workspace --release -- --test-threads=8 2>&1 | tee /mnt/work/output/test.log
```

- **Pass:** Every `test result: ok. N passed; 0 failed; …` line across every crate. `rg '^FAILED' /mnt/work/output/test.log` should be empty.
- **Budget:** ~20 min. The SAF-T smoke is the long tail (~5 min with 3× subprocesses each producing 1.8 GB on the ephemeral SSD).
- **Expected test counts (as a sanity check against silent skips):**
  - `datasynth-core`: 600+ tests
  - `datasynth-group`: ≥141 (v5.0 baseline after Chunk 4.2)
  - `datasynth-runtime`: 125 lib + full integration suite
  - `datasynth-cli`: saft_export_smoke passes (3 subprocess runs balanced + ~1.8 GB SAF-T each)
- **If a test fails:** Capture the log, open an issue, fix on the feature branch, re-run just that crate with `-p`, then re-run this full sweep.

#### 3.2.b — Orchestrator-heavy `#[ignore]`d tests

Some tests drive the full `EnhancedOrchestrator` end-to-end and consume
~17 GiB peak RSS per shard — they are marked `#[ignore]` so the
workstation's `cargo test` doesn't OOM the host. The XXL VM is the
canonical place to exercise them:

```bash
cargo test --workspace --release -- --test-threads=4 --include-ignored \
  2>&1 | tee /mnt/work/output/test-ignored.log
```

- **Pass:** Every previously-ignored test now reports `ok`. In particular,
  `datasynth-group::shard_runner::run_shard_writes_per_entity_output_and_summary`
  must produce a `journal_entries.json` for the entity and a
  `shard_summary.json` whose round-trip equals the in-memory summary.
  `datasynth-group::standalone_e2e::generate_standalone_produces_full_archive`
  must produce a complete archive — manifest persisted, both per-entity
  shards generated, and every consolidated artefact emitted.
- **Chunk 11 (`#[ignore]`d) — property tests + golden fixture:** in addition,
  - `datasynth-group::golden_archive::check_against_golden` (Task 11.1) — diffs
    the live archive against the committed `tests/golden/mini_acme/`. Run
    `cargo test -p datasynth-group --test golden_archive --release -- --ignored
    check_against_golden`. **First-run bootstrap:** until the very first
    `regenerate_golden` lands, the golden directory contains only
    `.gitkeep` and the test will fail with "golden archive is empty".
    Run `regenerate_golden` then re-run the check.
  - `datasynth-group::determinism_in_process::generate_standalone_twice_byte_identical`
    (Task 11.4) — back-to-back `generate_standalone` calls produce
    byte-identical archives.
  - `datasynth-group::determinism_in_process::subprocess_pipeline_matches_standalone`
    (Task 11.4) — `datasynth-data group manifest` + `shard …` + `aggregate`
    matches `generate_standalone` byte-for-byte.
- **Budget:** ~10 min on top of 3.2's budget. The shard-runner test alone
  is ~3 min single-entity at quarterly period.  Add ~70 min for the
  golden-archive + determinism harnesses (each runs the full Mini-Acme
  pipeline ≥ once).

### 3.3 Lint + format

```bash
cargo fmt --all --check 2>&1 | tee /mnt/work/output/fmt.log
cargo clippy --workspace --all-targets --no-deps -- -D warnings 2>&1 | tee /mnt/work/output/clippy.log
```

- **Pass:** Exit 0 for both. Any new warning on this branch vs. `main` is a regression — file an issue.
- **Budget:** ~3 min for clippy (uses build artifacts from 3.1).
- **Known exceptions:** `crates/datasynth-group/tests/expansion.rs:57` pre-existing `needless_range_loop` warning — track, don't block the release on it.

### 3.4 Mini-Acme golden regen check

```bash
# The Chunk 11 property test compares against a committed golden manifest.
cargo test -p datasynth-group --test manifest_golden --release -- --ignored \
  2>&1 | tee /mnt/work/output/golden.log
```

- **Pass:** The ignored `regenerate_golden` test runs and produces a manifest byte-identical to `tests/golden/mini_acme_manifest.json`. If it differs, `git diff` will show which fields drifted — investigate before merging.
- **Budget:** ~10 s.

### 3.5 Mini-Acme end-to-end generation (Chunk 10 CLI + Chunk 4 shard runner)

Once Chunk 10's CLI command lands (`datasynth-data group run --config <file>`), exercise it against the mini_acme fixture:

```bash
./target/release/datasynth-data group run \
  --config crates/datasynth-group/tests/fixtures/mini_acme.yaml \
  --output /mnt/work/output/mini_acme \
  2>&1 | tee /mnt/work/output/e2e-mini.log
```

- **Pass criteria:**
  - Exit 0.
  - `/mnt/work/output/mini_acme/entities/{ACME_SA,ACME_USA,ACME_DE,ACME_BR,ACME_JV}/` each contain `journal_entries.json` with ≥1 line.
  - Every JE whose `header.ic_pair_id` is `Some` has a mirror JE under the counterparty entity with the same `pair_id`.
  - `/mnt/work/output/mini_acme/consolidated/trial_balance.json` balances (total debit = total credit within ±0.01 CHF).
  - `shard_summary.json` and `aggregate_summary.json` exist and list every entity.
- **Budget:** ~30 s for mini_acme (5 entities × quarterly period).

### 3.6 Enterprise-2000 scale smoke (stretch)

This is the **load** test for the 2 000-entity / ~1 TB envelope the spec promises. Skip on first pass; run once per release-candidate once Chunks 4-9 are proven.

```bash
./target/release/datasynth-data group run \
  --config docs/superpowers/examples/enterprise-2000.yaml \
  --output /mnt/work/output/enterprise-2000 \
  --max-threads 32 \
  2>&1 | tee /mnt/work/output/e2e-enterprise.log
```

- **Pass criteria:**
  - Exit 0 within 120 min wall-clock.
  - Peak RSS < 180 GiB (monitor via `/usr/bin/time -v`).
  - Disk footprint < 1.1 TiB (leaves 100 GiB headroom on the 1.2 TiB ephemeral).
  - `aggregate_summary.json.ic_match_coverage >= 0.98` (spec threshold).
- **Budget:** ~90 min typical.

### 3.7 Final workspace verification (Task 12.6)

After 3.1–3.6 are green:

```bash
# 1. Format check
cargo fmt --all --check 2>&1 | tee /mnt/work/output/fmt-final.log

# 2. Workspace clippy
cargo clippy --workspace --all-targets --no-deps -- -D warnings \
  2>&1 | tee /mnt/work/output/clippy-final.log

# 3. Workspace test (release)
cargo test --workspace --release -- --test-threads=4 \
  2>&1 | tee /mnt/work/output/test-final.log

# 4. Workspace test with --include-ignored (release)
cargo test --workspace --release -- --test-threads=4 --include-ignored \
  2>&1 | tee /mnt/work/output/test-ignored-final.log

# 5. Generate Mini-Acme end-to-end
cargo run --release -p datasynth-cli -- group generate \
  --config configs/examples/group/mini_acme.yaml \
  --out /mnt/work/output/v5.0-mini-acme/

# 6. Inspect output layout
ls -la /mnt/work/output/v5.0-mini-acme/
ls -la /mnt/work/output/v5.0-mini-acme/consolidated/
ls -la /mnt/work/output/v5.0-mini-acme/entities/
cat /mnt/work/output/v5.0-mini-acme/ic_eliminations/ic_matching_coverage.json | jq .
```

- **Pass:** All 4 cargo commands exit 0; the Mini-Acme run completes and the
  output tree contains every directory documented in §1.4 of the spec, including
  `consolidated/consolidated_financial_statements.json`.
- **Budget:** ~30 min on the XXL VM in addition to 3.1–3.6.

### 3.8 Fraud-bias smoke

```bash
cargo test --release -p datasynth-runtime --test fraud_bias_smoke \
  2>&1 | tee /mnt/work/output/fraud-bias.log
```

- **Pass:** 1/1 green. Confirms the `fraud_bias` sweep still fires on every `is_fraud = true` path after v5.0's JE changes.
- **Budget:** ~15 s.

### 3.9 Performance data collection (for documentation)

The XXL VM run is also our chance to capture authoritative performance
numbers for v5.0 — the host workstation can't run these workloads, so
this is the only place they get measured. Capture all of the below into
`/mnt/work/output/perf/` and snapshot the directory back to the
workstation alongside the verification logs.

Each measurement uses **`/usr/bin/time -v`** for memory + wall-clock
data, **`perf stat`** for CPU counters where available, and **`du -sh`**
for output-size accounting.

#### 3.9.a — Per-shard generation profile (single entity, sequential)

```bash
# One entity, no rayon parallelism — establishes the baseline per-shard cost.
mkdir -p /mnt/work/output/perf

/usr/bin/time -v -o /mnt/work/output/perf/shard-ACME_SA.time \
  cargo run --release -p datasynth-cli -- group manifest \
    --config configs/examples/group/mini_acme.yaml \
    --out /mnt/work/output/perf/manifest.json

# Find the shard ACME_SA lives in (it's S_SIG_0001 in the canonical fixture)
SHARD_ID=$(jq -r '.ownership_graph.entities[] | select(.code=="ACME_SA") | .shard_id' \
  /mnt/work/output/perf/manifest.json)

/usr/bin/time -v -o /mnt/work/output/perf/shard-${SHARD_ID}.time \
  cargo run --release -p datasynth-cli -- group shard \
    --manifest /mnt/work/output/perf/manifest.json \
    --shard-id "$SHARD_ID" \
    --out /mnt/work/output/perf/shard-${SHARD_ID}/

du -sh /mnt/work/output/perf/shard-${SHARD_ID}/ \
  > /mnt/work/output/perf/shard-${SHARD_ID}.du
```

**Captures:** peak RSS, wall-clock, user/sys time, voluntary context
switches, output archive size. Per-entity expectation: **~17 GiB peak
RSS, ~3–5 min wall-clock per entity** at quarterly period; output
archive ~10 MB per entity.

#### 3.9.b — Standalone parallel-shard scaling

```bash
# Sequential (parallel_shards=false) — deterministic baseline
/usr/bin/time -v -o /mnt/work/output/perf/standalone-sequential.time \
  cargo run --release -p datasynth-cli -- group generate \
    --config configs/examples/group/mini_acme.yaml \
    --no-parallel-shards \
    --out /mnt/work/output/perf/standalone-seq/

# Parallel (default) — exercise rayon scheduling
/usr/bin/time -v -o /mnt/work/output/perf/standalone-parallel.time \
  cargo run --release -p datasynth-cli -- group generate \
    --config configs/examples/group/mini_acme.yaml \
    --out /mnt/work/output/perf/standalone-par/

du -sh /mnt/work/output/perf/standalone-{seq,par}/ \
  > /mnt/work/output/perf/standalone.du
```

**Captures:** sequential vs parallel speedup at 5 entities / 2 shards.
Expected: ~25–35 min sequential, ~12–18 min parallel (assuming
2 shards in flight with 2-3 entities each). The 320 GiB RAM lets us
run all shards in parallel without throttling.

#### 3.9.c — Workspace test timing breakdown

```bash
# Per-crate timing (write each to its own log so the hot crates stand out)
for crate in datasynth-core datasynth-group datasynth-runtime datasynth-cli \
             datasynth-generators datasynth-eval datasynth-output; do
  /usr/bin/time -v -o /mnt/work/output/perf/test-${crate}.time \
    cargo test --release -p $crate -- --test-threads=4 \
    2>&1 | tee /mnt/work/output/perf/test-${crate}.log
done

# Aggregated workspace
/usr/bin/time -v -o /mnt/work/output/perf/test-workspace.time \
  cargo test --workspace --release -- --test-threads=4 \
  2>&1 | tee /mnt/work/output/perf/test-workspace.log
```

**Captures:** per-crate test wall-clock + RSS so we can identify
hotspots and document realistic CI budgets.

#### 3.9.d — Build timing (cold + incremental)

```bash
# Cold build — clears target then times from scratch
cargo clean
/usr/bin/time -v -o /mnt/work/output/perf/build-cold.time \
  cargo build --workspace --release

# Incremental build — touch one file, time the rebuild
touch crates/datasynth-group/src/lib.rs
/usr/bin/time -v -o /mnt/work/output/perf/build-incr.time \
  cargo build --workspace --release
```

**Captures:** cold-build (~6 min expected on 40 vCPU) and incremental
rebuild (~30-60 s). Anchors the dev-loop and CI cost-of-build.

#### 3.9.e — IC matching coverage at scale

```bash
# 10-randomized property test (Task 11.2) but run with --release to
# capture realistic per-iteration timing.
/usr/bin/time -v -o /mnt/work/output/perf/ic-coverage.time \
  cargo test --release -p datasynth-group --test ic_coverage_property \
    -- --test-threads=1 --nocapture \
  2>&1 | tee /mnt/work/output/perf/ic-coverage.log
```

**Captures:** the matching-engine's throughput on randomized configs.
Expected: <100 ms per iteration even on configs with 15 entities ×
8 IC relationships.

#### 3.9.f — Output archive composition

```bash
# Detailed breakdown of what's in the standalone parallel run's archive
find /mnt/work/output/perf/standalone-par -type f \
  -exec du -b {} \; \
  | sort -nr \
  > /mnt/work/output/perf/archive-files.tsv

# Top-level directory sizes
du -sh /mnt/work/output/perf/standalone-par/* \
  > /mnt/work/output/perf/archive-dirs.tsv
```

**Captures:** per-file and per-directory size breakdown — feeds the
README's "expected output layout" section with real numbers.

#### Summary roll-up

After all 3.9.x runs land, generate a one-page summary:

```bash
cat > /mnt/work/output/perf/SUMMARY.md <<'EOF'
# v5.0 performance baseline — Standard_NC40ads_H100_v5

| Metric | Value | Captured in |
|---|---|---|
| Cold workspace build (release) | ${BUILD_COLD}      | build-cold.time     |
| Incremental rebuild            | ${BUILD_INCR}      | build-incr.time     |
| Workspace test (4-thread)      | ${TEST_WORKSPACE}  | test-workspace.time |
| Per-shard, 1 entity            | ${SHARD_1ENT}      | shard-S_SIG_0001    |
| Standalone, sequential         | ${STANDALONE_SEQ}  | standalone-seq      |
| Standalone, parallel-shards    | ${STANDALONE_PAR}  | standalone-par      |
| Mini-Acme archive size       | ${ARCHIVE_SIZE}    | archive-dirs.tsv    |
| IC matching, per iteration     | ${IC_PER_ITER}     | ic-coverage.time    |
EOF
# Fill in by hand from the .time files — the elapsed wall-clock and Maximum resident set size lines.
```

These numbers feed three places:
- The `## Performance` section in CLAUDE.md (currently a single-line stub).
- The `README.md` v5.0 section.
- A new `docs/performance/v5.0-baseline.md` reference document
  committed alongside the verification artefacts.

---

## 4. Artifacts to snapshot

Before tearing down the VM, copy these back to the workstation (or upload to the team's shared bucket):

```
/mnt/work/output/build.log
/mnt/work/output/test.log
/mnt/work/output/test-ignored.log
/mnt/work/output/fmt.log
/mnt/work/output/clippy.log
/mnt/work/output/golden.log
/mnt/work/output/e2e-mini.log
/mnt/work/output/e2e-enterprise.log          # if 3.6 ran (out of scope for v5.0)
/mnt/work/output/fraud-bias.log
/mnt/work/output/mini_acme/                # the generated archive — ~50 MB
/mnt/work/output/perf/                       # §3.9 performance data (~few MB of .time/.log files)
/mnt/work/output/perf/SUMMARY.md             # one-page perf roll-up — feeds CLAUDE.md / README / docs/performance/
/mnt/work/output/perf/standalone-par/        # canonical end-to-end archive used as performance reference
```

Commit nothing to the feature branch from the VM — keep it a pure verification harness. Logs and the mini_acme archive live outside the repo under `docs/superpowers/verification-runs/<date>/` once manually reviewed.

---

## 5. Teardown

```bash
az group delete --name ds-v5-verify --yes
```

This removes the VM, disks, public IP, and NIC — everything. One line, no lingering cost.

---

## 6. When to re-run

- **Before merging the feature branch to `main`** (required).
- **After any change to `datasynth-cli::saft_export_smoke` or `datasynth-runtime::enhanced_orchestrator`** (major paths this test hits).
- **After any change to the manifest schema** (`crates/datasynth-group/src/manifest/`), to regenerate the golden fixture.
- Not required for isolated changes inside `datasynth-group/src/shard/` once per-crate tests pass — those fit on the workstation's memory envelope.

---

## 7. Why this isn't CI (yet)

GitHub Actions' default runners top out at 16 GB RAM — not enough for the SAF-T smoke in parallel mode. A self-hosted runner on the same Azure SKU would work, but the v5.0 release cadence is small enough that a manual VM spin-up is cheaper than standing up a persistent runner. Revisit if release cadence rises above weekly.
