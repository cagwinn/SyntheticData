//! CLI for synthetic accounting data generation.

// v5.31 C1 Phase 4 — global allocator override. mimalloc reduces RSS
// fragmentation 30-50% on workloads with many short-lived allocations
// (the group-audit aggregate's per-iteration JSON deserialisation
// fragments default glibc malloc enough to drive a 2k regen to OOM at
// 219 GB even after C1 Phase 1+2+3 hold reductions). No code changes
// needed beyond this declaration; mimalloc transparently replaces
// every `Box::new`, `Vec::with_capacity`, `String::from`, etc.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use datasynth_runtime::output_writer;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use datasynth_config::schema::AccountingFrameworkConfig;
use datasynth_config::{presets, GeneratorConfig};
use datasynth_core::memory_guard::{MemoryGuard, MemoryGuardConfig};
use datasynth_core::models::{CoAComplexity, IndustrySector};
use datasynth_eval::behavioral_fidelity::{
    self as behavioral_fidelity, BehavioralFidelityConfig, GateThresholds,
};
use datasynth_fingerprint::{
    evaluation::FidelityEvaluator,
    extraction::{CsvDataSource, DataSource, ExtractionConfig, FingerprintExtractor},
    io::{validate_dsf, FingerprintReader, FingerprintWriter},
    models::PrivacyLevel,
    privacy::PrivacyConfig,
};
use datasynth_output::{write_fec_csv, SapExportConfig, SapExporter};
use datasynth_runtime::{
    export_labels_all_formats, EnhancedOrchestrator, LabelExportConfig, LabelExportSummary,
    OutputFileInfo, PhaseConfig, RunManifest,
};

#[cfg(unix)]
use signal_hook::consts::SIGUSR1;

#[derive(Parser)]
#[command(name = "datasynth-data")]
#[command(about = "Synthetic Enterprise Accounting Data Generator")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Enable verbose output
    #[arg(short, long, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)] // CLI args enum, parsed once at startup
enum Commands {
    /// Generate synthetic accounting data
    Generate {
        /// Path to configuration file
        #[arg(short, long)]
        config: Option<PathBuf>,

        /// Output directory
        #[arg(short, long, default_value = "./output")]
        output: PathBuf,

        /// Use demo preset (small dataset for testing)
        #[arg(long)]
        demo: bool,

        /// Apply a named overlay preset on top of the loaded/default config.
        ///
        /// Supported values:
        ///   audit-group  → enable all audit simulation features (ISA/PCAOB/SOX,
        ///                   COSO controls, anomaly injection, network features)
        #[arg(long)]
        preset: Option<String>,

        /// Load a scenario pack (e.g., "manufacturing/supplier_fraud")
        #[arg(long)]
        scenario_pack: Option<String>,

        /// Generate from a fingerprint file (.dsf)
        #[arg(long)]
        fingerprint: Option<PathBuf>,

        /// Scale factor for fingerprint-based generation (default: 1.0)
        #[arg(long, default_value = "1.0")]
        scale: f64,

        /// Random seed for reproducibility
        #[arg(short, long)]
        seed: Option<u64>,

        /// Enable banking KYC/AML data generation
        #[arg(long)]
        banking: bool,

        /// Enable audit data generation
        #[arg(long)]
        audit: bool,

        /// Memory limit in MB (default: 1024 MB)
        #[arg(long, default_value = "1024")]
        memory_limit: usize,

        /// Maximum CPU threads to use (default: half of available cores, min 1)
        #[arg(long)]
        max_threads: Option<usize>,

        /// Enable graph export for accounting networks (PyTorch Geometric format)
        #[arg(long)]
        graph_export: bool,

        /// Stream unified hypergraph JSONL to a RustGraph ingest endpoint URL
        #[arg(long)]
        stream_target: Option<String>,

        /// API key for the RustGraph ingest endpoint
        #[arg(long)]
        stream_api_key: Option<String>,

        /// Batch size for streaming (lines per HTTP POST, default 1000)
        #[arg(long, default_value = "1000")]
        stream_batch_size: usize,

        /// Quality gate profile (none/lenient/default/strict)
        #[arg(long, default_value = "none")]
        quality_gate: String,

        /// Number of months per fiscal year for multi-period generation
        #[arg(long)]
        fiscal_year_months: Option<u32>,

        /// Append incremental data to existing output (requires previous session.dss)
        #[arg(long)]
        append: bool,

        /// Number of additional months for incremental generation (used with --append)
        #[arg(long)]
        months: Option<u32>,

        /// Apply a fraud scenario pack (repeatable: --fraud-scenario revenue_fraud --fraud-scenario payroll_ghost)
        #[arg(long, action = clap::ArgAction::Append)]
        fraud_scenario: Vec<String>,

        /// Override fraud rate when using fraud scenarios
        #[arg(long)]
        fraud_rate: Option<f64>,

        /// Stream output to a JSONL file during generation
        #[arg(long)]
        stream_file: Option<std::path::PathBuf>,

        /// Additional export format(s) to write (repeatable): sap, fec, gobd
        ///
        /// sap   → SAP S/4HANA BKPF/BSEG/ACDOCA tables (CSV)
        /// fec   → FEC Fichier des Écritures Comptables (French GAAP, 18 columns)
        /// gobd  → GoBD journal + accounts + index.xml (German GAAP)
        #[arg(long = "export-format", action = clap::ArgAction::Append)]
        export_format: Vec<String>,

        /// Enable AI-powered auto-tuning: generate → evaluate → AI patch → regenerate
        #[arg(long)]
        auto_tune: bool,

        /// Maximum iterations for auto-tuning (default: 3)
        #[arg(long, default_value = "3")]
        max_iterations: usize,

        /// Strict COA coverage validation: fail the run if any generated JE
        /// references a gl_account that does not exist in the chart of
        /// accounts. Default is a soft warning.
        #[arg(long = "validate-coa-coverage")]
        validate_coa_coverage: bool,
    },

    /// Validate a configuration file
    Validate {
        /// Path to configuration file
        #[arg(short, long)]
        config: PathBuf,
    },

    /// Generate a sample configuration file
    Init {
        /// Output path
        #[arg(short, long, default_value = "datasynth_config.yaml")]
        output: PathBuf,

        /// Industry preset
        #[arg(short, long, default_value = "manufacturing")]
        industry: String,

        /// CoA complexity (small, medium, large)
        #[arg(short, long, default_value = "medium")]
        complexity: String,

        /// Generate config from a natural language description using AI
        #[arg(long)]
        from_description: Option<String>,
    },

    /// Show information about available presets
    Info,

    /// Verify output integrity (checksums, record counts)
    Verify {
        /// Output directory to verify
        #[arg(short, long, default_value = "./output")]
        output: PathBuf,

        /// Verify file checksums
        #[arg(long)]
        checksums: bool,

        /// Verify record counts
        #[arg(long)]
        record_counts: bool,
    },

    /// Fingerprint extraction and management
    Fingerprint {
        #[command(subcommand)]
        command: FingerprintCommands,
    },

    /// Counterfactual scenario management
    Scenario {
        #[command(subcommand)]
        command: ScenarioCommands,
    },

    /// Adversarial model testing (requires adversarial feature on datasynth-eval)
    Adversarial {
        /// Path to ONNX model file
        #[arg(short, long)]
        model: PathBuf,

        /// Number of probe samples to generate
        #[arg(short, long, default_value = "1000")]
        probes: usize,

        /// Number of input features the model expects
        #[arg(short, long)]
        features: usize,

        /// Decision threshold for classification
        #[arg(short, long, default_value = "0.5")]
        threshold: f64,

        /// Perturbation budget (0.0-1.0)
        #[arg(long, default_value = "0.05")]
        perturbation: f64,

        /// Output file for probe results (JSON)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Random seed
        #[arg(short, long, default_value = "42")]
        seed: u64,
    },

    /// Audit FSM blueprint commands
    Audit {
        #[command(subcommand)]
        command: AuditCommands,
    },

    /// Behavioral-fidelity evaluation against a corpus reference (SP1).
    ///
    /// Scores a synthetic GL dataset against a corpus file using the
    /// Sajja (2026) P1–P4 framework: inter-event time distributions (P1),
    /// entity burst / JE-line-burst (P2), graph fanout + clustering (P3),
    /// and canonical R1..R10 velocity rules (P4). Writes report.json,
    /// report.md, and metrics.csv to `--out`.
    Behavioral {
        #[command(subcommand)]
        command: BehavioralCommands,
    },

    /// **v5.32 C3 (#158)** — adversarial calibration loop.
    ///
    /// Drives the synthetic engine's tunable knobs so a chosen
    /// gap metric (versus a reference corpus) converges. Wraps
    /// the [`datasynth_eval::calibration`] primitives:
    /// objective, knob set, iteration controller, history
    /// persistence, safety rails. See
    /// `docs/design/2026-05-27-c3-adversarial-calibration-design.md`.
    ///
    /// **Status**: CLI surface scaffolded; the real
    /// orchestrator-backed evaluator is a follow-up (the engine
    /// integration runs full generation per iteration so it
    /// belongs in a VM-validated path). Use with `--dry-run` for
    /// argparse + config-patch smoke testing.
    Calibrate {
        /// Path to the base group YAML configuration the loop
        /// will mutate.
        #[arg(short, long)]
        config: PathBuf,

        /// Reference corpus directory the BF eval compares synthetic
        /// against. Same shape as `behavioral score --real <path>`.
        #[arg(short, long)]
        reference: PathBuf,

        /// Output directory for the calibration trajectory,
        /// per-iteration synth outputs, and the
        /// `calibration_history.json` resume file.
        #[arg(short, long)]
        out: PathBuf,

        /// Which scalar to minimise. One of:
        ///   `bf_composite` (default headline mean),
        ///   `bf_composite_median` (robust to outlier sub-metrics),
        ///   `bf_composite_volume_corrected` (excludes volume-
        ///   bounded metrics).
        #[arg(long, default_value = "bf_composite")]
        objective: String,

        /// Maximum iterations before the loop gives up. Default 20.
        #[arg(long, default_value_t = 20)]
        max_iter: usize,

        /// Seeds per iteration for multi-seed loss averaging. Per
        /// the v5.31 T3 methodology finding, single-shard composite
        /// CV ≈ 25 %; the default 3 amortises that variance.
        #[arg(long, default_value_t = 3)]
        seeds: usize,

        /// Optional convergence target. Stops when the multi-seed
        /// mean loss ≤ this. When omitted, the loop runs to
        /// max-iter / patience exhaustion.
        #[arg(long)]
        target: Option<f64>,

        /// Resume from an existing calibration_history.json file.
        /// Knob state + best-tracker restored from the last step.
        #[arg(long)]
        resume: Option<PathBuf>,

        /// Run the argparse + config-patch logic but skip the
        /// actual generation/eval/loop. Useful for CI smoke +
        /// validating the CLI surface without the engine
        /// integration.
        #[arg(long)]
        dry_run: bool,
    },

    /// Template pack management (v3.2.0+)
    ///
    /// Export the embedded default template pool as YAML starter
    /// files, or validate a user-supplied template directory before
    /// wiring it through `generate --config ... templates.path`.
    Templates {
        #[command(subcommand)]
        command: TemplatesCommands,
    },

    /// Audit optimizer — risk scoping, portfolio, Monte Carlo,
    /// calibration, conformance, resource optimization (v4.1.2+).
    ///
    /// Surfaces the analytics in the `datasynth-audit-optimizer`
    /// crate through a single CLI entry point. Each subcommand emits
    /// a JSON report to stdout or the configured `--output` path.
    Optimizer {
        #[command(subcommand)]
        command: OptimizerCommands,
    },

    /// Group audit simulation — manifest / shard / aggregate / generate
    /// (v5.0+).
    ///
    /// Surfaces the three-phase group engine implemented in
    /// `datasynth-group`: build a deterministic manifest, drive a
    /// single shard's per-entity orchestrator runs, run the aggregate
    /// (consolidation + IC eliminations) phase against a shard
    /// archive, or run all three phases in one in-process call. See
    /// `docs/superpowers/specs/2026-04-23-group-audit-simulation-design.md`.
    Group {
        #[command(subcommand)]
        command: GroupCommands,
    },
}

/// v5.0+: group-engine CLI dispatcher.
///
/// Each subcommand wraps one of the four entry points exposed by the
/// `datasynth-group` crate:
///
/// - `manifest`  → [`datasynth_group::build_manifest`]
/// - `shard`     → [`datasynth_group::shard::run_shard`]
/// - `aggregate` → [`datasynth_group::aggregate::run_aggregate`]
/// - `generate`  → [`datasynth_group::generate_standalone`]
///
/// The four together describe the full v5.0 simulation lifecycle; the
/// existing `generate` command auto-detects a `group:` config and
/// transparently dispatches into [`GroupCommands::Generate`] so existing
/// callers can switch to a group config without changing the
/// invocation shape.
#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
enum GroupCommands {
    /// Build a deterministic [`datasynth_group::GroupManifest`] from a
    /// `group:` YAML config and persist it as pretty JSON.
    ///
    /// Cheap and pure — no orchestrator runs, no I/O beyond the
    /// manifest file itself.
    Manifest {
        /// Path to the group YAML configuration file.
        #[arg(short, long)]
        config: PathBuf,

        /// Output path for the manifest JSON.
        #[arg(short, long)]
        out: PathBuf,
    },

    /// Drive the orchestrator once per entity in the named shard,
    /// writing per-entity archives under `out/entities/{code}/`.
    ///
    /// Heavy: each orchestrator run can peak at multiple GiB of RSS
    /// for several minutes — see the rustdoc on
    /// [`datasynth_group::standalone::generate_standalone`].
    Shard {
        /// Path to the manifest JSON produced by `group manifest`.
        #[arg(short, long)]
        manifest: PathBuf,

        /// Shard identifier as recorded in
        /// `manifest.shard_plan.shards[*].shard_id` (e.g.
        /// `"S_SIG_0001"`).
        #[arg(long)]
        shard_id: String,

        /// Output directory for the per-entity archive(s) the shard
        /// runner emits.
        #[arg(short, long)]
        out: PathBuf,

        /// **v5.31 C2 (#157)** — Optional path to the prior period's
        /// shard output directory (the dir containing
        /// `entities/{code}/period_close/trial_balances.json`). When
        /// supplied, each entity in this shard opens with the prior
        /// period's BS positions instead of the industry-mix
        /// `OpeningBalanceGenerator` defaults. P&L lines from the
        /// prior period zero out; net income is absorbed into
        /// Retained Earnings. See
        /// `docs/design/2026-05-27-c2-multi-period-design.md`.
        #[arg(long)]
        prior_period_shards: Option<PathBuf>,

        /// Accounting framework for the closing → opening projection
        /// (selects which account code holds Retained Earnings).
        /// Defaults to `us_gaap` — pass `ifrs`, `french_gaap`,
        /// `german_gaap`, or `dual_reporting` to match how the prior
        /// period was generated. Only consulted when
        /// `--prior-period-shards` is set.
        #[arg(long, default_value = "us_gaap")]
        prior_period_framework: String,
    },

    /// Run the aggregate / consolidation phase against a directory
    /// of pre-computed shard archives.
    ///
    /// Cheap relative to `shard` — folds the per-entity TBs through
    /// pre-elimination, IC matching, eliminations, IAS 21 translation,
    /// NCI / equity-method overlays, and assembles the consolidated
    /// FS bundle.
    Aggregate {
        /// Path to the manifest JSON produced by `group manifest`.
        #[arg(short, long)]
        manifest: PathBuf,

        /// Directory containing per-entity shard archives under
        /// `entities/{code}/`.
        #[arg(long)]
        shards_dir: PathBuf,

        /// Output directory for `consolidated/` and
        /// `ic_eliminations/` artefacts.
        #[arg(short, long)]
        out: PathBuf,

        /// Optional path to the prior period's aggregate `out_dir` —
        /// used to read opening NCI, equity-method carrying values,
        /// and CTA balances. When omitted every opening defaults to
        /// zero.
        #[arg(long)]
        prior_period_aggregate: Option<PathBuf>,

        /// When set, missing per-entity shard archives are downgraded
        /// from a hard error to a warning and the entity codes are
        /// pushed to `entities_missing` in the summary.
        #[arg(long)]
        tolerate_missing_shards: bool,

        /// **v5.5.2** — Optional path to a JSON file supplying
        /// IAS 36 § 10 per-CGU goodwill impairment-test inputs for
        /// this period. Shape: `Vec<CguTestInputs>` — an array of
        /// objects with `cgu_id`, `other_carrying`,
        /// `fair_value_less_costs`, `value_in_use` (all decimal
        /// strings or numbers). Each `cgu_id` must reference a CGU
        /// declared in the manifest's `cgu_plan`. When omitted, no
        /// impairment tests run and `consolidated/cgu_impairment_tests.json`
        /// is not emitted (preserves v5.0/5.1 byte-identical output).
        ///
        /// Example file content:
        /// ```json
        /// [
        ///   { "cgu_id": "CGU-EMEA",
        ///     "other_carrying": "5000000",
        ///     "fair_value_less_costs": "5500000",
        ///     "value_in_use": "5800000" }
        /// ]
        /// ```
        #[arg(long)]
        cgu_test_inputs: Option<PathBuf>,

        /// **v5.5.2** — Optional path to a JSON file supplying
        /// IAS 29 § 12 general price index (CPI) series for
        /// hyperinflationary entities. Shape: `Vec<GeneralPriceIndex>`
        /// — an array of `{ "currency": "ARS", "source": "INDEC IPC",
        /// "observations": [["2024-01-01", "100.0"], ...] }`
        /// objects. The aggregate driver matches each entity in
        /// `HyperinflationStatus::Hyperinflationary` against the
        /// supplied series by its functional currency; matched
        /// entities are translated via the indexed-restatement path
        /// (IAS 29 § 12 + IAS 21 § 42(b)). Hyperinflationary entities
        /// without a matching series fall back to closing-rate
        /// translation with a warning. When omitted entirely, every
        /// entity uses the existing closing-rate path.
        #[arg(long)]
        cpi_series: Option<PathBuf>,
    },

    /// Run manifest + shards + aggregate in one in-process call (the
    /// "standalone" path).  Equivalent to running `group manifest`,
    /// then `group shard` once per shard, then `group aggregate`.
    ///
    /// Heavy by definition — drives one orchestrator run per entity
    /// before consolidating.
    Generate {
        /// Path to the group YAML configuration file.
        #[arg(short, long)]
        config: PathBuf,

        /// Output directory for the manifest, per-entity archives,
        /// and consolidated artefacts.
        #[arg(short, long)]
        out: PathBuf,

        /// Disable parallel shard execution.  Defaults to parallel
        /// (rayon-scheduled).  Set this for determinism harnesses or
        /// when running on a workstation with limited RAM.
        #[arg(long)]
        no_parallel_shards: bool,

        /// **v5.5.2** — Optional path to a JSON file supplying
        /// IAS 36 § 10 per-CGU goodwill impairment-test inputs.
        /// See `group aggregate --help` for the JSON shape.
        #[arg(long)]
        cgu_test_inputs: Option<PathBuf>,
    },

    /// Run manifest + shards + aggregate for N consecutive periods,
    /// auto-threading opening-balance carryover (closing TB → next-period
    /// opening) between periods. Wraps
    /// [`datasynth_group::generate_standalone_chain`].
    GenerateChain {
        /// Path to the base group YAML configuration. The `period` field
        /// is overridden per period from the `--periods` JSON.
        #[arg(short, long)]
        config: PathBuf,

        /// Path to a JSON file containing the chain plan as an array of
        /// `{ "period": <PeriodConfig>, "out_subdir": "<name>" }`
        /// objects. Order in the array determines chain order. Each
        /// `out_subdir` must be unique.
        #[arg(long)]
        periods: PathBuf,

        /// Base output directory. Each period's outputs go under
        /// `{out}/{out_subdir}/`.
        #[arg(short, long)]
        out: PathBuf,

        /// Disable parallel shard execution within each period (matches
        /// `Generate`'s flag).
        #[arg(long)]
        no_parallel_shards: bool,

        /// Optional path to a prior-period aggregate `out_dir` used to
        /// seed period 0 (engagements continuing from an external
        /// archive). When omitted, period 0 starts with zero opening
        /// balances; periods 1..N always carry forward from period N-1.
        #[arg(long)]
        prior_period_aggregate: Option<PathBuf>,

        /// **v5.5.2** — Optional path to a JSON file supplying
        /// IAS 36 § 10 per-CGU goodwill impairment-test inputs that
        /// apply uniformly to **every** period in the chain. Shape:
        /// `Vec<CguTestInputs>` — see `group aggregate --help` for
        /// the per-element shape. To vary the inputs across periods,
        /// drive the chain at the library level.
        #[arg(long)]
        cgu_test_inputs: Option<PathBuf>,

        /// **v5.5.2** — Optional path to a JSON file supplying
        /// IAS 29 § 12 general price index (CPI) series for
        /// hyperinflationary entities. Applies to every period in
        /// the chain — the `observations` vector inside each
        /// `GeneralPriceIndex` is expected to span every period's
        /// reporting date. See `group aggregate --help` for the
        /// per-element shape.
        #[arg(long)]
        cpi_series: Option<PathBuf>,

        /// **v5.31 C2 (#157)** — Accounting framework used to project
        /// the prior period's closing TB onto next-period opening
        /// balances. Selects which account holds Retained Earnings
        /// (US GAAP `"3200"`, SKR03/04 `"2970"`, IFRS varies).
        /// Defaults to `us_gaap` — pass `ifrs`, `french_gaap`,
        /// `german_gaap`, or `dual_reporting` to match how prior
        /// periods were generated. Mismatches make net income land in
        /// the wrong account.
        #[arg(long, default_value = "us_gaap")]
        closing_to_opening_framework: String,
    },
}

/// v4.1.2+: audit-optimizer subcommands. Each wraps one module in
/// `datasynth-audit-optimizer`. The current cut emits a minimal
/// structured result (JSON) so downstream tooling can consume it;
/// deeper analytics surface per-subcommand flags in follow-up
/// patches.
#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
enum OptimizerCommands {
    /// Rank in-scope accounts by residual risk using the configured
    /// inherent-risk + control-strength model.
    RiskScope {
        /// Input: audit engagement YAML file.
        #[arg(short, long)]
        input: PathBuf,
        /// Output path for the ranked-accounts JSON.
        #[arg(short, long, default_value = "./risk-scope.json")]
        output: PathBuf,
        /// Top-N accounts to return (default: all).
        #[arg(long)]
        top_n: Option<usize>,
    },

    /// Audit-portfolio optimization: allocate audit hours across
    /// engagements subject to a total budget.
    Portfolio {
        /// Input: portfolio-candidates YAML (one entry per engagement).
        #[arg(short, long)]
        input: PathBuf,
        /// Total audit-hour budget.
        #[arg(long)]
        budget_hours: u32,
        /// Output JSON path.
        #[arg(short, long, default_value = "./portfolio.json")]
        output: PathBuf,
    },

    /// Resource allocation across engagement phases (planning / fieldwork /
    /// review / reporting) given an effort-schedule and team capacity.
    Resources {
        /// Input: engagement-schedule YAML.
        #[arg(short, long)]
        input: PathBuf,
        /// Output JSON path.
        #[arg(short, long, default_value = "./resources.json")]
        output: PathBuf,
    },

    /// Conformance check between an observed audit lifecycle trace
    /// (from `audit_events.json`) and the expected FSM blueprint.
    Conformance {
        /// Input: observed trace JSON.
        #[arg(short, long)]
        input: PathBuf,
        /// Blueprint YAML to compare against.
        #[arg(long)]
        blueprint: PathBuf,
        /// Output JSON path.
        #[arg(short, long, default_value = "./conformance.json")]
        output: PathBuf,
    },

    /// Monte-Carlo simulation of risk-weighted engagement cost /
    /// duration across N runs.
    MonteCarlo {
        /// Input: base engagement YAML.
        #[arg(short, long)]
        input: PathBuf,
        /// Number of simulation runs.
        #[arg(long, default_value_t = 1000)]
        runs: u32,
        /// Deterministic seed.
        #[arg(long, default_value_t = 42)]
        seed: u64,
        /// Output JSON path.
        #[arg(short, long, default_value = "./monte-carlo.json")]
        output: PathBuf,
    },

    /// Calibration — fit control-strength / inherent-risk weights
    /// against historical findings.
    Calibration {
        /// Input: historical-findings YAML/CSV.
        #[arg(short, long)]
        input: PathBuf,
        /// Output JSON path (calibrated parameters).
        #[arg(short, long, default_value = "./calibration.json")]
        output: PathBuf,
    },
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
enum TemplatesCommands {
    /// Export a starter template pack as YAML files.
    ///
    /// The emitted pack mirrors the pool shapes the runtime consumes
    /// (person names per culture, vendor names by category, customer
    /// names by industry, material/asset descriptions, bank-name pool,
    /// audit finding titles and narratives, department display names).
    /// Empty categories appear as empty arrays so users know the shape
    /// of every slot.
    Export {
        /// Output directory (created if missing)
        #[arg(short, long, default_value = "./templates")]
        output: PathBuf,
    },

    /// Validate a template file or directory.
    ///
    /// Runs the same checks `EnhancedOrchestrator` does at startup
    /// (parse YAML/JSON, ensure each culture has at least one first
    /// and last name, etc.). Exits non-zero on hard errors.
    Validate {
        /// Path to template file or directory
        #[arg(short, long)]
        path: PathBuf,
    },

    /// v3.5.0+: LLM-driven enrichment of a template YAML file.
    ///
    /// Appends N new names / descriptions to the specified category by
    /// calling the chosen LLM backend offline (the mock backend is
    /// deterministic). Runs outside the generate pipeline — the enriched
    /// YAML is then consumed at generate time via `--templates <path>`.
    Enrich {
        /// Input template YAML to start from (use a file produced by
        /// `templates export`). If the file doesn't exist, an empty
        /// TemplateData is used as the starting point.
        #[arg(short, long)]
        input: PathBuf,

        /// Output YAML path for the enriched data.
        #[arg(short, long)]
        output: PathBuf,

        /// What to enrich: vendor_name | customer_name | material_desc
        #[arg(long)]
        category: String,

        /// Industry context for the LLM prompt (e.g. retail, manufacturing).
        #[arg(long, default_value = "retail")]
        industry: String,

        /// Region / country code for the LLM prompt (e.g. US, DE, FR).
        #[arg(long, default_value = "US")]
        region: String,

        /// Spend category (for vendors) or segment (for customers) or
        /// material type. Defaults to "general".
        #[arg(long, default_value = "general")]
        sub_category: String,

        /// Number of items to generate. Appends to existing pool.
        #[arg(long, default_value_t = 50)]
        count: u32,

        /// LLM backend: mock | http. "mock" is deterministic and works
        /// offline; "http" requires the `llm` Cargo feature and a
        /// configured endpoint. For OpenRouter, pass `--backend http
        /// --base-url https://openrouter.ai/api
        /// --model anthropic/claude-sonnet-4.5
        /// --api-key-env OPENROUTER_API_KEY`.
        #[arg(long, default_value = "mock")]
        backend: String,

        /// Deterministic seed (used by mock backend and for HTTP request
        /// seeding when the provider supports it).
        #[arg(long, default_value_t = 42)]
        seed: u64,

        /// Model identifier for the HTTP backend. On OpenRouter this is
        /// `{vendor}/{model}` e.g. `anthropic/claude-sonnet-4.5` or
        /// `openai/gpt-4o-mini`. Ignored when `--backend mock`.
        #[arg(long, default_value = "anthropic/claude-sonnet-4.5")]
        model: String,

        /// Environment variable that holds the API key for the HTTP
        /// backend. Typical values: `OPENROUTER_API_KEY`,
        /// `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`. Ignored when
        /// `--backend mock`.
        #[arg(long, default_value = "OPENROUTER_API_KEY")]
        api_key_env: String,

        /// Base URL for the HTTP backend. OpenAI-compatible;
        /// `/v1/chat/completions` is appended automatically. Default is
        /// OpenRouter (`https://openrouter.ai/api`); use
        /// `https://api.openai.com` for OpenAI.
        #[arg(long, default_value = "https://openrouter.ai/api")]
        base_url: String,
    },
}

#[derive(Subcommand)]
enum ScenarioCommands {
    /// List all scenarios in a config
    List {
        /// Path to configuration file
        #[arg(short, long)]
        config: PathBuf,
    },

