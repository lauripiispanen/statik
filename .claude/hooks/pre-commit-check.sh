#!/bin/bash
# Pre-commit checks for statik
# Ensures code compiles, passes tests, and has no clippy warnings

set -e

echo "Running cargo fmt --check..."
cargo fmt -- --check 2>&1

echo "Running cargo check..."
cargo check 2>&1

echo "Running cargo clippy..."
cargo clippy 2>&1

echo "Running cargo test..."
cargo test 2>&1

echo "All pre-commit checks passed."
