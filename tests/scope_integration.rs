use std::path::Path;

use statik::cli::commands;
use statik::cli::OutputFormat;

/// Create a temporary TypeScript project with scope config.
fn setup_scope_project() -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();

    // Create directory structure
    std::fs::create_dir_all(root.join("src/app")).unwrap();
    std::fs::create_dir_all(root.join("src/db")).unwrap();
    std::fs::create_dir_all(root.join("tests")).unwrap();
    std::fs::create_dir_all(root.join("fixtures")).unwrap();
    std::fs::create_dir_all(root.join(".statik")).unwrap();

    // Production files
    std::fs::write(
        root.join("src/app/handler.ts"),
        "import { connect } from '../db/connection';\nexport function handle() { connect(); }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/db/connection.ts"),
        "export function connect() { return 'connected'; }\n",
    )
    .unwrap();

    // Test file (entry point, lint=false)
    std::fs::write(
        root.join("tests/handler.test.ts"),
        "import { handle } from '../src/app/handler';\nhandle();\n",
    )
    .unwrap();

    // Fixture file (entry point, analysis=false)
    std::fs::write(
        root.join("fixtures/mock_db.ts"),
        "export function mockConnect() { return 'mocked'; }\n",
    )
    .unwrap();

    // Scope config
    std::fs::write(
        root.join(".statik/rules.toml"),
        r#"
[scope.production]
include = ["src/**"]

[scope.test]
include = ["tests/**"]
role = "entry_point"
lint = false

[scope.fixture]
include = ["fixtures/**"]
role = "entry_point"
analysis = false

[[rules]]
id = "no-app-to-db"
severity = "warning"
description = "App should not directly access DB"

[rules.boundary]
from = ["src/app/**", "tests/**"]
deny = ["src/db/**"]
"#,
    )
    .unwrap();

    tmp
}

fn index_project(project_path: &Path) {
    let config = statik::discovery::DiscoveryConfig::default();
    let result = statik::cli::index::run_index(project_path, &config, false).unwrap();
    assert!(
        result.files_indexed > 0,
        "Should index at least one file, got {}",
        result.files_indexed
    );
}

// =============================================================================
// Scope classification: files get correct source_set labels
// =============================================================================

#[test]
fn test_scope_classification() {
    let tmp = setup_scope_project();
    index_project(tmp.path());

    let graph = commands::build_file_graph(
        &statik::db::Database::open(&tmp.path().join(".statik/index.db")).unwrap(),
        tmp.path(),
    )
    .unwrap();

    // Check that files are classified into correct source sets
    let mut found_production = false;
    let mut found_test = false;
    let mut found_fixture = false;

    for info in graph.files.values() {
        let rel_path = info
            .path
            .strip_prefix(tmp.path())
            .unwrap()
            .to_str()
            .unwrap();
        match info.source_set.as_deref() {
            Some("production") => {
                assert!(
                    rel_path.starts_with("src/"),
                    "Production file should be in src/: {}",
                    rel_path
                );
                found_production = true;
            }
            Some("test") => {
                assert!(
                    rel_path.starts_with("tests/"),
                    "Test file should be in tests/: {}",
                    rel_path
                );
                found_test = true;
            }
            Some("fixture") => {
                assert!(
                    rel_path.starts_with("fixtures/"),
                    "Fixture file should be in fixtures/: {}",
                    rel_path
                );
                found_fixture = true;
            }
            other => {
                panic!("File {} has unexpected source_set: {:?}", rel_path, other);
            }
        }
    }

    assert!(found_production, "Should have production files");
    assert!(found_test, "Should have test files");
    assert!(found_fixture, "Should have fixture files");
}

// =============================================================================
// role=entry_point: test and fixture files are entry points
// =============================================================================

#[test]
fn test_scope_role_entry_point() {
    let tmp = setup_scope_project();
    index_project(tmp.path());

    let graph = commands::build_file_graph(
        &statik::db::Database::open(&tmp.path().join(".statik/index.db")).unwrap(),
        tmp.path(),
    )
    .unwrap();

    for info in graph.files.values() {
        let rel_path = info
            .path
            .strip_prefix(tmp.path())
            .unwrap()
            .to_str()
            .unwrap();
        match info.source_set.as_deref() {
            Some("test") | Some("fixture") => {
                assert!(
                    info.is_entry_point,
                    "File {} in {} source set should be an entry point",
                    rel_path,
                    info.source_set.as_deref().unwrap()
                );
            }
            Some("production") => {
                // Production files should NOT be entry points (unless
                // they match hardcoded patterns, which these don't)
            }
            _ => {}
        }
    }
}

// =============================================================================
// lint=false: violations from test files are suppressed
// =============================================================================