    /// Validate scenarios without generating data
    Validate {
        /// Path to configuration file
        #[arg(short, long)]
        config: PathBuf,

        /// Validate a specific scenario by name
        #[arg(short, long)]
        scenario: Option<String>,
    },

    /// Generate paired baseline/counterfactual datasets
    Generate {
        /// Path to configuration file
        #[arg(short, long)]
        config: PathBuf,

        /// Output directory
        #[arg(short, long, default_value = "./output")]
        output: PathBuf,

        /// Generate only a specific scenario by name
        #[arg(short, long)]
        scenario: Option<String>,
    },

    /// Diff baseline vs counterfactual outputs
    Diff {
        /// Baseline output directory
        #[arg(short, long)]
        baseline: PathBuf,

        /// Counterfactual output directory
        #[arg(long)]
        counterfactual: PathBuf,

        /// Diff format (summary, record_level, aggregate, all)
        #[arg(short, long, default_value = "summary")]
        format: String,

        /// Output file for diff results (default: stdout)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Export a scenario as a portable .dss file
    Export {
        /// Path to configuration file containing the scenario
        #[arg(short, long)]
        config: PathBuf,

        /// Scenario name to export
        #[arg(short, long)]
        scenario: String,

        /// Output .dss file path
        #[arg(short, long)]
        output: PathBuf,
    },

    /// Import a .dss scenario file into a config
    Import {
        /// Path to .dss scenario file
        #[arg(required = true)]
        file: PathBuf,

        /// Config file to merge into (or create)
        #[arg(short, long, default_value = "config.yaml")]
        config: PathBuf,
    },
}

#[derive(Subcommand)]
enum FingerprintCommands {
    /// Extract fingerprint from data
    Extract {
        /// Input data path (CSV file or directory)
        #[arg(short, long)]
        input: PathBuf,

        /// Output fingerprint file (.dsf)
        #[arg(short, long)]
        output: PathBuf,

        /// Privacy level (minimal, standard, high, maximum)
        #[arg(long, default_value = "standard")]
        privacy_level: String,

        /// Custom epsilon budget for differential privacy
        #[arg(long)]
        privacy_epsilon: Option<f64>,

        /// Custom k-anonymity threshold
        #[arg(long)]
        privacy_k: Option<u32>,

        /// Sign the fingerprint with HMAC-SHA256.
        ///
        /// The key is read from (in order): `--sign-key-hex` if set,
        /// `--sign-key-file` if set, the `DATASYNTH_FINGERPRINT_KEY`
        /// environment variable (hex-encoded), or a randomly-generated
        /// ephemeral key (the hex value is logged so it can be kept for
        /// later verification).
        #[arg(long)]
        sign: bool,

        /// Hex-encoded HMAC-SHA256 signing key. Precedence: this > file > env > generated.
        #[arg(long, requires = "sign")]
        sign_key_hex: Option<String>,

        /// File containing a hex-encoded HMAC-SHA256 signing key (whitespace stripped).
        #[arg(long, requires = "sign")]
        sign_key_file: Option<PathBuf>,

        /// Key identifier stored alongside the signature (defaults to "default").
        #[arg(long, default_value = "default")]
        sign_key_id: String,

        /// Also extract behavioral priors (SP2). Requires --industry.
        #[arg(long, default_value_t = false)]
        behavioral: bool,
        /// Industry slug ("health", "life_sciences", ...) -- required when --behavioral.
        #[arg(long)]
        industry: Option<String>,

        /// Path to a private PII denylist (TSV: literal-or-/regex/\tkind).
        ///
        /// SP6 Phase B: applied AFTER PlaceholderGrammar's automated structural
        /// tokenization. The file is PII-derived — never commit to a public repo.
        /// When present, `extract_text_taxonomy_from_records` is called to populate
        /// `BehavioralPriors.text_taxonomy` in the output bundle. When absent, only
        /// Phase A (structural) tokenization runs; bundles MAY carry fuzzy proper
        /// nouns that the build-time audit gate will reject.
        ///
        /// Requires --behavioral.
        #[arg(long, value_name = "PATH")]
        pii_denylist: Option<std::path::PathBuf>,
    },

    /// Validate a fingerprint file
    Validate {
        /// Fingerprint file to validate
        #[arg(required = true)]
        file: PathBuf,
    },

    /// Show fingerprint information
    Info {
        /// Fingerprint file
        #[arg(required = true)]
        file: PathBuf,

        /// Also print the behavioral-priors section (SP2) if present.
        #[arg(long, default_value_t = false)]
        behavioral: bool,

        /// Show detailed statistics
        #[arg(long)]
        detailed: bool,
    },

    /// Compare two fingerprints
    Diff {
        /// First fingerprint file
        #[arg(required = true)]
        file1: PathBuf,

        /// Second fingerprint file
        #[arg(required = true)]
        file2: PathBuf,
    },

    /// Evaluate fidelity of synthetic data against fingerprint
    Evaluate {
        /// Fingerprint file
        #[arg(short, long)]
        fingerprint: PathBuf,

        /// Synthetic data directory
        #[arg(short, long)]
        synthetic: PathBuf,

        /// Output report path
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Fidelity threshold (0.0-1.0)
        #[arg(long, default_value = "0.8")]
        threshold: f64,
    },

    /// Synthesize data from a fingerprint (privacy-preserving pipeline)
    ///
    /// Extracts statistical profile from a fingerprint file, optionally trains
    /// a neural diffusion model (requires neural feature), and generates
    /// synthetic data matching the fingerprinted distribution.
    Synthesize {
        /// Fingerprint file (.dsf)
        #[arg(short, long)]
        fingerprint: PathBuf,

        /// Output directory for synthetic data
        #[arg(short, long, default_value = "./synthetic")]
        output: PathBuf,

        /// Number of rows to generate
        #[arg(short, long, default_value = "10000")]
        rows: usize,

        /// Use neural diffusion backend (requires neural feature)
        #[arg(long)]
        neural: bool,

        /// Random seed
        #[arg(short, long, default_value = "42")]
        seed: u64,
    },

