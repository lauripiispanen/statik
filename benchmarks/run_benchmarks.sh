#!/usr/bin/env bash
#
# run_benchmarks.sh - Run the full statik benchmark suite
#
# Usage:
#   ./benchmarks/run_benchmarks.sh                    # build + generate + benchmark
#   ./benchmarks/run_benchmarks.sh --skip-build       # skip cargo build
#   ./benchmarks/run_benchmarks.sh --skip-generate    # skip project generation
#   ./benchmarks/run_benchmarks.sh --label baseline   # tag output file with a label
#   ./benchmarks/run_benchmarks.sh --sizes small      # only benchmark "small" project
#   ./benchmarks/run_benchmarks.sh --export results.json  # export hyperfine JSON
#
# Requirements: hyperfine, python3
# Install hyperfine: brew install hyperfine  (or cargo install hyperfine)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
BENCH_DIR="/tmp/statik-bench"
LABEL=""
SKIP_BUILD=false
SKIP_GENERATE=false
SIZES="small medium large"
EXPORT_JSON=""
WARMUP=3
RUNS=10

# Parse arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        --skip-build)    SKIP_BUILD=true; shift ;;
        --skip-generate) SKIP_GENERATE=true; shift ;;
        --label)         LABEL="$2"; shift 2 ;;
        --sizes)         SIZES="$2"; shift 2 ;;
        --bench-dir)     BENCH_DIR="$2"; shift 2 ;;
        --warmup)        WARMUP="$2"; shift 2 ;;
        --runs)          RUNS="$2"; shift 2 ;;
        --export)        EXPORT_JSON="$2"; shift 2 ;;
        -h|--help)
            head -14 "$0" | tail -12
            exit 0
            ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

# Check dependencies
if ! command -v hyperfine &>/dev/null; then
    echo "Error: hyperfine is required. Install with: brew install hyperfine"
    exit 1
fi

if ! command -v python3 &>/dev/null; then
    echo "Error: python3 is required."
    exit 1
fi

STATIK="$REPO_ROOT/target/release/statik"

# Step 1: Build
if [ "$SKIP_BUILD" = false ]; then
    echo "=== Building release binary ==="
    cargo build --release --manifest-path "$REPO_ROOT/Cargo.toml" 2>&1
    echo ""
fi

if [ ! -f "$STATIK" ]; then
    echo "Error: Binary not found at $STATIK"
    echo "Run without --skip-build or build manually first."
    exit 1
fi

# Step 2: Generate test projects
if [ "$SKIP_GENERATE" = false ]; then
    echo "=== Generating benchmark projects ==="
    python3 "$SCRIPT_DIR/generate_projects.py" --output-dir "$BENCH_DIR" --sizes $SIZES
    echo ""
fi

# Step 3: Prepare results file
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
if [ -n "$LABEL" ]; then
    RESULTS_FILE="$SCRIPT_DIR/results_${LABEL}_${TIMESTAMP}.txt"
else
    RESULTS_FILE="$SCRIPT_DIR/results_${TIMESTAMP}.txt"
fi

# Collect system info
{
    echo "============================================================"
    echo "Statik Benchmark Results"
    echo "============================================================"
    echo "Date:     $(date -u '+%Y-%m-%d %H:%M:%S UTC')"
    echo "Label:    ${LABEL:-<none>}"
    echo "Binary:   $STATIK"
    echo "Platform: $(uname -s) $(uname -m)"
    echo "Warmup:   $WARMUP"
    echo "Runs:     $RUNS"
    if [ -d "$REPO_ROOT/.git" ]; then
        echo "Commit:   $(git -C "$REPO_ROOT" rev-parse --short HEAD 2>/dev/null || echo 'unknown')"
        echo "Branch:   $(git -C "$REPO_ROOT" branch --show-current 2>/dev/null || echo 'unknown')"
    fi
    echo "============================================================"
    echo ""
} | tee "$RESULTS_FILE"

# Hyperfine JSON export setup
HYPERFINE_EXPORT_ARGS=""
if [ -n "$EXPORT_JSON" ]; then
    HYPERFINE_EXPORT_ARGS="--export-json $EXPORT_JSON"
fi

