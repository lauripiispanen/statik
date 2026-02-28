use std::path::{Path, PathBuf};

use statik::cli::commands;
use statik::cli::OutputFormat;

fn fixture_source() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/java_source_sets")
}

/// Copy the fixture project to a temporary directory so tests don't conflict.
fn setup() -> tempfile::TempDir {
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

fn index_project(project_path: &Path) {
    let config = statik::discovery::DiscoveryConfig::default();
    let result = statik::cli::index::run_index(project_path, &config, false).unwrap();
    assert!(
        result.files_indexed > 0,
        "Should index at least one Java file, got {}",
        result.files_indexed
    );
}

// =============================================================================
// Source set visibility filtering: app -> framework resolves, framework !-> app
// =============================================================================

#[test]
fn test_source_set_visibility_filtering() {
    let tmp = setup();
    index_project(tmp.path());

    // AppController imports FrameService from framework — should resolve
    let output = commands::run_deps(
        tmp.path(),
        "app/src/main/java/com/example/app/AppController.java",
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

    assert!(
        import_paths.iter().any(|p| p.contains("FrameService.java")),
        "AppController should resolve import to FrameService.java, got {:?}",
        import_paths
    );

    // FrameService imports FrameUtil (same source set) — should resolve
    let output = commands::run_deps(
        tmp.path(),
        "framework/src/main/java/com/example/frame/FrameService.java",
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

    assert!(
        import_paths.iter().any(|p| p.contains("FrameUtil.java")),
        "FrameService should import FrameUtil (same source set), got {:?}",
        import_paths
    );

    // Framework should NOT have edges to app code
    assert!(
        !import_paths
            .iter()
            .any(|p| p.contains("AppController.java")),
        "Framework should not see app code, got {:?}",
        import_paths
    );
}

// =============================================================================
// Same-package resolution scoped: com.example.frame in app-test should not
// create edges to framework unless deps allow it
// =============================================================================

#[test]
fn test_source_set_prevents_cross_module_same_package() {
    let tmp = setup();
    index_project(tmp.path());

    // TestHelper is in com.example.frame package but in app-test source set.
    // app-test depends on [app, framework], so it CAN see framework.
    // But TestHelper itself should not create a spurious edge TO framework
    // files via same-package resolution since it has no import statements.
    let output = commands::run_deps(
        tmp.path(),
        "app/src/test/java/com/example/frame/TestHelper.java",
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

    // TestHelper has no import statements, so it should have no import edges
    assert!(
        imports.is_empty(),
        "TestHelper should have no imports (no import statements), got {:?}",
        imports
    );
}

// =============================================================================
// Dead code respects source set visibility
// =============================================================================

#[test]
fn test_source_set_dead_code_scoping() {
    let tmp = setup();
    index_project(tmp.path());

    let output = commands::run_dead_code(
        tmp.path(),
        "files",
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

    // FrameUtil is imported by FrameService within the framework source set.
    // It should NOT be dead even though nothing in other source sets imports it.
    assert!(
        !dead_paths.iter().any(|p| p.contains("FrameUtil.java")),
        "FrameUtil should NOT be dead (used within framework source set), dead: {:?}",
        dead_paths
    );

    // FrameService is imported by AppController (app -> framework dep allows it).
    assert!(
        !dead_paths.iter().any(|p| p.contains("FrameService.java")),
        "FrameService should NOT be dead (imported from app), dead: {:?}",
        dead_paths
    );
}

// =============================================================================
// Regression test for 10.8: Wildcard imports must not cross source set boundaries.
// WildcardImportTest.java is in framework-test and does `import com.example.frame.*`.
// This should resolve to framework files (FrameService, FrameUtil) but NOT to
// app-test files (TestHelper) even though they share the same package.
// =============================================================================

#[test]
fn test_wildcard_import_respects_source_set_boundaries() {
    let tmp = setup();
    index_project(tmp.path());

    let output = commands::run_deps(
        tmp.path(),
        "framework/src/test/java/com/example/frame/WildcardImportTest.java",
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

    // Should resolve to framework source files (framework-test deps include framework)
    assert!(
        import_paths.iter().any(|p| p.contains("FrameService.java")),
        "Wildcard should resolve to FrameService.java (same module), got {:?}",
        import_paths
    );
    assert!(
        import_paths.iter().any(|p| p.contains("FrameUtil.java")),
        "Wildcard should resolve to FrameUtil.java (same module), got {:?}",
        import_paths
    );

    // Must NOT resolve to app-test files even though they share the package name
    assert!(
        !import_paths.iter().any(|p| p.contains("TestHelper.java")),
        "Wildcard must NOT resolve to TestHelper.java (app-test source set, not visible to framework-test), got {:?}",
        import_paths
    );

    // Must NOT resolve to app code
    assert!(
        !import_paths
            .iter()
            .any(|p| p.contains("AppController.java")),
        "Wildcard must NOT resolve to AppController.java (app source set), got {:?}",
        import_paths
    );
}

// =============================================================================
// Backwards compatibility: no source sets = all files see all files
// =============================================================================

#[test]
fn test_no_source_sets_backwards_compat() {
    let tmp = setup();

    // Remove the source_sets config, keep only rules = []
    std::fs::write(tmp.path().join(".statik/rules.toml"), "rules = []\n").unwrap();

    index_project(tmp.path());

    // Without source sets, AppController should still resolve FrameService
    let output = commands::run_deps(
        tmp.path(),
        "app/src/main/java/com/example/app/AppController.java",
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

    assert!(
        import_paths.iter().any(|p| p.contains("FrameService.java")),
        "Without source sets, AppController should still import FrameService, got {:?}",
        import_paths
    );
}