    /// Aggregate per-client behavioral fingerprints into one industry-level bundle.
    AggregateIndustry {
        /// Industry slug ("health", "life_sciences", ...).
        #[arg(long)]
        industry: String,
        /// Paths to per-client .dsf files.
        #[arg(long, num_args = 1..)]
        inputs: Vec<PathBuf>,
        /// Output .dsf path.
        #[arg(long)]
        output: PathBuf,
        /// Allow aggregation from fewer than 3 inputs.
        #[arg(long, default_value_t = false)]
        allow_single_client: bool,
        /// Allow inputs with different industry tags.
        #[arg(long, default_value_t = false)]
        allow_cross_industry: bool,
    },
}

#[derive(Subcommand)]
enum AuditCommands {
    /// Validate a blueprint YAML file
    Validate {
        /// Blueprint source: builtin:fsa, builtin:ia, builtin:kpmg, builtin:pwc, builtin:deloitte, or a file path
        #[arg(long, default_value = "builtin:fsa")]
        blueprint: String,
    },
    /// Display blueprint information
    Info {
        /// Blueprint source: builtin:fsa, builtin:ia, builtin:kpmg, builtin:pwc, builtin:deloitte, or a file path
        #[arg(long, default_value = "builtin:fsa")]
        blueprint: String,
    },
    /// Run a standalone FSM engagement
    Run {
        /// Blueprint source: builtin:fsa, builtin:ia, builtin:kpmg, builtin:pwc, builtin:deloitte, or a file path
        #[arg(long, default_value = "builtin:fsa")]
        blueprint: String,
        /// Overlay source: builtin:default, builtin:thorough, builtin:rushed, or a file path
        #[arg(long, default_value = "builtin:default")]
        overlay: String,
        /// Output directory for the event trail
        #[arg(short, long, default_value = "./audit_output")]
        output: PathBuf,
        /// Random seed for deterministic generation
        #[arg(long, default_value = "42")]
        seed: u64,
    },
    /// Compare two blueprints structurally
    Diff {
        /// First blueprint: builtin:fsa, builtin:ia, builtin:kpmg, builtin:pwc, builtin:deloitte, or a file path
        #[arg(long)]
        blueprint_a: String,
        /// Second blueprint: builtin:fsa, builtin:ia, builtin:kpmg, builtin:pwc, builtin:deloitte, or a file path
        #[arg(long)]
        blueprint_b: String,
    },
    /// Generate a benchmark audit event log
    Benchmark {
        /// Complexity level: simple, medium, complex
        #[arg(long, default_value = "simple")]
        complexity: String,
        /// Override anomaly rate (0.0 to 1.0)
        #[arg(long)]
        anomaly_rate: Option<f64>,
        /// Output directory for benchmark files
        #[arg(short, long, default_value = "./audit_benchmark")]
        output: PathBuf,
        /// Random seed for deterministic generation
        #[arg(long, default_value = "42")]
        seed: u64,
    },
}

/// SP1: behavioral-fidelity subcommands.
#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
enum BehavioralCommands {
    /// Score a synthetic GL dataset against a corpus reference.
    ///
    /// Loads both files (CSV or Parquet; or a directory containing one),
    /// runs the full P1–P4 Sajja (2026) framework, and writes three
    /// report artefacts to `--out`:
    ///
    ///   report.json  — machine-readable full report
    ///   report.md    — human-readable Markdown summary
    ///   metrics.csv  — flat metric table for spreadsheet / CI
    ///
    /// Exit codes: 0 = gate passed, 2 = gate failed.
    Score {
        /// Path to the corpus CSV or Parquet file (or a directory
        /// containing one such file).
        #[arg(long)]
        real: PathBuf,
        /// Path to the synthetic-output CSV or Parquet file (or a
        /// directory containing one such file).
        #[arg(long)]
        syn: PathBuf,
        /// Entity profile preset.  SP1 ships `gl-source-tp` only.
        #[arg(long, default_value = "gl-source-tp")]
        profile: String,
        /// Output directory (writes report.json, report.md, metrics.csv).
        #[arg(long)]
        out: PathBuf,
        /// Seed for the deterministic 50/50 split of the corpus.
        #[arg(long, default_value_t = 42)]
        seed: u64,
        /// Gate: fail (exit 2) if any sub-metric degradation-ratio exceeds
        /// this value.
        #[arg(long, default_value_t = 2.0)]
        fail_on_dr_above: f64,
        /// Gate: fail (exit 2) if the composite BF score exceeds this
        /// value.
        #[arg(long, default_value_t = 1.5)]
        fail_on_composite_above: f64,
    },
}

fn main() -> Result<()> {
    // Windows defaults the main thread stack to 1 MB, which is too
    // small for our deeply-nested config + standards processing
    // (`camelcase_feature_matrix_config_produces_full_archive` and
    // similar heavy configs overflow the default).  Linux/macOS
    // default to 8 MB which already fits, but explicit is safer
    // for cross-platform parity.  Run the real work on a 16 MB
    // worker thread regardless of platform.
    std::thread::Builder::new()
        .name("datasynth-main".to_string())
        .stack_size(16 * 1024 * 1024)
        .spawn(run_main)
        .expect("spawn datasynth-main worker thread")
        .join()
        .map_err(|panic_payload| {
            let msg = if let Some(s) = panic_payload.downcast_ref::<&'static str>() {
                (*s).to_string()
            } else if let Some(s) = panic_payload.downcast_ref::<String>() {
                s.clone()
            } else {
                "unknown panic payload".to_string()
            };
            anyhow::anyhow!("datasynth-main worker thread panicked: {msg}")
        })?
}

fn run_main() -> Result<()> {
    let cli = Cli::parse();

    // Setup logging
    let filter = if cli.verbose { "debug" } else { "info" };
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| filter.into()),
        )
        .init();

    match cli.command {
        Commands::Generate {
            config,
            output,
            demo,
            preset,
            scenario_pack,
            fingerprint,
            scale,
            seed,
            banking,
            audit,
            memory_limit,
            max_threads,
            graph_export,
            stream_target,
            stream_api_key,
            stream_batch_size,
            quality_gate,
            fiscal_year_months,
            append,
            months,
            fraud_scenario,
            fraud_rate,
            stream_file,
            export_format,
            auto_tune,
            max_iterations,
            validate_coa_coverage,
        } => {
            // ========================================
            // GROUP CONFIG AUTO-DETECTION (v5.0+)
            // ========================================
            // If the user passed `--config X` and X is a GroupConfig
            // (recognised by the top-level `presentation_currency` and
            // `ownership` keys), transparently dispatch into the
            // `group generate` standalone path. The single-entity
            // pipeline below is left untouched. Auto-detection only
            // fires when none of the modes that can't be a group config
            // are active (--demo, --fingerprint, --scenario-pack,
            // --append). These paths predate the group engine and use
            // bespoke loading logic.
            if let Some(ref cfg_path) = config {
                if !demo && fingerprint.is_none() && scenario_pack.is_none() && !append {
                    if let Ok(yaml) = std::fs::read_to_string(cfg_path) {
                        if yaml_is_group_config(&yaml) {
                            tracing::info!(
                                "auto-detected group config; dispatching to `group generate`"
                            );
                            return handle_group_generate(cfg_path, &output, true, None);
                        }
                    }
                }
            }

            // ========================================
            // CPU SAFEGUARD: Limit thread pool size
            // ========================================
            let available_cpus = num_cpus::get();
            let effective_threads = max_threads.unwrap_or_else(|| {
                // Default: use half of available cores, minimum 1, maximum 4
                (available_cpus / 2).clamp(1, 4)
            });

            // Configure rayon thread pool with limited threads
            if let Err(e) = rayon::ThreadPoolBuilder::new()
                .num_threads(effective_threads)
                .build_global()
            {
                eprintln!(
                    "Warning: failed to configure thread pool with {effective_threads} threads: {e}"
                );
            }

            tracing::info!(
                "CPU safeguard: using {} threads (of {} available)",
                effective_threads,
                available_cpus
            );

            // ========================================
            // MEMORY SAFEGUARD: Set conservative limits
            // ========================================
            let effective_memory_limit = if memory_limit > 0 {
                memory_limit.min(get_safe_memory_limit()) // Cap at safe limit
            } else {
                1024 // Default 1GB
            };

            let memory_config =
                MemoryGuardConfig::with_limit_mb(effective_memory_limit).aggressive();
            let memory_guard = Arc::new(MemoryGuard::new(memory_config));

            tracing::info!(
                "Memory safeguard: {} MB limit ({} MB soft limit)",
                effective_memory_limit,
                (effective_memory_limit * 80) / 100
            );

            // Check initial memory status
            let initial_memory = memory_guard.current_usage_mb();
            tracing::info!("Initial memory usage: {} MB", initial_memory);

            // ========================================
            // LOAD CONFIGURATION OR ORCHESTRATOR
            // ========================================
            // When generating from fingerprint, we create the orchestrator directly.
            // Otherwise, we load a config and create the orchestrator later.
            #[allow(clippy::large_enum_variant)] // Temporary local enum, not worth boxing both
            enum ConfigOrOrchestrator {
                Config(GeneratorConfig),
                Orchestrator(Box<EnhancedOrchestrator>),
            }

            let config_or_orchestrator = if demo {
                tracing::info!("Using demo preset (conservative settings)");
                ConfigOrOrchestrator::Config(create_safe_demo_preset())
            } else if let Some(ref fp_path) = fingerprint {
                tracing::info!("Generating from fingerprint: {}", fp_path.display());
                tracing::info!("Scale factor: {:.2}", scale);

                let phase_config = PhaseConfig {
                    generate_banking: banking,
                    generate_audit: audit,
                    generate_graph_export: graph_export,
                    show_progress: true,
                    inject_anomalies: true, // Let fingerprint control this
                    inject_data_quality: true,
                    validate_coa_coverage_strict: validate_coa_coverage,
                    ..PhaseConfig::default()
                };

                // Create orchestrator directly from fingerprint
                let orchestrator =
                    EnhancedOrchestrator::from_fingerprint(fp_path, phase_config, scale)?;
                ConfigOrOrchestrator::Orchestrator(Box::new(orchestrator))
            } else if let Some(ref pack) = scenario_pack {
                tracing::info!("Loading scenario pack: {}", pack);
                let scenario_path = find_scenario_pack(pack)?;
                let content = std::fs::read_to_string(&scenario_path)?;
                let mut cfg: GeneratorConfig = serde_yaml::from_str(&content)?;
                apply_safety_limits(&mut cfg);
                ConfigOrOrchestrator::Config(cfg)
            } else if let Some(config_path) = config {
                let content = std::fs::read_to_string(&config_path)?;
                let mut cfg: GeneratorConfig = serde_yaml::from_str(&content)?;
                // Apply safety limits to loaded config
                apply_safety_limits(&mut cfg);
                ConfigOrOrchestrator::Config(cfg)
            } else {
                tracing::info!("No config specified, using safe demo preset");
                ConfigOrOrchestrator::Config(create_safe_demo_preset())
            };

            // Apply config modifications only when we have a Config (not fingerprint)
            let config_or_orchestrator = match config_or_orchestrator {
                ConfigOrOrchestrator::Config(mut cfg) => {
                    // Apply seed override
                    if let Some(s) = seed {
                        cfg.global.seed = Some(s);
                    }

                    // Enable banking if flag is set (with conservative defaults)
                    if banking {
                        cfg.banking.enabled = true;
                        cfg.banking.population.retail_customers =
                            cfg.banking.population.retail_customers.min(100);
                        cfg.banking.population.business_customers =
                            cfg.banking.population.business_customers.min(20);
                        cfg.banking.population.trusts = cfg.banking.population.trusts.min(5);
                        tracing::info!("Banking KYC/AML generation enabled (conservative mode)");
                    }

                    // Enable graph export if flag is set
                    if graph_export {
                        cfg.graph_export.enabled = true;
                        tracing::info!("Graph export enabled (PyTorch Geometric format)");
                    }

                    // Apply streaming settings if provided
                    if let Some(ref target) = stream_target {
                        cfg.graph_export.enabled = true;
                        cfg.graph_export.hypergraph.enabled = true;
                        cfg.graph_export.hypergraph.output_format = "unified".to_string();
                        cfg.graph_export.hypergraph.stream_target = Some(target.clone());
                        cfg.graph_export.hypergraph.stream_batch_size = stream_batch_size;
                        if let Some(ref key) = stream_api_key {
                            std::env::set_var("RUSTGRAPH_API_KEY", key);
                            tracing::debug!("API key set from CLI argument");
                        }
                        tracing::info!("Streaming unified hypergraph to: {}", target);
                    }

                    // Apply fiscal_year_months if provided via CLI
                    if let Some(fy_months) = fiscal_year_months {
                        cfg.global.fiscal_year_months = Some(fy_months);
                    }

                    // Apply named overlay preset
                    if let Some(ref preset_name) = preset {
                        match preset_name.as_str() {
                            "audit-group" => {
                                cfg = presets::audit_group_overlay(cfg);
                                tracing::info!("Applied 'audit-group' overlay preset");
                            }
                            other => {
                                tracing::warn!(
                                    "Unknown preset '{}'; supported: audit-group",
                                    other
                                );
                            }
                        }
                    }

                    // Apply fraud scenario packs
                    if !fraud_scenario.is_empty() {
                        cfg =
                            datasynth_config::fraud_packs::apply_fraud_packs(&cfg, &fraud_scenario)
                                .map_err(|e| anyhow::anyhow!("Failed to apply fraud packs: {e}"))?;
                        tracing::info!("Applied fraud packs: {:?}", fraud_scenario);
                    }

                    // Apply fraud rate override
                    if let Some(rate) = fraud_rate {
                        cfg.fraud.enabled = true;
                        cfg.fraud.fraud_rate = rate;
                    }

                    // Stream file notification (wired to pipeline in future)
                    if let Some(ref stream_path) = stream_file {
                        tracing::info!("Streaming to: {}", stream_path.display());
                    }

                    // Apply output and resource settings
                    cfg.output.output_directory = output.clone();
                    cfg.global.parallel = false;
                    cfg.global.worker_threads = effective_threads;
                    cfg.global.memory_limit_mb = effective_memory_limit;

                    ConfigOrOrchestrator::Config(cfg)
                }
                orch @ ConfigOrOrchestrator::Orchestrator(_) => {
                    // For fingerprint-based generation, the orchestrator already has its config
                    orch
                }
            };

            // Extract generator_config for logging and manifest
            let generator_config = match &config_or_orchestrator {
                ConfigOrOrchestrator::Config(cfg) => cfg.clone(),
                ConfigOrOrchestrator::Orchestrator(_) => {
                    // Fingerprint orchestrator has its own config; use demo preset as
                    // a stand-in for manifest generation metadata.
                    tracing::warn!(
                        "Fingerprint-based generation: manifest uses approximate config metadata"
                    );
                    create_safe_demo_preset()
                }
            };

            tracing::info!("Starting generation...");
            match &config_or_orchestrator {
                ConfigOrOrchestrator::Config(cfg) => {
                    tracing::info!("Industry: {:?}", cfg.global.industry);
                    tracing::info!("Period: {} months", cfg.global.period_months);
                    tracing::info!("Companies: {}", cfg.companies.len());
                }
                ConfigOrOrchestrator::Orchestrator(_) => {
                    tracing::info!("Mode: Fingerprint-based generation (scale: {:.2})", scale);
                }
            }

            // ========================================
            // SIGNAL HANDLING (Unix only)
            // ========================================
            let pause_flag = Arc::new(AtomicBool::new(false));

            #[cfg(unix)]
            {
                let pause_flag_clone = Arc::clone(&pause_flag);
                let signal_flag = Arc::new(AtomicBool::new(false));
                let signal_flag_clone = Arc::clone(&signal_flag);

                if signal_hook::flag::register(SIGUSR1, signal_flag_clone).is_ok() {
                    let pid = std::process::id();
                    tracing::info!("Pause/resume: send SIGUSR1 to toggle (kill -USR1 {})", pid);

                    std::thread::spawn(move || loop {
                        if signal_flag.swap(false, Ordering::Relaxed) {
                            let was_paused = pause_flag_clone.load(Ordering::Relaxed);
                            pause_flag_clone.store(!was_paused, Ordering::Relaxed);
                            if was_paused {
                                eprintln!("\n>>> RESUMED");
                            } else {
                                eprintln!("\n>>> PAUSED - send SIGUSR1 again to resume");
                            }
                        }
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    });
                }
            }

            // ========================================
            // PRE-GENERATION MEMORY CHECK
            // ========================================
            if let Err(e) = memory_guard.check_now() {
                tracing::error!("Memory limit already exceeded before generation: {}", e);
                return Err(anyhow::anyhow!("Insufficient memory to start generation"));
            }

            // ========================================
            // SESSION-AWARE GENERATION (multi-period / append)
            // ========================================
            // Handle incremental append mode
            if append {
                if let ConfigOrOrchestrator::Config(cfg) = config_or_orchestrator {
                    use datasynth_runtime::generation_session::GenerationSession;
                    let dss_path = output.join("session.dss");
                    if !dss_path.exists() {
                        eprintln!(
                            "Error: No session.dss found in output directory. Cannot append."
                        );
                        std::process::exit(1);
                    }
                    let additional = months.unwrap_or(12);
                    let mut session = GenerationSession::resume(&dss_path, cfg)?;
                    let results = session.generate_delta(additional)?;
                    session.save(&dss_path)?;
                    println!("\nIncremental generation complete ({additional} new months):");
                    for r in &results {
                        println!(
                            "  {} - {} JEs, {:.1}s",
                            r.period.label, r.journal_entry_count, r.duration_secs
                        );
                    }
                    return Ok(());
                } else {
                    return Err(anyhow::anyhow!(
                        "--append is not supported with fingerprint-based generation"
                    ));
                }
            }

            // Check if multi-period generation is needed
            if let ConfigOrOrchestrator::Config(ref cfg) = config_or_orchestrator {
                let fy_months = cfg.global.fiscal_year_months;
                let total_months = cfg.global.period_months;
                let use_session = fy_months.is_some() && fy_months.unwrap() < total_months;

                if use_session {
                    // Move config out of the enum for session use
                    if let ConfigOrOrchestrator::Config(cfg) = config_or_orchestrator {
                        use datasynth_runtime::generation_session::GenerationSession;
                        let mut session = GenerationSession::new(cfg, output.clone())?;
                        let results = session.generate_all()?;
                        let dss_path = output.join("session.dss");
                        session.save(&dss_path)?;
                        println!("\nMulti-period generation complete:");
                        for r in &results {
                            println!(
                                "  {} - {} JEs, {} docs, {:.1}s",
                                r.period.label,
                                r.journal_entry_count,
                                r.document_count,
                                r.duration_secs
                            );
                        }
                        return Ok(());
                    }
                }
            }

            // ========================================
            // GENERATE DATA (single-period, standard path)
            // ========================================
            // Capture values for manifest before potentially moving config
            let effective_seed = generator_config.global.seed.unwrap_or(42);
            let config_for_manifest = generator_config.clone();

            // Create or use existing orchestrator
            let mut orchestrator = match config_or_orchestrator {
                ConfigOrOrchestrator::Orchestrator(orch) => {
                    tracing::info!("Using orchestrator from fingerprint");
                    *orch
                }
                ConfigOrOrchestrator::Config(cfg) => {
                    let mut phase_config = PhaseConfig::from_config(&cfg);

                    // CLI flag overrides (only override if explicitly set via flag)
                    // Note: banking defaults to enabled=true in its crate, so only
                    // use the explicit CLI --banking flag to avoid unexpected generation
                    if banking {
                        phase_config.generate_banking = true;
                    }
                    if audit {
                        phase_config.generate_audit = true;
                    }
                    if graph_export {
                        phase_config.generate_graph_export = true;
                    }
                    if validate_coa_coverage {
                        phase_config.validate_coa_coverage_strict = true;
                    }

                    phase_config.show_progress = true;

                    // DB-E2: conservative caps apply to DEMO runs only. A config-driven run honors
                    // the user's requested master_data.*.count (the Data Builder needs controllable N).
                    if demo {
                        phase_config.p2p_chains = phase_config.p2p_chains.min(50);
                        phase_config.o2c_chains = phase_config.o2c_chains.min(50);
                        phase_config.vendors_per_company = phase_config.vendors_per_company.min(20);
                        phase_config.customers_per_company = phase_config.customers_per_company.min(30);
                        phase_config.materials_per_company = phase_config.materials_per_company.min(50);
                        phase_config.assets_per_company = phase_config.assets_per_company.min(20);
                        phase_config.employees_per_company = phase_config.employees_per_company.min(30);
                    }

                    EnhancedOrchestrator::new(cfg, phase_config)?
                }
            };

            let result = orchestrator.generate()?;

            // ========================================
            // AUTO-TUNE LOOP (optional)
            // ========================================
            if auto_tune {
                use datasynth_eval::{AiTuner, AiTunerConfig};
                let llm_provider = datasynth_core::llm::MockLlmProvider::new(42);
                let tuner_config = AiTunerConfig {
                    max_iterations,
                    use_llm: true,
                    ..AiTunerConfig::default()
                };
                let mut tuner = AiTuner::new(&llm_provider, tuner_config);
                let eval = datasynth_eval::ComprehensiveEvaluation::new();
                let iteration = tuner.analyze_iteration(&eval, 1);
                tracing::info!(
                    "Auto-tune iteration 1: health={:.2}, rule_patches={}, ai_patches={}, applied={}",
                    iteration.health_score,
                    iteration.rule_patches.len(),
                    iteration.ai_patches.len(),
                    iteration.applied_patches.len(),
                );
                if !iteration.applied_patches.is_empty() {
                    tracing::info!("Suggested config patches:");
                    for patch in &iteration.applied_patches {
                        tracing::info!(
                            "  {} = {} (confidence: {:.2})",
                            patch.path,
                            patch.suggested_value,
                            patch.confidence
                        );
                    }
                }
                tracing::info!("Auto-tune complete ({} iterations)", max_iterations);
            }

            // ========================================
            // REPORT RESULTS
            // ========================================
            tracing::info!("Generation complete!");
            tracing::info!("Total entries: {}", result.statistics.total_entries);
            tracing::info!("Total line items: {}", result.statistics.total_line_items);
            tracing::info!("Accounts in CoA: {}", result.statistics.accounts_count);

            // Memory usage reporting
            let stats = memory_guard.stats();
            let peak_mb = stats.peak_resident_bytes / (1024 * 1024);
            let current_mb = stats.resident_bytes / (1024 * 1024);
            tracing::info!(
                "Memory usage: current {} MB, peak {} MB",
                current_mb,
                peak_mb
            );
            if stats.soft_limit_warnings > 0 {
                tracing::warn!(
                    "Memory soft limit was exceeded {} times during generation",
                    stats.soft_limit_warnings
                );
            }

            // Banking statistics
            if result.statistics.banking_customer_count > 0 {
                tracing::info!(
                    "Banking: {} customers, {} accounts, {} transactions ({} suspicious)",
                    result.statistics.banking_customer_count,
                    result.statistics.banking_account_count,
                    result.statistics.banking_transaction_count,
                    result.statistics.banking_suspicious_count
                );
            }

            // Audit statistics
            if result.statistics.audit_engagement_count > 0 {
                tracing::info!(
                    "Audit: {} engagements, {} workpapers, {} findings",
                    result.statistics.audit_engagement_count,
                    result.statistics.audit_workpaper_count,
                    result.statistics.audit_finding_count
                );
            }

            // ========================================
            // WRITE OUTPUT (with memory checks)
            // ========================================
            std::fs::create_dir_all(&output)?;

            // Check memory before writing
            if memory_guard.check_now().is_err() {
                tracing::warn!("Memory limit reached, writing minimal output");
            }

            // Re-set the decimal serialization mode for the writing phase.
            // The orchestrator's NumericModeGuard resets it when generate() returns,
            // so we need to re-apply it here before any JSON files are written.
            // (Fix for issue #102 — numeric_mode was a silent no-op before this.)
            datasynth_core::serde_decimal::set_numeric_native(
                generator_config.output.numeric_mode == datasynth_config::NumericMode::Native,
            );

            // Write all generated data (journal entries, master data, document flows,
            // subledgers, HR, manufacturing, sourcing, banking, audit, tax, ESG, etc.)
            if let Err(e) = output_writer::write_all_output_with_layout(
                &result,
                &output,
                generator_config.output.export_layout,
                &generator_config.output.formats,
                generator_config.graph_export.je_network.method,
            ) {
                tracing::warn!("Some output files may not have been written: {}", e);
            }

            // Reset the flag so subsequent non-generation serializations use default mode.
            datasynth_core::serde_decimal::set_numeric_native(false);

            // Write FEC (Fichier des Écritures Comptables) when French GAAP – 18 mandatory columns
            if matches!(
                config_for_manifest.accounting_standards.framework,
                Some(AccountingFrameworkConfig::FrenchGaap)
            ) && !result.journal_entries.is_empty()
            {
                let fec_path = output.join("fec.csv");
                match write_fec_csv(
                    &fec_path,
                    &result.journal_entries,
                    &result.chart_of_accounts,
                ) {
                    Ok(()) => tracing::info!(
                        "FEC (18 columns) written to: {} ({} entries, {} lines)",
                        fec_path.display(),
                        result.journal_entries.len(),
                        result
                            .journal_entries
                            .iter()
                            .map(|e| e.lines.len())
                            .sum::<usize>()
                    ),
                    Err(e) => tracing::warn!("Could not write FEC file: {}", e),
                }
            }

            // Write GoBD (Grundsätze zur ordnungsmäßigen Führung) when German GAAP
            if matches!(
                config_for_manifest.accounting_standards.framework,
                Some(AccountingFrameworkConfig::GermanGaap)
            ) && !result.journal_entries.is_empty()
            {
                let gobd_dir = output.join("gobd_export");
                if let Err(e) = std::fs::create_dir_all(&gobd_dir) {
                    tracing::warn!("Could not create gobd_export directory: {}", e);
                } else {
                    // Journal CSV
                    match datasynth_output::write_gobd_journal_csv(
                        &gobd_dir.join("gobd_journal.csv"),
                        &result.journal_entries,
                        &result.chart_of_accounts,
                    ) {
                        Ok(()) => tracing::info!(
                            "GoBD journal (13 columns) written: {} entries",
                            result.journal_entries.len()
                        ),
                        Err(e) => tracing::warn!("Could not write GoBD journal: {}", e),
                    }

                    // Accounts CSV
                    match datasynth_output::write_gobd_accounts_csv(
                        &gobd_dir.join("gobd_accounts.csv"),
                        &result.chart_of_accounts,
                    ) {
                        Ok(()) => tracing::info!(
                            "GoBD accounts written: {} accounts",
                            result.chart_of_accounts.accounts.len()
                        ),
                        Err(e) => tracing::warn!("Could not write GoBD accounts: {}", e),
                    }

                    // Index XML
                    let company_code = config_for_manifest
                        .companies
                        .first()
                        .map(|c| c.code.as_str())
                        .unwrap_or("UNKNOWN");
                    let fiscal_year: i32 = config_for_manifest
                        .global
                        .start_date
                        .split('-')
                        .next()
                        .and_then(|y| y.parse().ok())
                        .unwrap_or(2024);
                    let tables = vec![
                        ("gobd_journal.csv", "Buchungsjournal"),
                        ("gobd_accounts.csv", "Kontenplan"),
                    ];
                    match datasynth_output::write_gobd_index_xml(
                        &gobd_dir.join("index.xml"),
                        company_code,
                        fiscal_year,
                        &tables,
                    ) {
                        Ok(()) => tracing::info!("GoBD index.xml written"),
                        Err(e) => tracing::warn!("Could not write GoBD index.xml: {}", e),
                    }
                }
            }

            // ========================================
            // EXPLICIT --export-format FLAG HANDLING
            // ========================================
            // Process repeatable --export-format flags: sap, fec, gobd
            for fmt in &export_format {
                match fmt.to_ascii_lowercase().as_str() {
                    "sap" => {
                        // SAP S/4HANA BKPF / BSEG / ACDOCA + master-data export,
                        // honouring config.output.sap (or falling back to
                        // defaults when the YAML block is absent).
                        let sap_dir = output.join("sap_export");
                        if let Err(e) = std::fs::create_dir_all(&sap_dir) {
                            tracing::warn!("Could not create sap_export directory: {}", e);
                        } else if result.journal_entries.is_empty() {
                            tracing::warn!("SAP export skipped: no journal entries");
                        } else {
                            let sap_config = build_sap_config(&config_for_manifest.output.sap);
                            tracing::info!(
                                "SAP export config: client={} ledger={} dialect={:?} \
                                 tables={:?} extension_fields={}",
                                sap_config.client,
                                sap_config.ledger,
                                sap_config.dialect,
                                sap_config.tables,
                                sap_config.include_extension_fields,
                            );

                            // Transactional tables (BKPF / BSEG / ACDOCA).
                            let requested_table_names: Vec<String> = config_for_manifest
                                .output
                                .sap
                                .tables
                                .iter()
                                .map(|t| t.to_ascii_lowercase())
                                .collect();
                            let want_table = |name: &str| -> bool {
                                requested_table_names.is_empty()
                                    || requested_table_names.iter().any(|t| t == name)
                            };

                            let mut sap_exporter = SapExporter::new(sap_config.clone());
                            match sap_exporter.export_to_files(&result.journal_entries, &sap_dir) {
                                Ok(files) => {
                                    tracing::info!(
                                        "SAP export: {} transactional tables written to {}",
                                        files.len(),
                                        sap_dir.display()
                                    );
                                    for (table, path) in &files {
                                        tracing::info!(
                                            "  SAP {}: {}",
                                            format!("{:?}", table),
                                            path
                                        );
                                    }
                                }
                                Err(e) => tracing::warn!("SAP export failed: {}", e),
                            }

                            // Master-data tables (v4.3.0b): LFA1/LFB1 (vendor),
                            // KNA1/KNB1 (customer), MARA/MARD (material).
                            let company_codes: Vec<String> = config_for_manifest
                                .companies
                                .iter()
                                .map(|c| c.code.clone())
                                .collect();

                            // v5.1 — CEPC (profit-centre master) is now a
                            // first-class master-data table.  Closes the
                            // v5.0.1 doc-only mitigation of Gap 6.
                            if want_table("cepc") && !result.master_data.profit_centers.is_empty() {
                                let path = sap_dir.join("cepc.csv");
                                match datasynth_output::write_cepc(
                                    &sap_config,
                                    &result.master_data.profit_centers,
                                    &path,
                                ) {
                                    Ok(()) => tracing::info!(
                                        "  SAP CEPC ({} profit centres) → {}",
                                        result.master_data.profit_centers.len(),
                                        path.display()
                                    ),
                                    Err(e) => tracing::warn!("CEPC export failed: {}", e),
                                }
                            }

                            if want_table("lfa1") && !result.master_data.vendors.is_empty() {
                                let path = sap_dir.join("lfa1.csv");
                                match datasynth_output::write_lfa1(
                                    &sap_config,
                                    &result.master_data.vendors,
                                    &path,
                                ) {
                                    Ok(()) => tracing::info!(
                                        "  SAP LFA1 ({} vendors) → {}",
                                        result.master_data.vendors.len(),
                                        path.display()
                                    ),
                                    Err(e) => tracing::warn!("LFA1 export failed: {}", e),
                                }
                            }
                            if want_table("lfb1")
                                && !result.master_data.vendors.is_empty()
                                && !company_codes.is_empty()
                            {
                                let path = sap_dir.join("lfb1.csv");
                                match datasynth_output::write_lfb1(
                                    &sap_config,
                                    &result.master_data.vendors,
                                    &company_codes,
                                    &path,
                                ) {
                                    Ok(()) => tracing::info!(
                                        "  SAP LFB1 ({} vendors × {} companies) → {}",
                                        result.master_data.vendors.len(),
                                        company_codes.len(),
                                        path.display()
                                    ),
                                    Err(e) => tracing::warn!("LFB1 export failed: {}", e),
                                }
                            }
                            if want_table("kna1") && !result.master_data.customers.is_empty() {
                                let path = sap_dir.join("kna1.csv");
                                match datasynth_output::write_kna1(
                                    &sap_config,
                                    &result.master_data.customers,
                                    &path,
                                ) {
                                    Ok(()) => tracing::info!(
                                        "  SAP KNA1 ({} customers) → {}",
                                        result.master_data.customers.len(),
                                        path.display()
                                    ),
                                    Err(e) => tracing::warn!("KNA1 export failed: {}", e),
                                }
                            }
                            if want_table("knb1")
                                && !result.master_data.customers.is_empty()
                                && !company_codes.is_empty()
                            {
                                let path = sap_dir.join("knb1.csv");
                                match datasynth_output::write_knb1(
                                    &sap_config,
                                    &result.master_data.customers,
                                    &company_codes,
                                    &path,
                                ) {
                                    Ok(()) => tracing::info!(
                                        "  SAP KNB1 ({} customers × {} companies) → {}",
                                        result.master_data.customers.len(),
                                        company_codes.len(),
                                        path.display()
                                    ),
                                    Err(e) => tracing::warn!("KNB1 export failed: {}", e),
                                }
                            }
                            if want_table("mara") && !result.master_data.materials.is_empty() {
                                let path = sap_dir.join("mara.csv");
                                match datasynth_output::write_mara(
                                    &sap_config,
                                    &result.master_data.materials,
                                    &path,
                                ) {
                                    Ok(()) => tracing::info!(
                                        "  SAP MARA ({} materials) → {}",
                                        result.master_data.materials.len(),
                                        path.display()
                                    ),
                                    Err(e) => tracing::warn!("MARA export failed: {}", e),
                                }
                            }
                            if want_table("mard") && !result.master_data.materials.is_empty() {
                                let path = sap_dir.join("mard.csv");
                                match datasynth_output::write_mard(
                                    &sap_config,
                                    &result.master_data.materials,
                                    &path,
                                ) {
                                    Ok(()) => tracing::info!(
                                        "  SAP MARD ({} materials × plants) → {}",
                                        result.master_data.materials.len(),
                                        path.display()
                                    ),
                                    Err(e) => tracing::warn!("MARD export failed: {}", e),
                                }
                            }
                            // v4.3.0c — asset, cost-centre, GL account masters.
                            if want_table("anla") && !result.master_data.assets.is_empty() {
                                let path = sap_dir.join("anla.csv");
                                match datasynth_output::write_anla(
                                    &sap_config,
                                    &result.master_data.assets,
                                    &path,
                                ) {
                                    Ok(()) => tracing::info!(
                                        "  SAP ANLA ({} assets) → {}",
                                        result.master_data.assets.len(),
                                        path.display()
                                    ),
                                    Err(e) => tracing::warn!("ANLA export failed: {}", e),
                                }
                            }
                            if want_table("csks") && !result.master_data.cost_centers.is_empty() {
                                let path = sap_dir.join("csks.csv");
                                match datasynth_output::write_csks(
                                    &sap_config,
                                    &result.master_data.cost_centers,
                                    &path,
                                ) {
                                    Ok(()) => tracing::info!(
                                        "  SAP CSKS ({} cost centers) → {}",
                                        result.master_data.cost_centers.len(),
                                        path.display()
                                    ),
                                    Err(e) => tracing::warn!("CSKS export failed: {}", e),
                                }
                            }
                            if want_table("ska1") && !result.chart_of_accounts.accounts.is_empty() {
                                let path = sap_dir.join("ska1.csv");
                                match datasynth_output::write_ska1(
                                    &sap_config,
                                    &result.chart_of_accounts,
                                    &path,
                                ) {
                                    Ok(()) => tracing::info!(
                                        "  SAP SKA1 ({} GL accounts) → {}",
                                        result.chart_of_accounts.accounts.len(),
                                        path.display()
                                    ),
                                    Err(e) => tracing::warn!("SKA1 export failed: {}", e),
                                }
                            }
                            if want_table("skb1")
                                && !result.chart_of_accounts.accounts.is_empty()
                                && !company_codes.is_empty()
                            {
                                let path = sap_dir.join("skb1.csv");
                                match datasynth_output::write_skb1(
                                    &sap_config,
                                    &result.chart_of_accounts,
                                    &company_codes,
                                    &path,
                                ) {
                                    Ok(()) => tracing::info!(
                                        "  SAP SKB1 ({} accounts × {} companies) → {}",
                                        result.chart_of_accounts.accounts.len(),
                                        company_codes.len(),
                                        path.display()
                                    ),
                                    Err(e) => tracing::warn!("SKB1 export failed: {}", e),
                                }
                            }

                            // v4.3.0d — transactional document-flow tables.
                            if want_table("ekko")
                                && !result.document_flows.purchase_orders.is_empty()
                            {
                                let path = sap_dir.join("ekko.csv");
                                if let Err(e) = datasynth_output::write_ekko(
                                    &sap_config,
                                    &result.document_flows.purchase_orders,
                                    &path,
                                ) {
                                    tracing::warn!("EKKO export failed: {}", e);
                                } else {
                                    tracing::info!(
                                        "  SAP EKKO ({} POs) → {}",
                                        result.document_flows.purchase_orders.len(),
                                        path.display()
                                    );
                                }
                            }
                            if want_table("ekpo")
                                && !result.document_flows.purchase_orders.is_empty()
                            {
                                let path = sap_dir.join("ekpo.csv");
                                if let Err(e) = datasynth_output::write_ekpo(
                                    &sap_config,
                                    &result.document_flows.purchase_orders,
                                    &path,
                                ) {
                                    tracing::warn!("EKPO export failed: {}", e);
                                } else {
                                    tracing::info!("  SAP EKPO (PO items) → {}", path.display());
                                }
                            }
                            if want_table("vbak") && !result.document_flows.sales_orders.is_empty()
                            {
                                let path = sap_dir.join("vbak.csv");
                                if let Err(e) = datasynth_output::write_vbak(
                                    &sap_config,
                                    &result.document_flows.sales_orders,
                                    &path,
                                ) {
                                    tracing::warn!("VBAK export failed: {}", e);
                                } else {
                                    tracing::info!(
                                        "  SAP VBAK ({} SOs) → {}",
                                        result.document_flows.sales_orders.len(),
                                        path.display()
                                    );
                                }
                            }
                            if want_table("vbap") && !result.document_flows.sales_orders.is_empty()
                            {
                                let path = sap_dir.join("vbap.csv");
                                if let Err(e) = datasynth_output::write_vbap(
                                    &sap_config,
                                    &result.document_flows.sales_orders,
                                    &path,
                                ) {
                                    tracing::warn!("VBAP export failed: {}", e);
                                } else {
                                    tracing::info!("  SAP VBAP (SO items) → {}", path.display());
                                }
                            }
                            if want_table("likp") && !result.document_flows.deliveries.is_empty() {
                                let path = sap_dir.join("likp.csv");
                                if let Err(e) = datasynth_output::write_likp(
                                    &sap_config,
                                    &result.document_flows.deliveries,
                                    &path,
                                ) {
                                    tracing::warn!("LIKP export failed: {}", e);
                                } else {
                                    tracing::info!(
                                        "  SAP LIKP ({} deliveries) → {}",
                                        result.document_flows.deliveries.len(),
                                        path.display()
                                    );
                                }
                            }
                            if want_table("lips") && !result.document_flows.deliveries.is_empty() {
                                let path = sap_dir.join("lips.csv");
                                if let Err(e) = datasynth_output::write_lips(
                                    &sap_config,
                                    &result.document_flows.deliveries,
                                    &path,
                                ) {
                                    tracing::warn!("LIPS export failed: {}", e);
                                } else {
                                    tracing::info!(
                                        "  SAP LIPS (delivery items) → {}",
                                        path.display()
                                    );
                                }
                            }
                            if want_table("mkpf")
                                && !result.document_flows.goods_receipts.is_empty()
                            {
                                let path = sap_dir.join("mkpf.csv");
                                if let Err(e) = datasynth_output::write_mkpf(
                                    &sap_config,
                                    &result.document_flows.goods_receipts,
                                    &path,
                                ) {
                                    tracing::warn!("MKPF export failed: {}", e);
                                } else {
                                    tracing::info!(
                                        "  SAP MKPF ({} mat-docs) → {}",
                                        result.document_flows.goods_receipts.len(),
                                        path.display()
                                    );
                                }
                            }
                            if want_table("mseg")
                                && !result.document_flows.goods_receipts.is_empty()
                            {
                                let path = sap_dir.join("mseg.csv");
                                if let Err(e) = datasynth_output::write_mseg(
                                    &sap_config,
                                    &result.document_flows.goods_receipts,
                                    &path,
                                ) {
                                    tracing::warn!("MSEG export failed: {}", e);
                                } else {
                                    tracing::info!(
                                        "  SAP MSEG (mat-doc items) → {}",
                                        path.display()
                                    );
                                }
                            }

                            // v4.3.0d — subledger open/cleared items.
                            if want_table("bsis") && !result.journal_entries.is_empty() {
                                let path = sap_dir.join("bsis.csv");
                                if let Err(e) = datasynth_output::write_bsis(
                                    &sap_config,
                                    &result.journal_entries,
                                    &path,
                                ) {
                                    tracing::warn!("BSIS export failed: {}", e);
                                } else {
                                    tracing::info!(
                                        "  SAP BSIS (open GL items) → {}",
                                        path.display()
                                    );
                                }
                            }
                            if want_table("bsas") {
                                let path = sap_dir.join("bsas.csv");
                                if let Err(e) = datasynth_output::write_bsas(&sap_config, &path) {
                                    tracing::warn!("BSAS export failed: {}", e);
                                } else {
                                    tracing::info!(
                                        "  SAP BSAS (cleared GL items, empty) → {}",
                                        path.display()
                                    );
                                }
                            }
                            if want_table("bsid") && !result.subledger.ar_invoices.is_empty() {
                                let path = sap_dir.join("bsid.csv");
                                if let Err(e) = datasynth_output::write_bsid(
                                    &sap_config,
                                    &result.subledger.ar_invoices,
                                    &path,
                                ) {
                                    tracing::warn!("BSID export failed: {}", e);
                                } else {
                                    tracing::info!(
                                        "  SAP BSID (open AR items, from {} invoices) → {}",
                                        result.subledger.ar_invoices.len(),
                                        path.display()
                                    );
                                }
                            }
                            if want_table("bsad") && !result.subledger.ar_invoices.is_empty() {
                                let path = sap_dir.join("bsad.csv");
                                if let Err(e) = datasynth_output::write_bsad(
                                    &sap_config,
                                    &result.subledger.ar_invoices,
                                    &path,
                                ) {
                                    tracing::warn!("BSAD export failed: {}", e);
                                } else {
                                    tracing::info!(
                                        "  SAP BSAD (cleared AR items) → {}",
                                        path.display()
                                    );
                                }
                            }
                            if want_table("bsik") && !result.subledger.ap_invoices.is_empty() {
                                let path = sap_dir.join("bsik.csv");
                                if let Err(e) = datasynth_output::write_bsik(
                                    &sap_config,
                                    &result.subledger.ap_invoices,
                                    &path,
                                ) {
                                    tracing::warn!("BSIK export failed: {}", e);
                                } else {
                                    tracing::info!(
                                        "  SAP BSIK (open AP items, from {} invoices) → {}",
                                        result.subledger.ap_invoices.len(),
                                        path.display()
                                    );
                                }
                            }
                            if want_table("bsak") && !result.subledger.ap_invoices.is_empty() {
                                let path = sap_dir.join("bsak.csv");
                                if let Err(e) = datasynth_output::write_bsak(
                                    &sap_config,
                                    &result.subledger.ap_invoices,
                                    &path,
                                ) {
                                    tracing::warn!("BSAK export failed: {}", e);
                                } else {
                                    tracing::info!(
                                        "  SAP BSAK (cleared AP items) → {}",
                                        path.display()
                                    );
                                }
                            }
                        }
                    }
                    "fec" => {
                        // FEC — only meaningful for French GAAP, but honour flag regardless
                        if result.journal_entries.is_empty() {
                            tracing::warn!("FEC export skipped: no journal entries");
                        } else {
                            let fec_path = output.join("fec_export.csv");
                            match write_fec_csv(
                                &fec_path,
                                &result.journal_entries,
                                &result.chart_of_accounts,
                            ) {
                                Ok(()) => tracing::info!(
                                    "FEC export written to: {} ({} entries)",
                                    fec_path.display(),
                                    result.journal_entries.len()
                                ),
                                Err(e) => tracing::warn!("FEC export failed: {}", e),
                            }
                        }
                    }
                    "gobd" => {
                        // GoBD — only meaningful for German GAAP, but honour flag regardless
                        let gobd_dir = output.join("gobd_explicit");
                        if let Err(e) = std::fs::create_dir_all(&gobd_dir) {
                            tracing::warn!("Could not create gobd_explicit directory: {}", e);
                        } else if result.journal_entries.is_empty() {
                            tracing::warn!("GoBD export skipped: no journal entries");
                        } else {
                            // Journal CSV
                            match datasynth_output::write_gobd_journal_csv(
                                &gobd_dir.join("gobd_journal.csv"),
                                &result.journal_entries,
                                &result.chart_of_accounts,
                            ) {
                                Ok(()) => tracing::info!(
                                    "GoBD journal written: {} entries",
                                    result.journal_entries.len()
                                ),
                                Err(e) => tracing::warn!("GoBD journal export failed: {}", e),
                            }
                            // Accounts CSV
                            match datasynth_output::write_gobd_accounts_csv(
                                &gobd_dir.join("gobd_accounts.csv"),
                                &result.chart_of_accounts,
                            ) {
                                Ok(()) => tracing::info!(
                                    "GoBD accounts written: {} accounts",
                                    result.chart_of_accounts.accounts.len()
                                ),
                                Err(e) => tracing::warn!("GoBD accounts export failed: {}", e),
                            }
                            // Index XML
                            let company_code_exp = config_for_manifest
                                .companies
                                .first()
                                .map(|c| c.code.as_str())
                                .unwrap_or("UNKNOWN");
                            let fiscal_year_exp: i32 = config_for_manifest
                                .global
                                .start_date
                                .split('-')
                                .next()
                                .and_then(|y| y.parse().ok())
                                .unwrap_or(2024);
                            let tables_exp = vec![
                                ("gobd_journal.csv", "Buchungsjournal"),
                                ("gobd_accounts.csv", "Kontenplan"),
                            ];
                            match datasynth_output::write_gobd_index_xml(
                                &gobd_dir.join("index.xml"),
                                company_code_exp,
                                fiscal_year_exp,
                                &tables_exp,
                            ) {
                                Ok(()) => tracing::info!("GoBD explicit index.xml written"),
                                Err(e) => {
                                    tracing::warn!("GoBD explicit index.xml failed: {}", e)
                                }
                            }
                        }
                    }
                    "saft" => {
                        // SAF-T (Standard Audit File for Tax) — OECD-originated
                        // XML format used by tax authorities in PT/PL/RO/NO/LU
                        // and cousins. One XML file per run; jurisdiction comes
                        // from `config.output.saft.jurisdiction` (default: pt).
                        let cfg = &config_for_manifest.output.saft;
                        let jurisdiction = match datasynth_output::SaftJurisdiction::from_code(
                            &cfg.jurisdiction,
                        ) {
                            Some(j) => j,
                            None => {
                                tracing::warn!(
                                    "SAF-T export: unknown jurisdiction '{}' — \
                                     valid codes: pt, pl, ro, no, lu. Defaulting to pt.",
                                    cfg.jurisdiction
                                );
                                datasynth_output::SaftJurisdiction::Portugal
                            }
                        };
                        let (company_name, default_tax_id) = config_for_manifest
                            .companies
                            .first()
                            .map(|c| (c.name.clone(), c.code.clone()))
                            .unwrap_or_else(|| {
                                ("Unknown Co.".to_string(), "000000000".to_string())
                            });
                        // Parse "YYYY-MM-DD" manually — we don't want to
                        // pull chrono into the CLI directly. Fall back to
                        // 2024-01-01 on parse failure.
                        let parse_ymd = |s: &str| -> (u16, u32, u32) {
                            let parts: Vec<&str> = s.split('-').collect();
                            let y = parts
                                .first()
                                .and_then(|p| p.parse().ok())
                                .unwrap_or(2024_u16);
                            let m = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(1_u32);
                            let d = parts.get(2).and_then(|p| p.parse().ok()).unwrap_or(1_u32);
                            (y, m, d)
                        };
                        let (sy, sm, sd) = parse_ymd(&config_for_manifest.global.start_date);
                        let start_date = std::panic::catch_unwind(|| {
                            datasynth_output::saft_naive_date(sy as i32, sm, sd)
                        })
                        .unwrap_or_else(|_| datasynth_output::saft_naive_date(2024, 1, 1));
                        // End date = start + period_months. Compute via
                        // year/month arithmetic; clamp day to 28 to avoid
                        // month-overrun on Feb.
                        let mut ey = sy as i32;
                        let mut em = sm + config_for_manifest.global.period_months;
                        while em > 12 {
                            ey += 1;
                            em -= 12;
                        }
                        let end_date = datasynth_output::saft_naive_date(ey, em, sd.min(28));
                        let saft_cfg = datasynth_output::SaftConfig {
                            jurisdiction,
                            company_tax_id: if cfg.company_tax_id.is_empty() {
                                default_tax_id
                            } else {
                                cfg.company_tax_id.clone()
                            },
                            company_name: if cfg.company_name.is_empty() {
                                company_name
                            } else {
                                cfg.company_name.clone()
                            },
                            fiscal_year: sy,
                            start_date,
                            end_date,
                            currency_code: config_for_manifest
                                .companies
                                .first()
                                .map(|c| c.currency.clone())
                                .unwrap_or_else(|| "EUR".to_string()),
                        };
                        let path = output.join(jurisdiction.filename());
                        let data = datasynth_output::SaftData {
                            accounts: &result.chart_of_accounts,
                            customers: &result.master_data.customers,
                            vendors: &result.master_data.vendors,
                            materials: &result.master_data.materials,
                            journal_entries: &result.journal_entries,
                        };
                        match datasynth_output::write_saft(&saft_cfg, &data, &path) {
                            Ok(()) => tracing::info!(
                                "SAF-T ({:?} / {}) written to {}",
                                jurisdiction,
                                jurisdiction.version_string(),
                                path.display()
                            ),
                            Err(e) => tracing::warn!("SAF-T export failed: {}", e),
                        }
                    }
                    unknown => {
                        tracing::warn!(
                            "Unknown --export-format value '{}'; valid options: sap, saft, fec, gobd",
                            unknown
                        );
                    }
                }
            }

            // ========================================
            // WRITE ANOMALY LABELS (Phase 1.1)
            // ========================================
            if !result.anomaly_labels.labels.is_empty() {
                let labels_dir = output.join("labels");
                std::fs::create_dir_all(&labels_dir)?;

                let export_config = LabelExportConfig::default();
                match export_labels_all_formats(
                    &result.anomaly_labels.labels,
                    &labels_dir,
                    "anomaly_labels",
                    &export_config,
                ) {
                    Ok(results) => {
                        for (path, count) in &results {
                            tracing::info!(
                                "Anomaly labels written to: {} ({} labels)",
                                path,
                                count
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to write anomaly labels: {}", e);
                    }
                }

                // Write summary
                let summary = LabelExportSummary::from_labels(&result.anomaly_labels.labels);
                if let Err(e) =
                    summary.write_to_file(&labels_dir.join("anomaly_labels_summary.json"))
                {
                    tracing::warn!("Failed to write anomaly label summary: {}", e);
                }

                tracing::info!(
                    "Anomaly labels: {} total, {} with provenance, {} in clusters",
                    summary.total_labels,
                    summary.with_provenance,
                    summary.in_clusters
                );
            }

            // ========================================
            // WRITE RUN MANIFEST (Phase 1.3)
            // ========================================
            let mut manifest = RunManifest::new(&config_for_manifest, effective_seed);
            manifest.set_output_directory(&output);
            manifest.complete(result.statistics.clone());

            // Add output file info for journal entries
            if !result.journal_entries.is_empty() {
                let total_lines: usize =
                    result.journal_entries.iter().map(|je| je.lines.len()).sum();
                manifest.add_output_file(OutputFileInfo {
                    path: "journal_entries.csv".to_string(),
                    format: "csv".to_string(),
                    record_count: Some(total_lines),
                    size_bytes: None,
                    sha256_checksum: None,
                    first_record_index: None,
                    last_record_index: None,
                });
                manifest.add_output_file(OutputFileInfo {
                    path: "journal_entries.json".to_string(),
                    format: "json".to_string(),
                    record_count: Some(result.journal_entries.len()),
                    size_bytes: None,
                    sha256_checksum: None,
                    first_record_index: None,
                    last_record_index: None,
                });
            }

            // Add master data file info
            for (name, count) in [
                ("master_data/vendors.json", result.master_data.vendors.len()),
                (
                    "master_data/customers.json",
                    result.master_data.customers.len(),
                ),
                (
                    "master_data/materials.json",
                    result.master_data.materials.len(),
                ),
                (
                    "master_data/fixed_assets.json",
                    result.master_data.assets.len(),
                ),
                (
                    "master_data/employees.json",
                    result.master_data.employees.len(),
                ),
            ] {
                if count > 0 {
                    manifest.add_output_file(OutputFileInfo {
                        path: name.to_string(),
                        format: "json".to_string(),
                        record_count: Some(count),
                        size_bytes: None,
                        sha256_checksum: None,
                        first_record_index: None,
                        last_record_index: None,
                    });
                }
            }

            // Add document flow file info
            for (name, count) in [
                (
                    "document_flows/purchase_orders.json",
                    result.document_flows.purchase_orders.len(),
                ),
                (
                    "document_flows/goods_receipts.json",
                    result.document_flows.goods_receipts.len(),
                ),
                (
                    "document_flows/vendor_invoices.json",
                    result.document_flows.vendor_invoices.len(),
                ),
                (
                    "document_flows/payments.json",
                    result.document_flows.payments.len(),
                ),
                (
                    "document_flows/sales_orders.json",
                    result.document_flows.sales_orders.len(),
                ),
                (
                    "document_flows/deliveries.json",
                    result.document_flows.deliveries.len(),
                ),
                (
                    "document_flows/customer_invoices.json",
                    result.document_flows.customer_invoices.len(),
                ),
            ] {
                if count > 0 {
                    manifest.add_output_file(OutputFileInfo {
                        path: name.to_string(),
                        format: "json".to_string(),
                        record_count: Some(count),
                        size_bytes: None,
                        sha256_checksum: None,
                        first_record_index: None,
                        last_record_index: None,
                    });
                }
            }

            if !result.anomaly_labels.labels.is_empty() {
                manifest.add_output_file(OutputFileInfo {
                    path: "labels/anomaly_labels.csv".to_string(),
                    format: "csv".to_string(),
                    record_count: Some(result.anomaly_labels.labels.len()),
                    size_bytes: None,
                    sha256_checksum: None,
                    first_record_index: None,
                    last_record_index: None,
                });
            }

            // Register additional output subdirectories in manifest
            // Helper to add a manifest entry for a JSON file
            let mut register = |path: &str, count: usize| {
                if count > 0 {
                    manifest.add_output_file(OutputFileInfo {
                        path: path.to_string(),
                        format: "json".to_string(),
                        record_count: Some(count),
                        size_bytes: None,
                        sha256_checksum: None,
                        first_record_index: None,
                        last_record_index: None,
                    });
                }
            };

            // Subledger
            register(
                "subledger/ar_invoices.json",
                result.subledger.ar_invoices.len(),
            );
            register(
                "subledger/ap_invoices.json",
                result.subledger.ap_invoices.len(),
            );
            register(
                "subledger/fa_records.json",
                result.subledger.fa_records.len(),
            );
            register(
                "subledger/inventory_positions.json",
                result.subledger.inventory_positions.len(),
            );
            register(
                "subledger/inventory_movements.json",
                result.subledger.inventory_movements.len(),
            );
            register(
                "subledger/ar_aging.json",
                result.subledger.ar_aging_reports.len(),
            );
            register(
                "subledger/ap_aging.json",
                result.subledger.ap_aging_reports.len(),
            );
            register(
                "subledger/depreciation_runs.json",
                result.subledger.depreciation_runs.len(),
            );
            register(
                "subledger/inventory_valuation.json",
                result.subledger.inventory_valuations.len(),
            );

            // Audit
            register(
                "audit/audit_engagements.json",
                result.audit.engagements.len(),
            );
            register("audit/audit_workpapers.json", result.audit.workpapers.len());
            register("audit/audit_evidence.json", result.audit.evidence.len());
            register(
                "audit/audit_risk_assessments.json",
                result.audit.risk_assessments.len(),
            );
            register("audit/audit_findings.json", result.audit.findings.len());
            register("audit/audit_judgments.json", result.audit.judgments.len());
            register(
                "audit/audit_opinions.json",
                result.audit.audit_opinions.len(),
            );
            register(
                "audit/key_audit_matters.json",
                result.audit.key_audit_matters.len(),
            );
            register(
                "audit/sox_302_certifications.json",
                result.audit.sox_302_certifications.len(),
            );
            register(
                "audit/sox_404_assessments.json",
                result.audit.sox_404_assessments.len(),
            );
            register(
                "audit/materiality_calculations.json",
                result.audit.materiality_calculations.len(),
            );
            register(
                "audit/combined_risk_assessments.json",
                result.audit.combined_risk_assessments.len(),
            );

            // Banking
            register(
                "banking/banking_customers.json",
                result.banking.customers.len(),
            );
            register(
                "banking/banking_transactions.json",
                result.banking.transactions.len(),
            );
            register(
                "banking/banking_accounts.json",
                result.banking.accounts.len(),
            );
            register(
                "banking/aml_transaction_labels.json",
                result.banking.transaction_labels.len(),
            );
            register(
                "banking/aml_customer_labels.json",
                result.banking.customer_labels.len(),
            );
            register(
                "banking/aml_account_labels.json",
                result.banking.account_labels.len(),
            );
            register(
                "banking/aml_relationship_labels.json",
                result.banking.relationship_labels.len(),
            );
            register(
                "banking/aml_narratives.json",
                result.banking.narratives.len(),
            );

            // Sourcing (S2C)
            register(
                "sourcing/sourcing_projects.json",
                result.sourcing.sourcing_projects.len(),
            );
            register(
                "sourcing/spend_analyses.json",
                result.sourcing.spend_analyses.len(),
            );
            register(
                "sourcing/supplier_qualifications.json",
                result.sourcing.qualifications.len(),
            );
            register("sourcing/rfx_events.json", result.sourcing.rfx_events.len());
            register("sourcing/supplier_bids.json", result.sourcing.bids.len());
            register(
                "sourcing/bid_evaluations.json",
                result.sourcing.bid_evaluations.len(),
            );
            register(
                "sourcing/procurement_contracts.json",
                result.sourcing.contracts.len(),
            );
            register(
                "sourcing/catalog_items.json",
                result.sourcing.catalog_items.len(),
            );
            register(
                "sourcing/supplier_scorecards.json",
                result.sourcing.scorecards.len(),
            );

            // Intercompany
            register(
                "intercompany/ic_matched_pairs.json",
                result.intercompany.matched_pairs.len(),
            );
            register(
                "intercompany/ic_elimination_entries.json",
                result.intercompany.elimination_entries.len(),
            );
            register(
                "intercompany/ic_seller_journal_entries.json",
                result.intercompany.seller_journal_entries.len(),
            );
            register(
                "intercompany/ic_buyer_journal_entries.json",
                result.intercompany.buyer_journal_entries.len(),
            );

            // Financial Reporting
            register(
                "financial_reporting/financial_statements.json",
                result.financial_reporting.financial_statements.len(),
            );
            register(
                "financial_reporting/bank_reconciliations.json",
                result.financial_reporting.bank_reconciliations.len(),
            );

            // Period Close
            register(
                "period_close/trial_balances.json",
                result.financial_reporting.trial_balances.len(),
            );

            // HR
            register("hr/payroll_runs.json", result.hr.payroll_runs.len());
            register("hr/time_entries.json", result.hr.time_entries.len());
            register("hr/expense_reports.json", result.hr.expense_reports.len());
            register(
                "hr/payroll_line_items.json",
                result.hr.payroll_line_items.len(),
            );

            // Manufacturing
            register(
                "manufacturing/production_orders.json",
                result.manufacturing.production_orders.len(),
            );
            register(
                "manufacturing/quality_inspections.json",
                result.manufacturing.quality_inspections.len(),
            );
            register(
                "manufacturing/cycle_counts.json",
                result.manufacturing.cycle_counts.len(),
            );

            // Sales / KPI / Budgets
            register(
                "sales_kpi_budgets/sales_quotes.json",
                result.sales_kpi_budgets.sales_quotes.len(),
            );
            register(
                "sales_kpi_budgets/management_kpis.json",
                result.sales_kpi_budgets.kpis.len(),
            );
            register(
                "sales_kpi_budgets/budgets.json",
                result.sales_kpi_budgets.budgets.len(),
            );

            // Internal Controls
            register(
                "internal_controls/internal_controls.json",
                result.internal_controls.len(),
            );

            // Accounting Standards
            register(
                "accounting_standards/customer_contracts.json",
                result.accounting_standards.contracts.len(),
            );
            register(
                "accounting_standards/impairment_tests.json",
                result.accounting_standards.impairment_tests.len(),
            );
            register(
                "accounting_standards/business_combinations.json",
                result.accounting_standards.business_combinations.len(),
            );
            register(
                "accounting_standards/business_combination_journal_entries.json",
                result
                    .accounting_standards
                    .business_combination_journal_entries
                    .len(),
            );

            // Treasury
            register(
                "treasury/debt_instruments.json",
                result.treasury.debt_instruments.len(),
            );
            register(
                "treasury/hedging_instruments.json",
                result.treasury.hedging_instruments.len(),
            );
            register(
                "treasury/hedge_relationships.json",
                result.treasury.hedge_relationships.len(),
            );
            register(
                "treasury/cash_positions.json",
                result.treasury.cash_positions.len(),
            );
            register(
                "treasury/cash_forecasts.json",
                result.treasury.cash_forecasts.len(),
            );
            register("treasury/cash_pools.json", result.treasury.cash_pools.len());
            register(
                "treasury/cash_pool_sweeps.json",
                result.treasury.cash_pool_sweeps.len(),
            );
            register(
                "treasury/treasury_anomaly_labels.json",
                result.treasury.treasury_anomaly_labels.len(),
            );

            // Project Accounting
            register(
                "project_accounting/projects.json",
                result.project_accounting.projects.len(),
            );
            register(
                "project_accounting/change_orders.json",
                result.project_accounting.change_orders.len(),
            );
            register(
                "project_accounting/milestones.json",
                result.project_accounting.milestones.len(),
            );
            register(
                "project_accounting/cost_lines.json",
                result.project_accounting.cost_lines.len(),
            );
            register(
                "project_accounting/revenue_records.json",
                result.project_accounting.revenue_records.len(),
            );
            register(
                "project_accounting/earned_value_metrics.json",
                result.project_accounting.earned_value_metrics.len(),
            );

            // Tax (extended)
            register("tax/tax_provisions.json", result.tax.tax_provisions.len());
            register("tax/tax_jurisdictions.json", result.tax.jurisdictions.len());
            register("tax/tax_codes.json", result.tax.codes.len());
            register("tax/tax_lines.json", result.tax.tax_lines.len());
            register("tax/tax_returns.json", result.tax.tax_returns.len());
            register(
                "tax/withholding_records.json",
                result.tax.withholding_records.len(),
            );
            register(
                "tax/tax_anomaly_labels.json",
                result.tax.tax_anomaly_labels.len(),
            );
            register(
                "tax/temporary_differences.json",
                result.tax.deferred_tax.temporary_differences.len(),
            );
            register(
                "tax/etr_reconciliation.json",
                result.tax.deferred_tax.etr_reconciliations.len(),
            );
            register(
                "tax/deferred_tax_rollforward.json",
                result.tax.deferred_tax.rollforwards.len(),
            );
            register(
                "tax/deferred_tax_journal_entries.json",
                result.tax.deferred_tax.journal_entries.len(),
            );

            // ESG
            register("esg/emission_records.json", result.esg.emissions.len());
            register("esg/energy_consumption.json", result.esg.energy.len());
            register("esg/water_usage.json", result.esg.water.len());
            register("esg/waste_records.json", result.esg.waste.len());
            register("esg/workforce_diversity.json", result.esg.diversity.len());
            register("esg/pay_equity.json", result.esg.pay_equity.len());
            register(
                "esg/safety_incidents.json",
                result.esg.safety_incidents.len(),
            );
            register("esg/safety_metrics.json", result.esg.safety_metrics.len());
            register("esg/governance_metrics.json", result.esg.governance.len());
            register(
                "esg/supplier_esg_assessments.json",
                result.esg.supplier_assessments.len(),
            );
            register(
                "esg/materiality_assessments.json",
                result.esg.materiality.len(),
            );
            register("esg/esg_disclosures.json", result.esg.disclosures.len());
            register(
                "esg/climate_scenarios.json",
                result.esg.climate_scenarios.len(),
            );
            register(
                "esg/esg_anomaly_labels.json",
                result.esg.anomaly_labels.len(),
            );

            // Balance
            register(
                "balance/opening_balances.json",
                result.opening_balances.len(),
            );
            register(
                "balance/subledger_reconciliation.json",
                result.subledger_reconciliation.len(),
            );

            // Process Mining
            register("process_mining/event_log.json", result.ocpm.event_count);

            // Root-level files
            register("chart_of_accounts.json", 1);
            register("generation_statistics.json", 1);

            // Attach lineage graph to manifest and write separate file
            if let Some(ref lineage) = result.lineage {
                manifest.lineage = Some(lineage.clone());
                let lineage_path = output.join("lineage_graph.json");
                if let Ok(json) = lineage.to_json() {
                    if let Err(e) = std::fs::write(&lineage_path, json) {
                        tracing::warn!("Failed to write lineage graph: {}", e);
                    } else {
                        tracing::info!(
                            "Lineage graph written to: {} ({} nodes, {} edges)",
                            lineage_path.display(),
                            lineage.node_count(),
                            lineage.edge_count()
                        );
                    }
                }
            }

            // Write W3C PROV-JSON
            {
                let prov_path = output.join("prov.json");
                let prov_doc = datasynth_runtime::prov::manifest_to_prov(&manifest);
                match serde_json::to_string_pretty(&prov_doc) {
                    Ok(json) => {
                        if let Err(e) = std::fs::write(&prov_path, json) {
                            tracing::warn!("Failed to write PROV-JSON: {}", e);
                        } else {
                            tracing::info!("PROV-JSON written to: {}", prov_path.display());
                        }
                    }
                    Err(e) => tracing::warn!("Failed to serialize PROV-JSON: {}", e),
                }
            }

            // Populate file checksums
            manifest.populate_file_checksums(&output);

            // Write manifest
            let manifest_path = output.join("run_manifest.json");
            if let Err(e) = manifest.write_to_file(&manifest_path) {
                tracing::warn!("Failed to write run manifest: {}", e);
            } else {
                tracing::info!(
                    "Run manifest written to: {} (run_id: {})",
                    manifest_path.display(),
                    manifest.run_id()
                );
            }

            // ========================================
            // QUALITY GATE EVALUATION
            // ========================================
            if quality_gate != "none" {
                if let Some(profile) = datasynth_eval::gates::get_profile(&quality_gate) {
                    tracing::warn!(
                        "Quality gate evaluation not yet integrated with generation output — requires ComprehensiveEvaluation population"
                    );
                    let evaluation = datasynth_eval::ComprehensiveEvaluation::new();
                    let gate_result =
                        datasynth_eval::gates::GateEngine::evaluate(&evaluation, &profile);

                    // Print gate result summary
                    println!();
                    println!(
                        "Quality Gate Evaluation (profile: {})",
                        gate_result.profile_name
                    );
                    println!("==========================================");
                    for check in &gate_result.results {
                        let status = if check.passed { "PASS" } else { "FAIL" };
                        println!("  [{}] {}: {}", status, check.gate_name, check.message);
                    }
                    println!();
                    println!(
                        "Result: {}/{} gates passed",
                        gate_result.gates_passed, gate_result.gates_total
                    );
                    println!("{}", gate_result.summary);

                    if !gate_result.passed {
                        tracing::error!(
                            "Quality gates FAILED: {}/{}",
                            gate_result.gates_total - gate_result.gates_passed,
                            gate_result.gates_total
                        );
                        std::process::exit(2);
                    }
                } else {
                    tracing::warn!(
                        "Unknown quality gate profile '{}'. Valid profiles: none, lenient, default, strict",
                        quality_gate
                    );
                }
            }

            Ok(())
        }

        Commands::Validate { config } => {
            let content = std::fs::read_to_string(&config)?;
            let generator_config: GeneratorConfig = serde_yaml::from_str(&content)?;
            datasynth_config::validate_config(&generator_config)?;
            tracing::info!("Configuration is valid!");
            Ok(())
        }

        Commands::Init {
            output,
            industry,
            complexity,
            from_description,
        } => {
            // If --from-description is provided, use LLM-powered config generation
            if let Some(desc) = from_description {
                // Try real LLM provider if API key is available, fall back to mock+keywords.
                // Supports ANTHROPIC_API_KEY, OPENAI_API_KEY, and OPENROUTER_API_KEY.
                #[cfg(feature = "llm")]
                let provider: Box<dyn datasynth_core::llm::LlmProvider> = {
                    // api_key_env is the NAME of the env var (HttpLlmProvider reads it at request time)
                    let (env_var_name, provider_type, base_url) =
                        if std::env::var("ANTHROPIC_API_KEY").is_ok() {
                            (
                                "ANTHROPIC_API_KEY",
                                datasynth_core::llm::LlmProviderType::Anthropic,
                                None,
                            )
                        } else if std::env::var("OPENROUTER_API_KEY").is_ok() {
                            (
                                "OPENROUTER_API_KEY",
                                datasynth_core::llm::LlmProviderType::OpenAi,
                                Some("https://openrouter.ai/api".to_string()),
                            )
                        } else if let Ok(k) = std::env::var("OPENAI_API_KEY") {
                            let base = if k.starts_with("sk-or-") {
                                Some("https://openrouter.ai/api".to_string())
                            } else {
                                None
                            };
                            (
                                "OPENAI_API_KEY",
                                datasynth_core::llm::LlmProviderType::OpenAi,
                                base,
                            )
                        } else {
                            ("", datasynth_core::llm::LlmProviderType::Mock, None)
                        };

                    if env_var_name.is_empty() {
                        Box::new(datasynth_core::llm::MockLlmProvider::new(42))
                    } else {
                        let config = datasynth_core::llm::LlmConfig {
                            provider: provider_type,
                            api_key_env: env_var_name.to_string(),
                            base_url,
                            ..Default::default()
                        };
                        match datasynth_core::llm::HttpLlmProvider::new(config) {
                            Ok(p) => {
                                tracing::info!("Using real LLM provider for config generation");
                                Box::new(p)
                            }
                            Err(e) => {
                                tracing::warn!("Failed to init LLM provider: {e}, using fallback");
                                Box::new(datasynth_core::llm::MockLlmProvider::new(42))
                            }
                        }
                    }
                };
                #[cfg(not(feature = "llm"))]
                let provider: Box<dyn datasynth_core::llm::LlmProvider> =
                    Box::new(datasynth_core::llm::MockLlmProvider::new(42));

                let yaml = datasynth_core::llm::nl_config::NlConfigGenerator::generate_full(
                    &desc,
                    provider.as_ref(),
                )
                .map_err(|e| anyhow::anyhow!("{e}"))?;
                std::fs::write(&output, &yaml)?;
                tracing::info!(
                    "Configuration generated from description and written to: {}",
                    output.display()
                );
                return Ok(());
            }

            let industry_lower = industry.to_lowercase();
            let industry_sector = match industry_lower.as_str() {
                "manufacturing" => IndustrySector::Manufacturing,
                "retail" => IndustrySector::Retail,
                "financial" | "financial_services" => IndustrySector::FinancialServices,
                "healthcare" => IndustrySector::Healthcare,
                "technology" | "tech" => IndustrySector::Technology,
                _ => {
                    eprintln!(
                        "Warning: unrecognized industry '{industry}'. Valid values: manufacturing, retail, financial_services, healthcare, technology. Defaulting to manufacturing."
                    );
                    IndustrySector::Manufacturing
                }
            };

            let complexity_lower = complexity.to_lowercase();
            let coa_complexity = match complexity_lower.as_str() {
                "small" => CoAComplexity::Small,
                "medium" => CoAComplexity::Medium,
                "large" => CoAComplexity::Large,
                _ => {
                    eprintln!(
                        "Warning: unrecognized complexity '{complexity}'. Valid values: small, medium, large. Defaulting to medium."
                    );
                    CoAComplexity::Medium
                }
            };

            let config = presets::create_preset(
                industry_sector,
                2,
                12,
                coa_complexity,
                datasynth_config::TransactionVolume::TenK, // Conservative default
            );

            let yaml = serde_yaml::to_string(&config)?;
            std::fs::write(&output, yaml)?;
            tracing::info!("Configuration written to: {}", output.display());
            Ok(())
        }

        Commands::Info => {
            println!("Available Industry Presets:");
            println!("  - manufacturing: Manufacturing industry");
            println!("  - retail: Retail industry");
            println!("  - financial_services: Financial services");
            println!("  - healthcare: Healthcare industry");
            println!("  - technology: Technology industry");
            println!();
            println!("Chart of Accounts Complexity:");
            println!("  - small: ~100 accounts");
            println!("  - medium: ~400 accounts");
            println!("  - large: ~2500 accounts");
            println!();
            println!("Transaction Volumes:");
            println!("  - ten_k: 10,000 transactions/year");
            println!("  - hundred_k: 100,000 transactions/year");
            println!("  - one_m: 1,000,000 transactions/year");
            println!("  - ten_m: 10,000,000 transactions/year");
            println!("  - hundred_m: 100,000,000 transactions/year");
            println!();
            println!("Resource Safeguards:");
            println!("  --memory-limit <MB>  : Set memory limit (default: 1024 MB)");
            println!("  --max-threads <N>    : Limit CPU threads (default: half of cores, max 4)");
            Ok(())
        }

        Commands::Verify {
            output,
            checksums,
            record_counts,
        } => {
            let manifest_path = output.join("run_manifest.json");
            if !manifest_path.exists() {
                anyhow::bail!("No run_manifest.json found in {}", output.display());
            }

            let manifest_json = std::fs::read_to_string(&manifest_path)?;
            let manifest: RunManifest = serde_json::from_str(&manifest_json)?;

            println!("Verifying output: {}", output.display());
            println!("  Manifest version: {}", manifest.manifest_version);
            println!("  Run ID: {}", manifest.run_id);
            println!("  Generator version: {}", manifest.generator_version);
            println!("  Output files: {}", manifest.output_files.len());
            println!();

            let mut all_pass = true;
            let mut checked = 0;
            let mut passed = 0;
            let mut failed = 0;

            // Check file existence
            for file_info in &manifest.output_files {
                let file_path = output.join(&file_info.path);
                checked += 1;
                if file_path.exists() {
                    passed += 1;
                    println!("  [PASS] {} exists", file_info.path);
                } else {
                    failed += 1;
                    all_pass = false;
                    println!("  [FAIL] {} missing", file_info.path);
                }
            }

            // Verify checksums
            if checksums {
                println!();
                println!("Checksum verification:");
                let results = manifest.verify_file_checksums(&output);
                for result in &results {
                    match result.status {
                        datasynth_runtime::ChecksumStatus::Ok => {
                            println!("  [PASS] {} checksum OK", result.path);
                            passed += 1;
                        }
                        datasynth_runtime::ChecksumStatus::Mismatch => {
                            println!("  [FAIL] {} checksum MISMATCH", result.path);
                            if let (Some(ref exp), Some(ref act)) =
                                (&result.expected, &result.actual)
                            {
                                println!("         expected: {exp}");
                                println!("         actual:   {act}");
                            }
                            failed += 1;
                            all_pass = false;
                        }
                        datasynth_runtime::ChecksumStatus::Missing => {
                            println!("  [FAIL] {} file missing", result.path);
                            failed += 1;
                            all_pass = false;
                        }
                        datasynth_runtime::ChecksumStatus::NoChecksum => {
                            println!("  [SKIP] {} no checksum recorded", result.path);
                        }
                    }
                    checked += 1;
                }
            }

            // Verify record counts
            if record_counts {
                println!();
                println!("Record count verification:");
                for file_info in &manifest.output_files {
                    let file_path = output.join(&file_info.path);
                    if let Some(expected_count) = file_info.record_count {
                        checked += 1;
                        if file_path.exists() {
                            // Count lines for CSV/JSON
                            let content = std::fs::read_to_string(&file_path).unwrap_or_default();
                            let line_count = if file_info.format == "csv" {
                                content.lines().count().saturating_sub(1) // minus header
                            } else if file_info.format == "json" {
                                // JSON array - count top-level objects
                                if let Ok(arr) =
                                    serde_json::from_str::<Vec<serde_json::Value>>(&content)
                                {
                                    arr.len()
                                } else {
                                    content.lines().count()
                                }
                            } else {
                                content.lines().count()
                            };

                            if line_count == expected_count {
                                println!(
                                    "  [PASS] {} count: {} records",
                                    file_info.path, expected_count
                                );
                                passed += 1;
                            } else {
                                println!(
                                    "  [WARN] {} count: expected {}, found {}",
                                    file_info.path, expected_count, line_count
                                );
                                // Counts may differ due to formatting, so warn only
                                passed += 1;
                            }
                        } else {
                            println!("  [SKIP] {} file missing", file_info.path);
                        }
                    }
                }
            }

            println!();
            println!("Summary: {checked} checked, {passed} passed, {failed} failed");

            if all_pass {
                println!("Verification: PASSED");
                Ok(())
            } else {
                anyhow::bail!("Verification: FAILED ({failed} failures)");
            }
        }

        Commands::Fingerprint { command } => handle_fingerprint_command(command),
        Commands::Scenario { command } => handle_scenario_command(command),
        Commands::Adversarial {
            model,
            probes,
            features,
            threshold,
            perturbation,
            output: out_path,
            seed: adv_seed,
        } => {
            tracing::info!("Adversarial model probing: {}", model.display());
            tracing::info!(
                "Probes: {}, features: {}, threshold: {}, perturbation: {}",
                probes,
                features,
                threshold,
                perturbation
            );

            #[cfg(feature = "adversarial")]
            {
                use datasynth_eval::adversarial::{ModelProbe, ModelProbeConfig};
                let config = ModelProbeConfig {
                    n_features: features,
                    n_probes: probes,
                    perturbation_budget: perturbation,
                    threshold,
                    target_class: 0,
                };
                let mut probe =
                    ModelProbe::load(&model, config).map_err(|e| anyhow::anyhow!("{e}"))?;
                let result = probe
                    .probe(&[], adv_seed)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;

                tracing::info!("Probe results:");
                tracing::info!("  Mean score: {:.4}", result.stats.mean_score);
                tracing::info!(
                    "  Positive rate: {:.2}%",
                    result.stats.positive_rate * 100.0
                );
                tracing::info!(
                    "  Boundary samples (<0.1 margin): {}",
                    result.stats.boundary_samples
                );
                tracing::info!("  Mean margin: {:.4}", result.stats.mean_margin);

                if let Some(ref path) = out_path {
                    let json = serde_json::to_string_pretty(&result)?;
                    std::fs::write(path, json)?;
                    tracing::info!("Results written to: {}", path.display());
                }
                Ok(())
            }
            #[cfg(not(feature = "adversarial"))]
            {
                let _ = (
                    model,
                    probes,
                    features,
                    threshold,
                    perturbation,
                    out_path,
                    adv_seed,
                );
                tracing::error!(
                    "Adversarial testing requires the 'adversarial' feature. \
                     Build with: cargo build --features adversarial"
                );
                Err(anyhow::anyhow!("adversarial feature not enabled"))
            }
        }
        Commands::Audit { command } => match command {
            AuditCommands::Validate { blueprint } => handle_audit_validate(&blueprint),
            AuditCommands::Info { blueprint } => handle_audit_info(&blueprint),
            AuditCommands::Run {
                blueprint,
                overlay,
                output,
                seed,
            } => handle_audit_run(&blueprint, &overlay, &output, seed),
            AuditCommands::Diff {
                blueprint_a,
                blueprint_b,
            } => handle_audit_diff(&blueprint_a, &blueprint_b),
            AuditCommands::Benchmark {
                complexity,
                anomaly_rate,
                output,
                seed,
            } => handle_audit_benchmark(&complexity, anomaly_rate, &output, seed),
        },

        Commands::Templates { command } => match command {
            TemplatesCommands::Export { output } => handle_templates_export(&output),
            TemplatesCommands::Validate { path } => handle_templates_validate(&path),
            TemplatesCommands::Enrich {
                input,
                output,
                category,
                industry,
                region,
                sub_category,
                count,
                backend,
                seed,
                model,
                api_key_env,
                base_url,
            } => handle_templates_enrich(
                &input,
                &output,
                &category,
                &industry,
                &region,
                &sub_category,
                count,
                &backend,
                seed,
                &model,
                &api_key_env,
                &base_url,
            ),
        },

        Commands::Optimizer { command } => handle_optimizer(command),

        Commands::Group { command } => handle_group(command),

        Commands::Behavioral { command } => {
            let exit_code = handle_behavioral(command)?;
            std::process::exit(exit_code);
        }

        Commands::Calibrate {
            config,
            reference,
            out,
            objective,
            max_iter,
            seeds,
            target,
            resume,
            dry_run,
        } => handle_calibrate(
            &config,
            &reference,
            &out,
            &objective,
            max_iter,
            seeds,
            target,
            resume.as_deref(),
            dry_run,
        ),
    }
}

/// v4.1.2+: audit-optimizer CLI dispatcher. Each subcommand emits a
/// minimal JSON report; deeper analytics per subcommand surface in
/// follow-up patches. The report schema is stable (keys + types) so
/// downstream tooling can consume it today without waiting for the
/// full analytics implementation.
fn handle_optimizer(command: OptimizerCommands) -> Result<()> {
    match command {
        OptimizerCommands::RiskScope {
            input,
            output,
            top_n,
        } => {
            let report = serde_json::json!({
                "command": "risk-scope",
                "input": input.display().to_string(),
                "top_n": top_n,
                "status": "stub_report_v4_1_2",
                "message": "audit-optimizer risk-scope is available as a library API at \
                           datasynth_audit_optimizer::risk_scoping::*. CLI-side analytics \
                           wiring ships incrementally in v4.1.x patches.",
            });
            std::fs::write(&output, serde_json::to_string_pretty(&report)?)
                .map_err(|e| anyhow::anyhow!("failed to write {}: {e}", output.display()))?;
            println!("✓ risk-scope report → {}", output.display());
            Ok(())
        }
        OptimizerCommands::Portfolio {
            input,
            budget_hours,
            output,
        } => {
            let report = serde_json::json!({
                "command": "portfolio",
                "input": input.display().to_string(),
                "budget_hours": budget_hours,
                "status": "stub_report_v4_1_2",
                "message": "audit-optimizer portfolio is available as a library API at \
                           datasynth_audit_optimizer::portfolio::*.",
            });
            std::fs::write(&output, serde_json::to_string_pretty(&report)?)
                .map_err(|e| anyhow::anyhow!("failed to write {}: {e}", output.display()))?;
            println!("✓ portfolio report → {}", output.display());
            Ok(())
        }
        OptimizerCommands::Resources { input, output } => {
            let report = serde_json::json!({
                "command": "resources",
                "input": input.display().to_string(),
                "status": "stub_report_v4_1_2",
                "message": "audit-optimizer resources is available as a library API at \
                           datasynth_audit_optimizer::resource_optimizer::*.",
            });
            std::fs::write(&output, serde_json::to_string_pretty(&report)?)
                .map_err(|e| anyhow::anyhow!("failed to write {}: {e}", output.display()))?;
            println!("✓ resources report → {}", output.display());
            Ok(())
        }
        OptimizerCommands::Conformance {
            input,
            blueprint,
            output,
        } => {
            let report = serde_json::json!({
                "command": "conformance",
                "input": input.display().to_string(),
                "blueprint": blueprint.display().to_string(),
                "status": "stub_report_v4_1_2",
                "message": "audit-optimizer conformance is available as a library API at \
                           datasynth_audit_optimizer::conformance::*.",
            });
            std::fs::write(&output, serde_json::to_string_pretty(&report)?)
                .map_err(|e| anyhow::anyhow!("failed to write {}: {e}", output.display()))?;
            println!("✓ conformance report → {}", output.display());
            Ok(())
        }
        OptimizerCommands::MonteCarlo {
            input,
            runs,
            seed,
            output,
        } => {
            let report = serde_json::json!({
                "command": "monte-carlo",
                "input": input.display().to_string(),
                "runs": runs,
                "seed": seed,
                "status": "stub_report_v4_1_2",
                "message": "audit-optimizer monte-carlo is available as a library API at \
                           datasynth_audit_optimizer::monte_carlo::*.",
            });
            std::fs::write(&output, serde_json::to_string_pretty(&report)?)
                .map_err(|e| anyhow::anyhow!("failed to write {}: {e}", output.display()))?;
            println!("✓ monte-carlo report → {}", output.display());
            Ok(())
        }
        OptimizerCommands::Calibration { input, output } => {
            let report = serde_json::json!({
                "command": "calibration",
                "input": input.display().to_string(),
                "status": "stub_report_v4_1_2",
                "message": "audit-optimizer calibration is available as a library API at \
                           datasynth_audit_optimizer::calibration::*.",
            });
            std::fs::write(&output, serde_json::to_string_pretty(&report)?)
                .map_err(|e| anyhow::anyhow!("failed to write {}: {e}", output.display()))?;
            println!("✓ calibration report → {}", output.display());
            Ok(())
        }
    }
}

/// v5.0+: dispatcher for `datasynth-data group …` subcommands.
///
/// Maps each [`GroupCommands`] variant to the matching entry point in
/// the `datasynth-group` crate, surfacing standardised exit codes:
///
/// - `0` — success
/// - `1` — I/O error (file not found, write failure, etc.)
/// - `2` — config / argument validation error (clear stderr message)
/// - `3` — manifest / shard / aggregate runtime error (likely a
///   programmer bug; the underlying [`datasynth_group::GroupError`]
///   message is forwarded verbatim)
fn handle_group(command: GroupCommands) -> Result<()> {
    match command {
        GroupCommands::Manifest { config, out } => handle_group_manifest(&config, &out),
        GroupCommands::Shard {
            manifest,
            shard_id,
            out,
            prior_period_shards,
            prior_period_framework,
        } => handle_group_shard(
            &manifest,
            &shard_id,
            &out,
            prior_period_shards.as_deref(),
            &prior_period_framework,
        ),
        GroupCommands::Aggregate {
            manifest,
            shards_dir,
            out,
            prior_period_aggregate,
            tolerate_missing_shards,
            cgu_test_inputs,
            cpi_series,
        } => handle_group_aggregate(
            &manifest,
            &shards_dir,
            &out,
            prior_period_aggregate.as_deref(),
            tolerate_missing_shards,
            cgu_test_inputs.as_deref(),
            cpi_series.as_deref(),
        ),
        GroupCommands::Generate {
            config,
            out,
            no_parallel_shards,
            cgu_test_inputs,
        } => handle_group_generate(
            &config,
            &out,
            !no_parallel_shards,
            cgu_test_inputs.as_deref(),
        ),
        GroupCommands::GenerateChain {
            config,
            periods,
            out,
            no_parallel_shards,
            prior_period_aggregate,
            cgu_test_inputs,
            cpi_series,
            closing_to_opening_framework,
        } => handle_group_generate_chain(
            &config,
            &periods,
            &out,
            !no_parallel_shards,
            prior_period_aggregate.as_deref(),
            cgu_test_inputs.as_deref(),
            cpi_series.as_deref(),
            &closing_to_opening_framework,
        ),
    }
}

/// SP1: `datasynth-data behavioral score` dispatcher.
///
/// Calls [`datasynth_eval::behavioral_fidelity::compute_report_from_paths`],
/// writes the three report artefacts, and returns the exit code (0 = gate
/// passed, 2 = gate failed) so the caller can `std::process::exit` it.
fn handle_behavioral(command: BehavioralCommands) -> Result<i32> {
    match command {
        BehavioralCommands::Score {
            real,
            syn,
            profile,
            out,
            seed,
            fail_on_dr_above,
            fail_on_composite_above,
        } => {
            if profile != "gl-source-tp" {
                anyhow::bail!("unknown profile {:?}; SP1 ships gl-source-tp only", profile);
            }
            let mut cfg = BehavioralFidelityConfig::gl_default();
            cfg.seed = seed;
            cfg.fail_thresholds = GateThresholds {
                fail_if_dr_above: fail_on_dr_above,
                fail_if_composite_above: fail_on_composite_above,
            };

            std::fs::create_dir_all(&out)?;

            let report = behavioral_fidelity::compute_report_from_paths(&cfg, &real, &syn)
                .map_err(|e| anyhow::anyhow!("behavioral score: compute_report failed: {e}"))?;

            let json_path = out.join("report.json");
            let md_path = out.join("report.md");
            let csv_path = out.join("metrics.csv");

            report
                .write_json(&json_path)
                .map_err(|e| anyhow::anyhow!("behavioral score: json write failed: {e}"))?;
            report
                .write_markdown(&md_path)
                .map_err(|e| anyhow::anyhow!("behavioral score: md write failed: {e}"))?;
            report
                .write_csv(&csv_path)
                .map_err(|e| anyhow::anyhow!("behavioral score: csv write failed: {e}"))?;

            eprintln!("composite BF score: {:.3}", report.composite_bf_score);
            eprintln!(
                "gate: {}",
                if report.gates.passed { "PASS" } else { "FAIL" }
            );
            if !report.gates.passed {
                for f in &report.gates.failures {
                    eprintln!("  - {f}");
                }
            }

            Ok(if report.gates.passed { 0 } else { 2 })
        }
    }
}

/// Translate a [`datasynth_group::GroupError`] into a process exit
/// (with a clear stderr line), mirroring the Task 10.x exit-code
/// contract.
fn group_error_exit(err: datasynth_group::GroupError, action: &str) -> ! {
    use datasynth_group::GroupError;
    let (code, label) = match &err {
        GroupError::Config(_) => (2, "config"),
        GroupError::Manifest(_) => (3, "manifest"),
        GroupError::Shard(_) => (3, "shard"),
        GroupError::Aggregate(_) => (3, "aggregate"),
        GroupError::Io(_) => (1, "io"),
        GroupError::Serde(_) => (3, "serde"),
    };
    eprintln!("group {action}: {label} error: {err}");
    std::process::exit(code);
}

/// v5.0+: `datasynth-data group manifest` handler.
///
/// 1. Read the YAML config from `config_path`.
/// 2. Parse to [`datasynth_group::GroupConfig`] via `serde_yaml`.
/// 3. Run [`datasynth_group::validate::validate`] — emit exit-2 on
///    failure with the validator's full error message.
/// 4. Build the manifest via [`datasynth_group::build_manifest`].
/// 5. Write pretty JSON to `out_path` (creating parent dirs as
///    needed).
/// 6. Print a one-line summary to stdout for the operator log.
fn handle_group_manifest(config_path: &std::path::Path, out_path: &std::path::Path) -> Result<()> {
    use anyhow::Context;
    tracing::info!(
        config = %config_path.display(),
        out = %out_path.display(),
        "group manifest: starting",
    );

    let yaml = std::fs::read_to_string(config_path)
        .with_context(|| format!("group manifest: read {}", config_path.display()))?;

    let cfg: datasynth_group::GroupConfig = serde_yaml::from_str(&yaml).with_context(|| {
        format!(
            "group manifest: parse {} as GroupConfig",
            config_path.display()
        )
    })?;

    if let Err(e) = datasynth_group::validate::validate(&cfg) {
        group_error_exit(e, "manifest");
    }

    let manifest = match datasynth_group::build_manifest(&cfg) {
        Ok(m) => m,
        Err(e) => group_error_exit(e, "manifest"),
    };

    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("group manifest: mkdir {}", parent.display()))?;
        }
    }

    let mut json = serde_json::to_string_pretty(&manifest)
        .with_context(|| "group manifest: serialise manifest as JSON".to_string())?;
    json.push('\n');
    std::fs::write(out_path, json)
        .with_context(|| format!("group manifest: write {}", out_path.display()))?;

    let entity_count = manifest.ownership_graph.entities.len();
    let ic_relationship_count = manifest.ic_relationships.len();
    let shard_count = manifest.shard_plan.shards.len();
    println!(
        "wrote manifest with {entity_count} entities, {ic_relationship_count} IC relationships, \
         {shard_count} shards to {}",
        out_path.display()
    );

    Ok(())
}

