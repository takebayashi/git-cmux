use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub head: String,
    pub branch: Option<String>,
}

pub fn repo_root() -> Result<PathBuf> {
    let stdout = git_stdout(
        ["rev-parse", "--show-toplevel"],
        "git rev-parse --show-toplevel",
    )?;
    Ok(repo_root_path(&stdout))
}

pub fn list_worktrees() -> Result<Vec<Worktree>> {
    let stdout = git_stdout(["worktree", "list", "--porcelain"], "git worktree list")?;
    parse_worktree_list(&stdout)
}

pub fn add_worktree(dest: &Path, branch: &str) -> Result<()> {
    let mut command = git_command();
    command.arg("worktree").arg("add");
    if branch_exists(branch)? {
        command.arg(dest).arg(branch);
    } else {
        command.arg("-b").arg(branch).arg(dest);
    }
    run_git_output(&mut command, "git worktree add")?;
    Ok(())
}

fn git_command() -> Command {
    Command::new("git")
}

fn git_stdout<const N: usize>(args: [&str; N], action: &str) -> Result<String> {
    let mut command = git_command();
    command.args(args);
    let output = run_git_output(&mut command, action)?;
    stdout_utf8(output, action)
}

fn run_git_output(command: &mut Command, action: &str) -> Result<Output> {
    let output = command.output().context("Failed to run git")?;
    if output.status.success() {
        return Ok(output);
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("not a git repository") {
        anyhow::bail!("Not inside a git repository");
    }

    anyhow::bail!("{} failed: {}", action, stderr.trim());
}

fn stdout_utf8(output: Output, action: &str) -> Result<String> {
    String::from_utf8(output.stdout).with_context(|| format!("Invalid UTF-8 in {}", action))
}

fn repo_root_path(stdout: &str) -> PathBuf {
    PathBuf::from(stdout.trim())
}

fn branch_exists(branch: &str) -> Result<bool> {
    let mut command = git_command();
    command.args(["show-ref", "--exists", &format!("refs/heads/{}", branch)]);
    let output = command
        .output()
        .with_context(|| format!("Failed to run git show-ref --exists for branch {}", branch))?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(2) => Ok(false),
        Some(code) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("not a git repository") {
                anyhow::bail!("Not inside a git repository");
            }
            let stderr = stderr.trim();
            if stderr.is_empty() {
                anyhow::bail!(
                    "git show-ref --exists failed for branch {} with exit code {}",
                    branch,
                    code
                );
            }
            anyhow::bail!(
                "git show-ref --exists failed for branch {} with exit code {}: {}",
                branch,
                code,
                stderr
            );
        }
        None => anyhow::bail!(
            "git show-ref --exists failed for branch {} without an exit code",
            branch
        ),
    }
}

fn parse_worktree_list(text: &str) -> Result<Vec<Worktree>> {
    text.split("\n\n")
        .map(str::trim)
        .filter(|block| !block.is_empty())
        .map(parse_worktree_block)
        .collect()
}

fn branch_name(value: &str) -> String {
    value
        .strip_prefix("refs/heads/")
        .unwrap_or(value)
        .to_string()
}

fn parse_worktree_block(block: &str) -> Result<Worktree> {
    let mut path: Option<PathBuf> = None;
    let mut head = String::new();
    let mut branch: Option<String> = None;

    for line in block.lines() {
        if let Some(value) = line.strip_prefix("worktree ") {
            path = Some(PathBuf::from(value));
        } else if let Some(value) = line.strip_prefix("HEAD ") {
            head = value.to_string();
        } else if let Some(value) = line.strip_prefix("branch ") {
            branch = Some(branch_name(value));
        }
    }

    let path = path.context("git worktree list returned an entry without a worktree path")?;
    Ok(Worktree { path, head, branch })
}

