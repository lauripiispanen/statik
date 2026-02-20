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

    // We have: User, Role, Auditable, AuditableUser, UserSummary, UserService,
    //          NotificationService, ReportService, StringUtils, UserController,
    //          AdminController, UserRepository, UnusedHelper, CycleA, CycleB,
    //          Application, UserVerification
    assert_eq!(
        result.files_indexed, 17,
        "Expected 17 Java files, got {}",
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

    // User.java is imported by: UserService, UserController, AdminController,
    // UserRepository, AuditableUser (extends), NotificationService, Application (via UserController)
    let imported_by = json["imported_by"].as_array().unwrap();
    assert!(
        imported_by.len() >= 5,
        "User.java should be imported by at least 5 files, got {}",
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

    // User.java is imported by UserService, UserController, AdminController,
    // UserRepository, AuditableUser, NotificationService
    // And UserService is imported by UserController, NotificationService, Application -> transitive
    assert!(
        total_affected >= 6,
        "Changing User.java should affect at least 6 files, got {}",
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
        total_files, 17,
        "Summary should report 17 Java files, got {}",
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

// =============================================================================
// STATIC IMPORTS - verify static import resolution
// =============================================================================

#[test]
fn test_java_static_import_resolves() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    // AuditableUser uses `import static com.example.util.StringUtils.sanitize`
    let output = commands::run_deps(
        tmp.path(),
        "src/main/java/com/example/model/AuditableUser.java",
        false,
        "out",
        None,
        &OutputFormat::Json,
        true,
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let imports = json["imports"].as_array().unwrap();
    let import_paths: Vec<&str> = imports
        .iter()
        .filter_map(|i| i["path"].as_str())
        .collect();

    // AuditableUser has:
    // - Static import: StringUtils.sanitize -> StringUtils.java
    // - Same-package type refs: extends User -> User.java, implements Auditable -> Auditable.java
    assert_eq!(
        imports.len(),
        3,
        "AuditableUser should have 3 resolved imports (User, Auditable, StringUtils), got {:?}",
        import_paths
    );
    assert!(
        import_paths.iter().any(|p| p.contains("StringUtils.java")),
        "Static import should resolve to StringUtils.java, got {:?}",
        import_paths
    );
    assert!(
        import_paths.iter().any(|p| p.contains("User.java")),
        "Same-package type ref should resolve to User.java, got {:?}",
        import_paths
    );
    assert!(
        import_paths.iter().any(|p| p.contains("Auditable.java")),
        "Same-package type ref should resolve to Auditable.java, got {:?}",
        import_paths
    );
}

// =============================================================================
// CROSS-PACKAGE IMPORTS - verify explicit import resolution
// =============================================================================

#[test]
fn test_java_deps_cross_package_imports() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    // NotificationService imports from model, util, and db packages
    let output = commands::run_deps(
        tmp.path(),
        "src/main/java/com/example/service/NotificationService.java",
        false,
        "out",
        None,
        &OutputFormat::Json,
        true,
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let imports = json["imports"].as_array().unwrap();
    let import_paths: Vec<&str> = imports
        .iter()
        .filter_map(|i| i["path"].as_str())
        .collect();

    // NotificationService explicitly imports from 3 different packages
    assert!(
        import_paths.iter().any(|p| p.contains("model/User.java")),
        "Should import model/User.java, got {:?}",
        import_paths
    );
    assert!(
        import_paths.iter().any(|p| p.contains("model/Auditable.java")),
        "Should import model/Auditable.java, got {:?}",
        import_paths
    );
    assert!(
        import_paths.iter().any(|p| p.contains("db/UserRepository.java")),
        "Should import db/UserRepository.java, got {:?}",
        import_paths
    );
}

// =============================================================================
// EXPORTS - verify interface exports
// =============================================================================

#[test]
fn test_java_exports_public_interface() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    let output = commands::run_exports(
        tmp.path(),
        "src/main/java/com/example/model/Auditable.java",
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
        names.contains(&"Auditable"),
        "Should export Auditable interface, got {:?}",
        names
    );
}

// =============================================================================
// DEAD CODE - entry points should not be detected as dead
// =============================================================================

#[test]
fn test_java_dead_code_excludes_entry_point() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    let output =
        commands::run_dead_code(tmp.path(), "files", &OutputFormat::Json, true).unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let dead_files = json["dead_files"].as_array().unwrap();
    let dead_paths: Vec<&str> = dead_files
        .iter()
        .filter_map(|f| f["path"].as_str())
        .collect();

    // Application.java has "Application" in its name, which is a Java entry point
    assert!(
        !dead_paths.iter().any(|p| p.contains("Application.java")),
        "Application.java should NOT be dead (it's an entry point), dead files: {:?}",
        dead_paths
    );

    // UnusedHelper should still be dead
    assert!(
        dead_paths.iter().any(|p| p.contains("UnusedHelper.java")),
        "UnusedHelper.java should still be detected as dead, got {:?}",
        dead_paths
    );
}

// =============================================================================
// SUMMARY - verify Java language breakdown
// =============================================================================

#[test]
fn test_java_summary_language_breakdown() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    let output = commands::run_summary(tmp.path(), &OutputFormat::Json, true).unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    // Verify language breakdown exists and shows Java
    if let Some(by_language) = json["files"]["by_language"].as_object() {
        let java_count = by_language
            .get("Java")
            .or_else(|| by_language.get("java"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        assert_eq!(
            java_count, 17,
            "Should report 17 Java files in language breakdown, got {}",
            java_count
        );
    }

    // Verify dead code count includes UnusedHelper
    let dead_count = json["dead_code"]["dead_files"].as_u64().unwrap_or(0);
    assert!(
        dead_count >= 1,
        "Should detect at least 1 dead file in summary, got {}",
        dead_count
    );
}

// =============================================================================
// IMPACT - verify transitive impact through new files
// =============================================================================

#[test]
fn test_java_impact_stringutils_transitive() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    // StringUtils is imported by UserService and AuditableUser (static import)
    let output = commands::run_impact(
        tmp.path(),
        "src/main/java/com/example/util/StringUtils.java",
        None,
        &OutputFormat::Json,
        true,
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let total_affected = json["summary"]["total_affected"].as_u64().unwrap();

    // StringUtils -> UserService -> UserController, Application, NotificationService
    // StringUtils -> AuditableUser
    // StringUtils -> NotificationService -> (nothing additional)
    assert!(
        total_affected >= 4,
        "Changing StringUtils.java should affect at least 4 files transitively, got {}",
        total_affected
    );

    // Verify specific affected files
    let affected = json["affected"].as_array().unwrap();
    let affected_paths: Vec<&str> = affected
        .iter()
        .filter_map(|a| a["path"].as_str())
        .collect();
    assert!(
        affected_paths.iter().any(|p| p.contains("UserService.java")),
        "UserService should be affected, got {:?}",
        affected_paths
    );
    assert!(
        affected_paths.iter().any(|p| p.contains("AuditableUser.java")),
        "AuditableUser should be affected (static import), got {:?}",
        affected_paths
    );
}

// =============================================================================
// DEPS - verify NotificationService has many imports (fan-out)
// =============================================================================

#[test]
fn test_java_deps_notification_service_fan_out() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    let output = commands::run_deps(
        tmp.path(),
        "src/main/java/com/example/service/NotificationService.java",
        false,
        "out",
        None,
        &OutputFormat::Json,
        true,
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let imports = json["imports"].as_array().unwrap();

    // NotificationService imports: User, Auditable, AuditableUser, Role, StringUtils, UserRepository
    assert!(
        imports.len() >= 5,
        "NotificationService should import at least 5 files, got {}",
        imports.len()
    );

    let import_paths: Vec<&str> = imports
        .iter()
        .filter_map(|i| i["path"].as_str())
        .collect();
    assert!(
        import_paths.iter().any(|p| p.contains("UserRepository.java")),
        "Should import UserRepository.java, got {:?}",
        import_paths
    );
}

// =============================================================================
// LINT - layer rule on Java
// =============================================================================

#[test]
fn test_java_lint_layer_rule() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    // Layer rule: layers are ordered top-to-bottom. Lower layers (later index)
    // must not import from higher layers (earlier index).
    // Define layers so data is at top, controller at bottom:
    //   data (index 0) -> service (index 1) -> controller (index 2)
    // AdminController (controller, index 2) imports UserRepository (data, index 0),
    // which is bottom-up, and thus a violation.
    std::fs::write(
        tmp.path().join(".statik/rules.toml"),
        r#"[[rules]]
id = "clean-layers"
severity = "error"
description = "Dependencies must flow top-down through layers"

[rules.layer]
layers = [
  { name = "data", patterns = ["src/main/java/com/example/db/**"] },
  { name = "service", patterns = ["src/main/java/com/example/service/**"] },
  { name = "controller", patterns = ["src/main/java/com/example/controller/**"] },
]
"#,
    )
    .unwrap();

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

    // AdminController (controller layer, index 2) imports UserRepository (data layer, index 0)
    // This is a bottom-up import, violating the layer rule
    let layer_violations: Vec<_> = violations
        .iter()
        .filter(|v| v["rule_id"].as_str() == Some("clean-layers"))
        .collect();

    assert!(
        !layer_violations.is_empty(),
        "Should detect layer violation: controller -> data (bottom-up), violations: {:?}",
        violations
    );
    assert!(has_errors, "Layer violation should be an error");
}

// =============================================================================
// LINT - fan-limit rule on Java
// =============================================================================

#[test]
fn test_java_lint_fan_limit_rule() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    // NotificationService has 6 imports - set limit to 4
    std::fs::write(
        tmp.path().join(".statik/rules.toml"),
        r#"[[rules]]
id = "max-deps"
severity = "error"
description = "Files should not have too many dependencies"

[rules.fan_limit]
pattern = ["src/main/java/com/example/service/**"]
max_fan_out = 4
"#,
    )
    .unwrap();

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

    let fan_violations: Vec<_> = violations
        .iter()
        .filter(|v| v["rule_id"].as_str() == Some("max-deps"))
        .collect();

    assert!(
        !fan_violations.is_empty(),
        "Should detect fan-limit violation for NotificationService, violations: {:?}",
        violations
    );
    assert!(has_errors, "Fan-limit violation should be an error");

    // Verify the violation source is NotificationService
    let source = fan_violations[0]["source_file"].as_str().unwrap();
    assert!(
        source.contains("NotificationService"),
        "Violation should be on NotificationService, got {}",
        source
    );
}

