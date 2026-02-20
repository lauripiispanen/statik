use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Get path to the statik binary built by `cargo build`.
fn statik_bin() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("target");
    path.push("debug");
    path.push("statik");
    path
}

/// Create a minimal TypeScript project in a temp directory for testing.
/// Returns the temp dir (must be kept alive for the duration of the test).
struct TestProject {
    dir: tempfile::TempDir,
}

impl TestProject {
    fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Create a file relative to the project root.
    fn write_file(&self, rel_path: &str, content: &str) {
        let full = self.dir.path().join(rel_path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&full, content).unwrap();
    }

    /// Run statik with the given subcommand args, with cwd set to project root.
    fn run(&self, args: &[&str]) -> std::process::Output {
        Command::new(statik_bin())
            .args(args)
            .current_dir(self.dir.path())
            .output()
            .expect("failed to run statik")
    }

    fn stdout(&self, args: &[&str]) -> String {
        let output = self.run(args);
        String::from_utf8_lossy(&output.stdout).to_string()
    }

}

/// Create a basic project with a known dependency structure:
///
///   src/index.ts  -->  src/services/userService.ts  -->  src/models/user.ts
///        |                       |
///        v                       v
///   src/utils/format.ts    src/utils/format.ts
///
///   src/db/connection.ts  (orphan, no imports to/from it)
///   src/ui/dashboard.ts   -->  src/db/connection.ts  (boundary violation candidate)
///   src/orphan.ts         (dead file, not imported by anyone, not an entry point)
fn create_basic_project() -> TestProject {
    let dir = tempfile::TempDir::new().unwrap();
    let proj = TestProject { dir };

    proj.write_file(
        "src/index.ts",
        r#"import { UserService } from './services/userService';
import { formatName } from './utils/format';

const service = new UserService();
console.log(formatName("test"));
"#,
    );

    proj.write_file(
        "src/models/user.ts",
        r#"export interface User {
  id: number;
  name: string;
}

export type UserId = number;
"#,
    );

    proj.write_file(
        "src/services/userService.ts",
        r#"import { User } from '../models/user';
import { formatName } from '../utils/format';

export class UserService {
  getUser(): User {
    return { name: formatName("test"), id: 1 };
  }
}

export function unusedHelper() {
  return "unused";
}
"#,
    );

    proj.write_file(
        "src/utils/format.ts",
        r#"export function formatName(name: string): string {
  return name.trim().toLowerCase();
}

export function unusedFormatter() {
  return "not used";
}
"#,
    );

    proj.write_file(
        "src/db/connection.ts",
        r#"export function getConnection() {
  return { host: "localhost" };
}
"#,
    );

    proj.write_file(
        "src/ui/dashboard.ts",
        r#"import { getConnection } from '../db/connection';

export function renderDashboard() {
  const conn = getConnection();
  return `Connected to ${conn.host}`;
}
"#,
    );

    proj.write_file(
        "src/orphan.ts",
        r#"// This file is not imported by anything and is not an entry point
export function orphanFunction() {
  return 42;
}
"#,
    );

    proj
}

/// Create a project with circular dependencies:
///   src/a.ts -> src/b.ts -> src/c.ts -> src/a.ts
fn create_circular_project() -> TestProject {
    let dir = tempfile::TempDir::new().unwrap();
    let proj = TestProject { dir };

    proj.write_file(
        "src/a.ts",
        r#"import { bFunc } from './b';
export function aFunc() { return bFunc(); }
"#,
    );

    proj.write_file(
        "src/b.ts",
        r#"import { cFunc } from './c';
export function bFunc() { return cFunc(); }
"#,
    );

    proj.write_file(
        "src/c.ts",
        r#"import { aFunc } from './a';
export function cFunc() { return aFunc(); }
"#,
    );

    proj
}

// =============================================================================
// INDEX command tests
// =============================================================================

