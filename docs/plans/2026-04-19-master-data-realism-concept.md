# Master-Data Realism: User Templates + LLM Enrichment — Concept

Status: **Concept / brainstorm** — no code changes proposed yet.
Owner: TBD.
Related: `crates/datasynth-core/src/templates/`, `crates/datasynth-generators/src/master_data/`, `crates/datasynth-config` (`TemplateConfig`).

## 1. Problem

Text emitted by generators (vendor names, customer names, material/asset
descriptions, employee names, audit-finding narratives, JE headers/lines,
bank names, department names, etc.) is a mix of:

- **File-based templates** that already work (names by culture, header/line
  text via `TemplateProvider::from_file` / `from_directory`).
- **Hardcoded `&'static [&str]` arrays** inside individual generators that
  bypass the template system.

The goal is to reach two capabilities *without* regressing determinism,
throughput, or offline operation:

- **(a)** A user can supply their own templates (industry/region/tenant-
  specific names, descriptions, tone) and have them picked up automatically.
- **(b)** An LLM (or similar) can *optionally* enrich / extend the corpus so
  realism scales beyond what any curated list can cover, with strict
  guardrails for reproducibility and privacy.

## 2. What already exists (reuse, don't rebuild)

| Capability | Location | Status |
|---|---|---|
| `TemplateProvider` trait (names, vendor/customer names, material/asset descriptions, line text, header text) | `datasynth-core/src/templates/provider.rs:21` | Complete. Dyn-compatible. Consumed by JE, description, name generators. |
| `TemplateData` YAML/JSON schema + `TemplateLoader` | `datasynth-core/src/templates/loader.rs:157` | Complete. Supports `Replace`, `Extend`, `MergePreferFile` merge strategies. |
| `DefaultTemplateProvider::from_file` / `from_directory` | `datasynth-core/src/templates/provider.rs:83-92` | Complete. Directory auto-merges all `.yaml/.yml/.json`. |
| `MultiCultureNameGenerator` (7 cultures: Western US, German, French, Chinese, Japanese, Indian, Hispanic) | `datasynth-core/src/templates/names.rs` | Complete. Culture-weighted sampling via `CultureDistribution`. |
| `realism/` submodule: `VendorNameGenerator`, `CompanyNameGenerator`, `AddressGenerator`, `DescriptionVariator`, `TypoGenerator`, `EnhancedReferenceGenerator`, `UserIdGenerator` | `datasynth-core/src/templates/realism/` | Complete, 8.4 kLoC. Already industry-aware. |
| `TemplateConfig` in workspace config | `datasynth-config/src/schema.rs:2003` (`templates: TemplateConfig`) | **Gap:** only exposes `names`, `descriptions`, `references` toggles — **no path** to a template file/directory. |
| HTTP client | `reqwest` already in workspace deps | Available. |
| LLM SDKs | none | Not integrated. |

## 3. What's still hardcoded (top rewire targets)

From an audit of `datasynth-generators/`:

| # | Site | Lines | Count |
|---|---|---|---|
| 1 | `master_data/customer_generator.rs` `CUSTOMER_NAME_TEMPLATES` | 302–407 | 8 industries × 8 names |
| 2 | `audit/finding_generator.rs` finding titles | 349–451 | 7 severities × 3–5 titles |
| 3 | `audit/finding_generator.rs` condition/criteria/cause/effect + recommendations | 457–575 | ~18 templates |
| 4 | `master_data/material_generator.rs` `MATERIAL_DESCRIPTIONS` | 57–136 | 6 types × 8 descs |
| 5 | `master_data/asset_generator.rs` `ASSET_DESCRIPTIONS` | 74–164 | 7 classes × 6–8 descs |
| 6 | `master_data/vendor_generator.rs` `BANK_NAMES` | 242–253 | 10 banks |
| 7 | `master_data/employee_generator.rs` department names / job-title defaults | 88–186 | 5 depts + roles |
| 8 | `document_flow/document_flow_je_generator.rs` account-description formatting | 148+ | config-driven but static |
| 9 | `templates/names.rs` embedded culture name pools | 113–1303 | 7 cultures × ~200 each |
| 10 | `banking/` customer-name generation (banking customers are generated separately from master_data customers) | — | needs audit |

Key observation: targets 1–7 should all flow through `TemplateProvider`
already — the generators just bypass it. Most of the lift is **plumbing**,
not new design.

## 4. Proposed two-track architecture

### Track A — User-supplied templates (ship first, low risk)

