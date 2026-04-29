# Group audit v5.0 — XXL Azure VM verification plan

**Context:** The current workstation killed twice while running `cargo test --workspace` — the `datasynth-cli::saft_export_smoke` test spawns three concurrent `datasynth-data generate` subprocesses that each materialize ~1.8 GB of data, which blows past ~30 GB RAM with the compiler + test harnesses already resident. v5.0 development on-host therefore runs **per-crate only** (`cargo test -p <crate> -- --test-threads=2`) and defers the full-stack smoke to a dedicated VM.

This document describes what that dedicated VM run looks like: machine, layout, commands, success criteria, and rollback. Run it **after Chunk 12 closes** and before we ship v5.0.

---

## 1. VM sizing

Azure SKU: **Standard_E32ds_v5** (or equivalent)
- 32 vCPU (Intel Ice Lake)
- 256 GiB RAM
- 1.2 TiB local SSD (ephemeral `/mnt`)
- Accelerated networking
- Zonal redundancy not required — single-zone is fine; the run is a couple of hours at most.

Rationale for 256 GiB:
- `cargo test --workspace --release` peaks at ~40 GiB RSS across rustc + linker + test binaries.
- `saft_export_smoke` alone holds ~6 GiB peak per subprocess (×3 parallel) ≈ 18 GiB.
- Mini-Nestlé Chunk 11 golden fixture regen is ~5 GiB peak.
- Full `enterprise-2000` scale smoke (2 000 entities × ~500 rows each) lands around ~80 GiB peak; budget 2× headroom.

Region: **Switzerland North** or **West Europe** (lowest egress cost vs. GitHub Actions cache if we re-home CI later).

Storage:
- OS disk: 128 GiB Premium SSD (Ubuntu 24.04 LTS).
- Data disk: use the ephemeral 1.2 TiB `/mnt` for `target/` and generated output. The `/mnt` drive is re-formatted on deallocation — that's a feature here: no cleanup needed after the run.
- Mount `/mnt` with `discard` and `noatime` for speed.

Cost envelope:
- Standard_E32ds_v5 Spot in `switzerlandnorth` ≈ USD 0.35/hr (pay-as-you-go is ~1.0/hr).
- 4 h budget (provisioning 15 min + full run ~3 h + teardown 15 min) ≈ USD 1.40–4.00 per pass.

---

## 2. One-shot provisioning

```bash
az group create --name ds-v5-verify --location switzerlandnorth

az vm create \
  --resource-group ds-v5-verify \
  --name ds-v5-xxl \
  --image Ubuntu2404 \
  --size Standard_E32ds_v5 \
  --priority Spot \
  --max-price -1 \
  --eviction-policy Delete \
  --admin-username michael \
  --ssh-key-values ~/.ssh/id_ed25519.pub \
  --public-ip-sku Standard

az vm open-port --resource-group ds-v5-verify --name ds-v5-xxl --port 22

# Wait until ssh is reachable, then:
ssh michael@<public-ip> 'sudo mkfs.ext4 -F /dev/disk/azure/resource && sudo mount /dev/disk/azure/resource /mnt'
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
mkdir -p /mnt/target /mnt/output
ln -s /mnt/target target
```

---

## 3. Verification matrix (what to run, what good looks like)

Run everything from the repo root, in this order. Each step has a **pass criterion** and a **time budget**.

### 3.1 Full workspace build

```bash
cargo build --workspace --release 2>&1 | tee /mnt/output/build.log
```

- **Pass:** Exit 0. No `error[` lines. Warnings okay.
- **Budget:** ~6 min on 32 vCPU (first build). Incremental rebuilds inside the VM can skip this.
- **If it fails:** Read the log, fix, rebuild. Don't move on until this is green.

### 3.2 Full workspace test

```bash
cargo test --workspace --release -- --test-threads=8 2>&1 | tee /mnt/output/test.log
```

- **Pass:** Every `test result: ok. N passed; 0 failed; …` line across every crate. `rg '^FAILED' /mnt/output/test.log` should be empty.
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
  2>&1 | tee /mnt/output/test-ignored.log
