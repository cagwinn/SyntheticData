#!/bin/bash
# check_template_drift.sh (v3.2.1+) — CI gate for template rewire completeness.
#
# Fails if:
#   1. Any customer-visible hardcoded name/description pool reappears in
#      crates/datasynth-generators/src/ (grep check).
#   2. CLAUDE.md's asserted crate count diverges from `cargo metadata`.
#
# This script is NOT exhaustive — ProcessIssueType enum, control codes,
# account-description formatters, and other internal-only constants stay
# allowed because they never surface as user-visible generator output.
# Additions to the allow-list below must come with justification.

set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

fail=0

# ---- 1. Hardcoded customer-visible pool detector ---------------------------

# Patterns to flag: `const IDENT_NAMES: &[&str]` or `const IDENT_DESCRIPTIONS: &[…]`
# inside crates/datasynth-generators/src/ — the public, customer-visible
# string pools users care about overriding.
pattern='^const[[:space:]]+[A-Z_]+_(NAMES|DESCRIPTIONS)[[:space:]]*:[[:space:]]*&'

# Allowed surviving pools after v3.2.1 — add with justification only.
#
# v3.2.1 policy: every pool listed here is either (a) a master-data
# rewired fallback (kept for byte-identical default behavior, see
# CHANGELOG v3.2.0) or (b) a generator NOT in Track A P1 scope —
# these 18 pools stay hardcoded until their respective generator is
# rewired (v3.3.0 for it_controls / prior_year / standards; v3.3.0
# for manufacturing descriptions). New pools introduced by a PR MUST
# be added here with a justification comment, or the PR must route
# through TemplateProvider instead.
allowlist=(
    # v3.2.0 / v3.2.1 rewired master-data fallbacks
    "crates/datasynth-generators/src/master_data/vendor_generator.rs:BANK_NAMES"
    "crates/datasynth-generators/src/master_data/material_generator.rs:MATERIAL_DESCRIPTIONS"
    "crates/datasynth-generators/src/master_data/asset_generator.rs:ASSET_DESCRIPTIONS"
    # Not-yet-rewired — scheduled per roadmap v3.3.0
    "crates/datasynth-generators/src/prior_year_generator.rs:FINDING_DESCRIPTIONS"
    "crates/datasynth-generators/src/it_controls_generator.rs:CONFIG_CHANGE_DESCRIPTIONS"
    "crates/datasynth-generators/src/it_controls_generator.rs:CODE_DEPLOYMENT_DESCRIPTIONS"
    "crates/datasynth-generators/src/it_controls_generator.rs:PATCH_DESCRIPTIONS"
    "crates/datasynth-generators/src/it_controls_generator.rs:ACCESS_CHANGE_DESCRIPTIONS"
    "crates/datasynth-generators/src/it_controls_generator.rs:EMERGENCY_FIX_DESCRIPTIONS"
    "crates/datasynth-generators/src/standards/revenue_recognition_generator.rs:CUSTOMER_NAMES"
    "crates/datasynth-generators/src/standards/revenue_recognition_generator.rs:GOOD_DESCRIPTIONS"
    "crates/datasynth-generators/src/standards/revenue_recognition_generator.rs:SERVICE_DESCRIPTIONS"
    "crates/datasynth-generators/src/standards/revenue_recognition_generator.rs:LICENSE_DESCRIPTIONS"
    "crates/datasynth-generators/src/standards/revenue_recognition_generator.rs:SERIES_DESCRIPTIONS"
    "crates/datasynth-generators/src/standards/revenue_recognition_generator.rs:WARRANTY_DESCRIPTIONS"
    "crates/datasynth-generators/src/standards/revenue_recognition_generator.rs:MATERIAL_RIGHT_DESCRIPTIONS"
    "crates/datasynth-generators/src/standards/revenue_recognition_generator.rs:VC_DESCRIPTIONS"
    "crates/datasynth-generators/src/standards/business_combination_generator.rs:ACQUIREE_NAMES"
    "crates/datasynth-generators/src/manufacturing/production_order_generator.rs:OPERATION_DESCRIPTIONS"
    "crates/datasynth-generators/src/manufacturing/quality_inspection_generator.rs:CHARACTERISTIC_NAMES"
    "crates/datasynth-generators/src/manufacturing/bom_generator.rs:COMPONENT_DESCRIPTIONS"
    # v3.3.1 — new with LeaseGenerator; TemplateProvider rewire
    # scheduled for v3.4+ alongside the other standards generators.
    "crates/datasynth-generators/src/standards/lease_generator.rs:LESSOR_NAMES"
)

echo "=== Hardcoded name/description pools in datasynth-generators/ ==="
matches=$(grep -rEn --include='*.rs' "$pattern" \
    crates/datasynth-generators/src/ || true)

new_pools=0
if [ -n "$matches" ]; then
    while IFS= read -r line; do
        # line format: path:line:content
        path="${line%%:*}"
        rest="${line#*:}"
        # Extract the const name from the matched content
        const_name=$(echo "$rest" | grep -oE 'const[[:space:]]+[A-Z_]+_(NAMES|DESCRIPTIONS)' | awk '{print $2}')
        key="${path}:${const_name}"
        allowed=0
        for entry in "${allowlist[@]}"; do
            if [ "$key" = "$entry" ]; then
                allowed=1
                break
            fi
        done
        if [ "$allowed" -eq 0 ]; then
            echo "  ✗ NEW customer-visible pool: $key"
            echo "    Context: $line"
            new_pools=$((new_pools + 1))
        else
            echo "  ✓ known fallback: $key"
        fi
    done <<< "$matches"
fi

if [ "$new_pools" -gt 0 ]; then
    echo "Found $new_pools new customer-visible pool(s); add via TemplateProvider or extend the allowlist with justification."
    fail=1
else
    echo "OK — no new customer-visible pools introduced."
fi

# ---- 2. CLAUDE.md crate count vs cargo metadata ---------------------------

echo
echo "=== CLAUDE.md crate count vs cargo metadata ==="

claimed=$(grep -oE '[0-9]+ active crates' CLAUDE.md | head -1 | grep -oE '[0-9]+' || echo "")
if [ -z "$claimed" ]; then
    echo "  ✗ CLAUDE.md no longer contains 'N active crates' claim — please re-add or update this gate."
    fail=1
else
    # Count workspace members excluding the one we know is intentionally excluded.
    actual=$(find crates -maxdepth 2 -name Cargo.toml | wc -l | tr -d ' ')
    # The graph-export crate lives under crates/ but is excluded; subtract if present.
    if [ -f crates/datasynth-graph-export/Cargo.toml ]; then
        actual=$((actual - 1))
    fi
    echo "  CLAUDE.md claims: $claimed active crates"
    echo "  Filesystem has:  $actual active (workspace-member) crates"
    if [ "$claimed" != "$actual" ]; then
        echo "  ✗ CLAUDE.md crate count ($claimed) disagrees with filesystem ($actual)"
        fail=1
    else
        echo "  ✓ CLAUDE.md crate count matches filesystem"
    fi
fi

exit "$fail"