// =============================================================================
// LINT - containment rule on Java
// =============================================================================

#[test]
fn test_java_lint_containment_rule() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    // Enforce that model package is only accessed through User.java as public API
    // AuditableUser.java and Auditable.java are internal
    std::fs::write(
        tmp.path().join(".statik/rules.toml"),
        r#"[[rules]]
id = "model-encapsulation"
severity = "error"
description = "Model package must be accessed through User.java only"

[rules.containment]
module = ["src/main/java/com/example/model/**"]
public_api = ["src/main/java/com/example/model/User.java", "src/main/java/com/example/model/Role.java"]
"#,
    )
    .unwrap();

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

    let containment_violations: Vec<_> = violations
        .iter()
        .filter(|v| v["rule_id"].as_str() == Some("model-encapsulation"))
        .collect();

    // NotificationService imports Auditable.java and AuditableUser.java which are not in public_api
    assert!(
        !containment_violations.is_empty(),
        "Should detect containment violation for imports of non-public-api model files, violations: {:?}",
        violations
    );
    assert!(has_errors, "Containment violation should be an error");
}

// =============================================================================
// LINT - multiple rule types combined on Java
// =============================================================================

#[test]
fn test_java_lint_multiple_rule_types() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    std::fs::write(
        tmp.path().join(".statik/rules.toml"),
        r#"[[rules]]
id = "no-controller-to-db"
severity = "error"
description = "Controllers must not import directly from the database layer"

[rules.boundary]
from = ["src/main/java/com/example/controller/**"]
deny = ["src/main/java/com/example/db/**"]

[[rules]]
id = "clean-layers"
severity = "warning"
description = "Dependencies must flow top-down through layers"

[rules.layer]
layers = [
  { name = "controller", patterns = ["src/main/java/com/example/controller/**"] },
  { name = "service", patterns = ["src/main/java/com/example/service/**"] },
  { name = "data", patterns = ["src/main/java/com/example/db/**"] },
]

[[rules]]
id = "fan-check"
severity = "info"
description = "Fan-out check"

[rules.fan_limit]
pattern = ["src/main/java/com/example/**"]
max_fan_out = 100
"#,
    )
    .unwrap();

    let (output, _) = commands::run_lint(
        tmp.path(),
        None,
        None,
        "info",
        &OutputFormat::Json,
        true,
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    // All 3 rules should be evaluated
    let rules_evaluated = json["summary"]["rules_evaluated"].as_u64().unwrap();
    assert_eq!(
        rules_evaluated, 3,
        "Should evaluate all 3 rules, got {}",
        rules_evaluated
    );

    // Should have at least the boundary violation from AdminController -> db
    let violations = json["violations"].as_array().unwrap();
    assert!(
        violations.iter().any(|v| v["rule_id"] == "no-controller-to-db"),
        "Should detect boundary violation, got {:?}",
        violations
    );
}

