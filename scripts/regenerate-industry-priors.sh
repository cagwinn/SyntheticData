#!/usr/bin/env bash
# Regenerate the five committed industry-priors bundles from the corpus.
# Run manually after pulling fresh client data or when extraction logic changes.
#
# Usage:
#   scripts/regenerate-industry-priors.sh <REAL_CORPUS_DIR> [--pii-denylist <PATH>]
#
#   REAL_CORPUS_DIR  Path to the private corpus directory.  May also be
#                    supplied via the REAL_CORPUS_DIR environment variable; the
#                    positional argument takes precedence.
#
#   --pii-denylist PATH
#                    SP6 Phase B — path to a private PII denylist (TSV:
#                    literal-or-/regex/<TAB>kind). The file is PII-derived and
#                    MUST NOT be committed to this repo. When supplied, each
#                    per-client extraction call passes the flag through to the
#                    'fingerprint extract' subcommand, which populates
#                    BehavioralPriors.text_taxonomy via
#                    extract_text_taxonomy_from_records. When omitted, only
#                    Phase A (automated structural) tokenization runs; the
#                    build-time residual-PII audit gate that runs after every
#                    regen WILL reject bundles that carry fuzzy proper nouns.
#
# Build-time audit gate:
#   After all bundles are written, the script automatically runs
#   'cargo test -p datasynth-runtime --test bundle_pii_audit'. If the gate
#   fails the script aborts with a non-zero exit code and prints a clear error.
#   Fix the extraction (add/extend --pii-denylist, or patch Phase A rules) and
#   re-run before committing regenerated bundles.

set -euo pipefail

# ---------------------------------------------------------------------------
# Argument parsing — positional REAL_CORPUS_DIR then optional flags
# ---------------------------------------------------------------------------
PII_DENYLIST=""

# Consume the first positional argument as REAL_CORPUS_DIR, then parse flags.
if [[ $# -gt 0 && "$1" != --* ]]; then
  REAL_CORPUS_DIR="$1"
  shift
fi

while [[ $# -gt 0 ]]; do
  case "$1" in
    --pii-denylist)
      if [[ $# -lt 2 ]]; then
        echo "ERROR: --pii-denylist requires a PATH argument." >&2
        exit 1
      fi
      PII_DENYLIST="$2"
      shift 2
      ;;
    *)
      echo "ERROR: unknown option '$1'" >&2
      echo "Usage: $0 <REAL_CORPUS_DIR> [--pii-denylist <PATH>]" >&2
      exit 1
      ;;
  esac
done

# Fall back to the environment variable when no positional argument was given.
REAL_CORPUS_DIR="${REAL_CORPUS_DIR:?Set REAL_CORPUS_DIR to your corpus directory}"
# Export so the Python heredoc below (and the per-client extract subshells) can
# read it via os.environ. Without this, the heredoc at line ~83 sees None and
# os.path.join() crashes with a TypeError before any industry runs.
export REAL_CORPUS_DIR

PRIORS_DIR="crates/datasynth-generators/resources/priors"
INTERMEDIATE_DIR="/tmp/sp2-per-client-priors"
DATASYNTH_BIN="./target/release/datasynth-data"

if [[ -z "$PII_DENYLIST" ]]; then
  echo "warning: --pii-denylist not supplied; Phase B (fuzzy proper-noun" >&2
  echo "         generalization) is skipped. Bundles may carry residual fuzzy" >&2
  echo "         PII that the post-regen audit gate will reject." >&2
fi

if [[ ! -x "$DATASYNTH_BIN" ]]; then
  echo "Building release binary..."
  cargo build --release -p datasynth-cli
fi

mkdir -p "$INTERMEDIATE_DIR" "$PRIORS_DIR"

# Map Client Id -> industry via the global selection parquet.
python3 - <<'PY'
import json
import pyarrow.parquet as pq
import os
import sys
real_dir = os.environ.get("REAL_CORPUS_DIR")
path = os.path.join(real_dir, "ADD_Client_Selection_Global.parquet")
table = pq.read_table(path).to_pandas()
mapping = {}
for _, row in table.iterrows():
    industry = str(row["Industry"]).strip().lower()
    industry = industry.replace(" ", "_").replace("&", "and")
    industry = industry.replace("__", "_").replace(",", "")
    mapping[str(row["Client Id"])] = industry
with open("/tmp/sp2-client-industries.json", "w") as f:
    json.dump(mapping, f)
PY

export REAL_CORPUS_DIR

# ---------------------------------------------------------------------------
# Build the --pii-denylist passthrough array used in each extract call.
# ---------------------------------------------------------------------------
DENYLIST_ARGS=()
if [[ -n "$PII_DENYLIST" ]]; then
  DENYLIST_ARGS+=("--pii-denylist" "$PII_DENYLIST")
fi

INDUSTRIES=(health life_sciences pharmaceutical technology power_and_utilities)
for industry in "${INDUSTRIES[@]}"; do
  echo "=== Industry: $industry ==="
  per_client_dsfs=()
  for client_id in $(python3 -c "
import json
m = json.load(open('/tmp/sp2-client-industries.json'))
for k, v in m.items():
    if v == '$industry':
        print(k)
"); do
    parquet="$REAL_CORPUS_DIR/JE_${client_id}.parquet"
    if [[ ! -f "$parquet" ]]; then
      echo "  (no $parquet -- skipping)"
      continue
    fi
    out="$INTERMEDIATE_DIR/JE_${client_id}.behavioral.dsf"
    echo "  Extracting $parquet -> $out"
    "$DATASYNTH_BIN" fingerprint extract \
      --input "$parquet" \
      --output "$out" \
      --behavioral \
      --industry "$industry" \
      "${DENYLIST_ARGS[@]}"
    per_client_dsfs+=("$out")
  done
  if [[ ${#per_client_dsfs[@]} -lt 3 ]]; then
    echo "  Skipping $industry: only ${#per_client_dsfs[@]} clients (<3)"
    continue
  fi
  out_bundle="$PRIORS_DIR/industry_priors_${industry}.dsf"
  echo "  Aggregating ${#per_client_dsfs[@]} client priors -> $out_bundle"
  "$DATASYNTH_BIN" fingerprint aggregate-industry \
    --industry "$industry" \
    --inputs "${per_client_dsfs[@]}" \
    --output "$out_bundle"
done

echo
echo "Done. Committed bundles:"
ls -la "$PRIORS_DIR"/industry_priors_*.dsf 2>/dev/null || echo "(no bundles produced)"

# ---------------------------------------------------------------------------
# SP6 — build-time residual-PII audit gate.
# The freshly regenerated bundles must carry zero residual PII before they
# are considered valid for commit. This test is the T13 bundle_pii_audit
# integration test in datasynth-runtime.
# ---------------------------------------------------------------------------
echo
echo "==> SP6 residual-PII audit on regenerated bundles"
if ! cargo test -p datasynth-runtime --test bundle_pii_audit -- --nocapture; then
  echo "ERROR: regenerated bundles failed the residual-PII audit — aborting." >&2
  echo "       Fix the extraction (extend --pii-denylist or patch Phase A rules)" >&2
  echo "       and re-run before committing." >&2
  exit 1
fi
echo "==> SP6 residual-PII audit passed."
