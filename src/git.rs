use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// A single file change within a commit (from --numstat output).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub lines_added: u64,
    pub lines_removed: u64,
}

/// A parsed commit record from git log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitRecord {
    pub sha: String,
    pub author_name: String,
    pub author_email: String,
    pub timestamp: i64,
    pub files: Vec<FileChange>,
}

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

    let mut git_archive = Command::new("git")
        .args(["archive", "--format=tar", git_ref])
        .current_dir(project_root)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .context("Failed to run git archive")?;

    let tar_output = Command::new("tar")
        .args(["-x", "-C"])
        .arg(target_dir)
        .stdin(git_archive.stdout.take().unwrap())
        .output()
        .context("Failed to run tar")?;

    // Reap the git archive child process to avoid zombies
    git_archive.wait().context("Failed to wait for git archive")?;

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

/// Run `git log --numstat` and parse the output into CommitRecord structs.
///
/// If `since_sha` is provided, only commits after that SHA are returned.
/// If `max_commits` is provided, the output is limited to that many commits.
pub fn git_log_numstat(
    project_root: &Path,
    since_sha: Option<&str>,
    max_commits: Option<usize>,
) -> Result<Vec<CommitRecord>> {
    // Use a record separator that won't appear in normal output
    let format = "%H|%an|%ae|%at";
    let mut args = vec![
        "log".to_string(),
        format!("--format={}", format),
        "--numstat".to_string(),
    ];

    if let Some(n) = max_commits {
        args.push(format!("-n{}", n));
    }

    if let Some(sha) = since_sha {
        args.push(format!("{}..HEAD", sha));
    }

    let output = Command::new("git")
        .args(&args)
        .current_dir(project_root)
        .output()
        .context("Failed to run git log --numstat")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git log failed: {}", stderr.trim());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_git_log_numstat(&stdout)
}