#[test]
fn test_index_creates_database() {
    let proj = create_basic_project();
    let output = proj.run(&["index", "."]);

    assert!(output.status.success(), "index should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should report files indexed
    assert!(
        stdout.contains("files"),
        "should mention files in output: {}",
        stdout
    );

    // Database should exist
    assert!(
        proj.path().join(".statik/index.db").exists(),
        ".statik/index.db should be created"
    );
}

#[test]
fn test_index_json_output() {
    let proj = create_basic_project();
    let output = proj.run(&["--format", "json", "index", "."]);

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("invalid JSON output: {}\n---\n{}", e, stdout));

    assert!(json.get("files_indexed").is_some() || json.get("files").is_some());
}

#[test]
fn test_index_incremental() {
    let proj = create_basic_project();

    // First index
    let out1 = proj.run(&["index", "."]);
    assert!(out1.status.success());

    // Second index (should be incremental, no files changed)
    let out2 = proj.run(&["index", "."]);
    assert!(out2.status.success());
    let stdout2 = String::from_utf8_lossy(&out2.stdout);
    // Should still succeed and report something
    assert!(
        stdout2.contains("files") || stdout2.contains("indexed"),
        "incremental index should produce output: {}",
        stdout2
    );
}

// =============================================================================
// DEPS command tests
// =============================================================================

#[test]
fn test_deps_shows_imports_and_importers() {
    let proj = create_basic_project();
    proj.run(&["index", "."]);

    let stdout = proj.stdout(&["--no-index", "deps", "src/services/userService.ts"]);

    // userService imports from models/user and utils/format
    assert!(
        stdout.contains("user") || stdout.contains("models"),
        "should show import of models/user: {}",
        stdout
    );
    assert!(
        stdout.contains("format") || stdout.contains("utils"),
        "should show import of utils/format: {}",
        stdout
    );

    // userService is imported by index.ts
    assert!(
        stdout.contains("index"),
        "should show that index.ts imports it: {}",
        stdout
    );
}

#[test]
fn test_deps_json_format() {
    let proj = create_basic_project();
    proj.run(&["index", "."]);

    let stdout = proj.stdout(&[
        "--format",
        "json",
        "--no-index",
        "deps",
        "src/services/userService.ts",
    ]);

    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("deps JSON should be valid");

    // Should have imports and imported_by arrays
    assert!(
        json.get("imports").is_some(),
        "JSON should have imports field: {}",
        stdout
    );
    assert!(
        json.get("imported_by").is_some(),
        "JSON should have imported_by field: {}",
        stdout
    );
}

#[test]
fn test_deps_direction_out() {
    let proj = create_basic_project();
    proj.run(&["index", "."]);

    let stdout = proj.stdout(&[
        "--no-index",
        "deps",
        "src/services/userService.ts",
        "--direction",
        "out",
    ]);

    // Should show imports but not imported_by
    assert!(
        stdout.contains("Imports") || stdout.contains("imports"),
        "should show imports section: {}",
        stdout
    );
}

#[test]
fn test_deps_file_not_found() {
    let proj = create_basic_project();
    proj.run(&["index", "."]);

    let output = proj.run(&["--no-index", "deps", "nonexistent.ts"]);
    assert!(
        !output.status.success(),
        "deps on nonexistent file should fail"
    );
}

// =============================================================================
// EXPORTS command tests
// =============================================================================

#[test]
fn test_exports_lists_symbols() {
    let proj = create_basic_project();
    proj.run(&["index", "."]);

    let stdout = proj.stdout(&["--no-index", "exports", "src/utils/format.ts"]);

    assert!(
        stdout.contains("formatName"),
        "should list formatName export: {}",
        stdout
    );
    assert!(
        stdout.contains("unusedFormatter"),
        "should list unusedFormatter export: {}",
        stdout
    );
}

#[test]
fn test_exports_shows_used_status() {
    let proj = create_basic_project();
    proj.run(&["index", "."]);

    let stdout = proj.stdout(&[
        "--format",
        "json",
        "--no-index",
        "exports",
        "src/utils/format.ts",
    ]);

    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let exports = json["exports"].as_array().expect("exports array");

    // Find formatName - should be used
    let format_name = exports
        .iter()
        .find(|e| e["name"] == "formatName")
        .expect("should have formatName");
    assert_eq!(format_name["is_used"], true, "formatName should be used");

    // Find unusedFormatter - should be unused
    let unused = exports
        .iter()
        .find(|e| e["name"] == "unusedFormatter")
        .expect("should have unusedFormatter");
    assert_eq!(
        unused["is_used"], false,
        "unusedFormatter should be unused"
    );
}

