# Installation

## Building from Source

```bash
git clone https://github.com/mivertowski/SyntheticData.git
cd SyntheticData
cargo build --release
```

The binary is at `target/release/datasynth-data`.

## Feature Flags

The default build includes all core generators. Optional features require explicit opt-in:

| Feature | Crate | Purpose |
|---------|-------|---------|
| `llm` | `datasynth-cli` | Natural language config generation (`--from-description`) |
| `adversarial` | `datasynth-cli` | ONNX model probing (`adversarial` subcommand) |
| `streaming` | `datasynth-cli` | HTTP streaming to RustGraph endpoints (enabled by default) |
| `neural` | `datasynth-core` | Candle-based neural diffusion backend |
| `rustgraph` | `datasynth-graph` | RustGraph bulk export support |

Build with optional features:

```bash
# LLM config generation
cargo build --release --features llm

# Adversarial model probing (requires ONNX Runtime)
cargo build --release --features adversarial

# Multiple features
cargo build --release --features "llm,adversarial"
```

## Verifying the Installation

```bash
target/release/datasynth-data --version
target/release/datasynth-data info
```

The `info` command lists available industry presets and complexity levels.

## Adding to PATH

```bash
# Option 1: symlink
ln -s "$(pwd)/target/release/datasynth-data" ~/.local/bin/datasynth-data

# Option 2: cargo install (from workspace root)
cargo install --path crates/datasynth-cli
```
