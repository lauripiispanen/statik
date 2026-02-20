use std::path::{Path, PathBuf};

use statik::cli::commands;
use statik::cli::OutputFormat;

fn java_fixture_source() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/java_project")
}

/// Copy the fixture project to a temporary directory so tests don't conflict.
fn setup_java_project() -> tempfile::TempDir {
    let src = java_fixture_source();
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

fn index_java_project(project_path: &Path) {
    let config = statik::discovery::DiscoveryConfig::default();
    let result = statik::cli::index::run_index(project_path, &config).unwrap();
    assert!(
        result.files_indexed > 0,
        "Should index at least one Java file, got {}",
        result.files_indexed
    );
}

// =============================================================================
// INDEX - verify Java files are discovered and indexed
// =============================================================================

#[test]
fn test_java_index_discovers_all_files() {
    let tmp = setup_java_project();
    let config = statik::discovery::DiscoveryConfig::default();
    let result = statik::cli::index::run_index(tmp.path(), &config).unwrap();

    // We have: User, Role, UserService, StringUtils, UserController,
    //          AdminController, UserRepository, UnusedHelper, CycleA, CycleB
    assert_eq!(
        result.files_indexed, 10,
        "Expected 10 Java files, got {}",
        result.files_indexed
    );
    assert!(
        result.symbols_extracted > 0,
        "Should extract symbols from Java files"
    );
}

// =============================================================================
// DEPS - verify dependency tracking works for Java imports
// =============================================================================

#[test]
fn test_java_deps_shows_imports() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    let output = commands::run_deps(
        tmp.path(),
        "src/main/java/com/example/service/UserService.java",
        false,
        "both",
        None,
        &OutputFormat::Json,
        true,
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    // UserService imports User, Role, and StringUtils
    let imports = json["imports"].as_array().unwrap();
    assert_eq!(
        imports.len(),
        3,
        "UserService should import 3 files (User, Role, StringUtils), got {}",
        imports.len()
    );

    let import_paths: Vec<&str> = imports
        .iter()
        .filter_map(|i| i["path"].as_str())
        .collect();
    assert!(
        import_paths.iter().any(|p| p.contains("User.java")),
        "Should import model/User.java, got {:?}",
        import_paths
    );
    assert!(
        import_paths.iter().any(|p| p.contains("Role.java")),
        "Should import model/Role.java, got {:?}",
        import_paths
    );
    assert!(
        import_paths.iter().any(|p| p.contains("StringUtils.java")),
        "Should import util/StringUtils.java, got {:?}",
        import_paths
    );
}

#[test]
fn test_java_deps_shows_importers() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    let output = commands::run_deps(
        tmp.path(),
        "src/main/java/com/example/model/User.java",
        false,
        "both",
        None,
        &OutputFormat::Json,
        true,
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    // User.java is imported by: UserService, UserController, AdminController, UserRepository
    let imported_by = json["imported_by"].as_array().unwrap();
    assert!(
        imported_by.len() >= 4,
        "User.java should be imported by at least 4 files, got {}",
        imported_by.len()
    );

    let importer_paths: Vec<&str> = imported_by
        .iter()
        .filter_map(|i| i["path"].as_str())
        .collect();
    assert!(
        importer_paths.iter().any(|p| p.contains("UserService.java")),
        "Should be imported by UserService, got {:?}",
        importer_paths
    );
    assert!(
        importer_paths
            .iter()
            .any(|p| p.contains("UserController.java")),
        "Should be imported by UserController, got {:?}",
        importer_paths
    );
}

#[test]
fn test_java_deps_text_output() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    let output = commands::run_deps(
        tmp.path(),
        "src/main/java/com/example/service/UserService.java",
        false,
        "both",
        None,
        &OutputFormat::Text,
        true,
    )
    .unwrap();

    assert!(
        output.contains("Dependencies for"),
        "Text output should have header"
    );
    assert!(
        output.contains("User.java") || output.contains("model"),
        "Text output should show imported files: {}",
        output
    );
}

// =============================================================================
// EXPORTS - verify Java public types are tracked as exports
// =============================================================================

#[test]
fn test_java_exports_public_class() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    let output = commands::run_exports(
        tmp.path(),
        "src/main/java/com/example/model/User.java",
        &OutputFormat::Json,
        true,
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let exports = json["exports"].as_array().unwrap();

    assert!(!exports.is_empty(), "User.java should have exports");

    let names: Vec<&str> = exports
        .iter()
        .filter_map(|e| e["name"].as_str())
        .collect();
    assert!(
        names.contains(&"User"),
        "Should export User class, got {:?}",
        names
    );
}

#[test]
fn test_java_exports_public_enum() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    let output = commands::run_exports(
        tmp.path(),
        "src/main/java/com/example/model/Role.java",
        &OutputFormat::Json,
        true,
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let exports = json["exports"].as_array().unwrap();

    let names: Vec<&str> = exports
        .iter()
        .filter_map(|e| e["name"].as_str())
        .collect();
    assert!(
        names.contains(&"Role"),
        "Should export Role enum, got {:?}",
        names
    );
}