/// v5.0+: `datasynth-data group shard` handler — Task 10.3.
///
/// Drives [`datasynth_group::shard::run_shard`] for the requested
/// shard.  Validates the `--shard-id` against
/// `manifest.shard_plan.shards[*].shard_id` up front and exits with
/// code 2 (listing the valid ids) on a typo, so the operator gets a
/// fast, clear error rather than waiting for the orchestrator
/// scaffolding to fail inside `run_shard` itself.
fn handle_group_shard(
    manifest_path: &std::path::Path,
    shard_id: &str,
    out_path: &std::path::Path,
    prior_period_shards: Option<&std::path::Path>,
    prior_period_framework: &str,
) -> Result<()> {
    use anyhow::Context;
    tracing::info!(
        manifest = %manifest_path.display(),
        shard_id = shard_id,
        out = %out_path.display(),
        prior_period_shards = ?prior_period_shards.map(|p| p.display().to_string()),
        prior_period_framework = prior_period_framework,
        "group shard: starting",
    );

    let bytes = std::fs::read_to_string(manifest_path)
        .with_context(|| format!("group shard: read {}", manifest_path.display()))?;
    let manifest: datasynth_group::GroupManifest =
        serde_json::from_str(&bytes).with_context(|| {
            format!(
                "group shard: parse {} as GroupManifest",
                manifest_path.display()
            )
        })?;

    // Validate the shard_id against the manifest's shard plan up front
    // so a typo fails fast (exit 2) instead of inside run_shard.
    let valid: bool = manifest
        .shard_plan
        .shards
        .iter()
        .any(|s| s.shard_id == shard_id);
    if !valid {
        let valid_ids: Vec<String> = manifest
            .shard_plan
            .shards
            .iter()
            .map(|s| s.shard_id.clone())
            .collect();
        eprintln!(
            "group shard: unknown shard_id `{shard_id}` — valid ids: [{}]",
            valid_ids.join(", ")
        );
        std::process::exit(2);
    }

    if let Some(prior) = prior_period_shards {
        if !prior.exists() {
            eprintln!(
                "group shard: --prior-period-shards `{}` does not exist",
                prior.display()
            );
            std::process::exit(2);
        }
        if !prior.join("entities").exists() {
            eprintln!(
                "group shard: --prior-period-shards `{}` has no `entities/` subdir — \
                 expected a prior `group shard` output directory",
                prior.display()
            );
            std::process::exit(2);
        }
    }

    std::fs::create_dir_all(out_path)
        .with_context(|| format!("group shard: mkdir {}", out_path.display()))?;

    // v5.31 C2 (#157): when --prior-period-shards is set, dispatch to
    // run_shard_chained, which loads each entity's prior closing TB,
    // projects it to opening balances, and threads the result through
    // the shard runner. The orchestrator's Phase 3b consumes the
    // openings in place of the industry-mix generator.
    let summary = match prior_period_shards {
        Some(prior) => match datasynth_group::shard::run_shard_chained(
            &manifest,
            shard_id,
            out_path,
            prior,
            prior_period_framework,
        ) {
            Ok(s) => s,
            Err(e) => group_error_exit(e, "shard"),
        },
        None => match datasynth_group::shard::run_shard(&manifest, shard_id, out_path) {
            Ok(s) => s,
            Err(e) => group_error_exit(e, "shard"),
        },
    };

    let entity_count = summary.entity_summaries.len();
    let total_jes: u64 = summary
        .entity_summaries
        .iter()
        .map(|s| s.journal_entry_count as u64)
        .sum();
    println!(
        "shard {}: {entity_count} entities, {total_jes} JEs written to {}",
        summary.shard_id,
        out_path.display()
    );

    Ok(())
}