// =============================================================================
// DEAD-CODE command tests
// =============================================================================

#[test]
fn test_dead_code_finds_orphan_file() {
    let proj = create_basic_project();
    proj.run(&["index", "."]);

    let stdout = proj.stdout(&["--no-index", "dead-code"]);

    // orphan.ts should be detected as dead (not imported, not an entry point)
    assert!(
        stdout.contains("orphan"),
        "should detect orphan.ts as dead: {}",
        stdout
    );
}

#[test]
fn test_dead_code_finds_unused_exports() {
    let proj = create_basic_project();
    proj.run(&["index", "."]);

    let stdout = proj.stdout(&["--no-index", "dead-code", "--scope", "exports"]);

    // unusedHelper and unusedFormatter should be dead exports
    assert!(
        stdout.contains("unusedHelper") || stdout.contains("unusedFormatter"),
        "should find unused exports: {}",
        stdout
    );
}

#[test]
fn test_dead_code_json() {
    let proj = create_basic_project();
    proj.run(&["index", "."]);

    let stdout = proj.stdout(&["--format", "json", "--no-index", "dead-code"]);

    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert!(
        json.get("dead_files").is_some(),
        "should have dead_files: {}",
        stdout
    );
    assert!(
        json.get("dead_exports").is_some(),
        "should have dead_exports: {}",
        stdout
    );
    assert!(
        json.get("summary").is_some(),
        "should have summary: {}",
        stdout
    );
}

// =============================================================================
// CYCLES command tests
// =============================================================================

#[test]
fn test_cycles_detects_circular_deps() {
    let proj = create_circular_project();
    proj.run(&["index", "."]);

    let stdout = proj.stdout(&["--no-index", "cycles"]);

    assert!(
        stdout.contains("Circular") || stdout.contains("cycle"),
        "should detect cycles: {}",
        stdout
    );
}

#[test]
fn test_cycles_json() {
    let proj = create_circular_project();
    proj.run(&["index", "."]);

    let stdout = proj.stdout(&["--format", "json", "--no-index", "cycles"]);

    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let cycles = json["cycles"].as_array().expect("cycles array");
    assert!(!cycles.is_empty(), "should find at least one cycle");
}

#[test]
fn test_no_cycles_in_acyclic_project() {
    let proj = create_basic_project();
    proj.run(&["index", "."]);

    let stdout = proj.stdout(&["--no-index", "cycles"]);

    assert!(
        stdout.contains("No circular") || stdout.contains("0 cycles"),
        "should report no cycles: {}",
        stdout
    );
}

// =============================================================================
// IMPACT command tests
// =============================================================================

#[test]
fn test_impact_shows_affected_files() {
    let proj = create_basic_project();
    proj.run(&["index", "."]);

    let stdout = proj.stdout(&["--no-index", "impact", "src/utils/format.ts"]);

    // format.ts is imported by index.ts and userService.ts, so changing it affects them
    assert!(
        stdout.contains("Affected") || stdout.contains("affected"),
        "should show affected files: {}",
        stdout
    );
    assert!(
        stdout.contains("index") || stdout.contains("userService"),
        "should show files that import format.ts: {}",
        stdout
    );
}

#[test]
fn test_impact_leaf_file() {
    let proj = create_basic_project();
    proj.run(&["index", "."]);

    // orphan.ts is not imported by anything
    let stdout = proj.stdout(&["--no-index", "impact", "src/orphan.ts"]);

    assert!(
        stdout.contains("No files affected") || stdout.contains("0 total"),
        "orphan should have no impact: {}",
        stdout
    );
}

#[test]
fn test_impact_json() {
    let proj = create_basic_project();
    proj.run(&["index", "."]);

    let stdout = proj.stdout(&[
        "--format",
        "json",
        "--no-index",
        "impact",
        "src/utils/format.ts",
    ]);

    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert!(
        json.get("affected").is_some(),
        "should have affected field"
    );
    let affected = json["affected"].as_array().unwrap();
    assert!(
        !affected.is_empty(),
        "format.ts should affect other files"
    );
}

