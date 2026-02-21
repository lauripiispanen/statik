# Statik Benchmarks

Reusable benchmark suite for measuring statik performance across project sizes.

## Quick Start

```bash
# Full run: build, generate projects, benchmark
./benchmarks/run_benchmarks.sh --label baseline

# After making changes, run again with a different label
./benchmarks/run_benchmarks.sh --label after

# Compare results
python3 benchmarks/compare_results.py \
    benchmarks/results_baseline_*.txt \
    benchmarks/results_after_*.txt
```

## Scripts

| Script | Purpose |
|---|---|
| `run_benchmarks.sh` | Main entry point. Builds, generates projects, runs hyperfine. |
| `generate_projects.py` | Creates synthetic TS projects (300/1000/3000 files). |
| `compare_results.py` | Parses two result files and shows a side-by-side comparison table. |

## Options

```
run_benchmarks.sh [options]
  --label NAME        Tag the output file (e.g., "baseline", "after-batch-queries")
  --skip-build        Don't rebuild the binary
  --skip-generate     Reuse existing projects in /tmp/statik-bench/
  --sizes "small"     Only benchmark specific sizes (small/medium/large)
  --runs N            Number of hyperfine runs per command (default: 10)
  --warmup N          Number of warmup runs (default: 3)
  --bench-dir PATH    Base directory for generated projects (default: /tmp/statik-bench)
  --export FILE       Export hyperfine results as JSON
```

## Project Sizes

| Name | Files | Typical warm query time |
|---|---|---|
| small | ~300 | ~12ms |
| medium | ~1000 | ~32ms |
| large | ~3000 | ~95ms |

## Requirements

- `hyperfine` (`brew install hyperfine`)
- `python3`
- `cargo` (for building)