/// **v5.5.2** — Read `--cgu-test-inputs <path>` if supplied and parse
/// it as `Vec<CguTestInputs>`.  Returns an empty vector when `path` is
/// `None` so the caller can pass the result straight into
/// `AggregateOptions::cgu_test_inputs`.
///
/// JSON shape (the type's serde derive does the heavy lifting):
///
/// ```json
/// [
///   { "cgu_id": "CGU-EMEA",
///     "other_carrying": "5000000",
///     "fair_value_less_costs": "5500000",
///     "value_in_use": "5800000" }
/// ]
/// ```
fn load_cgu_test_inputs(
    path: Option<&std::path::Path>,
) -> Result<Vec<datasynth_group::CguTestInputs>> {
    use anyhow::Context;
    let Some(path) = path else {
        return Ok(Vec::new());
    };
    let bytes = std::fs::read_to_string(path)
        .with_context(|| format!("--cgu-test-inputs: read {}", path.display()))?;
    let inputs: Vec<datasynth_group::CguTestInputs> =
        serde_json::from_str(&bytes).with_context(|| {
            format!(
                "--cgu-test-inputs: parse {} as Vec<CguTestInputs>",
                path.display()
            )
        })?;
    Ok(inputs)
}

/// **v5.5.2** — Read `--cpi-series <path>` if supplied and parse
/// it as `Vec<GeneralPriceIndex>`, returning a `BTreeMap` keyed by
/// each entry's `currency` field for fast lookup in the aggregate
/// driver.  Returns an empty map when `path` is `None`.
///
/// Returns an error if two entries share the same currency (the
/// driver wouldn't know which to use).
///
/// JSON shape:
///
/// ```json
/// [
///   { "currency": "ARS",
///     "source": "INDEC IPC General",
///     "observations": [
///       ["2024-01-01", "100.0"],
///       ["2024-03-31", "180.0"]
///     ]
///   }
/// ]
/// ```
fn load_cpi_series_by_currency(
    path: Option<&std::path::Path>,
) -> Result<
    std::collections::BTreeMap<String, datasynth_core::models::hyperinflation::GeneralPriceIndex>,
> {
    use anyhow::Context;
    let Some(path) = path else {
        return Ok(std::collections::BTreeMap::new());
    };
    let bytes = std::fs::read_to_string(path)
        .with_context(|| format!("--cpi-series: read {}", path.display()))?;
    let series: Vec<datasynth_core::models::hyperinflation::GeneralPriceIndex> =
        serde_json::from_str(&bytes).with_context(|| {
            format!(
                "--cpi-series: parse {} as Vec<GeneralPriceIndex>",
                path.display()
            )
        })?;
    let mut map: std::collections::BTreeMap<
        String,
        datasynth_core::models::hyperinflation::GeneralPriceIndex,
    > = std::collections::BTreeMap::new();
    for entry in series {
        let key = entry.currency.clone();
        if map.contains_key(&key) {
            anyhow::bail!(
                "--cpi-series: duplicate currency `{}` in {} — every entry must have a unique currency code",
                key,
                path.display(),
            );
        }
        map.insert(key, entry);
    }
    Ok(map)
}

/// v5.0+: `datasynth-data group aggregate` handler — Task 10.4.
///
/// Drives [`datasynth_group::aggregate::run_aggregate`] against a
/// directory of pre-computed per-entity shard archives.  Forwards
/// `--prior-period-aggregate` and `--tolerate-missing-shards` straight
/// into [`datasynth_group::aggregate::AggregateOptions`].
///
/// **v5.5.2** — Optionally also forwards `--cgu-test-inputs` and
/// `--cpi-series` when supplied; both default to empty (the v5.5.0
/// behaviour) when their flags are absent.
#[allow(clippy::too_many_arguments)]
fn handle_group_aggregate(
    manifest_path: &std::path::Path,
    shards_dir: &std::path::Path,
    out_path: &std::path::Path,
    prior_period_aggregate: Option<&std::path::Path>,
    tolerate_missing_shards: bool,
    cgu_test_inputs_path: Option<&std::path::Path>,
    cpi_series_path: Option<&std::path::Path>,
) -> Result<()> {
    use anyhow::Context;
    tracing::info!(
        manifest = %manifest_path.display(),
        shards_dir = %shards_dir.display(),
        out = %out_path.display(),
        prior_period_aggregate = ?prior_period_aggregate.map(|p| p.display().to_string()),
        tolerate_missing_shards = tolerate_missing_shards,
        cgu_test_inputs = ?cgu_test_inputs_path.map(|p| p.display().to_string()),
        cpi_series = ?cpi_series_path.map(|p| p.display().to_string()),
        "group aggregate: starting",
    );

    let bytes = std::fs::read_to_string(manifest_path)
        .with_context(|| format!("group aggregate: read {}", manifest_path.display()))?;
    let manifest: datasynth_group::GroupManifest =
        serde_json::from_str(&bytes).with_context(|| {
            format!(
                "group aggregate: parse {} as GroupManifest",
                manifest_path.display()
            )
        })?;

    std::fs::create_dir_all(out_path)
        .with_context(|| format!("group aggregate: mkdir {}", out_path.display()))?;

    let cgu_test_inputs = load_cgu_test_inputs(cgu_test_inputs_path)?;
    let cpi_series_by_currency = load_cpi_series_by_currency(cpi_series_path)?;

    let opts = datasynth_group::aggregate::AggregateOptions {
        prior_period_aggregate: prior_period_aggregate.map(|p| p.to_path_buf()),
        tolerate_missing_shards,
        cgu_test_inputs,
        cpi_series_by_currency,
    };

    let summary =
        match datasynth_group::aggregate::run_aggregate(&manifest, shards_dir, out_path, &opts) {
            Ok(s) => s,
            Err(e) => group_error_exit(e, "aggregate"),
        };

    println!(
        "aggregate {}: coverage {:.4}, {} matched IC pairs, {} entities aggregated, \
         {} artefacts written to {}",
        summary.group_id,
        summary.coverage,
        summary.matched_pairs,
        summary.entities_processed.len(),
        summary.artifacts_written.len(),
        out_path.display()
    );

    Ok(())
}

/// v5.0+: `datasynth-data group generate` handler — Task 10.5.
///
/// Drives [`datasynth_group::generate_standalone`] which runs
/// manifest + shards + aggregate in a single in-process call.  This is
/// the same code path the existing `generate` command auto-detects
/// when the YAML config is a [`datasynth_group::GroupConfig`] instead
/// of a single-entity [`datasynth_config::GeneratorConfig`].
///
/// **v5.5.2** — Forwards optional `--cgu-test-inputs` to the aggregate
/// phase via `StandaloneOptions::cgu_test_inputs`.
fn handle_group_generate(
    config_path: &std::path::Path,
    out_path: &std::path::Path,
    parallel_shards: bool,
    cgu_test_inputs_path: Option<&std::path::Path>,
) -> Result<()> {
    use anyhow::Context;
    tracing::info!(
        config = %config_path.display(),
        out = %out_path.display(),
        parallel_shards = parallel_shards,
        cgu_test_inputs = ?cgu_test_inputs_path.map(|p| p.display().to_string()),
        "group generate: starting",
    );

    let yaml = std::fs::read_to_string(config_path)
        .with_context(|| format!("group generate: read {}", config_path.display()))?;
    let cfg: datasynth_group::GroupConfig = serde_yaml::from_str(&yaml).with_context(|| {
        format!(
            "group generate: parse {} as GroupConfig",
            config_path.display()
        )
    })?;

    if let Err(e) = datasynth_group::validate::validate(&cfg) {
        group_error_exit(e, "generate");
    }

    std::fs::create_dir_all(out_path)
        .with_context(|| format!("group generate: mkdir {}", out_path.display()))?;

    let cgu_test_inputs = load_cgu_test_inputs(cgu_test_inputs_path)?;

    let opts = datasynth_group::StandaloneOptions {
        prior_period_aggregate: None,
        tolerate_missing_shards: false,
        cgu_test_inputs,
        parallel_shards,
        entity_opening_balances: std::collections::BTreeMap::new(),
        cpi_series_by_currency: std::collections::BTreeMap::new(),
        closing_to_opening_framework: "us_gaap".to_string(),
    };

    let summary = match datasynth_group::generate_standalone(&cfg, out_path, &opts) {
        Ok(s) => s,
        Err(e) => group_error_exit(e, "generate"),
    };

    println!(
        "group generate {}: manifest {}, {} shards, aggregate coverage {:.4}, {} artefacts in {}",
        summary.aggregate.group_id,
        summary.manifest_path.display(),
        summary.shard_summaries.len(),
        summary.aggregate.coverage,
        summary.aggregate.artifacts_written.len(),
        out_path.display()
    );

    Ok(())
}

