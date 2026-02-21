#!/usr/bin/env python3
"""Generate synthetic TypeScript projects for benchmarking statik.

Creates projects of varying sizes under /tmp/statik-bench-*/ with realistic
import structures, exports, and directory layouts.

Usage:
    python3 benchmarks/generate_projects.py [--output-dir /tmp/statik-bench]
"""

import argparse
import os
import random

# Fixed seed for reproducible benchmarks
random.seed(42)

DIRS = [
    "src/components", "src/hooks", "src/utils", "src/services", "src/models",
    "src/api", "src/types", "src/pages", "src/middleware", "src/config",
    "src/lib", "src/helpers", "src/store", "src/actions", "src/reducers",
    "src/features/auth", "src/features/dashboard", "src/features/settings",
    "src/features/profile", "src/features/analytics", "src/shared/ui",
    "src/shared/hooks", "src/shared/utils", "src/shared/types",
    "src/core", "src/core/auth", "src/core/api", "src/core/storage",
]

SIZES = {
    "small": 300,
    "medium": 1000,
    "large": 3000,
}

LINT_RULES = """\
[[rules]]
id = "no-api-to-ui"
severity = "error"
description = "API layer must not import from UI layer"

[rules.boundary]
from = ["src/api/**"]
deny = ["src/components/**"]

[[rules]]
id = "no-god-modules"
severity = "warning"
description = "Files should not have too many dependencies"

[rules.fan_limit]
pattern = ["src/**"]
max_fan_out = 10
"""


def generate_project(project_dir: str, num_files: int) -> None:
    """Generate a synthetic TS project with the given number of files."""
    for d in DIRS:
        os.makedirs(os.path.join(project_dir, d), exist_ok=True)

    files = []
    for i in range(num_files):
        d = random.choice(DIRS)
        ext = random.choice([".ts", ".ts", ".ts", ".tsx"])
        name = f"module{i}{ext}"
        path = os.path.join(d, name)
        files.append(path)

    # Write files with realistic imports and exports
    max_imports = min(10, num_files - 1)
    for idx, fpath in enumerate(files):
        full = os.path.join(project_dir, fpath)
        lines = []

        # Add imports from other files
        num_imports = random.randint(0, max_imports)
        targets = random.sample(
            [f for f in files if f != fpath], min(num_imports, len(files) - 1)
        )
        for t in targets:
            from_dir = os.path.dirname(fpath)
            rel = os.path.relpath(t, from_dir).replace(".ts", "").replace(".tsx", "")
            if not rel.startswith("."):
                rel = "./" + rel
            sym = f"thing{files.index(t)}"
            if random.random() < 0.3:
                lines.append(f"import type {{ {sym} }} from '{rel}';")
            else:
                lines.append(f"import {{ {sym} }} from '{rel}';")

        # Export some symbols
        num_exports = random.randint(1, 5)
        for e in range(num_exports):
            kind = random.choice(["function", "const", "class", "interface", "type"])
            sym_name = f"thing{idx}" if e == 0 else f"helper{idx}_{e}"
            if kind == "function":
                lines.append(f"export function {sym_name}() {{ return {idx}; }}")
            elif kind == "const":
                lines.append(f"export const {sym_name} = {idx};")
            elif kind == "class":
                lines.append(f"export class {sym_name} {{ value = {idx}; }}")
            elif kind == "interface":
                lines.append(f"export interface {sym_name} {{ id: number; }}")
            elif kind == "type":
                lines.append(f"export type {sym_name} = {{ id: number }};")

        with open(full, "w") as f:
            f.write("\n".join(lines) + "\n")

    # Create an entry point
    entry = os.path.join(project_dir, "src/index.ts")
    with open(entry, "w") as f:
        entry_imports = min(max(20, num_files // 50), len(files))
        for t in random.sample(files, entry_imports):
            rel = os.path.relpath(t, "src").replace(".ts", "").replace(".tsx", "")
            if not rel.startswith("."):
                rel = "./" + rel
            sym = f"thing{files.index(t)}"
            f.write(f"import {{ {sym} }} from '{rel}';\n")
        f.write("console.log('entry');\n")

    # Create .statik/rules.toml for lint benchmarks
    statik_dir = os.path.join(project_dir, ".statik")
    os.makedirs(statik_dir, exist_ok=True)
    with open(os.path.join(statik_dir, "rules.toml"), "w") as f:
        f.write(LINT_RULES)

    total = num_files + 1  # +1 for index.ts
    print(f"  Generated {total} files in {project_dir}")


def main():
    parser = argparse.ArgumentParser(description="Generate benchmark projects")
    parser.add_argument(
        "--output-dir",
        default="/tmp/statik-bench",
        help="Base output directory (default: /tmp/statik-bench)",
    )
    parser.add_argument(
        "--sizes",
        nargs="*",
        choices=list(SIZES.keys()),
        default=list(SIZES.keys()),
        help="Which project sizes to generate (default: all)",
    )
    args = parser.parse_args()

    print("Generating benchmark projects...")
    for size_name in args.sizes:
        num_files = SIZES[size_name]
        project_dir = os.path.join(args.output_dir, size_name)
        generate_project(project_dir, num_files)

    print("Done.")


if __name__ == "__main__":
    main()