// =============================================================================
// SUMMARY command tests
// =============================================================================

#[test]
fn test_summary_reports_stats() {
    let proj = create_basic_project();
    proj.run(&["index", "."]);

    let stdout = proj.stdout(&["--no-index", "summary"]);

    assert!(
        stdout.contains("Project Summary") || stdout.contains("summary"),
        "should have summary header: {}",
        stdout
    );
    assert!(
        stdout.contains("Files") || stdout.contains("files"),
        "should report file count: {}",
        stdout
    );
    assert!(
        stdout.contains("TypeScript") || stdout.contains("typescript"),
        "should mention TypeScript: {}",
        stdout
    );
}

#[test]
fn test_summary_json() {
    let proj = create_basic_project();
    proj.run(&["index", "."]);

    let stdout = proj.stdout(&["--format", "json", "--no-index", "summary"]);

    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert!(json.get("files").is_some(), "should have files");
    assert!(
        json.get("dependencies").is_some(),
        "should have dependencies"
    );
    assert!(json.get("dead_code").is_some(), "should have dead_code");
    assert!(json.get("cycles").is_some(), "should have cycles");

    let total_files = json["files"]["total"].as_u64().unwrap();
    assert_eq!(total_files, 7, "basic project has 7 files");
}

// =============================================================================
// LINT command tests
// =============================================================================

#[test]
fn test_lint_no_config_errors_gracefully() {
    let proj = create_basic_project();
    proj.run(&["index", "."]);

    // No rules.toml exists
    let output = proj.run(&["--no-index", "lint"]);
    assert!(
        !output.status.success(),
        "lint without config should fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("config") || stderr.contains("rules.toml"),
        "error should mention config: {}",
        stderr
    );
}

#[test]
fn test_lint_no_violations() {
    let proj = create_basic_project();
    proj.run(&["index", "."]);

    // Create a rule that won't trigger (no ui -> models violations in our project)
    proj.write_file(
        ".statik/rules.toml",
        r#"[[rules]]
id = "no-models-to-ui"
severity = "error"
description = "Models must not import from UI"

[rules.boundary]
from = ["src/models/**"]
deny = ["src/ui/**"]
"#,
    );

    let output = proj.run(&["--no-index", "lint"]);
    assert!(
        output.status.success(),
        "lint with no violations should exit 0"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("No lint violations"),
        "should say no violations: {}",
        stdout
    );
}