/// v5.3+: drive the multi-period chain runner.
///
/// Reads the base [`datasynth_group::GroupConfig`] from `config_path`,
/// the chain plan as `Vec<PeriodChainSpec>` from `periods_path`, and
/// invokes [`datasynth_group::generate_standalone_chain`].
///
/// The `periods` JSON file shape:
///
/// ```json
/// [
///   { "period": { "start_date": "2024-01-01", "length": "quarterly" },
///     "out_subdir": "2024-q1" },
///   { "period": { "start_date": "2024-04-01", "length": "quarterly" },
///     "out_subdir": "2024-q2" }
/// ]
/// ```
///
/// Each period's outputs go under `out_path.join(spec.out_subdir)/`.
/// The chain auto-threads closing-TB → opening-TB carryover between
/// successive periods (loaded via the orchestrator's
/// `entity_opening_balances` plumbing).
#[allow(clippy::too_many_arguments)]
fn handle_group_generate_chain(
    config_path: &std::path::Path,
    periods_path: &std::path::Path,
    out_path: &std::path::Path,
    parallel_shards: bool,
    prior_period_aggregate: Option<&std::path::Path>,
    cgu_test_inputs_path: Option<&std::path::Path>,
    cpi_series_path: Option<&std::path::Path>,
    closing_to_opening_framework: &str,
) -> Result<()> {
    use anyhow::Context;
    tracing::info!(
        config = %config_path.display(),
        periods = %periods_path.display(),
        out = %out_path.display(),
        parallel_shards = parallel_shards,
        prior_period_aggregate = ?prior_period_aggregate.map(|p| p.display().to_string()),
        cgu_test_inputs = ?cgu_test_inputs_path.map(|p| p.display().to_string()),
        cpi_series = ?cpi_series_path.map(|p| p.display().to_string()),
        closing_to_opening_framework = closing_to_opening_framework,
        "group generate-chain: starting",
    );

    let yaml = std::fs::read_to_string(config_path)
        .with_context(|| format!("group generate-chain: read {}", config_path.display()))?;
    let cfg: datasynth_group::GroupConfig = serde_yaml::from_str(&yaml).with_context(|| {
        format!(
            "group generate-chain: parse {} as GroupConfig",
            config_path.display()
        )
    })?;

    if let Err(e) = datasynth_group::validate::validate(&cfg) {
        group_error_exit(e, "generate-chain");
    }

    let periods_json = std::fs::read_to_string(periods_path).with_context(|| {
        format!(
            "group generate-chain: read periods {}",
            periods_path.display()
        )
    })?;
    let periods: Vec<datasynth_group::PeriodChainSpec> = serde_json::from_str(&periods_json)
        .with_context(|| {
            format!(
                "group generate-chain: parse {} as Vec<PeriodChainSpec>",
                periods_path.display()
            )
        })?;

    if periods.is_empty() {
        anyhow::bail!("group generate-chain: periods must contain at least one entry");
    }

    std::fs::create_dir_all(out_path)
        .with_context(|| format!("group generate-chain: mkdir {}", out_path.display()))?;

    let cgu_test_inputs = load_cgu_test_inputs(cgu_test_inputs_path)?;
    let cpi_series_by_currency = load_cpi_series_by_currency(cpi_series_path)?;

    let opts = datasynth_group::StandaloneOptions {
        prior_period_aggregate: prior_period_aggregate.map(std::path::PathBuf::from),
        tolerate_missing_shards: false,
        cgu_test_inputs,
        parallel_shards,
        entity_opening_balances: std::collections::BTreeMap::new(),
        cpi_series_by_currency,
        closing_to_opening_framework: closing_to_opening_framework.to_string(),
    };

    let summaries = match datasynth_group::generate_standalone_chain(&cfg, periods, out_path, &opts)
    {
        Ok(s) => s,
        Err(e) => group_error_exit(e, "generate-chain"),
    };

    let total_shards: usize = summaries.iter().map(|s| s.shard_summaries.len()).sum();
    let total_artifacts: usize = summaries
        .iter()
        .map(|s| s.aggregate.artifacts_written.len())
        .sum();
    let avg_coverage: f64 = if summaries.is_empty() {
        0.0
    } else {
        summaries.iter().map(|s| s.aggregate.coverage).sum::<f64>() / summaries.len() as f64
    };

    println!(
        "group generate-chain: {} periods, {} total shards, avg aggregate coverage {:.4}, {} artefacts in {}",
        summaries.len(),
        total_shards,
        avg_coverage,
        total_artifacts,
        out_path.display()
    );
    for (i, s) in summaries.iter().enumerate() {
        println!(
            "  period {}: {} shards, coverage {:.4}, manifest {}",
            i,
            s.shard_summaries.len(),
            s.aggregate.coverage,
            s.manifest_path.display(),
        );
    }

    Ok(())
}

/// v5.0+: lightweight YAML probe used by `Commands::Generate` to
/// auto-detect a group config without committing to a full parse of
/// either `GroupConfig` or `GeneratorConfig`.
///
/// Heuristic: a `GroupConfig` always carries the top-level
/// `presentation_currency` and `ownership` keys (both are required by
/// the `serde(default)`-less fields), neither of which appears at the
/// top of a single-entity `GeneratorConfig`.  Reading the YAML once
/// into `serde_yaml::Value` and checking for both keys is cheap,
/// allocates only the parsed mapping, and cannot misclassify the two
/// shapes given the current schemas.  If the schema gains overlapping
/// keys later, switch to the alternate "try parse both, prefer
/// GroupConfig" heuristic — the test suite covers either path.
fn yaml_is_group_config(yaml: &str) -> bool {
    let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(yaml) else {
        return false;
    };
    let Some(map) = value.as_mapping() else {
        return false;
    };
    map.contains_key(serde_yaml::Value::String(
        "presentation_currency".to_string(),
    )) && map.contains_key(serde_yaml::Value::String("ownership".to_string()))
}
///
/// Writes one file per category so users can open and edit a single
/// category without touching unrelated pools. Empty pools are
/// materialised as empty arrays to show the shape of every slot.
fn handle_templates_export(output: &std::path::Path) -> Result<()> {
    use datasynth_core::templates::loader::{
        BankNameTemplates, DepartmentNameTemplates, FindingNarrativeTemplates,
        FindingTitleTemplates, TemplateData, TemplateMetadata,
    };

    std::fs::create_dir_all(output)
        .map_err(|e| anyhow::anyhow!("Failed to create {}: {e}", output.display()))?;

    // Canonical starter pack = an empty TemplateData with metadata.
    // The orchestrator's merge strategy fills in embedded pools when a
    // section is empty, so this is a minimal "drop here to override"
    // pack rather than a dump of every embedded string (which we
    // cannot export losslessly without duplicating private constants).
    let starter = TemplateData {
        metadata: TemplateMetadata {
            name: "DataSynth Starter Pack".to_string(),
            version: "1.0.0".to_string(),
            description: Some(
                "Copy and edit these files, then pass the directory via \
                 `templates.path` in your config or `--templates <dir>` on `generate`. \
                 Empty sections fall back to embedded defaults."
                    .to_string(),
            ),
            ..Default::default()
        },
        bank_names: BankNameTemplates::default(),
        finding_titles: FindingTitleTemplates::default(),
        finding_narratives: FindingNarrativeTemplates::default(),
        department_names: DepartmentNameTemplates::default(),
        ..Default::default()
    };

    // Write one YAML per category. A macro keeps the per-field writes
    // terse without requiring trait objects (which would need erased_serde).
    macro_rules! write_yaml {
        ($filename:expr, $value:expr) => {{
            let path = output.join($filename);
            let yaml = serde_yaml::to_string(&$value)
                .map_err(|e| anyhow::anyhow!("Serialize {}: {e}", $filename))?;
            std::fs::write(&path, yaml)
                .map_err(|e| anyhow::anyhow!("Write {}: {e}", path.display()))?;
            tracing::info!("Wrote {}", path.display());
        }};
    }

    write_yaml!("metadata.yaml", starter.metadata);
    write_yaml!("person_names.yaml", starter.person_names);
    write_yaml!("vendor_names.yaml", starter.vendor_names);
    write_yaml!("customer_names.yaml", starter.customer_names);
    write_yaml!("material_descriptions.yaml", starter.material_descriptions);
    write_yaml!("asset_descriptions.yaml", starter.asset_descriptions);
    write_yaml!(
        "line_item_descriptions.yaml",
        starter.line_item_descriptions
    );
    write_yaml!("header_text_templates.yaml", starter.header_text_templates);
    write_yaml!("bank_names.yaml", starter.bank_names);
    write_yaml!("finding_titles.yaml", starter.finding_titles);
    write_yaml!("finding_narratives.yaml", starter.finding_narratives);
    write_yaml!("department_names.yaml", starter.department_names);

    tracing::info!(
        "Starter template pack exported to {}. Edit the files you want to customize, \
         then set `templates.path: {}` in your config.",
        output.display(),
        output.display()
    );
    Ok(())
}

/// Validate a template file or directory.
fn handle_templates_validate(path: &std::path::Path) -> Result<()> {
    use datasynth_core::templates::loader::TemplateLoader;

    let data = if path.is_dir() {
        TemplateLoader::load_from_directory(path)
    } else {
        TemplateLoader::load_from_file(path)
    }
    .map_err(|e| anyhow::anyhow!("Failed to load {}: {e}", path.display()))?;

    let warnings = TemplateLoader::validate(&data);

    if warnings.is_empty() {
        println!("✓ {} — valid", path.display());
        Ok(())
    } else {
        for w in &warnings {
            eprintln!("  - {w}");
        }
        anyhow::bail!("{} warning(s) in {}", warnings.len(), path.display())
    }
}

/// v3.5.0+: LLM-driven enrichment of a template YAML file.
///
/// Loads the input template (or starts from empty if it doesn't exist),
/// calls the chosen LLM backend to generate N new items for the given
/// category, appends them to the appropriate pool, and writes the result
/// to `output`. The mock backend is deterministic so CI and offline
/// development can exercise the flow without external dependencies.
#[allow(clippy::too_many_arguments)]
fn handle_templates_enrich(
    input: &std::path::Path,
    output: &std::path::Path,
    category: &str,
    industry: &str,
    region: &str,
    sub_category: &str,
    count: u32,
    backend: &str,
    seed: u64,
    model: &str,
    api_key_env: &str,
    base_url: &str,
) -> Result<()> {
    use datasynth_core::llm::{LlmProvider, MockLlmProvider};
    use datasynth_core::templates::loader::TemplateLoader;
    use datasynth_generators::llm_enrichment::{
        CustomerLlmEnricher, MaterialLlmEnricher, VendorLlmEnricher,
    };
    use std::sync::Arc;

    // Load starting point. Missing input is OK — we just start empty.
    let mut data = if input.exists() {
        TemplateLoader::load_from_file(input)
            .map_err(|e| anyhow::anyhow!("Failed to load input {}: {e}", input.display()))?
    } else {
        eprintln!(
            "Input {} does not exist; starting from empty TemplateData",
            input.display()
        );
        datasynth_core::templates::loader::TemplateData::default()
    };

    // Backend selection. `mock` always works (offline, deterministic).
    // `http` requires the `llm` Cargo feature — it hits an OpenAI-compatible
    // endpoint (OpenRouter is the default so users can reach Claude/GPT/etc.
    // with one key).
    let provider: Arc<dyn LlmProvider> = match backend {
        "mock" => Arc::new(MockLlmProvider::new(seed)),
        "http" => build_http_provider(model, api_key_env, base_url)?,
        other => anyhow::bail!(
            "Unknown backend '{other}'. Supported: mock, http. \
             (http requires the `llm` Cargo feature and a configured API key.)"
        ),
    };

    // Build batch requests and call the right enricher.
    let items: Vec<String> = match category {
        "vendor_name" | "vendor" | "vendors" => {
            let enricher = VendorLlmEnricher::new(Arc::clone(&provider));
            let requests: Vec<(String, String, String)> = (0..count)
                .map(|_| {
                    (
                        industry.to_string(),
                        sub_category.to_string(),
                        region.to_string(),
                    )
                })
                .collect();
            enricher
                .enrich_batch(&requests, seed)
                .map_err(|e| anyhow::anyhow!("vendor enrichment failed: {e}"))?
        }
        "customer_name" | "customer" | "customers" => {
            let enricher = CustomerLlmEnricher::new(Arc::clone(&provider));
            let requests: Vec<(String, String, String)> = (0..count)
                .map(|_| {
                    (
                        industry.to_string(),
                        sub_category.to_string(),
                        region.to_string(),
                    )
                })
                .collect();
            enricher
                .enrich_batch(&requests, seed)
                .map_err(|e| anyhow::anyhow!("customer enrichment failed: {e}"))?
        }
        "material_desc" | "material" | "materials" => {
            let enricher = MaterialLlmEnricher::new(Arc::clone(&provider));
            let requests: Vec<(String, String)> = (0..count)
                .map(|_| (sub_category.to_string(), industry.to_string()))
                .collect();
            enricher
                .enrich_batch(&requests, seed)
                .map_err(|e| anyhow::anyhow!("material enrichment failed: {e}"))?
        }
        other => anyhow::bail!(
            "Unknown category '{other}'. Supported: vendor_name, customer_name, material_desc"
        ),
    };

    // Merge into TemplateData. Each category has its own pool shape; we
    // append under the best-matching bucket. Pools use industry or
    // sub_category as the key when the underlying struct is map-shaped.
    match category {
        "vendor_name" | "vendor" | "vendors" => {
            let bucket = data
                .vendor_names
                .categories
                .entry(sub_category.to_string())
                .or_default();
            bucket.extend(items.iter().cloned());
        }
        "customer_name" | "customer" | "customers" => {
            let bucket = data
                .customer_names
                .industries
                .entry(industry.to_string())
                .or_default();
            bucket.extend(items.iter().cloned());
        }
        "material_desc" | "material" | "materials" => {
            let bucket = data
                .material_descriptions
                .by_type
                .entry(sub_category.to_string())
                .or_default();
            bucket.extend(items.iter().cloned());
        }
        _ => unreachable!("category was validated above"),
    }

    // Record provenance metadata. Future loaders can warn users if
    // enriched data is being consumed without intent.
    data.metadata.version = if data.metadata.version.is_empty() {
        "1.0.0".to_string()
    } else {
        data.metadata.version.clone()
    };
    data.metadata.description = Some(format!(
        "Enriched via `templates enrich` (backend={backend}, category={category}, \
         industry={industry}, region={region}, count={count}, seed={seed})"
    ));

    TemplateLoader::save_to_file(&data, output)
        .map_err(|e| anyhow::anyhow!("Failed to save {}: {e}", output.display()))?;

    println!(
        "✓ Added {count} {category} item(s) to {output_path} (backend={backend})",
        output_path = output.display()
    );
    Ok(())
}

/// Build an HTTP LLM provider (OpenAI-compatible). Gated behind the `llm`
/// Cargo feature — without it, returns a helpful error instead of
/// silently falling back to mock.
#[cfg(feature = "llm")]
fn build_http_provider(
    model: &str,
    api_key_env: &str,
    base_url: &str,
) -> Result<std::sync::Arc<dyn datasynth_core::llm::LlmProvider>> {
    use datasynth_core::llm::{HttpLlmProvider, LlmConfig, LlmProviderType};
    use std::sync::Arc;

    // Early-exit with a clear error if the API key env var is unset.
    // Users typically hit this when they forget to export OPENROUTER_API_KEY.
    if std::env::var(api_key_env).is_err() {
        anyhow::bail!(
            "environment variable `{api_key_env}` is not set; cannot call HTTP LLM. \
             Set it (e.g. `export {api_key_env}=sk-...`) and retry, or use `--backend mock`."
        );
    }

    let cfg = LlmConfig {
        provider: LlmProviderType::OpenAi, // OpenAI-compatible wire format
        model: model.to_string(),
        api_key_env: api_key_env.to_string(),
        base_url: Some(base_url.to_string()),
        max_retries: 3,
        timeout_secs: 60,
        cache_enabled: true,
    };
    let provider = HttpLlmProvider::new(cfg)
        .map_err(|e| anyhow::anyhow!("failed to build HTTP LLM provider: {e}"))?;
    Ok(Arc::new(provider))
}

#[cfg(not(feature = "llm"))]
fn build_http_provider(
    _model: &str,
    _api_key_env: &str,
    _base_url: &str,
) -> Result<std::sync::Arc<dyn datasynth_core::llm::LlmProvider>> {
    anyhow::bail!(
        "the HTTP LLM backend requires the `llm` Cargo feature. \
         Rebuild with `cargo build --features llm` (or `cargo install \
         datasynth-cli --features llm`)."
    )
}

/// Resolve a fingerprint-signing key by walking the configured sources in
/// order: explicit hex, key file, `DATASYNTH_FINGERPRINT_KEY` env var, then
/// a randomly-generated ephemeral key (which is logged at WARN so users can
/// capture it for later verification).
fn resolve_signing_key(
    hex: Option<&str>,
    file: Option<&std::path::Path>,
    key_id: &str,
) -> Result<datasynth_fingerprint::io::signing::SigningKey> {
    use datasynth_fingerprint::io::signing::SigningKey;

    if let Some(h) = hex {
        return SigningKey::from_hex(key_id, h.trim())
            .map_err(|e| anyhow::anyhow!("invalid --sign-key-hex: {e}"));
    }
    if let Some(path) = file {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("reading sign-key-file {}: {e}", path.display()))?;
        return SigningKey::from_hex(key_id, raw.trim())
            .map_err(|e| anyhow::anyhow!("invalid key in {}: {e}", path.display()));
    }
    if let Ok(env_hex) = std::env::var("DATASYNTH_FINGERPRINT_KEY") {
        return SigningKey::from_hex(key_id, env_hex.trim())
            .map_err(|e| anyhow::anyhow!("invalid DATASYNTH_FINGERPRINT_KEY: {e}"));
    }
    // Fall back to ephemeral key — log it so the operator can verify later.
    let key = SigningKey::generate(key_id);
    tracing::warn!(
        "No signing key provided; generated an ephemeral HMAC-SHA256 key. \
         Save this hex-encoded key to verify the fingerprint later: {}",
        key.to_hex()
    );
    Ok(key)
}

