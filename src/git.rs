use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

/// Resolve a git ref (branch, tag, SHA prefix) to a full commit SHA.
pub fn resolve_git_ref(project_root: &Path, ref_str: &str) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--verify", ref_str])
        .current_dir(project_root)
        .output()
        .context("Failed to run git rev-parse")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Could not resolve git ref '{}': {}", ref_str, stderr.trim());
    }

    let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(sha)
}

/// Export the source tree at a given git ref to a target directory.
///
/// Uses `git archive <ref> | tar -x -C <target>` to extract the full tree
/// without checking out (avoids modifying the working tree).
pub fn export_tree_at_ref(project_root: &Path, git_ref: &str, target_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(target_dir)
        .context(format!("Failed to create target dir: {}", target_dir.display()))?;

    let git_archive = Command::new("git")
        .args(["archive", "--format=tar", git_ref])
        .current_dir(project_root)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .context("Failed to run git archive")?;

    let tar_output = Command::new("tar")
        .args(["-x", "-C"])
        .arg(target_dir)
        .stdin(git_archive.stdout.unwrap())
        .output()
        .context("Failed to run tar")?;

    if !tar_output.status.success() {
        let stderr = String::from_utf8_lossy(&tar_output.stderr);
        anyhow::bail!(
            "Failed to extract tree at ref '{}': {}",
            git_ref,
            stderr.trim()
        );
    }

    Ok(())
}

/// Get the list of files changed between two git refs.
pub fn changed_files_between(
    project_root: &Path,
    ref1: &str,
    ref2: &str,
) -> Result<Vec<String>> {
    let output = Command::new("git")
        .args(["diff", "--name-only", ref1, ref2])
        .current_dir(project_root)
        .output()
        .context("Failed to run git diff --name-only")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "Failed to get changed files between '{}' and '{}': {}",
            ref1,
            ref2,
            stderr.trim()
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let files: Vec<String> = stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect();

    Ok(files)
}

/// Get the list of staged files (for --cached mode).
pub fn staged_files(project_root: &Path) -> Result<Vec<String>> {
    let output = Command::new("git")
        .args(["diff", "--cached", "--name-only", "HEAD"])
        .current_dir(project_root)
        .output()
        .context("Failed to run git diff --cached")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to get staged files: {}", stderr.trim());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let files: Vec<String> = stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect();

    Ok(files)
}

/// Return the path to a cached snapshot DB for a given SHA.
/// Returns None if the cache doesn't exist yet.
pub fn snapshot_cache_path(project_root: &Path, sha: &str) -> PathBuf {
    project_root
        .join(".statik")
        .join("snapshots")
        .join(format!("{}.db", sha))
}

/// Check if a snapshot cache exists for the given SHA.
pub fn has_cached_snapshot(project_root: &Path, sha: &str) -> bool {
    snapshot_cache_path(project_root, sha).exists()
}

/// Check if we're inside a git repository.
pub fn is_git_repo(project_root: &Path) -> bool {
    Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(project_root)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn init_git_repo(dir: &Path) {
        Command::new("git")
            .args(["init"])
            .current_dir(dir)
            .output()
            .expect("git init failed");
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir)
            .output()
            .expect("git config email failed");
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir)
            .output()
            .expect("git config name failed");
    }

    #[test]
    fn test_is_git_repo() {
        let dir = TempDir::new().unwrap();
        assert!(!is_git_repo(dir.path()));

        init_git_repo(dir.path());
        assert!(is_git_repo(dir.path()));
    }

    #[test]
    fn test_resolve_git_ref() {
        let dir = TempDir::new().unwrap();
        init_git_repo(dir.path());

        // Create a commit
        std::fs::write(dir.path().join("test.txt"), "hello").unwrap();
        Command::new("git")
            .args(["add", "test.txt"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        let sha = resolve_git_ref(dir.path(), "HEAD").unwrap();
        assert_eq!(sha.len(), 40); // full SHA

        // Resolve by short SHA
        let short = &sha[..7];
        let resolved = resolve_git_ref(dir.path(), short).unwrap();
        assert_eq!(resolved, sha);
    }

    #[test]
    fn test_resolve_invalid_ref() {
        let dir = TempDir::new().unwrap();
        init_git_repo(dir.path());

        let result = resolve_git_ref(dir.path(), "nonexistent-ref-xyz");
        assert!(result.is_err());
    }

    #[test]
    fn test_export_tree_at_ref() {
        let dir = TempDir::new().unwrap();
        init_git_repo(dir.path());

        // Create files and commit
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.ts"), "export const x = 1;").unwrap();
        std::fs::write(dir.path().join("src/utils.ts"), "export function foo() {}").unwrap();

        Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "add files"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        // Export to a temp dir
        let export_dir = TempDir::new().unwrap();
        export_tree_at_ref(dir.path(), "HEAD", export_dir.path()).unwrap();

        // Verify exported files
        assert!(export_dir.path().join("src/main.ts").exists());
        assert!(export_dir.path().join("src/utils.ts").exists());
        let content = std::fs::read_to_string(export_dir.path().join("src/main.ts")).unwrap();
        assert_eq!(content, "export const x = 1;");
    }

    #[test]
    fn test_changed_files_between() {
        let dir = TempDir::new().unwrap();
        init_git_repo(dir.path());

        // First commit
        std::fs::write(dir.path().join("a.txt"), "v1").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "v1"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        let sha1 = resolve_git_ref(dir.path(), "HEAD").unwrap();

        // Second commit: modify a.txt, add b.txt
        std::fs::write(dir.path().join("a.txt"), "v2").unwrap();
        std::fs::write(dir.path().join("b.txt"), "new").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "v2"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        let sha2 = resolve_git_ref(dir.path(), "HEAD").unwrap();

        let files = changed_files_between(dir.path(), &sha1, &sha2).unwrap();
        assert!(files.contains(&"a.txt".to_string()));
        assert!(files.contains(&"b.txt".to_string()));
    }

    #[test]
    fn test_snapshot_cache_path() {
        let root = Path::new("/project");
        let path = snapshot_cache_path(root, "abc123def456");
        assert_eq!(
            path,
            PathBuf::from("/project/.statik/snapshots/abc123def456.db")
        );
    }

    #[test]
    fn test_has_cached_snapshot() {
        let dir = TempDir::new().unwrap();
        assert!(!has_cached_snapshot(dir.path(), "abc123"));

        let cache_dir = dir.path().join(".statik/snapshots");
        std::fs::create_dir_all(&cache_dir).unwrap();
        std::fs::write(cache_dir.join("abc123.db"), "fake db").unwrap();
        assert!(has_cached_snapshot(dir.path(), "abc123"));
    }
}