// =============================================================================
// SAME-PACKAGE IMPLICIT DEPENDENCIES
// =============================================================================

#[test]
fn test_java_same_package_deps() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    // UserSummary uses User and Role from the same package without any imports
    let output = commands::run_deps(
        tmp.path(),
        "src/main/java/com/example/model/UserSummary.java",
        false,
        "out",
        None,
        &OutputFormat::Json,
        true,
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let imports = json["imports"].as_array().unwrap();
    let import_paths: Vec<&str> = imports
        .iter()
        .filter_map(|i| i["path"].as_str())
        .collect();

    assert!(
        import_paths.iter().any(|p| p.contains("User.java")),
        "UserSummary should depend on User.java via same-package type ref, got {:?}",
        import_paths
    );
    assert!(
        import_paths.iter().any(|p| p.contains("Role.java")),
        "UserSummary should depend on Role.java via same-package type ref, got {:?}",
        import_paths
    );
}

#[test]
fn test_java_same_package_dead_code_not_false_positive() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    let output =
        commands::run_dead_code(tmp.path(), "files", &OutputFormat::Json, true).unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let dead_files = json["dead_files"].as_array().unwrap();
    let dead_paths: Vec<&str> = dead_files
        .iter()
        .filter_map(|f| f["path"].as_str())
        .collect();

    // UserSummary uses User and Role via same-package refs; they should not be dead
    assert!(
        !dead_paths.iter().any(|p| p.ends_with("model/User.java")),
        "User.java should not be dead (used via same-package ref), dead: {:?}",
        dead_paths
    );
    assert!(
        !dead_paths.iter().any(|p| p.ends_with("model/Role.java")),
        "Role.java should not be dead (used via same-package ref), dead: {:?}",
        dead_paths
    );
}

