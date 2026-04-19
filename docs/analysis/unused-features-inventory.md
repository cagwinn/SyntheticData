# Unused / Partially-Wired Features Inventory

**Scope**: Rust workspace at the repository root, analyzed on branch
`claude/analyze-unused-features-fy9qk`.

**Goal**: Catalogue code that exists in the workspace but does not participate
in the default generation pipeline, so users (and future maintainers) can
distinguish *shipped capability* from *scaffolding* when reading the source
tree or the CLAUDE.md architecture overview.

**Methodology**: Every finding below was confirmed by grepping for the symbol's
name across `crates/` and observing that its only references are either
(a) inside its own module, (b) inside its own unit tests, (c) in documentation,
or (d) re-exports — but **not** in `datasynth-runtime`, `datasynth-cli`, or
`datasynth-server` production call paths.

---

## 1. Reader's Guide

### 1.1 The two orchestrators

There are **two** generation orchestrators in the codebase. They are **not**
equivalent:

| Orchestrator | File | Phases | Used by |
|---|---|---|---|
| `GenerationOrchestrator` (basic) | `crates/datasynth-runtime/src/orchestrator.rs` | 2 (CoA + JE) | No production caller |
| `EnhancedOrchestrator` | `crates/datasynth-runtime/src/enhanced_orchestrator.rs` | ~30 phases | CLI, server, Python wrapper |

The **basic** orchestrator is a minimal CoA→JE pipeline kept around for tests
and legacy call sites. The `datasynth-cli` `generate` subcommand, the server's
`/api/generate/bulk` endpoint, and the Python wrapper all route through
`EnhancedOrchestrator`. If a generator is not invoked from
`enhanced_orchestrator.rs`, it is effectively dormant from the user's point of
view.

Throughout this document, "wired" means "reachable from
`EnhancedOrchestrator::generate()` with some combination of configuration
flags". "Not wired" means the generator is defined, compiled, and unit-tested,
but never instantiated by the orchestrator, CLI, or server.

### 1.2 Severity levels

- **L1 — Not wired**: generator/feature exists, no production call site.
- **L2 — Partially wired**: called, but a subset of its config / capability is
  ignored (stub, feature-gated, or hardcoded).
- **L3 — Config schema without effect**: YAML section parses and validates,
  but no generator actually reads the parsed values.
- **L4 — Orphaned crate**: compiled standalone but excluded from the workspace
  or unreferenced outside its own tests.
- **L5 — Documentation drift**: source doc comments or CLAUDE.md disagree with
  the actual wiring state.

---

## 2. L1 — Generators Defined But Never Invoked

Each of the generators below lives in `crates/datasynth-generators/src/` and
is re-exported from that crate's `lib.rs`, but no call site exists in
`datasynth-runtime`, `datasynth-cli`, or `datasynth-server`. Unit tests inside
the generator's own file are the only exercises.

| Generator | File | Model(s) emitted | Re-exported at |
|---|---|---|---|
| `PriorYearGenerator` (WI-2) | `crates/datasynth-generators/src/prior_year_generator.rs:169` | `PriorYearComparative`, `prior_year` fields | `lib.rs:160` |
| `IndustryBenchmarkGenerator` (WI-3) | `crates/datasynth-generators/src/industry_benchmark_generator.rs:228` | `IndustryBenchmark` | `lib.rs:145` |
| `ItControlsGenerator` (WI-4) | `crates/datasynth-generators/src/it_controls_generator.rs:106` | `AccessLog`, `ChangeManagementRecord` | `lib.rs:154` |
| `OrganizationalProfileGenerator` (WI-6) | `crates/datasynth-generators/src/organizational_profile_generator.rs:203` | `OrganizationalProfile` | `lib.rs:151` |
| `ManagementReportGenerator` (WI-7) | `crates/datasynth-generators/src/management_report_generator.rs:86` | management-report artifacts | `lib.rs:157` |
| `LegalDocumentGenerator` | `crates/datasynth-generators/src/legal_document_generator.rs:118` | `LegalDocument` | `lib.rs` (via `legal_document_generator::*`) |
| `DriftEventGenerator` | `crates/datasynth-generators/src/drift_event_generator.rs:47` | `LabeledDriftEvent` | `lib.rs:134` |

**Verification**: grep for each generator's type name across the runtime
yields zero non-test hits (see e.g. searches for `PriorYearGenerator`,
`ItControlsGenerator`, `DriftEventGenerator`). Existing references inside
`CHANGELOG.md` and the executive-overview LaTeX document describe the models
as if shipped — that is documentation drift (see §6).

