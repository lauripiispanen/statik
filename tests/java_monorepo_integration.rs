use std::path::{Path, PathBuf};

use statik::cli::commands;
use statik::cli::OutputFormat;

fn java_monorepo_fixture_source() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/java_monorepo")
}

/// Copy the fixture project to a temporary directory so tests don't conflict.
fn setup_monorepo() -> tempfile::TempDir {
    let src = java_monorepo_fixture_source();
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

fn index_monorepo(project_path: &Path) {
    let config = statik::discovery::DiscoveryConfig::default();
    let result = statik::cli::index::run_index(project_path, &config, false).unwrap();
    assert!(
        result.files_indexed > 0,
        "Should index at least one Java file, got {}",
        result.files_indexed
    );
}

// =============================================================================
// INDEX - verify all 12 Java files are discovered
// =============================================================================

#[test]
fn test_monorepo_index_discovers_all_files() {
    let tmp = setup_monorepo();
    let config = statik::discovery::DiscoveryConfig::default();
    let result = statik::cli::index::run_index(tmp.path(), &config, false).unwrap();

    assert_eq!(
        result.files_indexed, 12,
        "Expected 12 Java files in monorepo, got {}",
        result.files_indexed
    );
    assert!(
        result.symbols_extracted > 0,
        "Should extract symbols from Java files"
    );
}

// =============================================================================
// CROSS-MODULE DEPS - verify imports across modules resolve
// =============================================================================

#[test]
fn test_monorepo_cross_module_deps() {
    let tmp = setup_monorepo();
    index_monorepo(tmp.path());

    // UserService (services/webapp) imports DataNode + NodeRegistry (core/platform)
    let output = commands::run_deps(
        tmp.path(),
        "services/webapp/src/main/java/com/webapp/api/service/UserService.java",
        false,
        "out",
        None,
        &OutputFormat::Json,
        true,
        false,
        None,
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let imports = json["imports"].as_array().unwrap();
    let import_paths: Vec<&str> = imports.iter().filter_map(|i| i["path"].as_str()).collect();

    assert_eq!(
        imports.len(),
        2,
        "UserService should import 2 files (DataNode, NodeRegistry), got {:?}",
        import_paths
    );
    assert!(
        import_paths.iter().any(|p| p.contains("DataNode.java")),
        "Should import DataNode.java from core module, got {:?}",
        import_paths
    );
    assert!(
        import_paths.iter().any(|p| p.contains("NodeRegistry.java")),
        "Should import NodeRegistry.java from core module, got {:?}",
        import_paths
    );
}

#[test]
fn test_monorepo_cross_module_deps_gateway() {
    let tmp = setup_monorepo();
    index_monorepo(tmp.path());

    // GatewayRouter (services/gateway, non-standard src/java layout) imports from core.http
    let output = commands::run_deps(
        tmp.path(),
        "services/gateway/src/java/com/webapp/gateway/GatewayRouter.java",
        false,
        "out",
        None,
        &OutputFormat::Json,
        true,
        false,
        None,
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let imports = json["imports"].as_array().unwrap();
    let import_paths: Vec<&str> = imports.iter().filter_map(|i| i["path"].as_str()).collect();

    assert_eq!(
        imports.len(),
        2,
        "GatewayRouter should import 2 files (RequestRouter, SessionManager), got {:?}",
        import_paths
    );
    assert!(
        import_paths
            .iter()
            .any(|p| p.contains("RequestRouter.java")),
        "Should import RequestRouter.java from core.http, got {:?}",
        import_paths
    );
    assert!(
        import_paths
            .iter()
            .any(|p| p.contains("SessionManager.java")),
        "Should import SessionManager.java from core.http, got {:?}",
        import_paths
    );
}

#[test]
fn test_monorepo_cross_module_deps_tools() {
    let tmp = setup_monorepo();
    index_monorepo(tmp.path());

    // SchemaGenerator (tools/codegen, flat src layout) imports from core.model
    let output = commands::run_deps(
        tmp.path(),
        "tools/codegen/src/com/webapp/codegen/SchemaGenerator.java",
        false,
        "out",
        None,
        &OutputFormat::Json,
        true,
        false,
        None,
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let imports = json["imports"].as_array().unwrap();
    let import_paths: Vec<&str> = imports.iter().filter_map(|i| i["path"].as_str()).collect();

    assert_eq!(
        imports.len(),
        1,
        "SchemaGenerator should import 1 file (DataNode), got {:?}",
        import_paths
    );
    assert!(
        import_paths.iter().any(|p| p.contains("DataNode.java")),
        "Should import DataNode.java from core.model, got {:?}",
        import_paths
    );
}

// =============================================================================
// DEAD CODE - only truly unused files should be flagged
// =============================================================================

#[test]
fn test_monorepo_dead_code_only_unused() {
    let tmp = setup_monorepo();
    index_monorepo(tmp.path());

    let output = commands::run_dead_code(
        tmp.path(),
        "both",
        &OutputFormat::Json,
        true,
        false,
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

    // UnusedFormatter should always be dead (nothing imports it, not an entry point)
    assert!(
        dead_paths
            .iter()
            .any(|p| p.contains("UnusedFormatter.java")),
        "UnusedFormatter.java should be detected as dead, dead files: {:?}",
        dead_paths
    );

    // Core files that are imported cross-module should NOT be dead
    assert!(
        !dead_paths.iter().any(|p| p.contains("DataNode.java")),
        "DataNode.java should NOT be dead (imported cross-module), dead: {:?}",
        dead_paths
    );
    assert!(
        !dead_paths.iter().any(|p| p.contains("NodeRegistry.java")),
        "NodeRegistry.java should NOT be dead (imported cross-module), dead: {:?}",
        dead_paths
    );

    // Entry point files should not be dead
    assert!(
        !dead_paths.iter().any(|p| p.contains("WebApplication.java")),
        "WebApplication.java should NOT be dead (it's an entry point), dead: {:?}",
        dead_paths
    );
}

// =============================================================================
// SUMMARY - verify low unresolved imports (cross-module resolution works)
// =============================================================================

#[test]
fn test_monorepo_summary_low_unresolved() {
    let tmp = setup_monorepo();
    index_monorepo(tmp.path());

    let output =
        commands::run_summary(tmp.path(), &OutputFormat::Json, true, None, false).unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let total_files = json["files"]["total"].as_u64().unwrap();
    let total_imports = json["dependencies"]["total_imports"].as_u64().unwrap();
    let unresolved = json["dependencies"]["unresolved_imports"].as_u64().unwrap();

    assert_eq!(total_files, 12, "Should have 12 Java files");

    // All cross-module imports should resolve. The only "unresolved" should be
    // external imports (java.util.*, annotations, etc.) classified as External.
    // With 16 total imports, resolved (internal) should be at least 6.
    let resolved = total_imports - unresolved;
    assert!(
        resolved >= 6,
        "At least 6 imports should resolve to project files, got {} resolved out of {} total",
        resolved,
        total_imports
    );
}

// =============================================================================
// CONFIGURED SOURCE ROOTS - explicit [java] config works
// =============================================================================

#[test]
fn test_monorepo_configured_source_roots() {
    let tmp = setup_monorepo();

    // Write a config file with explicit source roots
    std::fs::create_dir_all(tmp.path().join(".statik")).unwrap();
    std::fs::write(
        tmp.path().join(".statik/rules.toml"),
        r#"
rules = []

[java]
source_roots = [
    "core/platform/src/main/java",
    "services/webapp/src/main/java",
    "services/webapp/src/test/java",
    "services/gateway/src/java",
    "tools/codegen/src",
]
"#,
    )
    .unwrap();

    index_monorepo(tmp.path());

    // UserService should still resolve cross-module imports with explicit config
    let output = commands::run_deps(
        tmp.path(),
        "services/webapp/src/main/java/com/webapp/api/service/UserService.java",
        false,
        "out",
        None,
        &OutputFormat::Json,
        true,
        false,
        None,
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let imports = json["imports"].as_array().unwrap();
    let import_paths: Vec<&str> = imports.iter().filter_map(|i| i["path"].as_str()).collect();

    assert_eq!(
        imports.len(),
        2,
        "UserService should still import 2 files with explicit config, got {:?}",
        import_paths
    );
    assert!(
        import_paths.iter().any(|p| p.contains("DataNode.java")),
        "Should import DataNode.java with explicit config, got {:?}",
        import_paths
    );
}