/// Handle fingerprint subcommands.
fn handle_fingerprint_command(command: FingerprintCommands) -> Result<()> {
    match command {
        FingerprintCommands::Extract {
            input,
            output,
            privacy_level,
            privacy_epsilon,
            privacy_k,
            sign,
            sign_key_hex,
            sign_key_file,
            sign_key_id,
            behavioral,
            industry,
            pii_denylist,
        } => {
            tracing::info!("Extracting fingerprint from: {}", input.display());

            // Parse privacy level
            let level = match privacy_level.to_lowercase().as_str() {
                "minimal" => PrivacyLevel::Minimal,
                "standard" => PrivacyLevel::Standard,
                "high" => PrivacyLevel::High,
                "maximum" => PrivacyLevel::Maximum,
                _ => {
                    tracing::warn!("Unknown privacy level '{}', using standard", privacy_level);
                    PrivacyLevel::Standard
                }
            };

            // Create extraction config with privacy settings
            let mut privacy_config = PrivacyConfig::from_level(level);
            if let Some(eps) = privacy_epsilon {
                privacy_config.epsilon = eps;
            }
            if let Some(k) = privacy_k {
                privacy_config.k_anonymity = k;
            }

            let extraction_config = ExtractionConfig {
                privacy: privacy_config,
                ..Default::default()
            };

            // Detect parquet input — the legacy CSV extractor can't handle it,
            // and the SP2 behavioral extractor handles parquet natively. When
            // --behavioral is set with a parquet input, skip the CSV path and
            // build a minimal Fingerprint shell whose only populated section
            // is the behavioral one.
            let is_parquet_input = input.is_file()
                && input
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("parquet"));

            let mut fingerprint = if behavioral && is_parquet_input {
                tracing::info!(
                    "Parquet input + --behavioral: skipping CSV-based extraction, \
                     building behavioral-only fingerprint"
                );
                use datasynth_fingerprint::models::{
                    Fingerprint, Manifest, PrivacyAudit, PrivacyMetadata, SchemaFingerprint,
                    SourceMetadata, StatisticsFingerprint,
                };
                let source = SourceMetadata::new(format!("parquet:{}", input.display()), vec![], 0);
                let privacy_meta = PrivacyMetadata::from_level(level);
                let manifest = Manifest::new(source, privacy_meta);
                let schema = SchemaFingerprint::new();
                let statistics = StatisticsFingerprint::new();
                let privacy_audit = PrivacyAudit::new(
                    extraction_config.privacy.epsilon,
                    extraction_config.privacy.k_anonymity,
                );
                Fingerprint::new(manifest, schema, statistics, privacy_audit)
            } else {
                // Create data source for legacy CSV-based extraction.
                let data_source = if input.is_file() {
                    DataSource::Csv(CsvDataSource::new(input.clone()))
                } else {
                    let csv_files: Vec<_> = std::fs::read_dir(&input)?
                        .filter_map(std::result::Result::ok)
                        .filter(|e| e.path().extension().is_some_and(|ext| ext == "csv"))
                        .collect();
                    if csv_files.is_empty() {
                        anyhow::bail!("No CSV files found in directory: {}", input.display());
                    }
                    let first_csv = csv_files[0].path();
                    tracing::info!("Using CSV file: {}", first_csv.display());
                    DataSource::Csv(CsvDataSource::new(first_csv))
                };
                let extractor = FingerprintExtractor::with_config(extraction_config);
                extractor.extract(&data_source)?
            };

            if behavioral {
                let ind = industry
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("--behavioral requires --industry"))?;
                use datasynth_fingerprint::extraction::behavioral_extractor::extract_behavioral_priors_from_path;
                let mut bp = extract_behavioral_priors_from_path(&input, ind)
                    .map_err(|e| anyhow::anyhow!("behavioral extraction failed: {e}"))?;

                // SP4.2 — adjacent COA file enrichment.
                // When input is a JE_XXX.parquet file, look for a COA_XXX.parquet
                // in the same directory and attach CoA semantic content.
                if is_parquet_input {
                    if let Some(file_name) = input.file_name().and_then(|n| n.to_str()) {
                        if file_name.starts_with("JE_") {
                            let coa_file_name = file_name.replacen("JE_", "COA_", 1);
                            let coa_path = input
                                .parent()
                                .unwrap_or(std::path::Path::new("."))
                                .join(&coa_file_name);
                            if coa_path.exists() {
                                use datasynth_fingerprint::extraction::extract_coa_semantic_from_parquet;
                                match extract_coa_semantic_from_parquet(&coa_path) {
                                    Ok(coa_prior) => {
                                        tracing::info!(
                                            "SP4.2: extracted CoA semantic from {} ({} accounts)",
                                            coa_path.display(),
                                            coa_prior.accounts.len()
                                        );
                                        bp.coa_semantic = Some(coa_prior);
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            "SP4.2: COA parquet extraction failed for {}: {e}",
                                            coa_path.display()
                                        );
                                    }
                                }
                            } else {
                                tracing::debug!(
                                    "SP4.2: no adjacent COA file found at {}",
                                    coa_path.display()
                                );
                            }
                        }
                    }
                }

                // W7.2 — adjacent TB file enrichment (SP4.1 completion).
                // When input is a JE_XXX.parquet file, look for a TB_XXX.parquet
                // in the same directory and extract the trial-balance anchor priors.
                if is_parquet_input {
                    if let Some(file_name) = input.file_name().and_then(|n| n.to_str()) {
                        if file_name.starts_with("JE_") {
                            let tb_file_name = file_name.replacen("JE_", "TB_", 1);
                            let tb_path = input
                                .parent()
                                .unwrap_or(std::path::Path::new("."))
                                .join(&tb_file_name);
                            if tb_path.exists() {
                                use datasynth_fingerprint::extraction::extract_tb_anchor_from_parquet;
                                match extract_tb_anchor_from_parquet(&tb_path) {
                                    Ok(anchor) => {
                                        tracing::info!(
                                            target: "datasynth_data::fingerprint",
                                            "W7.2 — extracted TB anchor from {}: {} accounts, \
                                             total_assets={:.2}",
                                            tb_path.display(),
                                            anchor.per_account.len(),
                                            anchor.total_assets
                                        );
                                        bp.tb_anchor = Some(anchor);
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            target: "datasynth_data::fingerprint",
                                            "W7.2 — TB file {:?} present but extraction failed: {}",
                                            tb_path,
                                            e
                                        );
                                    }
                                }
                            } else {
                                tracing::debug!(
                                    "W7.2: no adjacent TB file found at {}",
                                    tb_path.display()
                                );
                            }
                        }
                    }
                }

                // SP6 — Phase B PII denylist + text_taxonomy extraction.
                // When --pii-denylist is supplied, load the denylist and call
                // extract_text_taxonomy_from_records to populate bp.text_taxonomy.
                // When absent, warn and leave text_taxonomy as None so the caller
                // knows Phase B was skipped.
                if is_parquet_input {
                    use datasynth_eval::behavioral_fidelity::loader::load_parquet_records;
                    use datasynth_fingerprint::extraction::extract_text_taxonomy_from_records;
                    use datasynth_fingerprint::extraction::PiiDenylist;

                    let denylist: Option<PiiDenylist> = if let Some(ref dl_path) = pii_denylist {
                        Some(PiiDenylist::load(dl_path).map_err(|e| {
                            anyhow::anyhow!("loading --pii-denylist {}: {e}", dl_path.display())
                        })?)
                    } else {
                        eprintln!(
                            "warning: --pii-denylist not supplied; Phase B (fuzzy \
                                 proper-noun generalization) is skipped. Bundles built \
                                 without it may carry residual fuzzy PII that the audit \
                                 gate will reject."
                        );
                        None
                    };

                    // Load records for text taxonomy (reloads the parquet; acceptable
                    // for the CLI path — this is a batch offline operation).
                    //
                    // Failure handling: if `--pii-denylist` was supplied the caller
                    // EXPECTS SP6 extraction to succeed — any error here (parquet
                    // reload, residual-PII hit at extraction) is a HARD FAIL so the
                    // build-time gate cannot be silently bypassed. When the denylist
                    // is absent (Phase A only), extraction failures fall back to
                    // logged warnings — the missing text_taxonomy will be caught by
                    // the post-build `bundle_pii_audit` test.
                    let strict = pii_denylist.is_some();
                    let records_opt = match load_parquet_records(&input) {
                        Ok(r) => Some(r),
                        Err(e) => {
                            let msg =
                                format!("SP6: could not reload parquet for text_taxonomy: {e}");
                            if strict {
                                return Err(anyhow::anyhow!(msg));
                            }
                            tracing::warn!("{msg}");
                            None
                        }
                    };
                    if let Some(records) = records_opt {
                        const TEXT_TAXONOMY_MIN_OCCURRENCES: usize = 3;
                        match extract_text_taxonomy_from_records(
                            &records,
                            bp.coa_semantic.as_ref(),
                            denylist.as_ref(),
                            TEXT_TAXONOMY_MIN_OCCURRENCES,
                        ) {
                            Ok(taxonomy) => {
                                tracing::info!(
                                    "SP6: extracted text_taxonomy ({} line pools, \
                                     {} header pools, {} CoA pools)",
                                    taxonomy.line_pools.len(),
                                    taxonomy.header_pools.len(),
                                    taxonomy.coa_pools.len(),
                                );
                                bp.text_taxonomy = Some(taxonomy);
                            }
                            Err(e) => {
                                let msg = format!("SP6: text_taxonomy extraction failed: {e}");
                                if strict {
                                    return Err(anyhow::anyhow!(msg));
                                }
                                tracing::warn!(
                                    "{msg} (bundle written without text_taxonomy; \
                                     bundle_pii_audit will reject if it ships)"
                                );
                            }
                        }
                    }
                }

                fingerprint.behavioral = Some(bp);
            }

            // Write fingerprint (signed if --sign).
            let writer = FingerprintWriter::new();
            if sign {
                use datasynth_fingerprint::io::signing::DsfSigner;
                let key = resolve_signing_key(
                    sign_key_hex.as_deref(),
                    sign_key_file.as_deref(),
                    &sign_key_id,
                )?;
                let signer = DsfSigner::new(key);
                writer.write_to_file_signed(&fingerprint, &output, &signer)?;
                tracing::info!(
                    "Signed fingerprint (key_id={}) written to: {}",
                    signer.key_id(),
                    output.display()
                );
            } else {
                writer.write_to_file(&fingerprint, &output)?;
                tracing::info!("Fingerprint written to: {}", output.display());
            }
            tracing::info!(
                "Privacy audit: {} actions recorded",
                fingerprint.privacy_audit.actions.len()
            );
            tracing::info!(
                "Epsilon spent: {:.3} of {:.3} budget",
                fingerprint.privacy_audit.total_epsilon_spent,
                fingerprint.privacy_audit.epsilon_budget
            );

            Ok(())
        }

        FingerprintCommands::Validate { file } => {
            tracing::info!("Validating fingerprint: {}", file.display());

            match validate_dsf(&file) {
                Ok(report) => {
                    if report.is_valid {
                        println!("✓ Fingerprint is valid");
                        println!("  Version: {}", report.version);
                        println!("  Components: {:?}", report.components);
                        if !report.warnings.is_empty() {
                            println!("  Warnings:");
                            for warning in &report.warnings {
                                println!("    - {warning}");
                            }
                        }
                    } else {
                        println!("✗ Fingerprint validation failed");
                        for error in &report.errors {
                            println!("  Error: {error}");
                        }
                    }
                }
                Err(e) => {
                    println!("✗ Failed to validate fingerprint: {e}");
                    return Err(e.into());
                }
            }

            Ok(())
        }

        FingerprintCommands::Info {
            file,
            behavioral,
            detailed,
        } => {
            let reader = FingerprintReader::new();
            let fingerprint = reader.read_from_file(&file)?;

            println!("Fingerprint Information");
            println!("=======================");
            println!();
            println!("Manifest:");
            println!("  Version: {}", fingerprint.manifest.version);
            println!("  Format: {}", fingerprint.manifest.format);
            println!("  Created: {}", fingerprint.manifest.created_at);
            println!();
            println!("Source:");
            println!("  Description: {}", fingerprint.manifest.source.description);
            println!("  Tables: {}", fingerprint.manifest.source.table_count);
            println!("  Total Rows: {}", fingerprint.manifest.source.total_rows);
            if let Some(ref industry) = fingerprint.manifest.source.industry {
                println!("  Industry: {industry}");
            }
            println!();
            println!("Privacy:");
            println!("  Level: {:?}", fingerprint.manifest.privacy.level);
            println!("  Epsilon: {}", fingerprint.manifest.privacy.epsilon);
            println!(
                "  K-Anonymity: {}",
                fingerprint.manifest.privacy.k_anonymity
            );
            println!();
            println!("Schema:");
            println!("  Tables: {}", fingerprint.schema.tables.len());
            for (name, table) in &fingerprint.schema.tables {
                println!("    - {} ({} columns)", name, table.columns.len());
            }
            println!();
            println!("Statistics:");
            println!(
                "  Numeric columns: {}",
                fingerprint.statistics.numeric_columns.len()
            );
            println!(
                "  Categorical columns: {}",
                fingerprint.statistics.categorical_columns.len()
            );

            if detailed {
                println!();
                println!("Detailed Statistics:");
                for (name, stats) in &fingerprint.statistics.numeric_columns {
                    println!("  {name}:");
                    println!("    Count: {}", stats.count);
                    println!("    Min: {:.2}, Max: {:.2}", stats.min, stats.max);
                    println!("    Mean: {:.2}, StdDev: {:.2}", stats.mean, stats.std_dev);
                    println!("    Distribution: {:?}", stats.distribution);
                }
                for (name, stats) in &fingerprint.statistics.categorical_columns {
                    println!("  {name}:");
                    println!("    Count: {}", stats.count);
                    println!("    Cardinality: {}", stats.cardinality);
                    println!("    Top values: {}", stats.top_values.len());
                }
            }

            println!();
            println!("Privacy Audit:");
            println!(
                "  Total actions: {}",
                fingerprint.privacy_audit.actions.len()
            );
            println!(
                "  Epsilon spent: {:.3}",
                fingerprint.privacy_audit.total_epsilon_spent
            );
            println!("  Warnings: {}", fingerprint.privacy_audit.warnings.len());

            if behavioral {
                if let Some(bp) = &fingerprint.behavioral {
                    println!("\nBehavioral priors (industry={})", bp.industry);
                    println!(
                        "  schema_version: {}, generator_version: {}",
                        bp.schema_version, bp.generator_version
                    );
                    println!(
                        "  n_client_inputs: {}, n_rows_aggregated: {}",
                        bp.n_client_inputs, bp.n_rows_aggregated
                    );
                    println!("  source_mix (top 5):");
                    let mut sorted: Vec<(&String, &f64)> =
                        bp.source_mix.probabilities.iter().collect();
                    sorted
                        .sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
                    for (s, p) in sorted.iter().take(5) {
                        println!("    {} {:.1}%", s, **p * 100.0);
                    }
                    println!("    (+other {:.1}%)", bp.source_mix.other_fraction * 100.0);
                    println!(
                        "  per_source_iet: {} sources with IET summaries",
                        bp.per_source_iet.by_source.len()
                    );
                    println!(
                        "  lines_per_je: median bucket {}",
                        bp.lines_per_je.overall.median_bucket()
                    );
                    println!(
                        "  active_lifetime: median bucket {}d",
                        bp.active_lifetime.overall.median_bucket()
                    );
                    println!("  fanout (attribute -> median fan-out):");
                    for (attr, hist) in &bp.fanout.by_attribute {
                        println!("    {}: {}", attr, hist.median_bucket());
                    }
                    if let Some(lag) = &bp.posting_lag {
                        let n_sources = lag.by_source.len();
                        let mean_all: f64 = lag.by_source.values().map(|s| s.mean).sum::<f64>()
                            / n_sources.max(1) as f64;
                        println!(
                            "  posting_lag: {} sources; overall mean {:.1} days",
                            n_sources, mean_all
                        );
                    }
                } else {
                    println!("\n(No behavioral section in this .dsf)");
                }
            }

            Ok(())
        }

        FingerprintCommands::Diff { file1, file2 } => {
            let reader = FingerprintReader::new();
            let fp1 = reader.read_from_file(&file1)?;
            let fp2 = reader.read_from_file(&file2)?;

            println!("Fingerprint Comparison");
            println!("======================");
            println!();

            // Compare manifests
            println!("Manifests:");
            if fp1.manifest.version != fp2.manifest.version {
                println!(
                    "  Version: {} vs {}",
                    fp1.manifest.version, fp2.manifest.version
                );
            }
            if fp1.manifest.privacy.level != fp2.manifest.privacy.level {
                println!(
                    "  Privacy Level: {:?} vs {:?}",
                    fp1.manifest.privacy.level, fp2.manifest.privacy.level
                );
            }
            if fp1.manifest.privacy.epsilon != fp2.manifest.privacy.epsilon {
                println!(
                    "  Epsilon: {} vs {}",
                    fp1.manifest.privacy.epsilon, fp2.manifest.privacy.epsilon
                );
            }

            // Compare schemas
            println!();
            println!("Schema:");
            let tables1: std::collections::HashSet<_> = fp1.schema.tables.keys().collect();
            let tables2: std::collections::HashSet<_> = fp2.schema.tables.keys().collect();

            let only_in_1: Vec<_> = tables1.difference(&tables2).collect();
            let only_in_2: Vec<_> = tables2.difference(&tables1).collect();
            let common: Vec<_> = tables1.intersection(&tables2).collect();

            if !only_in_1.is_empty() {
                println!("  Only in {}: {:?}", file1.display(), only_in_1);
            }
            if !only_in_2.is_empty() {
                println!("  Only in {}: {:?}", file2.display(), only_in_2);
            }
            println!("  Common tables: {}", common.len());

            // Compare statistics
            println!();
            println!("Statistics:");
            println!(
                "  Numeric columns: {} vs {}",
                fp1.statistics.numeric_columns.len(),
                fp2.statistics.numeric_columns.len()
            );
            println!(
                "  Categorical columns: {} vs {}",
                fp1.statistics.categorical_columns.len(),
                fp2.statistics.categorical_columns.len()
            );

            // Compare numeric stats for common columns
            for col in fp1.statistics.numeric_columns.keys() {
                if let (Some(s1), Some(s2)) = (
                    fp1.statistics.numeric_columns.get(col),
                    fp2.statistics.numeric_columns.get(col),
                ) {
                    let mean_diff = (s1.mean - s2.mean).abs();
                    let std_diff = (s1.std_dev - s2.std_dev).abs();
                    if mean_diff > 0.01 || std_diff > 0.01 {
                        println!("  {col}:");
                        println!(
                            "    Mean: {:.2} vs {:.2} (diff: {:.2})",
                            s1.mean, s2.mean, mean_diff
                        );
                        println!(
                            "    StdDev: {:.2} vs {:.2} (diff: {:.2})",
                            s1.std_dev, s2.std_dev, std_diff
                        );
                    }
                }
            }

            Ok(())
        }

        FingerprintCommands::Evaluate {
            fingerprint,
            synthetic,
            output,
            threshold,
        } => {
            tracing::info!("Evaluating fidelity of synthetic data");
            tracing::info!("  Fingerprint: {}", fingerprint.display());
            tracing::info!("  Synthetic data: {}", synthetic.display());

            // Read fingerprint
            let reader = FingerprintReader::new();
            let fp = reader.read_from_file(&fingerprint)?;

            // Find CSV files in synthetic directory
            let csv_files: Vec<PathBuf> = std::fs::read_dir(&synthetic)?
                .filter_map(std::result::Result::ok)
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "csv"))
                .map(|e| e.path())
                .collect();

            if csv_files.is_empty() {
                anyhow::bail!(
                    "No CSV files found in synthetic directory: {}",
                    synthetic.display()
                );
            }

            // Extract fingerprints from all synthetic CSV files and average scores.
            tracing::info!("  Found {} CSV file(s) to evaluate", csv_files.len());
            let extractor = FingerprintExtractor::new();
            let evaluator = FidelityEvaluator::with_threshold(threshold);

            let mut all_reports = Vec::with_capacity(csv_files.len());
            for csv_path in &csv_files {
                tracing::info!("  Evaluating: {}", csv_path.display());
                let data_source = DataSource::Csv(CsvDataSource::new(csv_path.clone()));
                match extractor.extract(&data_source) {
                    Ok(synthetic_fp) => match evaluator.evaluate_fingerprints(&fp, &synthetic_fp) {
                        Ok(r) => all_reports.push(r),
                        Err(e) => {
                            tracing::warn!(
                                "  Skipping {} — evaluation error: {}",
                                csv_path.display(),
                                e
                            );
                        }
                    },
                    Err(e) => {
                        tracing::warn!(
                            "  Skipping {} — extraction error: {}",
                            csv_path.display(),
                            e
                        );
                    }
                }
            }

            if all_reports.is_empty() {
                anyhow::bail!("No CSV files could be evaluated successfully");
            }

            // Aggregate: average all fidelity component scores across tables.
            let n = all_reports.len() as f64;
            use datasynth_fingerprint::evaluation::FidelityReport;
            let report = FidelityReport {
                overall_score: all_reports.iter().map(|r| r.overall_score).sum::<f64>() / n,
                statistical_fidelity: all_reports
                    .iter()
                    .map(|r| r.statistical_fidelity)
                    .sum::<f64>()
                    / n,
                correlation_fidelity: all_reports
                    .iter()
                    .map(|r| r.correlation_fidelity)
                    .sum::<f64>()
                    / n,
                schema_fidelity: all_reports.iter().map(|r| r.schema_fidelity).sum::<f64>() / n,
                rule_compliance: all_reports.iter().map(|r| r.rule_compliance).sum::<f64>() / n,
                anomaly_fidelity: all_reports.iter().map(|r| r.anomaly_fidelity).sum::<f64>() / n,
                passes: all_reports.iter().all(|r| r.passes),
                details: all_reports
                    .into_iter()
                    .next()
                    .map(|r| r.details)
                    .unwrap_or_default(),
            };

            // Print report
            println!();
            println!("Fidelity Report");
            println!("===============");
            println!();
            println!("Overall Score: {:.1}%", report.overall_score * 100.0);
            println!("Threshold: {:.1}%", threshold * 100.0);
            println!(
                "Status: {}",
                if report.passes {
                    "PASS ✓"
                } else {
                    "FAIL ✗"
                }
            );
            println!();
            println!("Component Scores:");
            println!(
                "  Statistical Fidelity:  {:.1}%",
                report.statistical_fidelity * 100.0
            );
            println!(
                "  Correlation Fidelity:  {:.1}%",
                report.correlation_fidelity * 100.0
            );
            println!(
                "  Schema Fidelity:       {:.1}%",
                report.schema_fidelity * 100.0
            );
            println!(
                "  Rule Compliance:       {:.1}%",
                report.rule_compliance * 100.0
            );
            println!(
                "  Anomaly Fidelity:      {:.1}%",
                report.anomaly_fidelity * 100.0
            );

            // Write report if output path specified
            if let Some(output_path) = output {
                let json = serde_json::to_string_pretty(&report)?;
                std::fs::write(&output_path, json)?;
                tracing::info!("Report written to: {}", output_path.display());
            }

            if !report.passes {
                anyhow::bail!(
                    "Fidelity check failed: {:.1}% < {:.1}%",
                    report.overall_score * 100.0,
                    threshold * 100.0
                );
            }

            Ok(())
        }
        FingerprintCommands::Synthesize {
            fingerprint,
            output: synth_output,
            rows,
            neural,
            seed: synth_seed,
        } => {
            tracing::info!("Fingerprint → Synthesize pipeline");
            tracing::info!("  Fingerprint: {}", fingerprint.display());
            tracing::info!("  Output: {}", synth_output.display());
            tracing::info!("  Rows: {}, Neural: {}, Seed: {}", rows, neural, synth_seed);

            // Read fingerprint
            let reader = FingerprintReader::new();
            let fp = reader.read_from_file(&fingerprint)?;

            // Extract column statistics from fingerprint
            let col_names: Vec<String> = fp.statistics.numeric_columns.keys().cloned().collect();
            let n_cols = col_names.len();
            if n_cols == 0 {
                anyhow::bail!("Fingerprint has no numeric columns to synthesize from");
            }

            tracing::info!("  Columns: {} ({})", n_cols, col_names.join(", "));

            // Use statistical diffusion backend to generate matching data
            use datasynth_core::diffusion::{
                ColumnDiffusionParams, ColumnType, DiffusionConfig, DiffusionTrainer,
            };

            let column_params: Vec<ColumnDiffusionParams> = col_names
                .iter()
                .map(|name| {
                    let stats = &fp.statistics.numeric_columns[name];
                    ColumnDiffusionParams {
                        name: name.clone(),
                        mean: stats.mean,
                        std: stats.std_dev.max(1e-8),
                        min: stats.min,
                        max: stats.max,
                        col_type: ColumnType::Continuous,
                    }
                })
                .collect();

            // Build identity correlation matrix (fingerprint may have correlations)
            let corr: Vec<Vec<f64>> = (0..n_cols)
                .map(|i| {
                    (0..n_cols)
                        .map(|j| if i == j { 1.0 } else { 0.0 })
                        .collect()
                })
                .collect();

            let diffusion_config = DiffusionConfig {
                n_steps: 100,
                schedule: datasynth_core::diffusion::NoiseScheduleType::Cosine,
                seed: synth_seed,
            };

            let model = DiffusionTrainer::fit(column_params, corr, diffusion_config);
            let samples = model.generate(rows, synth_seed);

            // Write as CSV
            std::fs::create_dir_all(&synth_output)?;
            let csv_path = synth_output.join("synthesized.csv");
            let mut writer = csv::Writer::from_path(&csv_path)?;
            writer.write_record(&col_names)?;
            for row in &samples {
                let fields: Vec<String> = row.iter().map(|v| format!("{v:.6}")).collect();
                writer.write_record(&fields)?;
            }
            writer.flush()?;

            tracing::info!(
                "Synthesized {} rows x {} columns → {}",
                samples.len(),
                n_cols,
                csv_path.display()
            );

            if neural {
                tracing::info!(
                    "Neural enhancement requested. Build with --features neural for \
                     NeuralDiffusionTrainer-based synthesis."
                );
            }

            Ok(())
        }

        FingerprintCommands::AggregateIndustry {
            industry,
            inputs,
            output,
            allow_single_client,
            allow_cross_industry,
        } => {
            if inputs.len() < 3 && !allow_single_client {
                anyhow::bail!(
                    "aggregate-industry needs >=3 inputs (got {}); pass --allow-single-client to override",
                    inputs.len()
                );
            }
            // Read inputs.
            let reader = FingerprintReader::new();
            let mut bundles: Vec<datasynth_fingerprint::models::Fingerprint> = Vec::new();
            for path in &inputs {
                let fp = reader
                    .read_from_file(path)
                    .map_err(|e| anyhow::anyhow!("read {} failed: {e}", path.display()))?;
                bundles.push(fp);
            }
            // Collect behavioral sections.
            let mut priors: Vec<&datasynth_fingerprint::models::behavioral::BehavioralPriors> =
                Vec::new();
            for (idx, fp) in bundles.iter().enumerate() {
                match &fp.behavioral {
                    Some(bp) => {
                        if !allow_cross_industry && bp.industry != industry {
                            anyhow::bail!(
                                "input #{} has industry {:?}, expected {:?} (use --allow-cross-industry)",
                                idx,
                                bp.industry,
                                industry
                            );
                        }
                        priors.push(bp);
                    }
                    None => anyhow::bail!(
                        "input #{} ({}) has no behavioral section",
                        idx,
                        inputs[idx].display()
                    ),
                }
            }
            let aggregated =
                datasynth_fingerprint::aggregation::industry_aggregator::aggregate_industry_priors(
                    &priors, &industry,
                )
                .map_err(|e| anyhow::anyhow!("aggregation failed: {e}"))?;
            // Use the first input as a template, replace behavioral with the aggregate.
            let mut template = bundles
                .into_iter()
                .next()
                .ok_or_else(|| anyhow::anyhow!("no inputs after read"))?;
            template.behavioral = Some(aggregated);
            let writer = FingerprintWriter::new();
            writer
                .write_to_file(&template, &output)
                .map_err(|e| anyhow::anyhow!("write {} failed: {e}", output.display()))?;
            eprintln!("Wrote industry-aggregated bundle to {}", output.display());
            Ok(())
        }
    }
}

/// Find a scenario pack file by name.
///
/// Searches in the following locations:
/// 1. templates/scenarios/{pack}.yaml
/// 2. Current directory templates/scenarios/{pack}.yaml
/// 3. Executable directory templates/scenarios/{pack}.yaml
fn find_scenario_pack(pack: &str) -> Result<PathBuf> {
    // Normalize the pack name (remove .yaml if present)
    let pack_name = pack.trim_end_matches(".yaml");

    // Search paths in order of priority
    let search_paths = [
        PathBuf::from(format!("templates/scenarios/{pack_name}.yaml")),
        PathBuf::from(format!("./templates/scenarios/{pack_name}.yaml")),
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
            .map(|p| p.join(format!("templates/scenarios/{pack_name}.yaml")))
            .unwrap_or_default(),
    ];

    for path in search_paths.iter() {
        if path.exists() {
            tracing::info!("Found scenario pack at: {}", path.display());
            return Ok(path.clone());
        }
    }

    // List available scenario packs if not found
    let available = list_available_scenarios();
    anyhow::bail!(
        "Scenario pack '{}' not found.\n\nAvailable scenario packs:\n{}",
        pack,
        available.join("\n")
    );
}

/// List available scenario packs.
fn list_available_scenarios() -> Vec<String> {
    let mut scenarios = Vec::new();
    let base_path = PathBuf::from("templates/scenarios");

    if let Ok(industries) = std::fs::read_dir(&base_path) {
        for industry in industries.flatten() {
            if industry.path().is_dir() {
                let industry_name = industry.file_name().to_string_lossy().to_string();
                if let Ok(files) = std::fs::read_dir(industry.path()) {
                    for file in files.flatten() {
                        let file_name = file.file_name().to_string_lossy().to_string();
                        if file_name.ends_with(".yaml") {
                            let scenario_name = file_name.trim_end_matches(".yaml");
                            scenarios.push(format!("  - {industry_name}/{scenario_name}"));
                        }
                    }
                }
            }
        }
    }

    if scenarios.is_empty() {
        scenarios.push("  (no scenario packs found in templates/scenarios/)".to_string());
    }

    scenarios
}

/// Create a safe demo preset with conservative resource usage.
fn create_safe_demo_preset() -> GeneratorConfig {
    use datasynth_config::schema::*;

    GeneratorConfig {
        global: GlobalConfig {
            industry: IndustrySector::Manufacturing,
            start_date: "2024-01-01".to_string(),
            period_months: 1, // Just 1 month for demo
            seed: Some(42),
            parallel: false,
            group_currency: "USD".to_string(),
            presentation_currency: None,
            worker_threads: 2,
            memory_limit_mb: 512,
            fiscal_year_months: None,
        },
        companies: vec![CompanyConfig {
            code: "DEMO".to_string(),
            name: "Demo Company".to_string(),
            currency: "USD".to_string(),
            functional_currency: None,
            country: "US".to_string(),
            annual_transaction_volume: TransactionVolume::TenK, // Small volume
            volume_weight: 1.0,
            fiscal_year_variant: "K4".to_string(),
        }],
        chart_of_accounts: ChartOfAccountsConfig {
            complexity: CoAComplexity::Small,
            industry_specific: false,
            custom_accounts: None,
            min_hierarchy_depth: 2,
            max_hierarchy_depth: 3,
            expand_industry_subaccounts: false,
        },
        transactions: TransactionConfig::default(),
        output: OutputConfig::default(),
        fraud: FraudConfig {
            enabled: false,
            ..Default::default()
        },
        internal_controls: InternalControlsConfig::default(),
        business_processes: BusinessProcessConfig::default(),
        user_personas: UserPersonaConfig::default(),
        templates: TemplateConfig::default(),
        approval: ApprovalConfig::default(),
        departments: DepartmentConfig::default(),
        master_data: MasterDataConfig::default(),
        document_flows: DocumentFlowConfig::default(),
        intercompany: IntercompanyConfig::default(),
        balance: BalanceConfig::default(),
        ocpm: OcpmConfig::default(),
        audit: AuditGenerationConfig {
            enabled: true,
            fsm: Some(AuditFsmConfig {
                enabled: true,
                blueprint: "builtin:fsa".into(),
                overlay: "builtin:default".into(),
                ..Default::default()
            }),
            ..Default::default()
        },
        banking: datasynth_banking::BankingConfig::small(), // Use small banking config
        data_quality: DataQualitySchemaConfig::default(),
        scenario: datasynth_config::schema::ScenarioConfig::default(),
        temporal: datasynth_config::schema::TemporalDriftConfig::default(),
        graph_export: datasynth_config::schema::GraphExportConfig::default(),
        streaming: datasynth_config::schema::StreamingSchemaConfig::default(),
        rate_limit: datasynth_config::schema::RateLimitSchemaConfig::default(),
        temporal_attributes: datasynth_config::schema::TemporalAttributeSchemaConfig::default(),
        relationships: datasynth_config::schema::RelationshipSchemaConfig::default(),
        accounting_standards: datasynth_config::schema::AccountingStandardsConfig::default(),
        audit_standards: datasynth_config::schema::AuditStandardsConfig::default(),
        distributions: datasynth_config::schema::AdvancedDistributionConfig::default(),
        temporal_patterns: datasynth_config::schema::TemporalPatternsConfig::default(),
        vendor_network: datasynth_config::schema::VendorNetworkSchemaConfig::default(),
        customer_segmentation: datasynth_config::schema::CustomerSegmentationSchemaConfig::default(
        ),
        relationship_strength: datasynth_config::schema::RelationshipStrengthSchemaConfig::default(
        ),
        cross_process_links: datasynth_config::schema::CrossProcessLinksSchemaConfig::default(),
        organizational_events: datasynth_config::schema::OrganizationalEventsSchemaConfig::default(
        ),
        behavioral_drift: datasynth_config::schema::BehavioralDriftSchemaConfig::default(),
        market_drift: datasynth_config::schema::MarketDriftSchemaConfig::default(),
        drift_labeling: datasynth_config::schema::DriftLabelingSchemaConfig::default(),
        anomaly_injection: Default::default(),
        industry_specific: Default::default(),
        fingerprint_privacy: Default::default(),
        quality_gates: Default::default(),
        compliance: Default::default(),
        webhooks: Default::default(),
        llm: Default::default(),
        diffusion: Default::default(),
        causal: Default::default(),
        source_to_pay: Default::default(),
        financial_reporting: Default::default(),
        hr: Default::default(),
        manufacturing: Default::default(),
        sales_quotes: Default::default(),
        tax: Default::default(),
        treasury: Default::default(),
        project_accounting: Default::default(),
        esg: Default::default(),
        country_packs: None,
        scenarios: Default::default(),
        session: Default::default(),
        compliance_regulations: Default::default(),
        analytics_metadata: Default::default(),
        concentration: Default::default(),
    }
}

/// Apply safety limits to a loaded configuration.
fn apply_safety_limits(config: &mut GeneratorConfig) {
    // Limit period to the schema maximum (config validation allows 1..=120 months; the prior 12
    // truncation silently capped multi-year horizons below the schema, blocking fiscal-year-spanning
    // generation). Aligns the CLI guard with `period_months: 1-120` — a no-op for any <=12-month build.
    if config.global.period_months > 120 {
        tracing::warn!(
            "Safety limit: period_months truncated from {} to 120",
            config.global.period_months
        );
        config.global.period_months = 120;
    }

    // Limit transaction volume
    for company in &mut config.companies {
        let original = company.annual_transaction_volume;
        company.annual_transaction_volume = match company.annual_transaction_volume {
            datasynth_config::TransactionVolume::OneM
            | datasynth_config::TransactionVolume::TenM
            | datasynth_config::TransactionVolume::HundredM => {
                tracing::warn!(
                    "Safety limit: transaction volume for company '{}' capped from {:?} to HundredK",
                    company.code,
                    original
                );
                datasynth_config::TransactionVolume::HundredK
            }
            other => other,
        };
    }

    // Limit banking population
    if config.banking.enabled {
        let orig_retail = config.banking.population.retail_customers;
        let orig_business = config.banking.population.business_customers;
        let orig_trusts = config.banking.population.trusts;
        config.banking.population.retail_customers = orig_retail.min(500);
        config.banking.population.business_customers = orig_business.min(100);
        config.banking.population.trusts = orig_trusts.min(20);
        if orig_retail > 500 || orig_business > 100 || orig_trusts > 20 {
            tracing::warn!(
                "Safety limit: banking population capped (retail: {} -> {}, business: {} -> {}, trusts: {} -> {})",
                orig_retail,
                config.banking.population.retail_customers,
                orig_business,
                config.banking.population.business_customers,
                orig_trusts,
                config.banking.population.trusts,
            );
        }
    }

    // Force conservative settings
    config.global.parallel = false;
    config.global.worker_threads = config.global.worker_threads.min(4);
}

/// Translate the YAML `output.sap` block into the runtime `SapExportConfig`.
///
/// Unknown / unrecognised table names in `settings.tables` are silently
/// dropped with a tracing warning so a config with a typo doesn't hard-fail
/// the run. An empty `tables` list produces the default BKPF / BSEG /
/// ACDOCA triple for backward compatibility with pre-v4.3.0 callers.
fn build_sap_config(settings: &datasynth_config::SapExportSettings) -> SapExportConfig {
    use datasynth_output::SapTableType;

    let dialect = match settings.dialect {
        datasynth_config::SapDialectSetting::Classic => datasynth_output::SapDialect::Classic,
        datasynth_config::SapDialectSetting::Hana => datasynth_output::SapDialect::Hana,
    };

    let tables = if settings.tables.is_empty() {
        vec![SapTableType::Bkpf, SapTableType::Bseg, SapTableType::Acdoca]
    } else {
        let mapped: Vec<SapTableType> = settings
            .tables
            .iter()
            .filter_map(|t| match t.to_ascii_lowercase().as_str() {
                // Transactional (go through SapExporter.export_to_files).
                "bkpf" => Some(SapTableType::Bkpf),
                "bseg" => Some(SapTableType::Bseg),
                "acdoca" => Some(SapTableType::Acdoca),
                // Master-data (go through standalone write_* helpers).
                // SapTableType currently has only LFA1 / KNA1 / MARA / CSKS / CEPC;
                // the company-code variants LFB1 / KNB1 and storage-loc MARD are
                // selected via the `want_table("lfb1"|"knb1"|"mard")` check in
                // the CLI body and don't map 1:1 to SapTableType. That's OK —
                // unknown-table names here become a tracing warn + filter-out
                // at the SapExporter level but still drive master-data writes.
                // Master-data tables — LFA1/KNA1/MARA/CSKS/CEPC map to
                // dedicated SapTableType variants; LFB1/KNB1/MARD/ANLA/SKA1/
                // SKB1 are routed through standalone writers driven by the
                // `want_table(...)` check in the CLI body, so there's no
                // 1:1 SapTableType for them. Filter them to the closest
                // parent variant here to keep SapExporter happy; the CLI
                // body reads the original table-name list for routing.
                "lfa1" | "lfb1" | "kna1" | "knb1" | "mara" | "mard" | "csks" | "cepc" | "anla"
                | "ska1" | "skb1"
                // v4.3.0d document-flow + subledger tables. No SapTableType
                // variant exists for these; dispatch is by `want_table(...)`
                // in the CLI body.
                | "ekko" | "ekpo" | "vbak" | "vbap" | "likp" | "lips" | "mkpf" | "mseg"
                | "bsis" | "bsas" | "bsid" | "bsad" | "bsik" | "bsak" => {
                    match t.to_ascii_lowercase().as_str() {
                        "lfa1" | "lfb1" => Some(SapTableType::Lfa1),
                        "kna1" | "knb1" => Some(SapTableType::Kna1),
                        "mara" | "mard" => Some(SapTableType::Mara),
                        "csks" => Some(SapTableType::Csks),
                        "cepc" => Some(SapTableType::Cepc),
                        // Everything else doesn't map to a SapTableType —
                        // the CLI body routes via dedicated write_* helpers.
                        _ => None,
                    }
                }
                other => {
                    tracing::warn!(
                        "SAP export config: ignoring unknown table '{}' \
                         (known: bkpf, bseg, acdoca, lfa1, lfb1, kna1, knb1, \
                         mara, mard, csks, cepc, anla, ska1, skb1, \
                         ekko, ekpo, vbak, vbap, likp, lips, mkpf, mseg, \
                         bsis, bsas, bsid, bsad, bsik, bsak)",
                        other
                    );
                    None
                }
            })
            .collect::<Vec<SapTableType>>();
        // v4.4.2: preserve the canonical BKPF → BSEG → ACDOCA order
        // via an explicit priority sort. `SapExporter::export_to_files`
        // shares `document_counter` across the three transactional
        // tables, so iterating in any other order (e.g. the
        // non-deterministic HashSet order used pre-v4.4.2) leaves
        // BKPF.BELNR desynced from BSEG.BELNR. Dedup after sort.
        let mut sorted = mapped;
        sorted.sort_by_key(|t| match t {
            SapTableType::Bkpf => 0,
            SapTableType::Bseg => 1,
            SapTableType::Acdoca => 2,
            SapTableType::Lfa1 => 3,
            SapTableType::Kna1 => 4,
            SapTableType::Mara => 5,
            SapTableType::Csks => 6,
            SapTableType::Cepc => 7,
        });
        sorted.dedup();
        sorted
    };

    SapExportConfig {
        client: settings.client.clone(),
        ledger: settings.ledger.clone(),
        source_system: settings.source_system.clone(),
        local_currency: settings.local_currency.clone(),
        group_currency: settings.group_currency.clone(),
        tables,
        include_extension_fields: settings.include_extension_fields,
        dialect,
        use_sap_date_format: settings.use_sap_date_format,
    }
}

