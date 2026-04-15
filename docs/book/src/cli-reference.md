# CLI Reference

Binary: `datasynth-data`

## `generate`

Generate synthetic accounting data.

```bash
datasynth-data generate [OPTIONS]
```

| Flag | Description |
|------|-------------|
| `-c, --config <PATH>` | Path to YAML configuration file |
| `-o, --output <DIR>` | Output directory (default: `./output`) |
| `--demo` | Use built-in demo preset |
| `--preset <NAME>` | Apply overlay preset (`audit-group`) |
| `--scenario-pack <NAME>` | Load a scenario pack YAML |
| `--fingerprint <PATH>` | Generate from a `.dsf` fingerprint file |
| `--scale <FLOAT>` | Scale factor for fingerprint generation (default: 1.0) |
| `-s, --seed <INT>` | Random seed for reproducibility |
| `--banking` | Enable KYC/AML data generation |
| `--audit` | Enable audit engagement generation |
| `--memory-limit <MB>` | Memory ceiling in MB (default: 1024) |
| `--max-threads <N>` | Max CPU threads (default: half of cores) |
| `--graph-export` | Enable PyTorch Geometric graph export |
| `--stream-target <URL>` | Stream hypergraph JSONL to endpoint |
| `--stream-api-key <KEY>` | API key for stream endpoint |
| `--stream-batch-size <N>` | Lines per HTTP POST (default: 1000) |
| `--quality-gate <LEVEL>` | Quality gate: `none`, `lenient`, `default`, `strict` |
| `--fiscal-year-months <N>` | Months per fiscal year for multi-period |
| `--append` | Append to existing output (requires `session.dss`) |
| `--months <N>` | Additional months for incremental generation |
| `--fraud-scenario <NAME>` | Apply fraud scenario (repeatable) |
| `--fraud-rate <FLOAT>` | Override fraud rate |
| `--stream-file <PATH>` | Stream output to JSONL file |
| `--export-format <FMT>` | Additional exports: `sap`, `fec`, `gobd` (repeatable) |
| `--auto-tune` | AI-powered generate/evaluate/patch loop |
| `--max-iterations <N>` | Max auto-tune iterations (default: 3) |

## `validate`

Check a configuration file for errors.

```bash
datasynth-data validate --config config.yaml
```

## `init`

Generate a sample configuration file.

```bash
datasynth-data init [OPTIONS]
```

| Flag | Description |
|------|-------------|
| `-o, --output <PATH>` | Output path (default: `datasynth_config.yaml`) |
| `-i, --industry <NAME>` | Industry preset (default: `manufacturing`) |
| `-c, --complexity <LEVEL>` | `small`, `medium`, `large` (default: `medium`) |
| `--from-description <TEXT>` | Generate config from natural language (requires `llm` feature) |

## `info`

Display available presets, industries, and complexity levels.

## `verify`

Verify output integrity (checksums, record counts).

```bash
datasynth-data verify --output ./output --checksums --record-counts
```

## `fingerprint`

Privacy-preserving fingerprint extraction and synthesis. See [Fingerprinting](fingerprinting.md).

| Subcommand | Description |
|------------|-------------|
| `extract` | Extract fingerprint from CSV data to `.dsf` |
| `validate` | Validate a `.dsf` file |
| `info` | Show fingerprint statistics |
| `diff` | Compare two fingerprints |
| `evaluate` | Evaluate synthetic data fidelity against fingerprint |
| `synthesize` | Generate synthetic data from fingerprint |

## `scenario`

Counterfactual scenario management. See [Scenario Engine](scenarios/README.md).

| Subcommand | Description |
|------------|-------------|
| `list` | List scenarios in a config |
| `validate` | Validate scenario definitions |
| `generate` | Generate paired baseline/counterfactual datasets |
| `diff` | Compare baseline vs. counterfactual outputs |
| `export` | Export scenario as portable `.dss` file |
| `import` | Import `.dss` scenario into a config |

## `adversarial`

ONNX model boundary probing. Requires `adversarial` feature. See [Adversarial Testing](ai/adversarial.md).

```bash
datasynth-data adversarial --model model.onnx --features 10 --probes 1000
```

## `audit`

Audit FSM blueprint commands. See [Audit FSM Engine](audit-fsm.md).

| Subcommand | Description |
|------------|-------------|
| `validate` | Validate a blueprint YAML |
| `info` | Display blueprint information |
| `run` | Run standalone FSM engagement |
| `diff` | Compare two blueprints structurally |
| `benchmark` | Generate benchmark audit event log |