#[cfg(test)]
mod tests {
    use super::{Worktree, branch_exists, branch_name, parse_worktree_list, repo_root_path};
    use anyhow::Result;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn parse_worktree_list_reads_branch_entries() {
        let text = "\
worktree /repo
HEAD 0123456789abcdef
branch refs/heads/main
";

        let worktrees = parse_worktree_list(text).unwrap();

        assert_eq!(
            worktrees,
            vec![Worktree {
                path: PathBuf::from("/repo"),
                head: "0123456789abcdef".to_string(),
                branch: Some("main".to_string()),
            }]
        );
    }

    #[test]
    fn parse_worktree_list_reads_detached_head_entries() {
        let text = "\
worktree /repo-detached
HEAD fedcba9876543210
detached
";

        let worktrees = parse_worktree_list(text).unwrap();

        assert_eq!(
            worktrees,
            vec![Worktree {
                path: PathBuf::from("/repo-detached"),
                head: "fedcba9876543210".to_string(),
                branch: None,
            }]
        );
    }

    #[test]
    fn parse_worktree_list_ignores_empty_blocks() {
        let text = "\n\nworktree /repo\nHEAD 0123\n\n\n";

        let worktrees = parse_worktree_list(text).unwrap();

        assert_eq!(worktrees.len(), 1);
        assert_eq!(worktrees[0].path, PathBuf::from("/repo"));
    }

    #[test]
    fn parse_worktree_list_requires_worktree_path() {
        let text = "\
HEAD 0123456789abcdef
branch refs/heads/main
";

        let err = parse_worktree_list(text).unwrap_err();

        assert!(err.to_string().contains("without a worktree path"));
    }

    #[test]
    fn branch_name_strips_local_ref_prefix() {
        assert_eq!(branch_name("refs/heads/feature/test"), "feature/test");
        assert_eq!(branch_name("origin/main"), "origin/main");
    }

    #[test]
    fn repo_root_path_trims_trailing_newline() {
        assert_eq!(repo_root_path("/tmp/repo\n"), PathBuf::from("/tmp/repo"),);
    }

    #[test]
    fn branch_exists_maps_exists_and_missing() {
        let repo = init_repo();
        run_git(repo.path(), &["branch", "feature/test"]);

        assert!(branch_exists_at(&repo, "feature/test").unwrap());
        assert!(!branch_exists_at(&repo, "missing").unwrap());
    }

    #[test]
    fn branch_exists_maps_not_a_git_repository() {
        let temp = TempDir::new();
        let cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(temp.path()).unwrap();

        let err = branch_exists("main").unwrap_err();

        std::env::set_current_dir(cwd).unwrap();

        assert_eq!(err.to_string(), "Not inside a git repository");
    }

    #[test]
    fn branch_exists_handles_git_errors_without_printing_to_terminal() {
        let repo = init_repo();
        let refs_path = repo.path().join(".git/refs/heads");
        fs::remove_dir_all(&refs_path).unwrap();
        fs::write(&refs_path, b"broken").unwrap();

        let err = branch_exists_at(&repo, "main").unwrap_err();

        assert!(
            err.to_string()
                .contains("git show-ref --exists failed for branch main with exit code")
        );
    }

    fn init_repo() -> TempDir {
        let repo = TempDir::new();
        run_git(repo.path(), &["init"]);
        run_git(repo.path(), &["config", "user.name", "git-cmux test"]);
        run_git(
            repo.path(),
            &["config", "user.email", "git-cmux@example.com"],
        );
        run_git(repo.path(), &["commit", "--allow-empty", "-m", "init"]);
        repo
    }

    fn branch_exists_at(repo: &TempDir, branch: &str) -> Result<bool> {
        let cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(repo.path()).unwrap();
        let result = branch_exists(branch);
        std::env::set_current_dir(cwd).unwrap();
        result
    }

    fn run_git(repo: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed with status {:?}: {}",
            args,
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let millis = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis();
            let path = std::env::temp_dir().join(format!(
                "git-cmux-test-{}-{}-{}",
                std::process::id(),
                millis,
                unique
            ));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