**Impact**: Users configuring the pipeline cannot enable these outputs. The
corresponding model types in `datasynth-core/src/models/` (`prior_year.rs`,
`industry_benchmark.rs`, `organizational_profile.rs`, `legal_document.rs`,
`drift_events.rs`, IT-controls models) are never populated at runtime.

**Note — work-item labels (WI-2..WI-7)**: the comments in
`datasynth-generators/src/lib.rs:39-52` mark these as work items, suggesting
they are tracked for future integration. The code is in-tree but pending
orchestrator wiring.

---

## 3. L1 — Accounting Standards: Types Without Generators

`datasynth-standards` defines thorough type systems for several accounting
standards, but only a subset has a corresponding generator. Outputs labelled
"n/a" are never produced.

| Standard | Types | Generator | Output |
|---|---|---|---|
| Revenue recognition (ASC 606 / IFRS 15) | `CustomerContract`, `PerformanceObligation`, … | `RevenueRecognitionGenerator` (wired at `enhanced_orchestrator.rs:6942`) | `accounting_standards/customer_contracts` |
| Impairment (ASC 360 / IAS 36) | `ImpairmentTest`, … | `ImpairmentGenerator` (wired at `enhanced_orchestrator.rs:6973`) | `accounting_standards/impairment_tests` |
| ECL / CECL | `EclModel`, `EclProvisionMovement` | `EclGenerator` (wired at `enhanced_orchestrator.rs:7029`) | `accounting_standards/ecl_*` |
| Provisions / contingent liabilities | `Provision`, `ContingentLiability` | `ProvisionGenerator` (wired at `enhanced_orchestrator.rs:7107`) | `accounting_standards/provisions` |
| Business combinations | `BusinessCombination`, `PurchasePriceAllocation` | `BusinessCombinationGenerator` (wired at `enhanced_orchestrator.rs:6998`) | `accounting_standards/business_combinations` |
| **Leases (ASC 842 / IFRS 16)** | `Lease`, `ROUAsset`, `LeaseLiability` | **none** | **n/a** |
| **Fair value (ASC 820 / IFRS 13)** | `FairValueMeasurement`, `ValuationInput` | **none** | **n/a** |
| **Framework reconciliation** | `FrameworkReconciliation`, `ReconcilingItem` | **none** | **n/a** |

**Verification**: grep for `LeaseGenerator`, `FairValueGenerator`,
`lease_generator`, `fair_value_generator` returns no matches in the entire
workspace.

**Config gap**: `accounting_standards.leases` and
`accounting_standards.fair_value` YAML blocks are defined in
`datasynth-config/src/schema.rs` and validated in tests (e.g.
`audit_preset_test.rs`), but no generator reads them. Setting
`accounting_standards.leases.enabled = true` in a user config is a no-op.

---

## 4. L2 — Partially Wired Features

### 4.1 Neural diffusion backend is a stub

`EnhancedOrchestrator` exposes three diffusion backends via
`config.diffusion.backend`: `statistical`, `neural`, `hybrid`. Only the
**statistical** backend actually generates samples.

- `crates/datasynth-runtime/src/enhanced_orchestrator.rs:3207-3248` — when
  `backend == "neural"` or `"hybrid"`, the orchestrator validates the config,
  logs the intent, and records statistics
  (`neural_hybrid_weight`, `neural_hybrid_strategy`,
  `neural_routed_column_count`). **No training or inference occurs.** The
  closing comment at line 3241-3247 explicitly notes: *"Neural enhancement
  integrates via the DiffusionBackend trait: 1. NeuralDiffusionTrainer::train
  on generated amounts, 2. HybridGenerator blends rule-based + neural at
  configured weight, 3. TabularTransformer for conditional column prediction,
  4. GnnGraphTrainer for entity relationship structure. Actual training
  requires the `neural` cargo feature on datasynth-core. The orchestrator
  delegates to the diffusion module which is feature-gated."* — but no such
  delegation call is present in the function body.
- `crates/datasynth-runtime/src/enhanced_orchestrator.rs:4553-4599` —
  `phase_diffusion_enhancement()` instantiates `StatisticalDiffusionBackend`
  only. Types such as `NeuralDiffusionTrainer`, `HybridGenerator`,
  `TabularTransformer`, `GnnGenerator`, `GnnGraphTrainer`, and
  `TrainedGnnGenerator` (all defined under
  `crates/datasynth-core/src/diffusion/`) are gated behind
  `#[cfg(feature = "neural")]` (see `diffusion/mod.rs:21-49`) and are **not**
  called from any runtime path — even when the `neural` feature is enabled,
  the orchestrator never invokes them.

