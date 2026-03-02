//! Dogfood test: run statik on its own codebase to verify symbol-level dead code
//! detection with cross-file linking produces reasonable results.

use std::path::PathBuf;

use statik::cli::commands;
use statik::cli::OutputFormat;
use statik::discovery::DiscoveryConfig;

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn test_dogfood_dead_symbols_cross_file_linking() {
    let root = project_root();

    // Index the project
    let config = DiscoveryConfig::default();
    statik::cli::index::run_index(&root, &config, false).unwrap();

    // Run symbol-level dead code analysis
    let output = commands::run_dead_code(
        &root,
        "symbols",
        &OutputFormat::Json,
        true,
        false,
        None,
        None,
        None,
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let total = json["summary"]["total_symbols"].as_u64().unwrap();
    let dead = json["summary"]["dead_symbols"].as_u64().unwrap();
    let entry = json["summary"]["entry_point_symbols"].as_u64().unwrap();
    let resolved = json["summary"]["resolved_references"].as_u64().unwrap();
    let unresolved = json["summary"]["unresolved_references"].as_u64().unwrap();

    eprintln!(
        "Dogfood: total={}, dead={}, entry={}, resolved={}, unresolved={}",
        total, dead, entry, resolved, unresolved
    );
    eprintln!(
        "Dogfood: dead ratio = {:.1}%",
        dead as f64 / total as f64 * 100.0
    );

    assert!(
        total > 100,
        "Should have a substantial number of symbols, got {}",
        total
    );
    assert!(entry > 0, "Should have entry point symbols, got {}", entry);
    assert!(
        resolved > 100,
        "Should have many resolved references (including cross-file), got {}",
        resolved
    );

    // Cross-file linking should have resolved most references
    // (unresolved should be small relative to resolved)
    let resolution_ratio = resolved as f64 / (resolved + unresolved).max(1) as f64;
    assert!(
        resolution_ratio > 0.80,
        "Reference resolution ratio should be > 80%, got {:.1}% ({}/{})",
        resolution_ratio * 100.0,
        resolved,
        resolved + unresolved
    );

    // Key functions called cross-file from main.rs should be alive
    let dead_names: std::collections::HashSet<String> = json["dead_symbols"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap().to_string())
        .collect();

    let must_be_alive = [
        "build_file_graph",
        "run_deps",
        "run_dead_code",
        "run_cycles",
        "run_exports",
        "run_summary",
        "run_impact",
        "run_index",
    ];

    for name in &must_be_alive {
        assert!(
            !dead_names.contains(*name),
            "Cross-file linked function '{}' should be alive, but was flagged dead",
            name
        );
    }

    // The dead ratio should be reasonable. Note: test fixtures, test functions,
    // and method calls (which the parser doesn't fully track) inflate this.
    // Only exports actually imported (per linker) are seeded as entry points,
    // so exports never imported by anyone are correctly flagged dead.
    // We expect < 85% given the above known gaps.
    let dead_ratio = dead as f64 / total as f64;
    assert!(
        dead_ratio < 0.85,
        "Dead symbol ratio should be < 85%, got {:.1}% ({}/{})",
        dead_ratio * 100.0,
        dead,
        total
    );
}
