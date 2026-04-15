# Performance

DataSynth is designed for high throughput with bounded memory usage. Key optimizations include streaming writes, format-aware output, and parallel journal entry generation.

## Benchmark Results

Measured on a modern multi-core system:

| Scale | JE Count | CSV-Only | CSV+JSON | Notes |
|-------|----------|----------|----------|-------|
| Demo | ~500 | < 1s | < 1s | Single company, 3 months |
| 10K | 10,000 | ~1s | ~2s | Standard development config |
| 100K | 100,000 | ~5s | ~12s | Medium production run |
| XXL | 200,000+ | ~20s | ~80s | Full pipeline with all generators |

CSV-only mode (`formats: [csv]`) provides approximately 4x speedup over CSV+JSON by skipping JSON serialization.

## Streaming JSON Writer

Large datasets are written using a streaming JSON writer that:
- Writes records incrementally (no full-dataset buffering)
- Flushes periodically to bound memory usage
- Supports parallel writes for independent output files

## Format-Aware Output

```yaml
output:
  formats: [csv]    # Skip JSON entirely
```

When only CSV is requested, the pipeline avoids constructing JSON representations, reducing both CPU time and memory allocation.

## Parallel JE Writes

Journal entry output is parallelized across files when both CSV and JSON are requested. Each format writer operates on its own thread, fed from a shared channel.

## Optimization Tips

1. **Use CSV-only for large runs** -- `formats: [csv]` skips JSON overhead
2. **Set appropriate memory limits** -- `--memory-limit 4096` for XXL runs
3. **Limit thread count on shared systems** -- `--max-threads 4`
4. **Disable unused generators** -- Each `enabled: false` section is zero-cost
5. **Use `small` complexity for CI** -- ~100 GL accounts vs. 2,500 for `large`
6. **Seed for caching** -- Deterministic output enables build caching in CI pipelines

## Resource Monitoring

The orchestrator tracks resource usage in real time:

| Guard | Metric | Action |
|-------|--------|--------|
| `MemoryGuard` | RSS via `/proc/self/statm` | Triggers degradation at limit |
| `DiskGuard` | Free space via `statvfs` | Warns or stops if disk is low |
| `CpuMonitor` | CPU utilization | Auto-throttles at 95% |

Degradation levels: Normal -> Reduced -> Minimal -> Emergency. Each level progressively disables non-essential generators to keep the run within resource bounds.

## Throughput

Single-threaded baseline: ~200K+ journal entries per second. Throughput scales with available cores for parallel phases (document flows, subledgers, graph export).