**User-visible effect**: setting `diffusion.backend: neural` or `hybrid` in
YAML produces the same output as `statistical`, plus a log line describing
what *would* have been done. Statistics fields such as
`neural_hybrid_weight` are populated for observability but do not reflect an
actual neural pass.

### 4.2 Audit fine-grained config fields are ignored

`AuditGenerationConfig` in `crates/datasynth-config/src/schema.rs:4013-4036`
self-documents five fields as `[Not yet wired]`:

- `generate_workpapers` (line 4016)
- `engagement_types` (line 4021)
- `workpapers` (line 4026)
- `team` (line 4031)
- `review` (line 4036)

`audit.enabled = true` drives the full audit generation path, but these
sub-flags have no effect on the output. Users who configure e.g.
`audit.team.partner_count = 3` or `audit.review.review_rounds = 2` will see
the audit generator use its internal defaults regardless.

### 4.3 Basic orchestrator is unused

`crates/datasynth-runtime/src/orchestrator.rs` defines
`GenerationOrchestrator` with only two phases (CoA construction + JE
generation). No production call site references it — the CLI, server, and
Python wrapper all use `EnhancedOrchestrator`. It remains compiled and
exposed as `pub` API surface, but is effectively a legacy stub. Deciding
whether to delete it or document it as "minimal demo" is a separate
question.

### 4.4 `datasynth-audit-optimizer` is CLI-only, not part of the pipeline

`datasynth-audit-optimizer` is a workspace member with a rich API
(`discovery`, `portfolio`, `risk_scoping`, `resource_optimizer`,
`conformance`, `calibration`, `overlay_fitting`, `benchmark_comparison`,
`monte_carlo`, `shortest_path`, `graph`, `group_audit`, `yoy_chain`). Its
only production reference is in `crates/datasynth-cli/src/main.rs:3402`,
where `discovery::compare_blueprints` and `discovery::discover_blueprint`
back the `blueprint diff` CLI subcommand. Everything else — resource
optimization, risk scoping, Monte Carlo, portfolio planning, Wave-2/3/4
flows — is exercised only by that crate's own test suite
(`wave2_e2e.rs`, `wave3_e2e.rs`, `wave4_e2e.rs`, `big4_benchmark.rs`,
`wave2_evaluation.rs`).

**Impact**: users of `datasynth-data generate` never get optimizer output.
The optimizer is reachable only through specific `datasynth-data` audit
subcommands and not surfaced in the generation pipeline.

### 4.5 LLM enrichment is wired but requires explicit config + env

`phase_llm_enrichment` (`enhanced_orchestrator.rs:4458-4545`) runs only when
`config.llm.enabled = true`. The default providers are `mock` and `http`; the
`custom` provider branch at line 4475 expects an environment variable
`LLM_API_KEY`. The phase is feature-gated (`feature = "llm"` on
`datasynth-core`, declared in `datasynth-core/Cargo.toml:34`). Defaults
therefore produce no enrichment — only vendor-name strings pass through
unchanged. This is intentional, but not obvious from the YAML schema.


---

## 5. L3 — Config Schema Without Effect

These top-level YAML sections validate cleanly and appear in the
user-facing schema, but no generator reads the parsed values. Setting
fields under them has no observable effect on the output.

### 5.1 `distributions:` is validated, not applied

- **Schema**: `crates/datasynth-config/src/schema.rs` defines the
  `distributions:` section with sub-blocks for `amounts`, `correlations`,
  `regime_changes`, `validation`, `industry_profile`, etc. — see CLAUDE.md
  for the advertised shape.
- **Only reference in non-test runtime/generator code**:
  `crates/datasynth-config/src/validation.rs:776` (`let dist =
  &config.distributions;`) — validation only.
- **Where amount sampling actually gets its parameters**:
  `crates/datasynth-generators/src/je_generator.rs:219` calls
  `AmountSampler::with_config(seed + 2, config.amounts.clone())`, but that
  `config.amounts` is `TransactionsConfig.amounts`, **not**
  `DistributionsConfig.amounts`. The two are separate structs.