```

- **Pass:** Every previously-ignored test now reports `ok`. In particular,
  `datasynth-group::shard_runner::run_shard_writes_per_entity_output_and_summary`
  must produce a `journal_entries.json` for the entity and a
  `shard_summary.json` whose round-trip equals the in-memory summary.
- **Budget:** ~10 min on top of 3.2's budget. The shard-runner test alone
  is ~3 min single-entity at quarterly period.

### 3.3 Lint + format

```bash
cargo fmt --all --check 2>&1 | tee /mnt/output/fmt.log
cargo clippy --workspace --all-targets --no-deps -- -D warnings 2>&1 | tee /mnt/output/clippy.log
```

- **Pass:** Exit 0 for both. Any new warning on this branch vs. `main` is a regression — file an issue.
- **Budget:** ~3 min for clippy (uses build artifacts from 3.1).
- **Known exceptions:** `crates/datasynth-group/tests/expansion.rs:57` pre-existing `needless_range_loop` warning — track, don't block the release on it.

### 3.4 Mini-Nestlé golden regen check

```bash
# The Chunk 11 property test compares against a committed golden manifest.
cargo test -p datasynth-group --test manifest_golden --release -- --ignored \
  2>&1 | tee /mnt/output/golden.log
```

- **Pass:** The ignored `regenerate_golden` test runs and produces a manifest byte-identical to `tests/golden/mini_nestle_manifest.json`. If it differs, `git diff` will show which fields drifted — investigate before merging.
- **Budget:** ~10 s.

### 3.5 Mini-Nestlé end-to-end generation (Chunk 10 CLI + Chunk 4 shard runner)

Once Chunk 10's CLI command lands (`datasynth-data group run --config <file>`), exercise it against the mini_nestle fixture:

```bash
./target/release/datasynth-data group run \
  --config crates/datasynth-group/tests/fixtures/mini_nestle.yaml \
  --output /mnt/output/mini_nestle \
  2>&1 | tee /mnt/output/e2e-mini.log
```

- **Pass criteria:**
  - Exit 0.
  - `/mnt/output/mini_nestle/entities/{NESTLE_SA,NESTLE_USA,NESTLE_DE,NESTLE_BR,NESTLE_JV}/` each contain `journal_entries.json` with ≥1 line.
  - Every JE whose `header.ic_pair_id` is `Some` has a mirror JE under the counterparty entity with the same `pair_id`.
  - `/mnt/output/mini_nestle/consolidated/trial_balance.json` balances (total debit = total credit within ±0.01 CHF).
  - `shard_summary.json` and `aggregate_summary.json` exist and list every entity.
- **Budget:** ~30 s for mini_nestle (5 entities × quarterly period).

### 3.6 Enterprise-2000 scale smoke (stretch)

This is the **load** test for the 2 000-entity / ~1 TB envelope the spec promises. Skip on first pass; run once per release-candidate once Chunks 4-9 are proven.

```bash
./target/release/datasynth-data group run \
  --config docs/superpowers/examples/enterprise-2000.yaml \
  --output /mnt/output/enterprise-2000 \
  --max-threads 32 \
  2>&1 | tee /mnt/output/e2e-enterprise.log
```

- **Pass criteria:**
  - Exit 0 within 120 min wall-clock.
  - Peak RSS < 180 GiB (monitor via `/usr/bin/time -v`).
  - Disk footprint < 1.1 TiB (leaves 100 GiB headroom on the 1.2 TiB ephemeral).
  - `aggregate_summary.json.ic_match_coverage >= 0.98` (spec threshold).
- **Budget:** ~90 min typical.

### 3.7 Fraud-bias smoke

```bash
cargo test --release -p datasynth-runtime --test fraud_bias_smoke \
  2>&1 | tee /mnt/output/fraud-bias.log
```

- **Pass:** 1/1 green. Confirms the `fraud_bias` sweep still fires on every `is_fraud = true` path after v5.0's JE changes.
- **Budget:** ~15 s.

---

## 4. Artifacts to snapshot

Before tearing down the VM, copy these back to the workstation (or upload to the team's shared bucket):

```
/mnt/output/build.log
/mnt/output/test.log
/mnt/output/fmt.log
/mnt/output/clippy.log
/mnt/output/golden.log
/mnt/output/e2e-mini.log
/mnt/output/e2e-enterprise.log          # if 3.6 ran
/mnt/output/fraud-bias.log
/mnt/output/mini_nestle/                # the generated archive — ~50 MB
```

Commit nothing to the feature branch from the VM — keep it a pure verification harness. Logs and the mini_nestle archive live outside the repo under `docs/superpowers/verification-runs/<date>/` once manually reviewed.

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