// =============================================================================
// WILDCARD IMPORT RESOLUTION
// =============================================================================

#[test]
fn test_java_wildcard_import_creates_edges() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    // ReportService has `import com.example.model.*`
    let output = commands::run_deps(
        tmp.path(),
        "src/main/java/com/example/service/ReportService.java",
        false,
        "out",
        None,
        &OutputFormat::Json,
        true,
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let imports = json["imports"].as_array().unwrap();
    let import_paths: Vec<&str> = imports
        .iter()
        .filter_map(|i| i["path"].as_str())
        .collect();

    // Wildcard import should create edges to all files in com.example.model
    assert!(
        import_paths.iter().any(|p| p.contains("model/User.java")),
        "Wildcard import should include User.java, got {:?}",
        import_paths
    );
    assert!(
        import_paths.iter().any(|p| p.contains("model/Role.java")),
        "Wildcard import should include Role.java, got {:?}",
        import_paths
    );
    assert!(
        import_paths.iter().any(|p| p.contains("model/Auditable.java")),
        "Wildcard import should include Auditable.java, got {:?}",
        import_paths
    );
    assert!(
        import_paths.iter().any(|p| p.contains("model/AuditableUser.java")),
        "Wildcard import should include AuditableUser.java, got {:?}",
        import_paths
    );
    assert!(
        import_paths.iter().any(|p| p.contains("model/UserSummary.java")),
        "Wildcard import should include UserSummary.java, got {:?}",
        import_paths
    );
}