/// Parse the output of `git log --format='%H|%an|%ae|%at' --numstat`.
fn parse_git_log_numstat(output: &str) -> Result<Vec<CommitRecord>> {
    let mut commits = Vec::new();
    let mut current_commit: Option<CommitRecord> = None;

    for line in output.lines() {
        let line = line.trim();

        if line.is_empty() {
            continue;
        }

        // Try to parse as a commit header line (SHA|name|email|timestamp)
        let parts: Vec<&str> = line.splitn(4, '|').collect();
        if parts.len() == 4 && parts[0].len() == 40 && parts[0].chars().all(|c| c.is_ascii_hexdigit()) {
            // Save previous commit if any
            if let Some(commit) = current_commit.take() {
                commits.push(commit);
            }

            let timestamp: i64 = parts[3].parse().unwrap_or(0);
            current_commit = Some(CommitRecord {
                sha: parts[0].to_string(),
                author_name: parts[1].to_string(),
                author_email: parts[2].to_string(),
                timestamp,
                files: Vec::new(),
            });
        } else if let Some(ref mut commit) = current_commit {
            // Try to parse as numstat line (added\tremoved\tpath)
            let stat_parts: Vec<&str> = line.split('\t').collect();
            if stat_parts.len() == 3 {
                // Binary files show "-" for added/removed
                let lines_added = stat_parts[0].parse::<u64>().unwrap_or(0);
                let lines_removed = stat_parts[1].parse::<u64>().unwrap_or(0);
                commit.files.push(FileChange {
                    path: stat_parts[2].to_string(),
                    lines_added,
                    lines_removed,
                });
            }
        }
    }

    // Don't forget the last commit
    if let Some(commit) = current_commit {
        commits.push(commit);
    }

    Ok(commits)
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

    #[test]
    fn test_parse_git_log_numstat_basic() {
        let output = "\
a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2|Alice|alice@example.com|1700000000

10\t2\tsrc/main.rs
5\t0\tsrc/lib.rs

f1e2d3c4b5a6f1e2d3c4b5a6f1e2d3c4b5a6f1e2|Bob|bob@example.com|1699999000

3\t1\tREADME.md
";
        let commits = parse_git_log_numstat(output).unwrap();
        assert_eq!(commits.len(), 2);

        assert_eq!(commits[0].sha, "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2");
        assert_eq!(commits[0].author_name, "Alice");
        assert_eq!(commits[0].author_email, "alice@example.com");
        assert_eq!(commits[0].timestamp, 1700000000);
        assert_eq!(commits[0].files.len(), 2);
        assert_eq!(commits[0].files[0].path, "src/main.rs");
        assert_eq!(commits[0].files[0].lines_added, 10);
        assert_eq!(commits[0].files[0].lines_removed, 2);
        assert_eq!(commits[0].files[1].path, "src/lib.rs");
        assert_eq!(commits[0].files[1].lines_added, 5);
        assert_eq!(commits[0].files[1].lines_removed, 0);

        assert_eq!(commits[1].sha, "f1e2d3c4b5a6f1e2d3c4b5a6f1e2d3c4b5a6f1e2");
        assert_eq!(commits[1].author_name, "Bob");
        assert_eq!(commits[1].files.len(), 1);
        assert_eq!(commits[1].files[0].path, "README.md");
    }

    #[test]
    fn test_parse_git_log_numstat_empty() {
        let commits = parse_git_log_numstat("").unwrap();
        assert!(commits.is_empty());
    }

    #[test]
    fn test_parse_git_log_numstat_binary_files() {
        let output = "\
a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2|Alice|alice@example.com|1700000000

-\t-\timage.png
5\t2\tsrc/app.ts
";
        let commits = parse_git_log_numstat(output).unwrap();
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].files.len(), 2);
        // Binary files show 0 for both (parsed from "-")
        assert_eq!(commits[0].files[0].path, "image.png");
        assert_eq!(commits[0].files[0].lines_added, 0);
        assert_eq!(commits[0].files[0].lines_removed, 0);
    }

    #[test]
    fn test_parse_git_log_numstat_no_files() {
        // A merge commit with no file changes
        let output = "\
a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2|Alice|alice@example.com|1700000000
";
        let commits = parse_git_log_numstat(output).unwrap();
        assert_eq!(commits.len(), 1);
        assert!(commits[0].files.is_empty());
    }

    #[test]
    fn test_git_log_numstat_integration() {
        let dir = TempDir::new().unwrap();
        init_git_repo(dir.path());

        // Create first commit
        std::fs::write(dir.path().join("a.txt"), "hello\nworld\n").unwrap();
        Command::new("git")
            .args(["add", "a.txt"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "first"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        // Create second commit
        std::fs::write(dir.path().join("b.txt"), "new file\n").unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello\nworld\nmore\n").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "second"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        let commits = git_log_numstat(dir.path(), None, None).unwrap();
        assert_eq!(commits.len(), 2);

        // Most recent commit first
        assert_eq!(commits[0].author_name, "Test");
        assert_eq!(commits[0].author_email, "test@test.com");
        assert!(!commits[0].files.is_empty());

        // Test max_commits
        let limited = git_log_numstat(dir.path(), None, Some(1)).unwrap();
        assert_eq!(limited.len(), 1);
    }

    #[test]
    fn test_git_log_numstat_since_sha() {
        let dir = TempDir::new().unwrap();
        init_git_repo(dir.path());

        // Create first commit
        std::fs::write(dir.path().join("a.txt"), "v1\n").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "first"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        let first_sha = resolve_git_ref(dir.path(), "HEAD").unwrap();

        // Create second commit
        std::fs::write(dir.path().join("b.txt"), "v2\n").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "second"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        // Since first SHA should only return second commit
        let commits = git_log_numstat(dir.path(), Some(&first_sha), None).unwrap();
        assert_eq!(commits.len(), 1);
        assert!(commits[0].files.iter().any(|f| f.path == "b.txt"));
    }
}