**Effect**: users who configure `distributions.amounts.components` expecting
to steer the JE amount mixture see nothing change. The mixture parameters
actually used come from `transactions.amounts` and from per-generator
defaults. Advanced knobs such as `distributions.regime_changes`,
`distributions.correlations.copula_type`, and
`distributions.industry_profile` are fully inert.

### 5.2 Advanced distribution samplers are implemented but unused

`crates/datasynth-core/src/distributions/` ships five advanced samplers:

| File | Type | Used in runtime? |
|---|---|---|
| `pareto.rs` | `ParetoSampler` | No (only fingerprint crate uses Pareto elsewhere) |
| `weibull.rs` | `WeibullSampler` | No |
| `beta.rs` | `BetaSampler` | No |
| `zero_inflated.rs` | `ZeroInflatedSampler` | No |
| `conditional.rs` | `ConditionalDistribution` | No |
| `copula.rs` | Gaussian/Clayton/Gumbel/Frank/StudentT copulas | No — copulas are used only inside `datasynth-fingerprint` synthesis, not in the main generation path |

**Verification**: grep for each sampler's type across `crates/` excluding
tests shows references only inside their defining module, the
`distributions/mod.rs` re-exports, and `industry_profiles.rs`. The
`CorrelationEngine` (`distributions/correlation.rs`) is similarly
referenced only by the config schema and its own implementation.

**Effect**: the `distributions` config promises copula-driven correlation
and heavy-tailed modelling. In practice, the pipeline uses the simpler
`AmountSampler` / `BenfordSampler` / `GaussianMixtureSampler` /
`LogNormalMixtureSampler` combo in `je_generator.rs`.

### 5.3 Temporal helpers bypass non-JE generators

The following are defined in `crates/datasynth-core/src/distributions/` but
only `je_generator.rs` consumes them:

- `HolidayCalendar` (`holidays.rs`, 15 regions)
- `BusinessDayCalculator` (`business_day.rs`)
- `PeriodEndDynamics` (`period_end.rs`)
- `ProcessingLagCalculator` (`processing_lag.rs`)
- `TimezoneHandler` (`timezone.rs`)
- `TemporalSampler` (`temporal.rs`)

**Verification**: grep over non-test files shows the only cross-crate
consumer is `datasynth-generators/src/je_generator.rs`. Document-flow
generators (`p2p_generator`, `o2c_generator`), HR generators (`payroll`,
`time_entry`), treasury generators (`cash_position`, `cash_forecast`),
manufacturing generators, and FX/period-close generators do **not** consult
the holiday calendar or timezone handler.

**Effect**: documents such as vendor invoices, payroll runs, and
intraday treasury positions do not respect `temporal_patterns.calendars`,
`temporal_patterns.intraday`, or
`temporal_patterns.business_days.settlement_rules` — those configuration
blocks influence JE timing only.

---

## 6. L4 — Orphaned Crate: `datasynth-graph-export`

- `Cargo.toml` at the workspace root lists the crate in the `exclude`
  array: `exclude = ["fuzz", "crates/datasynth-graph-export"]` (line 21).
- The crate has a full implementation (`src/lib.rs`, `src/pipeline.rs`,
  `src/edges/`, `src/nodes/`, `src/properties/`) and its own comprehensive
  test suite (`tests/edge_synthesizers.rs`, `tests/pipeline_smoke.rs`,
  `tests/property_serializers.rs`, `tests/full_pipeline_test.rs`).
- No other crate depends on it. The runtime uses the older
  `datasynth-graph` crate (PyTorch Geometric, Neo4j, DGL exporters) for
  production graph output.

**Effect**: `datasynth-graph-export`'s edge synthesizers
(`DocumentChainEdgeSynthesizer`, `RiskControlEdgeSynthesizer`, etc.) and
property serializers do not participate in the default build or in any
user-invoked command. Running `cargo build` from the workspace root does
not compile this crate.

**Recommendation placeholder**: either include it in the workspace and
plumb it into the orchestrator's graph-export phase, or move it to
`attic/` with an explanatory README. Leaving it compiled-only-in-isolation
is the current state.

---

## 7. L5 — Documentation Drift

These are inaccuracies in docs or source comments that caused confusion
during this analysis; fixing them is low-risk.

### 7.1 CLAUDE.md crate count is stale

CLAUDE.md opens with *"Rust workspace with 15 crates"*. The actual
`Cargo.toml` workspace has **16** active members plus one excluded:

