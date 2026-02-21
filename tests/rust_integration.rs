use std::path::{Path, PathBuf};

use statik::cli::commands;
use statik::cli::OutputFormat;

fn rust_fixture_source() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rust_project")
}

fn setup_rust_project() -> tempfile::TempDir {
    let src = rust_fixture_source();
    let tmp = tempfile::TempDir::new().unwrap();
    copy_dir_recursive(&src, tmp.path()).unwrap();
    tmp
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            let name = entry.file_name();
            if name == ".statik" {
                std::fs::create_dir_all(&dst_path)?;
                for inner in std::fs::read_dir(entry.path())? {
                    let inner = inner?;
                    if inner.file_name() != "index.db" {
                        std::fs::copy(inner.path(), dst_path.join(inner.file_name()))?;
                    }
                }
            } else {
                copy_dir_recursive(&entry.path(), &dst_path)?;
            }
        } else {
            std::fs::copy(entry.path(), &dst_path)?;
        }
    }
    Ok(())
}

fn index_rust_project(project_path: &Path) {
    let config = statik::discovery::DiscoveryConfig::default();
    let result = statik::cli::index::run_index(project_path, &config).unwrap();
    assert!(
        result.files_indexed > 0,
        "Should index at least one Rust file, got {}",
        result.files_indexed
    );
}

// =============================================================================
// INDEX - verify Rust files are discovered and indexed
// =============================================================================

#[test]
fn test_rust_index_discovers_all_files() {
    let tmp = setup_rust_project();
    let config = statik::discovery::DiscoveryConfig::default();
    let result = statik::cli::index::run_index(tmp.path(), &config).unwrap();

    // Files: lib.rs, main.rs, model/mod.rs, model/user.rs, service/mod.rs,
    //        service/user_service.rs, util.rs, cycle/mod.rs, cycle/a.rs,
    //        cycle/b.rs, tests/integration.rs
    assert_eq!(
        result.files_indexed, 11,
        "Expected 11 Rust files, got {}",
        result.files_indexed
    );
    assert!(
        result.symbols_extracted > 0,
        "Should extract symbols from Rust files"
    );
}

// =============================================================================
// DEPS - verify dependency tracking via mod declarations
// =============================================================================

