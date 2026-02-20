use std::path::{Path, PathBuf};

use statik::cli::commands;
use statik::cli::OutputFormat;

fn fixture_source() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/basic_project")
}

/// Copy the fixture project to a temporary directory so tests don't conflict.
fn setup_project() -> tempfile::TempDir {
    let src = fixture_source();
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
            // Skip .statik directory (we don't want to copy index.db)
            let name = entry.file_name();
            if name == ".statik" {
                // Only copy config files, not the index
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

/// Index a project and return the project path.
fn index_project(project_path: &Path) {
    let config = statik::discovery::DiscoveryConfig::default();
    let result = statik::cli::index::run_index(project_path, &config).unwrap();
    assert!(
        result.files_indexed > 0,
        "Should index at least one file, got {}",
        result.files_indexed
    );
}

#[test]
fn test_index_discovers_all_files() {
    let tmp = setup_project();
    let config = statik::discovery::DiscoveryConfig::default();
    let result = statik::cli::index::run_index(tmp.path(), &config).unwrap();

    // We have: index.ts, services/userService.ts, models/user.ts, utils/format.ts,
    // orphan.ts, ui/UserForm.tsx, db/connection.ts, cycle/a.ts, cycle/b.ts
    assert!(
        result.files_indexed >= 9,
        "Expected at least 9 files, got {}",
        result.files_indexed
    );
    assert!(
        result.symbols_extracted > 0,
        "Should extract symbols from the project"
    );
}

#[test]
fn test_deps_command_json() {
    let tmp = setup_project();
    index_project(tmp.path());

    let output = commands::run_deps(
        tmp.path(),
        "src/services/userService.ts",
        false,
        "both",
        None,
        &OutputFormat::Json,
        true,
        false,
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    // userService imports user.ts and format.ts
    let imports = json["imports"].as_array().unwrap();
    assert!(
        imports.len() >= 2,
        "userService should import at least 2 files, got {}",
        imports.len()
    );

    let import_paths: Vec<&str> = imports
        .iter()
        .filter_map(|i| i["path"].as_str())
        .collect();
    assert!(
        import_paths.iter().any(|p| p.contains("user.ts")),
        "Should import models/user.ts, got {:?}",
        import_paths
    );
    assert!(
        import_paths.iter().any(|p| p.contains("format.ts")),
        "Should import utils/format.ts, got {:?}",
        import_paths
    );

    // userService is imported by index.ts
    let imported_by = json["imported_by"].as_array().unwrap();
    assert!(
        !imported_by.is_empty(),
        "userService should be imported by at least one file"
    );
}

#[test]
fn test_deps_command_text() {
    let tmp = setup_project();
    index_project(tmp.path());

    let output = commands::run_deps(
        tmp.path(),
        "src/services/userService.ts",
        false,
        "both",
        None,
        &OutputFormat::Text,
        true,
        false,
    )
    .unwrap();

    assert!(
        output.contains("Dependencies for"),
        "Text output should have header"
    );
    assert!(
        output.contains("Confidence:"),
        "Text output should show confidence"
    );
}

#[test]
fn test_dead_code_detects_orphan() {
    let tmp = setup_project();
    index_project(tmp.path());

    let output =
        commands::run_dead_code(tmp.path(), "both", &OutputFormat::Json, true, false).unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    let dead_files = json["dead_files"].as_array().unwrap();
    let dead_paths: Vec<&str> = dead_files
        .iter()
        .filter_map(|f| f["path"].as_str())
        .collect();

    assert!(
        dead_paths.iter().any(|p| p.contains("orphan.ts")),
        "orphan.ts should be detected as dead, dead files: {:?}",
        dead_paths
    );
}

#[test]
fn test_dead_code_text_output() {
    let tmp = setup_project();
    index_project(tmp.path());

    let output =
        commands::run_dead_code(tmp.path(), "both", &OutputFormat::Text, true, false).unwrap();

    assert!(
        output.contains("orphan.ts"),
        "Text output should mention orphan.ts"
    );
    assert!(
        output.contains("Summary:"),
        "Text output should have summary"
    );
}

#[test]
fn test_cycles_detects_circular_deps() {
    let tmp = setup_project();
    index_project(tmp.path());

    let output = commands::run_cycles(tmp.path(), &OutputFormat::Json, true, false).unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let cycles = json["cycles"].as_array().unwrap();

    assert!(
        !cycles.is_empty(),
        "Should detect the a.ts <-> b.ts cycle"
    );

    // Check that the cycle involves cycle/a.ts and cycle/b.ts
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
        all_cycle_paths.iter().any(|p| p.contains("cycle")),
        "Cycle should involve files in cycle/ directory, got {:?}",
        all_cycle_paths
    );
}

#[test]
fn test_cycles_text_output() {
    let tmp = setup_project();
    index_project(tmp.path());

    let output = commands::run_cycles(tmp.path(), &OutputFormat::Text, true, false).unwrap();

    assert!(
        output.contains("Circular dependencies"),
        "Text output should mention circular dependencies"
    );
    assert!(
        output.contains("(cycle)"),
        "Text output should show the cycle closure"
    );
}

#[test]
fn test_impact_analysis() {
    let tmp = setup_project();
    index_project(tmp.path());

    let output = commands::run_impact(
        tmp.path(),
        "src/utils/format.ts",
        None,
        &OutputFormat::Json,
        true,
        false,
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let total_affected = json["summary"]["total_affected"].as_u64().unwrap();

    // format.ts is imported by userService.ts and index.ts
    assert!(
        total_affected >= 2,
        "Changing format.ts should affect at least 2 files, got {}",
        total_affected
    );
}

#[test]
fn test_exports_command() {
    let tmp = setup_project();
    index_project(tmp.path());

    let output = commands::run_exports(
        tmp.path(),
        "src/services/userService.ts",
        &OutputFormat::Json,
        true,
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let exports = json["exports"].as_array().unwrap();

    assert!(!exports.is_empty(), "userService.ts should have exports");

    let names: Vec<&str> = exports
        .iter()
        .filter_map(|e| e["name"].as_str())
        .collect();

    assert!(
        names.contains(&"UserService"),
        "Should export UserService, got {:?}",
        names
    );
}

#[test]
fn test_exports_text_output() {
    let tmp = setup_project();
    index_project(tmp.path());

    let output = commands::run_exports(
        tmp.path(),
        "src/services/userService.ts",
        &OutputFormat::Text,
        true,
    )
    .unwrap();

    assert!(
        output.contains("Exports for"),
        "Text output should have header"
    );
    assert!(
        output.contains("Name"),
        "Text output should have table headers"
    );
}

#[test]
fn test_summary_command() {
    let tmp = setup_project();
    index_project(tmp.path());

    let output = commands::run_summary(tmp.path(), &OutputFormat::Json, true).unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let total_files = json["files"]["total"].as_u64().unwrap();

    assert!(
        total_files >= 9,
        "Summary should report at least 9 files, got {}",
        total_files
    );

    let cycle_count = json["cycles"]["cycle_count"].as_u64().unwrap();
    assert!(cycle_count >= 1, "Should detect at least 1 cycle");
}

#[test]
fn test_summary_text_output() {
    let tmp = setup_project();
    index_project(tmp.path());

    let output = commands::run_summary(tmp.path(), &OutputFormat::Text, true).unwrap();

    assert!(
        output.contains("Project Summary"),
        "Text output should have header"
    );
    assert!(
        output.contains("Files:"),
        "Text output should show file count"
    );
}

#[test]
fn test_lint_detects_boundary_violation() {
    let tmp = setup_project();
    index_project(tmp.path());

    let (output, has_errors) = commands::run_lint(
        tmp.path(),
        None,
        None,
        "info",
        &OutputFormat::Json,
        true,
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let violations = json["violations"].as_array().unwrap();

    assert!(
        !violations.is_empty(),
        "Should detect UI -> DB boundary violation"
    );
    assert!(
        has_errors,
        "Should have errors (UI -> DB is severity: error)"
    );

    let violation = &violations[0];
    assert_eq!(violation["rule_id"].as_str().unwrap(), "no-ui-to-db");

    let source = violation["source_file"].as_str().unwrap();
    assert!(
        source.contains("UserForm"),
        "Violation source should be UserForm.tsx, got {}",
        source
    );
}

#[test]
fn test_lint_text_output() {
    let tmp = setup_project();
    index_project(tmp.path());

    let (output, _) = commands::run_lint(
        tmp.path(),
        None,
        None,
        "info",
        &OutputFormat::Text,
        true,
    )
    .unwrap();

    assert!(
        output.contains("error[no-ui-to-db]"),
        "Text output should show violation with rule ID, got:\n{}",
        output
    );
    assert!(
        output.contains("errors"),
        "Text output should have summary line"
    );
}

#[test]
fn test_lint_severity_threshold_filters() {
    let tmp = setup_project();
    index_project(tmp.path());

    let (output, has_errors) = commands::run_lint(
        tmp.path(),
        None,
        None,
        "warning",
        &OutputFormat::Json,
        true,
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let violations = json["violations"].as_array().unwrap();
    assert!(
        !violations.is_empty(),
        "Error-severity violations should pass warning threshold"
    );
    assert!(has_errors);
}