```
┌─────────────┐
│ user YAML/  │   templates.path: "./my_templates"
│ JSON files  │──►──┐
└─────────────┘     │
┌─────────────┐     ▼
│ embedded    │──► TemplateLoader::merge ──► DefaultTemplateProvider ──► all generators
│ defaults    │    (Replace|Extend|MergePreferFile)
└─────────────┘
```

**Concrete deltas:**

1. Extend `TemplateConfig` (datasynth-config/src/schema.rs:2003) with:
   ```yaml
   templates:
     path: ./templates              # file or directory, optional
     merge_strategy: extend         # extend | replace | merge_prefer_file
     # existing: names, descriptions, references
   ```
2. In the runtime bootstrap (where `DefaultTemplateProvider::new()` is
   currently instantiated — search for `default_provider()` / `Arc::new(...)`
   at generator construction), branch on `templates.path` and call
   `from_file` or `from_directory` with the configured merge strategy.
3. **Rewire every hardcoded array in §3 to go through `TemplateProvider`.**
   The trait already has `get_customer_name`, `get_material_description`,
   `get_asset_description`; extend it only for new categories that don't fit
   (e.g. `get_bank_name`, `get_finding_title(severity, finding_type)`,
   `get_finding_narrative(section, severity)`, `get_department_name(locale)`).
4. Ship a **template pack directory** under `templates/` at repo root with
   the embedded defaults exported as YAML — so the same pack is also the
   canonical example users copy and edit. One file per
   category (`names.yaml`, `vendors.yaml`, `customers.yaml`,
   `materials.yaml`, `assets.yaml`, `audit_findings.yaml`, `line_text.yaml`,
   `header_text.yaml`, `banking.yaml`).
5. CLI surface: `datasynth-data templates export --output ./templates`
   (dump defaults), `datasynth-data templates validate <path>` (reuse the
   existing `TemplateLoader::validate`), `datasynth-data generate --templates
   ./my_templates ...` (shortcut flag overriding config).
6. Locale/industry overlays: a template file's `metadata.region` and
   `metadata.sector` are already declared but unused. The loader should
   honor an overlay rule: `base → industry overlay → region overlay → user
   overlay` with `Extend` by default, `MergePreferFile` at the last step.

This track unlocks (a) entirely and also **improves (b)**, because an LLM
pipeline is easier to bolt on once every text emission goes through one
trait.

### Track B — LLM / model-based enrichment (offline-first, deterministic)

Two modes, deliberately **not** live per-record inference in the hot path:

**B1. Offline corpus expansion (ship second).**
An `enrich-templates` CLI subcommand that:
  - takes an existing template pack and a prompt spec (per category),
  - calls an LLM (Anthropic/OpenAI/local) via a trait we introduce,
  - appends novel, de-duplicated entries to the corpus with provenance
    metadata,
  - writes back YAML files the runtime already loads.

Runtime stays **100% offline and deterministic** — the LLM never runs
inside `cargo run`. Provenance is stored so re-runs can regenerate the
same pack given the same prompt + seed + model version.

```
templates/base.yaml ──► enrich-templates ──► templates/enriched.yaml ──► generate (offline)
                           ▲
                           │ prompts/vendor_names.md, prompts/finding_titles.md
                           │ LlmClient trait (Anthropic | OpenAI | Ollama)
```

**B2. Live enrichment (optional, opt-in, guardrailed).**
A cache-backed `LlmTemplateProvider` that wraps `DefaultTemplateProvider`
and, on a cache miss for a key like `(category=customer, industry=retail,
region=DE)`, calls the LLM for N candidates, persists them to disk
(`~/.cache/datasynth/llm/*.yaml`), and seeds them into the provider.
Second run is fully offline.

Guardrails:
- **Determinism**: runtime `rng` samples from the populated pool; the LLM
  only grows the pool. Re-runs with the same seed are byte-identical as
  long as the cache exists.
- **Privacy**: no customer/tenant data is ever sent. Only the
  *category descriptor* (industry, region, material type) — all
  enumerated, never user-supplied free text.
- **PII scrub**: post-filter LLM output against a blocklist of real
  company-name patterns, celebrity names, etc., before caching.
- **Rate and cost caps**: config-level max calls/run, max tokens/call.
- **Air-gap by default**: B2 is off unless `templates.llm.enabled: true`.
- **Reproducibility**: cache files are committed-or-shared artifacts. The
  cache key includes model ID + prompt hash so upgrading the model
  invalidates cleanly.

**Proposed trait (thin, Claude-native defaults):**