#[test]
fn test_lint_detects_violations_exit_code_1() {
    let proj = create_basic_project();
    proj.run(&["index", "."]);

    // ui/dashboard.ts imports from db/connection.ts - this should be a violation
    proj.write_file(
        ".statik/rules.toml",
        r#"[[rules]]
id = "no-ui-to-db"
severity = "error"
description = "UI must not import from database layer"
rationale = "UI should go through the service layer"
fix_direction = "Import from src/services/ instead"

[rules.boundary]
from = ["src/ui/**"]
deny = ["src/db/**"]
"#,
    );

    let output = proj.run(&["--no-index", "lint"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "lint with error violations should exit 1"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("no-ui-to-db"),
        "should show rule id: {}",
        stdout
    );
    assert!(
        stdout.contains("dashboard") || stdout.contains("ui"),
        "should mention source file: {}",
        stdout
    );
    assert!(
        stdout.contains("connection") || stdout.contains("db"),
        "should mention target file: {}",
        stdout
    );
    assert!(
        stdout.contains("1 errors"),
        "should count 1 error: {}",
        stdout
    );
}

#[test]
fn test_lint_warning_only_exit_code_0() {
    let proj = create_basic_project();
    proj.run(&["index", "."]);

    // Same violation but as warning - should exit 0
    proj.write_file(
        ".statik/rules.toml",
        r#"[[rules]]
id = "no-ui-to-db"
severity = "warning"
description = "UI should not import from database layer"

[rules.boundary]
from = ["src/ui/**"]
deny = ["src/db/**"]
"#,
    );

    let output = proj.run(&["--no-index", "lint"]);
    assert!(
        output.status.success(),
        "lint with only warnings should exit 0"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("warning") || stdout.contains("Warning"),
        "should show warnings: {}",
        stdout
    );
    assert!(
        stdout.contains("0 errors"),
        "should have 0 errors: {}",
        stdout
    );
}

#[test]
fn test_lint_json_output() {
    let proj = create_basic_project();
    proj.run(&["index", "."]);

    proj.write_file(
        ".statik/rules.toml",
        r#"[[rules]]
id = "no-ui-to-db"
severity = "error"
description = "UI must not import from database layer"

[rules.boundary]
from = ["src/ui/**"]
deny = ["src/db/**"]
"#,
    );

    let stdout = proj.stdout(&["--format", "json", "--no-index", "lint"]);

    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("lint JSON should be valid");

    let violations = json["violations"].as_array().expect("violations array");
    assert!(
        !violations.is_empty(),
        "should have violations in JSON"
    );
    assert_eq!(violations[0]["rule_id"], "no-ui-to-db");
    assert_eq!(violations[0]["severity"], "error");

    let summary = &json["summary"];
    assert_eq!(summary["errors"], 1);
}

#[test]
fn test_lint_except_clause() {
    let proj = create_basic_project();
    proj.run(&["index", "."]);

    // Use except to allow the specific import
    proj.write_file(
        ".statik/rules.toml",
        r#"[[rules]]
id = "no-ui-to-db"
severity = "error"
description = "UI must not import from database layer"

[rules.boundary]
from = ["src/ui/**"]
deny = ["src/db/**"]
except = ["src/db/connection.ts"]
"#,
    );

    let output = proj.run(&["--no-index", "lint"]);
    assert!(
        output.status.success(),
        "lint with exception should pass"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("No lint violations"),
        "exception should prevent violation: {}",
        stdout
    );
}

#[test]
fn test_lint_multiple_rules() {
    let proj = create_basic_project();
    proj.run(&["index", "."]);

    proj.write_file(
        ".statik/rules.toml",
        r#"[[rules]]
id = "no-ui-to-db"
severity = "error"
description = "UI must not import from database layer"

[rules.boundary]
from = ["src/ui/**"]
deny = ["src/db/**"]

[[rules]]
id = "no-service-to-ui"
severity = "warning"
description = "Services should not import from UI"

[rules.boundary]
from = ["src/services/**"]
deny = ["src/ui/**"]
"#,
    );

    let stdout = proj.stdout(&["--format", "json", "--no-index", "lint"]);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    // Should have the UI-to-DB violation
    let violations = json["violations"].as_array().unwrap();
    assert!(
        violations.iter().any(|v| v["rule_id"] == "no-ui-to-db"),
        "should have UI-to-DB violation"
    );

    // Should evaluate both rules
    assert_eq!(json["summary"]["rules_evaluated"], 2);
}

#[test]
fn test_lint_rule_filter() {
    let proj = create_basic_project();
    proj.run(&["index", "."]);

    proj.write_file(
        ".statik/rules.toml",
        r#"[[rules]]
id = "no-ui-to-db"
severity = "error"
description = "UI must not import from database layer"

[rules.boundary]
from = ["src/ui/**"]
deny = ["src/db/**"]

[[rules]]
id = "no-service-to-ui"
severity = "warning"
description = "Services should not import from UI"

[rules.boundary]
from = ["src/services/**"]
deny = ["src/ui/**"]
"#,
    );

    // Only evaluate the second rule (no violations expected for it)
    let output = proj.run(&["--no-index", "lint", "--rule", "no-service-to-ui"]);
    assert!(
        output.status.success(),
        "only evaluating non-violating rule should pass"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("No lint violations"),
        "filtered rule should show no violations: {}",
        stdout
    );
}

#[test]
fn test_lint_severity_threshold() {
    let proj = create_basic_project();
    proj.run(&["index", "."]);

    // Create a rule with warning severity
    proj.write_file(
        ".statik/rules.toml",
        r#"[[rules]]
id = "no-ui-to-db"
severity = "warning"
description = "UI should not import from database layer"

[rules.boundary]
from = ["src/ui/**"]
deny = ["src/db/**"]
"#,
    );

    // With error threshold, warnings should be filtered out
    let stdout = proj.stdout(&[
        "--format",
        "json",
        "--no-index",
        "lint",
        "--severity-threshold",
        "error",
    ]);

    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let violations = json["violations"].as_array().unwrap();
    assert!(
        violations.is_empty(),
        "error threshold should filter out warnings"
    );
}

#[test]
fn test_lint_invalid_toml() {
    let proj = create_basic_project();
    proj.run(&["index", "."]);

    proj.write_file(
        ".statik/rules.toml",
        "this is not valid toml [[[broken",
    );

    let output = proj.run(&["--no-index", "lint"]);
    assert!(
        !output.status.success(),
        "invalid TOML should cause failure"
    );
}

#[test]
fn test_lint_custom_config_path() {
    let proj = create_basic_project();
    proj.run(&["index", "."]);

    // Write config to a non-standard location
    proj.write_file(
        "custom-rules.toml",
        r#"[[rules]]
id = "no-ui-to-db"
severity = "error"
description = "UI must not import from database layer"

[rules.boundary]
from = ["src/ui/**"]
deny = ["src/db/**"]
"#,
    );

    let output = proj.run(&["--no-index", "lint", "--config", "custom-rules.toml"]);
    // Should find the violation via custom config
    assert_eq!(
        output.status.code(),
        Some(1),
        "custom config should still detect violations"
    );
}

// =============================================================================
// AUTO-INDEX tests (commands without --no-index)
// =============================================================================

#[test]
fn test_auto_index_on_first_run() {
    let proj = create_basic_project();

    // Run deps without prior indexing - should auto-index
    let output = proj.run(&["deps", "src/index.ts"]);
    assert!(
        output.status.success(),
        "should auto-index and succeed"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("auto-index") || stderr.contains("Indexed") || stderr.contains("No index"),
        "should mention auto-indexing: {}",
        stderr
    );
}

#[test]
fn test_no_index_flag_without_existing_index() {
    let proj = create_basic_project();

    // --no-index without existing index should fail
    let output = proj.run(&["--no-index", "deps", "src/index.ts"]);
    assert!(
        !output.status.success(),
        "--no-index without index should fail"
    );
}

// =============================================================================
// EDGE CASES
// =============================================================================

#[test]
fn test_empty_project() {
    let dir = tempfile::TempDir::new().unwrap();
    let proj = TestProject { dir };

    let output = proj.run(&["index", "."]);
    // Should succeed even with no files
    assert!(output.status.success(), "empty project index should succeed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("0") || stdout.contains("files"),
        "should report zero files: {}",
        stdout
    );
}

#[test]
fn test_single_file_project() {
    let dir = tempfile::TempDir::new().unwrap();
    let proj = TestProject { dir };

    proj.write_file(
        "src/main.ts",
        r#"export function main() {
  console.log("hello");
}
"#,
    );

    proj.run(&["index", "."]);
    let stdout = proj.stdout(&["--no-index", "summary"]);
    assert!(
        stdout.contains("1") || stdout.contains("file"),
        "should report 1 file: {}",
        stdout
    );
}