Active members: `datasynth-core`, `datasynth-config`, `datasynth-generators`,
`datasynth-output`, `datasynth-runtime`, `datasynth-cli`, `datasynth-graph`,
`datasynth-server`, `datasynth-test-utils`, `datasynth-eval`,
`datasynth-ocpm`, `datasynth-audit-fsm`, `datasynth-audit-optimizer`,
`datasynth-banking`, `datasynth-fingerprint`, `datasynth-standards`.

Excluded: `datasynth-graph-export` (see §6).

CLAUDE.md's architecture table is missing both `datasynth-audit-fsm` and
`datasynth-audit-optimizer`.

### 7.2 `RevenueGenerator` doc comment is outdated

`crates/datasynth-generators/src/project_accounting/revenue_generator.rs:19`
states *"Not yet wired into the runtime orchestrator"*. In reality the
generator **is** wired at
`crates/datasynth-runtime/src/enhanced_orchestrator.rs:8580`, gated on
`config.project_accounting.revenue_recognition.enabled`. The doc string
should be updated.

### 7.3 `CHANGELOG.md` and exec-overview reference unwired generators

`CHANGELOG.md:1403` announces `DriftEventGenerator` as a shipped feature,
and `docs/datasynth-executive-overview.tex:844` describes it as a
meta-generator producing `LabeledDriftEvent` records. In both cases the
generator exists but is never invoked (see §2). Readers of those
documents will expect drift-labelled output that the pipeline does not
produce.

---

## 8. Summary Table

| Area | Severity | Count | Representative examples |
|---|---|---|---|
| Generators defined, never invoked | L1 | 7 | `PriorYearGenerator`, `ItControlsGenerator`, `DriftEventGenerator` |
| Standards with types but no generator | L1 | 3 | Leases, Fair Value, Framework Reconciliation |
| Neural/hybrid diffusion backend | L2 | 1 | `enhanced_orchestrator.rs:3207-3248` |
| Audit fine-grained config fields | L2 | 5 | `audit.team`, `audit.workpapers`, etc. |
| Basic orchestrator | L2 | 1 | `orchestrator.rs` superseded by `EnhancedOrchestrator` |
| Audit optimizer — only `blueprint diff` | L2 | 1 | `datasynth-audit-optimizer` API ≫ CLI surface |
| LLM enrichment (intentionally opt-in) | L2 | 1 | `phase_llm_enrichment` |
| Config sections without generator | L3 | 1 | `distributions:` |
| Advanced distribution samplers | L3 | 5+ | Pareto, Weibull, Beta, ZeroInflated, Conditional, copulas |
| Temporal helpers | L3 | 6 | Holiday/timezone/business-day used by JE only |
| Orphaned crate | L4 | 1 | `datasynth-graph-export` |
| Doc drift | L5 | 3 | crate count, RevenueGenerator comment, CHANGELOG claims |

---

## 9. Notes for Maintainers

1. **If you want to wire an L1 generator**: find the adjacent phase in
   `enhanced_orchestrator.rs` (e.g. `ItControlsGenerator` belongs near
   the existing control-generation block around line 2286-2312, and
   `ManagementReportGenerator` belongs near the KPI/financial-reporting
   phase around line 7442-7549), add a `PhaseConfig` flag if appropriate,
   and add an output sink in `datasynth-output` plus a row in the
   CLAUDE.md export-files table.

2. **If you want to close an L3 gap**: the cleanest path for the
   `distributions:` section is to thread the parsed
   `DistributionsConfig` through `EnhancedOrchestrator::new()` into the
   generators that currently read `TransactionsConfig.amounts`, then
   deprecate the duplicated fields under `transactions.amounts`.

3. **If you want to delete**: `datasynth-graph-export` and the basic
   `GenerationOrchestrator` are the safest candidates for removal —
   neither has any non-test consumer in the workspace.

4. **What is correctly wired**: the bulk of the workspace is wired.
   Master data, document flows, journal entries, subledgers, period close,
   audit (including ISA 210/315/320/402/520/530/540/560/570/600/700/SOX),
   banking KYC/AML, OCPM, sourcing, HR, manufacturing, tax, ESG,
   treasury, project accounting (including revenue recognition and
   earned-value), intercompany with consolidation, FX translation,
   financial reporting (standalone + consolidated + segment),
   anomaly/quality/fraud injection, control generation with SoD,
   COSO framework, and compliance/regulatory generation all flow through
   `EnhancedOrchestrator::generate()`. This document is an inventory of
   exceptions, not a correction to the overall architecture narrative.