#[test]
fn test_scope_lint_false_suppresses_violations() {
    let tmp = setup_scope_project();
    index_project(tmp.path());

    let (output, _has_errors) = commands::run_lint(
        tmp.path(),
        None,
        None,
        "info",
        &OutputFormat::Json,
        true,
        None,
        false,
        None,
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let violations = json["violations"].as_array().unwrap();

    // The boundary rule denies tests/** -> src/db/**
    // But since test source set has lint=false, the test file violation should
    // be suppressed. Only the production file (handler.ts -> connection.ts)
    // should generate a violation.
    for violation in violations {
        let source = violation["source_file"].as_str().unwrap();
        assert!(
            !source.starts_with("tests/"),
            "Test file violation should be suppressed by lint=false, got source: {}",
            source
        );
    }

    // The production file (src/app/handler.ts) should still generate a violation
    let has_production_violation = violations.iter().any(|v| {
        v["source_file"]
            .as_str()
            .unwrap()
            .contains("src/app/handler")
    });
    assert!(
        has_production_violation,
        "Production file should still generate violation, violations: {:?}",
        violations
    );
}

// =============================================================================
// analysis=false: fixture files excluded from dead-code output
// =============================================================================

#[test]
fn test_scope_analysis_false_excludes_from_dead_code() {
    let tmp = setup_scope_project();
    index_project(tmp.path());

    let output = commands::run_dead_code(
        tmp.path(),
        "files",
        &OutputFormat::Json,
        true,
        false,
        None,
        None,
        None,
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let dead_files = json["dead_files"].as_array().unwrap();

    // Fixture files should NOT appear in dead code output (analysis=false)
    for dead in dead_files {
        let path = dead["path"].as_str().unwrap();
        assert!(
            !path.contains("fixtures/"),
            "Fixture file should not appear in dead code output (analysis=false), got: {}",
            path
        );
    }
}

// =============================================================================
// Backward compatibility: no [scope] config = all files visible
// =============================================================================

#[test]
fn test_scope_backward_compat_no_config() {
    let tmp = setup_scope_project();

    // Overwrite config to remove scope section
    std::fs::write(
        tmp.path().join(".statik/rules.toml"),
        r#"
[[rules]]
id = "test-rule"
severity = "warning"
description = "Test rule"

[rules.boundary]
from = ["src/app/**"]
deny = ["src/db/**"]
"#,
    )
    .unwrap();

    index_project(tmp.path());

    let graph = commands::build_file_graph(
        &statik::db::Database::open(&tmp.path().join(".statik/index.db")).unwrap(),
        tmp.path(),
    )
    .unwrap();

    // Without scope config, all files should have source_set = None
    for info in graph.files.values() {
        assert!(
            info.source_set.is_none(),
            "Without scope config, source_set should be None for {}, got {:?}",
            info.path.display(),
            info.source_set
        );
    }

    // Dead code should include fixture files (no analysis=false filtering)
    let output = commands::run_dead_code(
        tmp.path(),
        "files",
        &OutputFormat::Json,
        true,
        false,
        None,
        None,
        None,
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let dead_files = json["dead_files"].as_array().unwrap();
    let dead_paths: Vec<&str> = dead_files
        .iter()
        .filter_map(|f| f["path"].as_str())
        .collect();

    // Without scope config, fixture files should appear in dead code
    // (mock_db.ts is never imported by anything)
    let has_fixture = dead_paths.iter().any(|p| p.contains("mock_db"));
    assert!(
        has_fixture,
        "Without scope config, fixture file should appear in dead code, got: {:?}",
        dead_paths
    );
}

// =============================================================================
// --source-set filter: restrict analysis to a specific source set
// =============================================================================

#[test]
fn test_scope_source_set_filter() {
    let tmp = setup_scope_project();
    index_project(tmp.path());

    // Filter to production scope only
    let output = commands::run_dead_code(
        tmp.path(),
        "files",
        &OutputFormat::Json,
        true,
        false,
        None,
        None,
        Some("production"),
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let dead_files = json["dead_files"].as_array().unwrap();

    // Only production files should appear
    for dead in dead_files {
        let path = dead["path"].as_str().unwrap();
        assert!(
            path.contains("src/"),
            "With --source-set production, only src/ files should appear, got: {}",
            path
        );
    }
}

// =============================================================================
// --source-set filter with invalid scope name
// =============================================================================

#[test]
fn test_scope_source_set_filter_invalid() {
    let tmp = setup_scope_project();
    index_project(tmp.path());

    let result = commands::run_dead_code(
        tmp.path(),
        "files",
        &OutputFormat::Json,
        true,
        false,
        None,
        None,
        Some("nonexistent"),
    );

    assert!(result.is_err(), "Should fail with an unknown scope name");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("nonexistent"),
        "Error should mention the unknown scope name, got: {}",
        err
    );
}