```rust
// datasynth-core/src/templates/llm.rs (new)
pub trait LlmClient: Send + Sync {
    fn generate(
        &self,
        prompt: &LlmPrompt,       // category + context + n + constraints
    ) -> Result<Vec<String>, LlmError>;
}

pub struct LlmPrompt {
    pub category: TemplateCategory,   // enum: CustomerName, VendorName, MaterialDesc, ...
    pub industry: Option<String>,
    pub region:   Option<String>,
    pub count:    usize,
    pub style:    Option<String>,     // e.g., "formal", "B2B SaaS", "industrial"
    pub constraints: Vec<String>,     // e.g., "avoid real brands", "≤ 40 chars"
}
```

Backends (feature-gated):
- `claude-sdk` (default): Anthropic SDK via `claude_api` skill / `anthropic`
  crate — use Claude Haiku 4.5 for cost, Claude Sonnet 4.6 for quality on
  tricky categories. Enable prompt caching (the repeated system prompt +
  category schema is the same across calls).
- `openai`: feature flag.
- `ollama` / `llamacpp`: local, no-network fallback — important for
  air-gapped users.
- `mock`: deterministic stub for tests (and for `enrich-templates`
  dry-run).

Crate placement: put `LlmClient` + `LlmTemplateProvider` in
`datasynth-core/src/templates/llm.rs` but keep the actual backends behind
`--features llm-claude`, `--features llm-openai`, `--features llm-ollama`
so the default build has zero new runtime deps.

## 5. Out-of-scope / non-goals

- **No per-record LLM calls in the hot path.** 200K entries/sec
  single-threaded is a headline number; synchronous LLM calls would
  destroy it.
- **No new data-quality signals** from the LLM (e.g., the LLM does not
  generate fraud indicators — those stay in `fraud_bias.rs` and
  `anomaly/`).
- **No change to deterministic RNG semantics** (ChaCha8 + seed) —
  verified by existing smoke tests.
- **No LLM validation of numerical fields** — amounts, dates, balances
  remain under `distributions/` and `balance/` ownership.

## 6. Phased rollout

| Phase | Scope | Exit criterion |
|---|---|---|
| P0 | Wire `templates.path` into `TemplateConfig`; runtime loads user templates via existing `DefaultTemplateProvider::from_directory`. Add `datasynth-data templates export|validate`. | `generate --templates ./mine` swaps vendor/customer/material/asset names. No generator code changes yet. |
| P1 | Rewire top-7 hardcoded sites (§3 #1–#7) through `TemplateProvider`. Extend trait for bank names, finding titles, finding narratives, department names. Export embedded defaults as YAML under `templates/`. | `grep` for `const .*NAMES.*&\[&str\]` in `datasynth-generators/` returns only non-customer-visible cases. |
| P2 | Industry/region overlay loader (`base → industry → region → user`). Ship `templates/packs/{retail,manufacturing,financial_services,healthcare,technology}/{us,de,gb,fr,jp}/*.yaml`. | `--industry manufacturing --region de` visibly changes names without a user pack. |
| P3 | `datasynth-data enrich-templates` (offline LLM corpus expansion, B1). `LlmClient` trait + Claude backend + mock backend. Feature-gated. | CI job runs `enrich-templates --backend mock` and diffs YAML deterministically. |
| P4 | Optional `LlmTemplateProvider` (live B2) with on-disk cache. Off by default. | Cache hit is byte-deterministic across runs. |

P0 + P1 can ship in one or two PRs and deliver most of the user-visible
realism win. P3/P4 are independent.

## 7. Open questions

- Should the embedded default pools be **deleted** once exported to YAML
  (source of truth = YAML) or **kept** as a fallback (source of truth =
  code)? Current code keeps both in `names.rs`. Recommendation: move to
  YAML-as-source-of-truth with a `build.rs` that compiles them in as
  `include_str!` so there's no runtime I/O cost for the default pack.
- For LLM backends, do we depend on the official `anthropic` Rust crate or
  write a thin `reqwest`-based client? A thin client avoids pulling in a
  new top-level dependency and mirrors what `fingerprint/` already does
  for its own HTTP needs.
- Do finding narratives (§3 #3) need structure-aware templating (slots
  like `{account}`, `{period}`, `{amount}`) or are flat pools enough? The
  existing `HeaderTextPattern` in `descriptions.rs` already supports
  placeholders — extend rather than invent.
- Banking-module customer names (`datasynth-banking`) duplicate some of
  the master-data customer logic. Worth consolidating through
  `TemplateProvider` or keeping separate for AML realism?

## 8. Recommendation

Start with **P0 + P1** (Track A). The existing `TemplateProvider` and
`TemplateLoader` already solve 80 % of the problem; the remaining work
is mostly **deleting `const …: &[&str]` arrays and routing calls through
the trait**, plus one config field and a CLI export helper. Track B
(LLM) becomes a natural extension once every text emission is on the
trait.
