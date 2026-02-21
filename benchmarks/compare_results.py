#!/usr/bin/env python3
"""Compare two benchmark result files and display a summary table.

Parses the output from run_benchmarks.sh to extract timing data and
shows a side-by-side comparison with speedup ratios.

Usage:
    python3 benchmarks/compare_results.py benchmarks/results_baseline_*.txt benchmarks/results_after_*.txt
"""

import re
import sys


def parse_results(filepath: str) -> dict:
    """Parse a benchmark results file into a nested dict.

    Returns: {project_size: {command: mean_ms}}
    """
    results = {}
    current_project = None

    # Match project header: "Project: small (301 files)"
    project_re = re.compile(r"Project:\s+(\w+)\s+\((\d+)\s+files\)")
    # Match command header: "--- deps ---"
    command_re = re.compile(r"^---\s+(.+?)\s+---$")
    # Match hyperfine output: "Time (mean +/- s): 30.1 ms +/- 1.2 ms"
    time_re = re.compile(
        r"Time\s+\(mean\s*.*?\):\s+"
        r"(\d+\.?\d*)\s*(ms|s)\s*"
        r"[+-±]+\s*"
        r"(\d+\.?\d*)\s*(ms|s)"
    )

    current_command = None

    with open(filepath) as f:
        for line in f:
            line = line.strip()

            m = project_re.search(line)
            if m:
                current_project = m.group(1)
                file_count = int(m.group(2))
                if current_project not in results:
                    results[current_project] = {"_files": file_count}
                continue

            m = command_re.match(line)
            if m:
                current_command = m.group(1)
                continue

            m = time_re.search(line)
            if m and current_project and current_command:
                mean_val = float(m.group(1))
                mean_unit = m.group(2)
                sigma_val = float(m.group(3))
                sigma_unit = m.group(4)

                # Normalize to ms
                if mean_unit == "s":
                    mean_val *= 1000
                if sigma_unit == "s":
                    sigma_val *= 1000

                results[current_project][current_command] = {
                    "mean": mean_val,
                    "sigma": sigma_val,
                }
                current_command = None

    return results


def format_time(ms: float) -> str:
    """Format milliseconds for display."""
    if ms < 1:
        return f"{ms:.2f}ms"
    elif ms < 100:
        return f"{ms:.1f}ms"
    else:
        return f"{ms:.0f}ms"


def main():
    if len(sys.argv) != 3:
        print("Usage: compare_results.py <baseline.txt> <after.txt>")
        sys.exit(1)

    baseline_path = sys.argv[1]
    after_path = sys.argv[2]

    baseline = parse_results(baseline_path)
    after = parse_results(after_path)

    print("=" * 90)
    print("Benchmark Comparison")
    print("=" * 90)
    print(f"  Baseline: {baseline_path}")
    print(f"  After:    {after_path}")
    print()

    # Collect all project sizes and commands
    all_sizes = sorted(
        set(baseline.keys()) | set(after.keys()),
        key=lambda s: {"small": 0, "medium": 1, "large": 2}.get(s, 3),
    )

    all_commands = set()
    for size in all_sizes:
        if size in baseline:
            all_commands |= {
                k for k in baseline[size] if k != "_files"
            }
        if size in after:
            all_commands |= {k for k in after[size] if k != "_files"}

    # Deterministic command order
    command_order = [
        "index (cold)",
        "deps",
        "deps --transitive",
        "dead-code",
        "cycles",
        "impact",
        "summary",
        "lint",
        "exports",
    ]
    all_commands_sorted = [c for c in command_order if c in all_commands]
    all_commands_sorted += sorted(all_commands - set(command_order))

    for size in all_sizes:
        b = baseline.get(size, {})
        a = after.get(size, {})
        files = b.get("_files") or a.get("_files", "?")

        print(f"  {size} ({files} files)")
        print(f"  {'Command':<22} {'Baseline':>12} {'After':>12} {'Change':>12} {'Speedup':>10}")
        print(f"  {'-' * 70}")

        for cmd in all_commands_sorted:
            b_data = b.get(cmd)
            a_data = a.get(cmd)

            b_str = format_time(b_data["mean"]) if b_data else "-"
            a_str = format_time(a_data["mean"]) if a_data else "-"

            if b_data and a_data:
                diff = a_data["mean"] - b_data["mean"]
                pct = (diff / b_data["mean"]) * 100
                ratio = b_data["mean"] / a_data["mean"] if a_data["mean"] > 0 else 0

                if diff < 0:
                    change_str = f"{pct:+.1f}%"
                    speedup_str = f"{ratio:.2f}x"
                else:
                    change_str = f"{pct:+.1f}%"
                    speedup_str = f"{ratio:.2f}x"
            else:
                change_str = "-"
                speedup_str = "-"

            print(
                f"  {cmd:<22} {b_str:>12} {a_str:>12} {change_str:>12} {speedup_str:>10}"
            )

        print()

    print("=" * 90)
    print("  Speedup > 1.0x means the 'after' version is faster.")
    print("  Speedup < 1.0x means the 'after' version is slower (regression).")
    print("=" * 90)


if __name__ == "__main__":
    main()