# Step 4: Run benchmarks for each project size
for SIZE in $SIZES; do
    PROJECT="$BENCH_DIR/$SIZE"
    if [ ! -d "$PROJECT" ]; then
        echo "Warning: Project directory $PROJECT not found, skipping $SIZE"
        continue
    fi

    FILE_COUNT=$(find "$PROJECT/src" -name '*.ts' -o -name '*.tsx' | wc -l | tr -d ' ')

    {
        echo "=========================================="
        echo "Project: $SIZE ($FILE_COUNT files)"
        echo "=========================================="
        echo ""
    } | tee -a "$RESULTS_FILE"

    # Index the project first
    cd "$PROJECT"
    rm -f .statik/index.db

    # Benchmark: index (cold)
    echo "--- index (cold) ---" | tee -a "$RESULTS_FILE"
    hyperfine \
        --warmup 1 \
        --runs "$RUNS" \
        --prepare "rm -f .statik/index.db" \
        "$STATIK index ." \
        2>&1 | tee -a "$RESULTS_FILE"
    echo "" | tee -a "$RESULTS_FILE"

    # Ensure index exists for warm queries
    "$STATIK" index . >/dev/null 2>&1

    # Benchmark: deps
    echo "--- deps ---" | tee -a "$RESULTS_FILE"
    hyperfine \
        --warmup "$WARMUP" \
        --runs "$RUNS" \
        "$STATIK deps src/index.ts --no-index" \
        2>&1 | tee -a "$RESULTS_FILE"
    echo "" | tee -a "$RESULTS_FILE"

    # Benchmark: deps --transitive
    echo "--- deps --transitive ---" | tee -a "$RESULTS_FILE"
    hyperfine \
        --warmup "$WARMUP" \
        --runs "$RUNS" \
        "$STATIK deps src/index.ts --no-index --transitive" \
        2>&1 | tee -a "$RESULTS_FILE"
    echo "" | tee -a "$RESULTS_FILE"

    # Benchmark: dead-code
    echo "--- dead-code ---" | tee -a "$RESULTS_FILE"
    hyperfine \
        --warmup "$WARMUP" \
        --runs "$RUNS" \
        "$STATIK dead-code --no-index" \
        2>&1 | tee -a "$RESULTS_FILE"
    echo "" | tee -a "$RESULTS_FILE"

    # Benchmark: cycles
    echo "--- cycles ---" | tee -a "$RESULTS_FILE"
    hyperfine \
        --warmup "$WARMUP" \
        --runs "$RUNS" \
        "$STATIK cycles --no-index" \
        2>&1 | tee -a "$RESULTS_FILE"
    echo "" | tee -a "$RESULTS_FILE"

    # Benchmark: impact
    echo "--- impact ---" | tee -a "$RESULTS_FILE"
    hyperfine \
        --warmup "$WARMUP" \
        --runs "$RUNS" \
        "$STATIK impact src/index.ts --no-index" \
        2>&1 | tee -a "$RESULTS_FILE"
    echo "" | tee -a "$RESULTS_FILE"

    # Benchmark: summary
    echo "--- summary ---" | tee -a "$RESULTS_FILE"
    hyperfine \
        --warmup "$WARMUP" \
        --runs "$RUNS" \
        "$STATIK summary --no-index" \
        2>&1 | tee -a "$RESULTS_FILE"
    echo "" | tee -a "$RESULTS_FILE"

    # Benchmark: lint
    echo "--- lint ---" | tee -a "$RESULTS_FILE"
    hyperfine \
        --warmup "$WARMUP" \
        --runs "$RUNS" \
        -i \
        "$STATIK lint --no-index" \
        2>&1 | tee -a "$RESULTS_FILE"
    echo "" | tee -a "$RESULTS_FILE"

    # Benchmark: exports
    echo "--- exports ---" | tee -a "$RESULTS_FILE"
    hyperfine \
        --warmup "$WARMUP" \
        --runs "$RUNS" \
        "$STATIK exports src/index.ts --no-index" \
        2>&1 | tee -a "$RESULTS_FILE"
    echo "" | tee -a "$RESULTS_FILE"
done

echo "============================================================" | tee -a "$RESULTS_FILE"
echo "Results saved to: $RESULTS_FILE"
echo "============================================================"