// =============================================================================
// NEW RULE TYPE INTEGRATION TESTS
// =============================================================================

#[test]
fn test_lint_layer_rule_violation() {
    let proj = create_basic_project();

    proj.write_file(
        ".statik/rules.toml",
        r#"
[[rules]]
id = "clean-layers"
severity = "error"
description = "Dependencies must flow top-down"

[rules.layer]
layers = [
  { name = "presentation", patterns = ["src/ui/**"] },
  { name = "service", patterns = ["src/services/**"] },
  { name = "data", patterns = ["src/db/**"] },
]
"#,
    );

    // src/ui/dashboard.ts imports from src/db/connection.ts
    // That's presentation (index 0) -> data (index 2), which is top-down (valid).
    // Need a bottom-up violation: data -> presentation
    proj.write_file(
        "src/db/renderer.ts",
        r#"import { renderDashboard } from '../ui/dashboard';
export const html = renderDashboard();
"#,
    );

    let output = proj.run(&["lint"]);
    assert!(!output.status.success(), "should exit 1 for layer violation");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("clean-layers"), "should mention the layer rule: {}", stdout);
    assert!(stdout.contains("data"), "should mention the data layer: {}", stdout);
    assert!(stdout.contains("presentation"), "should mention the presentation layer: {}", stdout);
}