// =============================================================================
// ANNOTATION-BASED ENTRY POINTS
// =============================================================================

#[test]
fn test_java_annotation_entry_point_spring() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    let output =
        commands::run_dead_code(tmp.path(), "files", &OutputFormat::Json, true).unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let dead_files = json["dead_files"].as_array().unwrap();
    let dead_paths: Vec<&str> = dead_files
        .iter()
        .filter_map(|f| f["path"].as_str())
        .collect();

    // Application.java has @SpringBootApplication -> entry point
    assert!(
        !dead_paths.iter().any(|p| p.contains("Application.java")),
        "Application.java should not be dead (@SpringBootApplication entry point), dead: {:?}",
        dead_paths
    );
}

#[test]
fn test_java_annotation_entry_point_test() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    let output =
        commands::run_dead_code(tmp.path(), "files", &OutputFormat::Json, true).unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let dead_files = json["dead_files"].as_array().unwrap();
    let dead_paths: Vec<&str> = dead_files
        .iter()
        .filter_map(|f| f["path"].as_str())
        .collect();

    // UserVerification has @Test annotations -> entry point despite non-standard name
    assert!(
        !dead_paths
            .iter()
            .any(|p| p.contains("UserVerification.java")),
        "UserVerification.java should not be dead (@Test entry point), dead: {:?}",
        dead_paths
    );
}

// =============================================================================
// INNER CLASS EXPORTS
// =============================================================================

#[test]
fn test_java_inner_class_exported() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    let output = commands::run_exports(
        tmp.path(),
        "src/main/java/com/example/service/NotificationService.java",
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
        names.contains(&"NotificationService"),
        "Should export NotificationService, got {:?}",
        names
    );
    assert!(
        names.contains(&"Config"),
        "Should export public static inner class Config, got {:?}",
        names
    );
}

// =============================================================================
// SAME-PACKAGE CYCLE DETECTION
// =============================================================================

#[test]
fn test_java_same_package_cycles_still_detected() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    let output = commands::run_cycles(tmp.path(), &OutputFormat::Json, true).unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let cycles = json["cycles"].as_array().unwrap();

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

    // CycleA and CycleB should still be detected as a cycle (explicit imports)
    assert!(
        all_cycle_paths.iter().any(|p| p.contains("CycleA.java")),
        "CycleA.java should be in a cycle, got {:?}",
        all_cycle_paths
    );
    assert!(
        all_cycle_paths.iter().any(|p| p.contains("CycleB.java")),
        "CycleB.java should be in a cycle, got {:?}",
        all_cycle_paths
    );
}

// =============================================================================
// CUSTOM ENTRY POINTS - user-configured patterns and annotations
// =============================================================================