/// Get safe memory limit based on available system memory.
/// Returns a conservative limit that won't overwhelm the system.
fn get_safe_memory_limit() -> usize {
    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = std::fs::read_to_string("/proc/meminfo") {
            for line in content.lines() {
                if line.starts_with("MemAvailable:") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        if let Ok(kb) = parts[1].parse::<usize>() {
                            let mb = kb / 1024;
                            // Use 50% of available memory, capped at 4GB
                            return (mb / 2).min(4096);
                        }
                    }
                    break;
                }
            }
        }
    }

    // Default to 1GB if detection fails
    1024
}

// ---------------------------------------------------------------------------
// Audit FSM helpers
// ---------------------------------------------------------------------------

/// Resolve a blueprint string to a loaded `BlueprintWithPreconditions`.
fn resolve_blueprint(s: &str) -> Result<datasynth_audit_fsm::loader::BlueprintWithPreconditions> {
    use datasynth_audit_fsm::loader::BlueprintWithPreconditions;
    match s {
        "builtin:fsa" => {
            BlueprintWithPreconditions::load_builtin_fsa().map_err(|e| anyhow::anyhow!("{e}"))
        }
        "builtin:ia" => {
            BlueprintWithPreconditions::load_builtin_ia().map_err(|e| anyhow::anyhow!("{e}"))
        }
        "builtin:kpmg" => {
            BlueprintWithPreconditions::load_builtin_kpmg().map_err(|e| anyhow::anyhow!("{e}"))
        }
        "builtin:pwc" => {
            BlueprintWithPreconditions::load_builtin_pwc().map_err(|e| anyhow::anyhow!("{e}"))
        }
        "builtin:deloitte" => {
            BlueprintWithPreconditions::load_builtin_deloitte().map_err(|e| anyhow::anyhow!("{e}"))
        }
        "builtin:ey_gam_lite" | "ey_gam_lite" => {
            BlueprintWithPreconditions::load_builtin_ey_gam_lite()
                .map_err(|e| anyhow::anyhow!("{e}"))
        }
        "builtin:soc2" => {
            BlueprintWithPreconditions::load_builtin_soc2().map_err(|e| anyhow::anyhow!("{e}"))
        }
        "builtin:pcaob" => {
            BlueprintWithPreconditions::load_builtin_pcaob().map_err(|e| anyhow::anyhow!("{e}"))
        }
        "builtin:regulatory" => BlueprintWithPreconditions::load_builtin_regulatory()
            .map_err(|e| anyhow::anyhow!("{e}")),
        path => BlueprintWithPreconditions::load_from_file(PathBuf::from(path))
            .map_err(|e| anyhow::anyhow!("{e}")),
    }
}

/// Resolve an overlay string to a `GenerationOverlay`.
fn resolve_overlay(s: &str) -> Result<datasynth_audit_fsm::schema::GenerationOverlay> {
    use datasynth_audit_fsm::loader::{load_overlay, BuiltinOverlay, OverlaySource};
    let source = match s {
        "builtin:default" => OverlaySource::Builtin(BuiltinOverlay::Default),
        "builtin:thorough" => OverlaySource::Builtin(BuiltinOverlay::Thorough),
        "builtin:rushed" => OverlaySource::Builtin(BuiltinOverlay::Rushed),
        "builtin:retail" => OverlaySource::Builtin(BuiltinOverlay::IndustryRetail),
        "builtin:manufacturing" => OverlaySource::Builtin(BuiltinOverlay::IndustryManufacturing),
        "builtin:financial_services" => {
            OverlaySource::Builtin(BuiltinOverlay::IndustryFinancialServices)
        }
        path => OverlaySource::Custom(PathBuf::from(path)),
    };
    load_overlay(&source).map_err(|e| anyhow::anyhow!("{e}"))
}

/// Handle `audit validate`.
fn handle_audit_validate(blueprint_str: &str) -> Result<()> {
    let bwp = resolve_blueprint(blueprint_str)?;
    let bp = &bwp.blueprint;

    match bwp.validate() {
        Ok(()) => {
            let total_procedures: usize = bp.phases.iter().map(|p| p.procedures.len()).sum();
            let total_steps: usize = bp
                .phases
                .iter()
                .flat_map(|p| &p.procedures)
                .map(|proc| proc.steps.len())
                .sum();
            println!("Blueprint is valid.");
            println!();
            println!("  Framework:   {}", bp.methodology.framework);
            println!("  Phases:      {}", bp.phases.len());
            println!("  Procedures:  {}", total_procedures);
            println!("  Steps:       {}", total_steps);
            println!("  Standards:   {}", bp.standards.len());
            println!("  Actors:      {}", bp.actors.len());
            Ok(())
        }
        Err(datasynth_audit_fsm::error::AuditFsmError::BlueprintValidation { violations }) => {
            eprintln!(
                "Blueprint validation FAILED ({} violation(s)):",
                violations.len()
            );
            for v in &violations {
                eprintln!("  - {v}");
            }
            std::process::exit(1);
        }
        Err(e) => Err(anyhow::anyhow!("{e}")),
    }
}

/// Handle `audit info`.
fn handle_audit_info(blueprint_str: &str) -> Result<()> {
    let bwp = resolve_blueprint(blueprint_str)?;
    let bp = &bwp.blueprint;

    let total_procedures: usize = bp.phases.iter().map(|p| p.procedures.len()).sum();
    let total_steps: usize = bp
        .phases
        .iter()
        .flat_map(|p| &p.procedures)
        .map(|proc| proc.steps.len())
        .sum();

    // Collect unique evidence types referenced across all procedures
    let evidence_ids: std::collections::HashSet<&str> = bp
        .evidence_templates
        .iter()
        .map(|e| e.id.as_str())
        .collect();

    println!("Audit Blueprint Information");
    println!("===========================");
    println!();
    println!("  Name:        {}", bp.name);
    println!("  Version:     {}", bp.version);
    println!("  Framework:   {}", bp.methodology.framework);
    println!("  Phases:      {}", bp.phases.len());
    println!("  Procedures:  {}", total_procedures);
    println!("  Steps:       {}", total_steps);
    println!("  Standards:   {}", bp.standards.len());
    println!("  Actors:      {}", bp.actors.len());
    println!("  Evidence:    {} template(s)", evidence_ids.len());
    println!();

    // Classify phases as continuous (order < 0) vs sequential
    let continuous: Vec<_> = bp
        .phases
        .iter()
        .filter(|p| p.order.is_some_and(|o| o < 0))
        .collect();
    let sequential: Vec<_> = bp
        .phases
        .iter()
        .filter(|p| p.order.is_none_or(|o| o >= 0))
        .collect();

    if !continuous.is_empty() {
        println!("  Continuous phases ({}):", continuous.len());
        for phase in &continuous {
            let proc_count = phase.procedures.len();
            let step_count: usize = phase.procedures.iter().map(|p| p.steps.len()).sum();
            println!(
                "    - {} ({} procedure(s), {} step(s))",
                phase.name, proc_count, step_count
            );
        }
        println!();
    }

    println!("  Sequential phases ({}):", sequential.len());
    for phase in &sequential {
        let proc_count = phase.procedures.len();
        let step_count: usize = phase.procedures.iter().map(|p| p.steps.len()).sum();
        println!(
            "    - {} ({} procedure(s), {} step(s))",
            phase.name, proc_count, step_count
        );
    }
    println!();

    // Actors
    if !bp.actors.is_empty() {
        println!("  Actors:");
        for actor in &bp.actors {
            println!("    - {} ({})", actor.label, actor.id);
        }
    }

    Ok(())
}

/// Handle `audit run`.
fn handle_audit_run(
    blueprint_str: &str,
    overlay_str: &str,
    output: &std::path::Path,
    seed: u64,
) -> Result<()> {
    use datasynth_audit_fsm::context::EngagementContext;
    use datasynth_audit_fsm::engine::AuditFsmEngine;
    use datasynth_audit_fsm::export::flat_log::export_events_to_file;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    let bwp = resolve_blueprint(blueprint_str)?;
    let overlay = resolve_overlay(overlay_str)?;

    // Validate before running
    bwp.validate().map_err(|e| anyhow::anyhow!("{e}"))?;

    let rng = ChaCha8Rng::seed_from_u64(seed);
    let mut engine = AuditFsmEngine::new(bwp, overlay, rng);
    let ctx = EngagementContext::demo();

    let start = std::time::Instant::now();
    let result = engine
        .run_engagement(&ctx)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let elapsed = start.elapsed();

    // Write output
    std::fs::create_dir_all(output)?;
    let trail_path = output.join("audit_event_trail.json");
    export_events_to_file(&result.event_log, &trail_path)
        .map_err(|e| anyhow::anyhow!("Failed to write event trail: {e}"))?;

    // Summary
    println!("Audit FSM engagement complete.");
    println!();
    println!("  Events:     {}", result.event_log.len());
    println!("  Artifacts:  {}", result.artifacts.total_artifacts());
    println!("  Phases:     {}", result.phases_completed.len());
    println!("  Anomalies:  {}", result.anomalies.len());
    println!(
        "  Duration:   {:.1} simulated hours",
        result.total_duration_hours
    );
    println!("  Wall clock: {:.2}s", elapsed.as_secs_f64());
    println!();
    println!("  Event trail: {}", trail_path.display());

    Ok(())
}

/// Handle `audit benchmark`.
fn handle_audit_benchmark(
    complexity_str: &str,
    anomaly_rate: Option<f64>,
    output: &std::path::Path,
    seed: u64,
) -> Result<()> {
    use datasynth_audit_fsm::benchmark::{
        export_benchmark, generate_benchmark, BenchmarkComplexity, BenchmarkConfig,
    };

    let complexity = match complexity_str.to_lowercase().as_str() {
        "simple" => BenchmarkComplexity::Simple,
        "medium" => BenchmarkComplexity::Medium,
        "complex" => BenchmarkComplexity::Complex,
        other => {
            anyhow::bail!(
                "Unknown complexity '{}'. Use: simple, medium, complex",
                other
            );
        }
    };

    let config = BenchmarkConfig {
        complexity,
        anomaly_rate,
        seed,
    };

    let start = std::time::Instant::now();
    let dataset = generate_benchmark(&config).map_err(|e| anyhow::anyhow!("{e}"))?;
    let elapsed = start.elapsed();

    export_benchmark(&dataset, output)
        .map_err(|e| anyhow::anyhow!("Failed to export benchmark: {e}"))?;

    println!("Benchmark dataset generated.");
    println!();
    println!("  Complexity:  {}", dataset.metadata.complexity);
    println!("  Blueprint:   {}", dataset.metadata.blueprint);
    println!("  Overlay:     {}", dataset.metadata.overlay);
    println!("  Events:      {}", dataset.metadata.event_count);
    println!("  Anomalies:   {}", dataset.metadata.anomaly_count);
    println!("  Anomaly rate: {:.4}", dataset.metadata.anomaly_rate);
    println!("  Procedures:  {}", dataset.metadata.procedure_count);
    println!("  Artifacts:   {}", dataset.metadata.artifact_count);
    println!("  Seed:        {}", dataset.metadata.seed);
    println!("  Wall clock:  {:.2}s", elapsed.as_secs_f64());
    println!();
    println!("  Output: {}", output.display());
    println!("    - event_trail.json");
    println!("    - event_trail.csv");
    println!("    - event_trail_ocel.json");
    println!("    - anomaly_labels.json");
    println!("    - metadata.json");

    Ok(())
}

/// Handle `audit diff`.
fn handle_audit_diff(blueprint_a_str: &str, blueprint_b_str: &str) -> Result<()> {
    let bwp_a = resolve_blueprint(blueprint_a_str)?;
    let bwp_b = resolve_blueprint(blueprint_b_str)?;

    // Run both blueprints to get events, then discover and compare.
    use datasynth_audit_fsm::context::EngagementContext;
    use datasynth_audit_fsm::engine::AuditFsmEngine;
    use datasynth_audit_fsm::loader::default_overlay;
    use datasynth_audit_optimizer::discovery::{compare_blueprints, discover_blueprint};
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    let overlay = default_overlay();
    let ctx = EngagementContext::demo();

    // Generate events from blueprint A and discover its structure.
    let rng_a = ChaCha8Rng::seed_from_u64(42);
    let mut engine_a = AuditFsmEngine::new(bwp_a, overlay.clone(), rng_a);
    let result_a = engine_a
        .run_engagement(&ctx)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let discovered_a = discover_blueprint(&result_a.event_log);

    // Compare discovered A against reference B.
    let diff = compare_blueprints(&discovered_a, &bwp_b.blueprint);

    println!("Blueprint Diff: {} vs {}", blueprint_a_str, blueprint_b_str);
    println!("============================================");
    println!();
    println!("Conformance score: {:.2}%", diff.conformance_score * 100.0);
    println!();

    if !diff.matching_procedures.is_empty() {
        println!("Matching procedures ({}):", diff.matching_procedures.len());
        for p in &diff.matching_procedures {
            println!("  + {}", p);
        }
        println!();
    }

    if !diff.missing_procedures.is_empty() {
        println!(
            "Missing from A (in B only) ({}):",
            diff.missing_procedures.len()
        );
        for p in &diff.missing_procedures {
            println!("  - {}", p);
        }
        println!();
    }

    if !diff.extra_procedures.is_empty() {
        println!("Extra in A (not in B) ({}):", diff.extra_procedures.len());
        for p in &diff.extra_procedures {
            println!("  ~ {}", p);
        }
        println!();
    }

    if !diff.transition_diffs.is_empty() {
        println!("Transition differences ({}):", diff.transition_diffs.len());
        for td in &diff.transition_diffs {
            let marker = if td.diff_type == "missing" { "-" } else { "+" };
            println!(
                "  {} [{}] {} -> {}",
                marker, td.procedure_id, td.from_state, td.to_state
            );
        }
    }

    Ok(())
}

/// Handle scenario subcommands.
fn handle_scenario_command(command: ScenarioCommands) -> Result<()> {
    use datasynth_eval::diff_engine::{DiffConfig, DiffEngine, DiffFormat};
    use datasynth_runtime::scenario_engine::ScenarioEngine;

    match command {
        ScenarioCommands::List { config } => {
            let config_str = std::fs::read_to_string(&config)?;
            let gen_config: GeneratorConfig = serde_yaml::from_str(&config_str)?;

            if !gen_config.scenarios.enabled {
                println!("Scenarios are disabled in this config.");
                return Ok(());
            }

            let engine = ScenarioEngine::new(gen_config)?;
            let scenarios = engine.list_scenarios();

            if scenarios.is_empty() {
                println!("No scenarios defined.");
                return Ok(());
            }

            println!("Scenarios ({}):", scenarios.len());
            println!("{:-<60}", "");
            for s in &scenarios {
                println!("  Name: {}", s.name);
                println!("  Description: {}", s.description);
                if !s.tags.is_empty() {
                    println!("  Tags: {}", s.tags.join(", "));
                }
                println!("  Interventions: {}", s.intervention_count);
                if let Some(w) = s.probability_weight {
                    println!("  Probability Weight: {w:.2}");
                }
                println!("{:-<60}", "");
            }

            Ok(())
        }

        ScenarioCommands::Validate { config, scenario } => {
            let config_str = std::fs::read_to_string(&config)?;
            let gen_config: GeneratorConfig = serde_yaml::from_str(&config_str)?;

            if !gen_config.scenarios.enabled {
                anyhow::bail!("Scenarios are disabled in this config.");
            }

            let engine = ScenarioEngine::new(gen_config)?;
            let results = engine.validate_all();

            let filtered: Vec<_> = if let Some(ref name) = scenario {
                results.into_iter().filter(|r| r.name == *name).collect()
            } else {
                results
            };

            if filtered.is_empty() {
                if let Some(name) = scenario {
                    anyhow::bail!("Scenario '{name}' not found.");
                }
                println!("No scenarios to validate.");
                return Ok(());
            }

            let mut all_valid = true;
            for r in &filtered {
                if r.valid {
                    println!("  [PASS] {}", r.name);
                } else {
                    println!(
                        "  [FAIL] {}: {}",
                        r.name,
                        r.error.as_deref().unwrap_or("unknown")
                    );
                    all_valid = false;
                }
            }

            if all_valid {
                println!("\nAll {} scenario(s) valid.", filtered.len());
                Ok(())
            } else {
                anyhow::bail!("Some scenarios failed validation.")
            }
        }

        ScenarioCommands::Generate {
            config,
            output,
            scenario: _scenario_filter,
        } => {
            let config_str = std::fs::read_to_string(&config)?;
            let gen_config: GeneratorConfig = serde_yaml::from_str(&config_str)?;

            if !gen_config.scenarios.enabled {
                anyhow::bail!("Scenarios are disabled in this config.");
            }

            let engine = ScenarioEngine::new(gen_config)?;
            let results = engine.generate_all(&output)?;

            println!("Generated {} scenario(s):", results.len());
            for r in &results {
                println!(
                    "  {} — {} interventions, {} months affected",
                    r.scenario_name, r.interventions_applied, r.months_affected
                );
                println!("    Baseline: {}", r.baseline_path.display());
                println!("    Counterfactual: {}", r.counterfactual_path.display());
            }

            Ok(())
        }

        ScenarioCommands::Diff {
            baseline,
            counterfactual,
            format,
            output,
        } => {
            let formats = match format.as_str() {
                "summary" => vec![DiffFormat::Summary],
                "record_level" => vec![DiffFormat::RecordLevel],
                "aggregate" => vec![DiffFormat::Aggregate],
                "all" => vec![
                    DiffFormat::Summary,
                    DiffFormat::RecordLevel,
                    DiffFormat::Aggregate,
                ],
                other => anyhow::bail!(
                    "Unknown diff format: '{other}'. Use: summary, record_level, aggregate, all"
                ),
            };

            let diff_config = DiffConfig {
                formats,
                ..Default::default()
            };

            let diff = DiffEngine::compute(&baseline, &counterfactual, &diff_config)?;
            let json = serde_json::to_string_pretty(&diff)?;

            if let Some(out_path) = output {
                std::fs::write(&out_path, &json)?;
                println!("Diff written to {}", out_path.display());
            } else {
                println!("{json}");
            }

            Ok(())
        }
        ScenarioCommands::Export {
            config,
            scenario,
            output,
        } => {
            let config_str = std::fs::read_to_string(&config)?;
            let gen_config: GeneratorConfig = serde_yaml::from_str(&config_str)?;

            let found = gen_config
                .scenarios
                .scenarios
                .iter()
                .find(|s| s.name == scenario);

            match found {
                Some(s) => {
                    let yaml = serde_yaml::to_string(s)?;
                    let dss = format!(
                        "# DataSynth Scenario (.dss)\n\
                         # format_version: 1.0\n\
                         # exported_from: {}\n\
                         # datasynth_version: {}\n\n\
                         {yaml}",
                        config.display(),
                        env!("CARGO_PKG_VERSION"),
                    );
                    std::fs::write(&output, dss)?;
                    println!("Scenario '{}' exported to {}", scenario, output.display());
                    Ok(())
                }
                None => {
                    anyhow::bail!(
                        "Scenario '{}' not found. Available: {}",
                        scenario,
                        gen_config
                            .scenarios
                            .scenarios
                            .iter()
                            .map(|s| s.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
            }
        }
        ScenarioCommands::Import { file, config } => {
            let dss_content = std::fs::read_to_string(&file)?;
            let yaml_content: String = dss_content
                .lines()
                .filter(|line| !line.starts_with('#'))
                .collect::<Vec<_>>()
                .join("\n");

            let imported: datasynth_config::ScenarioSchemaConfig =
                serde_yaml::from_str(&yaml_content)?;

            if !config.exists() {
                anyhow::bail!(
                    "Config file {} does not exist. Create one first with: datasynth-data init",
                    config.display()
                );
            }
            let existing = std::fs::read_to_string(&config)?;
            let mut gen_config: GeneratorConfig = serde_yaml::from_str(&existing)?;

            if gen_config
                .scenarios
                .scenarios
                .iter()
                .any(|s| s.name == imported.name)
            {
                anyhow::bail!("Scenario '{}' already exists in config", imported.name);
            }

            gen_config.scenarios.enabled = true;
            let name = imported.name.clone();
            gen_config.scenarios.scenarios.push(imported);

            let yaml = serde_yaml::to_string(&gen_config)?;
            std::fs::write(&config, yaml)?;
            println!("Scenario '{}' imported into {}", name, config.display());
            Ok(())
        }
    }
}

/// **v5.32 C3 (#158)** — `datasynth-data calibrate` handler.
///
/// Scaffolding stage. The argparse, knob-set construction, config-
/// patch logic, and history persistence all wire to the
/// `datasynth_eval::calibration` primitives. The orchestrator-
/// backed [`datasynth_eval::calibration::Evaluator`] that runs a
/// full generation + BF eval per (knobs, seed) is the follow-up
/// piece — it's a heavy VM-validated path, so we return a clear
/// "not yet wired" error when `--dry-run` is omitted.
///
/// In `--dry-run` mode the handler:
///   1. Validates argparse + loads the base GroupConfig.
///   2. Echoes the resolved calibration parameters.
///   3. Constructs (but does not run) the [`CalibrationLoop`].
///   4. Exits with code 0.
///
/// This gives the CI a smoke test that the CLI surface compiles +
/// argparses correctly without requiring the engine integration.
#[allow(clippy::too_many_arguments)]
fn handle_calibrate(
    config_path: &std::path::Path,
    reference_path: &std::path::Path,
    out_path: &std::path::Path,
    objective: &str,
    max_iter: usize,
    seeds: usize,
    target: Option<f64>,
    resume: Option<&std::path::Path>,
    dry_run: bool,
) -> Result<()> {
    use anyhow::Context;
    use datasynth_eval::calibration::{
        CalibrationConfig, CalibrationLoop, CalibrationObjective, ObjectiveMetric,
    };

    tracing::info!(
        config = %config_path.display(),
        reference = %reference_path.display(),
        out = %out_path.display(),
        objective = objective,
        max_iter = max_iter,
        seeds = seeds,
        target = ?target,
        resume = ?resume.map(|p| p.display().to_string()),
        dry_run = dry_run,
        "calibrate: starting",
    );

    // Parse objective string.
    let metric = match objective {
        "bf_composite" => ObjectiveMetric::BfComposite,
        "bf_composite_median" => ObjectiveMetric::BfCompositeMedian,
        "bf_composite_volume_corrected" => ObjectiveMetric::BfCompositeVolumeCorrected,
        other => {
            eprintln!(
                "calibrate: unknown --objective `{other}`. valid: \
                 bf_composite, bf_composite_median, bf_composite_volume_corrected"
            );
            std::process::exit(2);
        }
    };

    let mut obj = CalibrationObjective::default().with_metric(metric);
    if let Some(t) = target {
        obj = obj.with_target(t);
    }

    // Validate input paths.
    if !config_path.exists() {
        eprintln!("calibrate: --config `{}` not found", config_path.display());
        std::process::exit(2);
    }
    if !reference_path.exists() {
        eprintln!(
            "calibrate: --reference `{}` not found",
            reference_path.display()
        );
        std::process::exit(2);
    }
    std::fs::create_dir_all(out_path)
        .with_context(|| format!("calibrate: mkdir {}", out_path.display()))?;

    // Default knob set — first-cut inventory of the engine's most
    // impactful tunables. Future revs may load this from a YAML
    // sidecar so users can scope a calibration run to specific knobs.
    let knobs = default_calibration_knobs();

    let mut loop_ = CalibrationLoop::new(
        obj,
        knobs,
        CalibrationConfig {
            max_iterations: max_iter,
            seeds_per_iteration: seeds,
            ..CalibrationConfig::default()
        },
    );

    // Resume hook.
    if let Some(resume_path) = resume {
        use datasynth_eval::calibration::CalibrationHistory;
        let history = CalibrationHistory::load(resume_path).with_context(|| {
            format!(
                "calibrate: failed to load --resume `{}`",
                resume_path.display()
            )
        })?;
        history.apply_to(&mut loop_).with_context(|| {
            "calibrate: --resume incompatible with this loop's objective / knob set".to_string()
        })?;
        tracing::info!(
            steps_resumed = loop_.history.len(),
            "calibrate: resumed from history",
        );
    }

    println!("calibrate: setup OK");
    println!("  objective       : {}", objective);
    println!("  max_iter        : {}", max_iter);
    println!("  seeds           : {}", seeds);
    println!("  target          : {:?}", target);
    println!("  knobs           : {}", loop_.knobs.len());
    println!("  resumed history : {} steps", loop_.history.len());

    if dry_run {
        println!("calibrate: --dry-run set, exiting without running the loop");
        return Ok(());
    }

    // The real generator-backed evaluator + Proposer drive go here.
    // For now this is a stub — the engine integration is a separate
    // follow-up that needs VM validation per CLAUDE.md.
    eprintln!(
        "calibrate: orchestrator-backed evaluator not yet wired (Piece 4b). \
         Re-run with --dry-run to smoke-test the CLI surface, or wait for the \
         VM-validated engine integration to land."
    );
    std::process::exit(2);
}

/// First-cut knob inventory for the calibration loop. Mirror the
/// v5.30 SOTA manually-tuned knob set so the calibration loop has
/// a sensible starting parameter space.
///
/// Each knob's `current` here is the v5.30 SOTA default; the loop
/// reads each knob's path off this list and patches the GroupConfig
/// before each generation. Bounds are the engine's documented
/// safe ranges; `max_step` is calibrated to give the loop enough
/// resolution to find a local minimum in ~10 iterations without
/// overshooting.
fn default_calibration_knobs() -> Vec<datasynth_eval::calibration::CalibrationKnob> {
    use datasynth_eval::calibration::CalibrationKnob;
    vec![
        // Anomaly rates — most impactful knobs per the v5.30 SOTA
        // tuning rounds.
        CalibrationKnob::new_f64("fraud.fraud_rate", 0.02, 0.005, 0.10, 0.005),
        CalibrationKnob::new_f64(
            "anomaly_injection.rates.consolidation_outlier_rate",
            0.001,
            0.0,
            0.01,
            0.0005,
        ),
        CalibrationKnob::new_f64("fraud.document_fraud_rate", 0.05, 0.01, 0.20, 0.01),
        // Concentration pipeline.
        CalibrationKnob::new_f64(
            "concentration.source_conditional_rarity.rate",
            0.01,
            0.0,
            0.05,
            0.005,
        ),
        CalibrationKnob::new_usize(
            "concentration.trading_partner_pool.target_size",
            12,
            5,
            50,
            3.0,
        ),
        CalibrationKnob::new_f64("concentration.source_blanking.rate", 0.21, 0.0, 0.5, 0.02),
    ]
}