#[test]
fn test_lint_containment_rule_violation() {
    let proj = create_basic_project();

    proj.write_file(
        "src/auth/index.ts",
        r#"export { login } from './service';"#,
    );
    proj.write_file(
        "src/auth/service.ts",
        r#"export function login() { return "token"; }"#,
    );

    // Direct import of internal auth file (violation)
    proj.write_file(
        "src/consumer.ts",
        r#"import { login } from './auth/service';"#,
    );

    proj.write_file(
        ".statik/rules.toml",
        r#"
[[rules]]
id = "auth-encapsulation"
severity = "error"
description = "Auth module must be accessed through its public API"

[rules.containment]
module = ["src/auth/**"]
public_api = ["src/auth/index.ts"]
"#,
    );

    let output = proj.run(&["lint"]);
    assert!(!output.status.success(), "should exit 1 for containment violation");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("auth-encapsulation"), "should report containment rule: {}", stdout);
    assert!(stdout.contains("src/auth/service.ts"), "should show internal target: {}", stdout);
}

#[test]
fn test_lint_import_restriction_type_only() {
    let proj = create_basic_project();

    proj.write_file(
        ".statik/rules.toml",
        r#"
[[rules]]
id = "models-type-only"
severity = "error"
description = "Imports from models/ must be type-only"

[rules.import_restriction]
target = ["src/models/**"]
require_type_only = true
"#,
    );

    // src/services/userService.ts has `import { User } from '../models/user'`
    // which is NOT type-only, so it should violate
    let output = proj.run(&["lint"]);
    assert!(!output.status.success(), "should exit 1 for type-only violation");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("models-type-only"), "should report import restriction: {}", stdout);
    assert!(stdout.contains("type-only"), "should mention type-only: {}", stdout);
}

#[test]
fn test_lint_fan_limit_violation() {
    let proj = create_basic_project();

    // Create a god module that imports everything
    proj.write_file(
        "src/god.ts",
        r#"import { UserService } from './services/userService';
import { formatName } from './utils/format';
import { getConnection } from './db/connection';
import { User } from './models/user';

export function godFunction() {
  return "I import too much";
}
"#,
    );

    proj.write_file(
        ".statik/rules.toml",
        r#"
[[rules]]
id = "no-god-modules"
severity = "error"
description = "Files should not have too many dependencies"

[rules.fan_limit]
pattern = ["src/**"]
max_fan_out = 3
"#,
    );

    let output = proj.run(&["lint"]);
    assert!(!output.status.success(), "should exit 1 for fan-out violation");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("no-god-modules"), "should report fan limit: {}", stdout);
    assert!(stdout.contains("fan-out"), "should mention fan-out: {}", stdout);
}

#[test]
fn test_lint_mixed_rule_types_json() {
    let proj = create_basic_project();

    proj.write_file(
        ".statik/rules.toml",
        r#"
[[rules]]
id = "boundary"
severity = "error"
description = "UI must not import from DB"

[rules.boundary]
from = ["src/ui/**"]
deny = ["src/db/**"]

[[rules]]
id = "layers"
severity = "warning"
description = "Layer enforcement"

[rules.layer]
layers = [
  { name = "ui", patterns = ["src/ui/**"] },
  { name = "db", patterns = ["src/db/**"] },
]

[[rules]]
id = "fan-check"
severity = "info"
description = "Fan limits"

[rules.fan_limit]
pattern = ["src/**"]
max_fan_out = 100
"#,
    );

    let output = proj.run(&["lint", "--format", "json"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse JSON to verify structure
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("invalid JSON: {}: {}", e, stdout));

    assert_eq!(json["rules_evaluated"], 3);
    assert!(json["violations"].as_array().unwrap().len() >= 1);
    assert_eq!(json["summary"]["rules_evaluated"], 3);
}