#[test]
fn test_java_custom_entry_point_pattern() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    // Without config, UnusedHelper is dead
    let output =
        commands::run_dead_code(tmp.path(), "files", &OutputFormat::Json, true).unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let dead_paths: Vec<&str> = json["dead_files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["path"].as_str())
        .collect();
    assert!(
        dead_paths.iter().any(|p| p.contains("UnusedHelper.java")),
        "UnusedHelper should be dead without custom config"
    );

    // Add custom entry point pattern that matches UnusedHelper
    std::fs::write(
        tmp.path().join(".statik/rules.toml"),
        r#"
rules = []

[entry_points]
patterns = ["**/orphan/**"]
"#,
    )
    .unwrap();

    // Now UnusedHelper should NOT be dead
    let output =
        commands::run_dead_code(tmp.path(), "files", &OutputFormat::Json, true).unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let dead_paths: Vec<&str> = json["dead_files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["path"].as_str())
        .collect();
    assert!(
        !dead_paths.iter().any(|p| p.contains("UnusedHelper.java")),
        "UnusedHelper should NOT be dead with custom entry point pattern, dead: {:?}",
        dead_paths
    );
}

#[test]
fn test_java_custom_entry_point_annotation() {
    let tmp = setup_java_project();

    // Create a file with a custom annotation that built-in heuristics don't recognize
    let custom_dir = tmp
        .path()
        .join("src/main/java/com/example/batch");
    std::fs::create_dir_all(&custom_dir).unwrap();
    std::fs::write(
        custom_dir.join("BatchJob.java"),
        r#"
package com.example.batch;

@Scheduled
public class BatchJob {
    public void run() {}
}
"#,
    )
    .unwrap();

    index_java_project(tmp.path());

    // Without config, BatchJob is dead (Scheduled is not a built-in entry annotation)
    let output =
        commands::run_dead_code(tmp.path(), "files", &OutputFormat::Json, true).unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let dead_paths: Vec<&str> = json["dead_files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["path"].as_str())
        .collect();
    assert!(
        dead_paths.iter().any(|p| p.contains("BatchJob.java")),
        "BatchJob should be dead without custom annotation config, dead: {:?}",
        dead_paths
    );

    // Add custom annotation entry point
    std::fs::write(
        tmp.path().join(".statik/rules.toml"),
        r#"
rules = []

[entry_points]
annotations = ["Scheduled"]
"#,
    )
    .unwrap();

    // Now BatchJob should NOT be dead
    let output =
        commands::run_dead_code(tmp.path(), "files", &OutputFormat::Json, true).unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let dead_paths: Vec<&str> = json["dead_files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["path"].as_str())
        .collect();
    assert!(
        !dead_paths.iter().any(|p| p.contains("BatchJob.java")),
        "BatchJob should NOT be dead with custom annotation entry point, dead: {:?}",
        dead_paths
    );
}

#[test]
fn test_java_default_entry_points_unchanged_without_config() {
    let tmp = setup_java_project();
    index_java_project(tmp.path());

    // Remove any existing config to test defaults
    let _ = std::fs::remove_file(tmp.path().join(".statik/rules.toml"));
    let _ = std::fs::remove_file(tmp.path().join("statik.toml"));

    let output =
        commands::run_dead_code(tmp.path(), "files", &OutputFormat::Json, true).unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let dead_paths: Vec<&str> = json["dead_files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["path"].as_str())
        .collect();

    // Built-in heuristics should still work: Application.java is an entry point
    assert!(
        !dead_paths.iter().any(|p| p.contains("Application.java")),
        "Application.java should still be entry point without config, dead: {:?}",
        dead_paths
    );
    // UnusedHelper should still be dead
    assert!(
        dead_paths.iter().any(|p| p.contains("UnusedHelper.java")),
        "UnusedHelper should still be dead without config, dead: {:?}",
        dead_paths
    );
}