#[test]
fn test_rust_deps_mod_declarations() {
    let tmp = setup_rust_project();
    index_rust_project(tmp.path());

    let output = commands::run_deps(
        tmp.path(),
        "src/lib.rs",
        false,
        "out",
        None,
        &OutputFormat::Json,
        true,
        false,
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let imports = json["imports"].as_array().unwrap();
    let import_paths: Vec<&str> = imports.iter().filter_map(|i| i["path"].as_str()).collect();

    // lib.rs has mod model; mod service; mod cycle; -> edges to those module files
    assert!(
        import_paths
            .iter()
            .any(|p| p.contains("model") && (p.ends_with("mod.rs") || p.ends_with("model.rs"))),
        "lib.rs should import model module, got {:?}",
        import_paths
    );
    assert!(
        import_paths.iter().any(|p| p.contains("service")),
        "lib.rs should import service module, got {:?}",
        import_paths
    );
    assert!(
        import_paths.iter().any(|p| p.contains("cycle")),
        "lib.rs should import cycle module, got {:?}",
        import_paths
    );
}

#[test]
fn test_rust_deps_use_import() {
    let tmp = setup_rust_project();
    index_rust_project(tmp.path());

    let output = commands::run_deps(
        tmp.path(),
        "src/service/user_service.rs",
        false,
        "out",
        None,
        &OutputFormat::Json,
        true,
        false,
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let imports = json["imports"].as_array().unwrap();
    let import_paths: Vec<&str> = imports.iter().filter_map(|i| i["path"].as_str()).collect();

    // user_service.rs has `use crate::model::User;` -> edge to model/user.rs or model/mod.rs
    assert!(
        import_paths.iter().any(|p| p.contains("model")),
        "user_service.rs should import from model, got {:?}",
        import_paths
    );
}

// =============================================================================
// EXPORTS - verify pub items are tracked as exports
// =============================================================================

#[test]
fn test_rust_exports_pub_struct() {
    let tmp = setup_rust_project();
    index_rust_project(tmp.path());

    let output =
        commands::run_exports(tmp.path(), "src/model/user.rs", &OutputFormat::Json, true).unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let exports = json["exports"].as_array().unwrap();
    let names: Vec<&str> = exports.iter().filter_map(|e| e["name"].as_str()).collect();

    assert!(
        names.contains(&"User"),
        "Should export User struct, got {:?}",
        names
    );
}

#[test]
fn test_rust_exports_reexport() {
    let tmp = setup_rust_project();
    index_rust_project(tmp.path());

    let output =
        commands::run_exports(tmp.path(), "src/lib.rs", &OutputFormat::Json, true).unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let exports = json["exports"].as_array().unwrap();
    let names: Vec<&str> = exports.iter().filter_map(|e| e["name"].as_str()).collect();

    assert!(
        names.contains(&"User"),
        "lib.rs should re-export User, got {:?}",
        names
    );
}

// =============================================================================
// DEAD CODE - verify orphan file detection
// =============================================================================

#[test]
fn test_rust_dead_code_detects_orphan() {
    let tmp = setup_rust_project();
    index_rust_project(tmp.path());

    let output =
        commands::run_dead_code(tmp.path(), "files", &OutputFormat::Json, true, false).unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let dead_files = json["dead_files"].as_array().unwrap();
    let dead_paths: Vec<&str> = dead_files
        .iter()
        .filter_map(|f| f["path"].as_str())
        .collect();

    // util.rs is not mod-declared from any crate root chain
    assert!(
        dead_paths.iter().any(|p| p.contains("util.rs")),
        "util.rs should be detected as dead, dead files: {:?}",
        dead_paths
    );
}

#[test]
fn test_rust_dead_code_excludes_entry_points() {
    let tmp = setup_rust_project();
    index_rust_project(tmp.path());

    let output =
        commands::run_dead_code(tmp.path(), "files", &OutputFormat::Json, true, false).unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let dead_files = json["dead_files"].as_array().unwrap();
    let dead_paths: Vec<&str> = dead_files
        .iter()
        .filter_map(|f| f["path"].as_str())
        .collect();

    // lib.rs and main.rs are entry points and should NOT be dead
    assert!(
        !dead_paths.iter().any(|p| p.ends_with("lib.rs")),
        "lib.rs should not be dead (entry point), dead: {:?}",
        dead_paths
    );
    assert!(
        !dead_paths.iter().any(|p| p.ends_with("main.rs")),
        "main.rs should not be dead (entry point), dead: {:?}",
        dead_paths
    );

    // tests/integration.rs should not be dead (tests/ is entry point)
    assert!(
        !dead_paths
            .iter()
            .any(|p| p.contains("tests/integration.rs")),
        "tests/integration.rs should not be dead (in tests/ dir), dead: {:?}",
        dead_paths
    );
}

// =============================================================================
// CYCLES - verify circular dependency detection
// =============================================================================

#[test]
fn test_rust_cycles_detected() {
    let tmp = setup_rust_project();
    index_rust_project(tmp.path());

    let output = commands::run_cycles(tmp.path(), &OutputFormat::Json, true, false).unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let cycles = json["cycles"].as_array().unwrap();

    assert!(!cycles.is_empty(), "Should detect the a.rs <-> b.rs cycle");

    let all_cycle_paths: Vec<&str> = cycles
        .iter()
        .flat_map(|c| {
            c["files"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|f| f["path"].as_str())
        })
        .collect();

    assert!(
        all_cycle_paths.iter().any(|p| p.contains("cycle/a.rs")),
        "Cycle should involve a.rs, got {:?}",
        all_cycle_paths
    );
    assert!(
        all_cycle_paths.iter().any(|p| p.contains("cycle/b.rs")),
        "Cycle should involve b.rs, got {:?}",
        all_cycle_paths
    );
}

// =============================================================================
// SUMMARY - verify project summary includes Rust files
// =============================================================================

#[test]
fn test_rust_summary_command() {
    let tmp = setup_rust_project();
    index_rust_project(tmp.path());

    let output = commands::run_summary(tmp.path(), &OutputFormat::Json, true).unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let total_files = json["files"]["total"].as_u64().unwrap();

    assert_eq!(
        total_files, 11,
        "Summary should report 11 Rust files, got {}",
        total_files
    );

    // Check language breakdown
    if let Some(by_language) = json["files"]["by_language"].as_object() {
        let rust_count = by_language
            .get("Rust")
            .or_else(|| by_language.get("rust"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        assert_eq!(
            rust_count, 11,
            "Should report 11 Rust files in language breakdown, got {}",
            rust_count
        );
    }
}

// =============================================================================
// IMPACT - verify impact analysis
// =============================================================================

#[test]
fn test_rust_impact_analysis() {
    let tmp = setup_rust_project();
    index_rust_project(tmp.path());

    let output = commands::run_impact(
        tmp.path(),
        "src/model/user.rs",
        None,
        &OutputFormat::Json,
        true,
        false,
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let total_affected = json["summary"]["total_affected"].as_u64().unwrap();

    // user.rs is imported by model/mod.rs which is imported by lib.rs
    // user_service.rs also imports from model
    assert!(
        total_affected >= 1,
        "Changing user.rs should affect at least 1 file, got {}",
        total_affected
    );
}