// =============================================================================
// DEAD CODE - verify orphan detection works for Java
// =============================================================================

#[test]
fn test_java_dead_code_detects_orphan() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    let output =
        commands::run_dead_code(tmp.path(), "both", &OutputFormat::Json, true).unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    let dead_files = json["dead_files"].as_array().unwrap();
    let dead_paths: Vec<&str> = dead_files
        .iter()
        .filter_map(|f| f["path"].as_str())
        .collect();

    assert!(
        dead_paths.iter().any(|p| p.contains("UnusedHelper.java")),
        "UnusedHelper.java should be detected as dead, dead files: {:?}",
        dead_paths
    );
}

#[test]
fn test_java_dead_code_text_output() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    let output =
        commands::run_dead_code(tmp.path(), "both", &OutputFormat::Text, true).unwrap();

    assert!(
        output.contains("UnusedHelper"),
        "Text output should mention UnusedHelper.java: {}",
        output
    );
    assert!(
        output.contains("Summary:"),
        "Text output should have summary"
    );
}

// =============================================================================
// CYCLES - verify circular dependency detection for Java
// =============================================================================

#[test]
fn test_java_cycles_detected() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    let output = commands::run_cycles(tmp.path(), &OutputFormat::Json, true).unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let cycles = json["cycles"].as_array().unwrap();

    assert!(
        !cycles.is_empty(),
        "Should detect the CycleA <-> CycleB cycle"
    );

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
        all_cycle_paths.iter().any(|p| p.contains("CycleA.java")),
        "Cycle should involve CycleA.java, got {:?}",
        all_cycle_paths
    );
    assert!(
        all_cycle_paths.iter().any(|p| p.contains("CycleB.java")),
        "Cycle should involve CycleB.java, got {:?}",
        all_cycle_paths
    );
}

#[test]
fn test_java_cycles_text_output() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    let output = commands::run_cycles(tmp.path(), &OutputFormat::Text, true).unwrap();

    assert!(
        output.contains("Circular dependencies") || output.contains("cycle"),
        "Text output should mention circular dependencies: {}",
        output
    );
}

// =============================================================================
// IMPACT - verify impact analysis works for Java
// =============================================================================

#[test]
fn test_java_impact_analysis() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    let output = commands::run_impact(
        tmp.path(),
        "src/main/java/com/example/model/User.java",
        None,
        &OutputFormat::Json,
        true,
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let total_affected = json["summary"]["total_affected"].as_u64().unwrap();

    // User.java is imported by UserService, UserController, AdminController, UserRepository
    // And UserService is imported by UserController -> transitive impact
    assert!(
        total_affected >= 4,
        "Changing User.java should affect at least 4 files, got {}",
        total_affected
    );
}

#[test]
fn test_java_impact_orphan_file() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    let output = commands::run_impact(
        tmp.path(),
        "src/main/java/com/example/orphan/UnusedHelper.java",
        None,
        &OutputFormat::Json,
        true,
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let total_affected = json["summary"]["total_affected"].as_u64().unwrap();

    assert_eq!(
        total_affected, 0,
        "Orphan file should have no impact, got {}",
        total_affected
    );
}

// =============================================================================
// SUMMARY - verify project summary includes Java files
// =============================================================================

#[test]
fn test_java_summary_command() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    let output = commands::run_summary(tmp.path(), &OutputFormat::Json, true).unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let total_files = json["files"]["total"].as_u64().unwrap();

    assert_eq!(
        total_files, 10,
        "Summary should report 10 Java files, got {}",
        total_files
    );

    let cycle_count = json["cycles"]["cycle_count"].as_u64().unwrap();
    assert!(cycle_count >= 1, "Should detect at least 1 cycle");
}

#[test]
fn test_java_summary_text_output() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    let output = commands::run_summary(tmp.path(), &OutputFormat::Text, true).unwrap();

    assert!(
        output.contains("Project Summary"),
        "Text output should have header"
    );
    assert!(
        output.contains("Java") || output.contains("java"),
        "Text output should mention Java: {}",
        output
    );
}

// =============================================================================
// LINT - verify lint rules work with Java project
// =============================================================================

#[test]
fn test_java_lint_boundary_violation() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

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

    // AdminController imports from db layer -> boundary violation
    let boundary_violations: Vec<_> = violations
        .iter()
        .filter(|v| v["rule_id"].as_str() == Some("no-controller-to-db"))
        .collect();

    assert!(
        !boundary_violations.is_empty(),
        "Should detect AdminController -> db boundary violation, violations: {:?}",
        violations
    );
    assert!(has_errors, "Boundary violation should be an error");

    let violation = &boundary_violations[0];
    let source = violation["source_file"].as_str().unwrap();
    assert!(
        source.contains("AdminController"),
        "Violation source should be AdminController, got {}",
        source
    );
}

#[test]
fn test_java_lint_text_output() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

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
        output.contains("no-controller-to-db"),
        "Text output should show rule ID: {}",
        output
    );
    assert!(
        output.contains("AdminController") || output.contains("controller"),
        "Text output should mention violating file: {}",
        output
    );
}
