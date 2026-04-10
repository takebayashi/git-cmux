use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output};

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
    let status = command
        .status()
        .with_context(|| format!("Failed to run git show-ref --exists for branch {}", branch))?;
    branch_exists_from_status(branch, status)
}

fn branch_exists_from_status(branch: &str, status: ExitStatus) -> Result<bool> {
    match status.code() {
        Some(0) => Ok(true),
        Some(2) => Ok(false),
        Some(code) => anyhow::bail!(
            "git show-ref --exists failed for branch {} with exit code {}",
            branch,
            code
        ),
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
    use super::{
        Worktree, branch_exists_from_status, branch_name, parse_worktree_list, repo_root_path,
    };
    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;
    use std::path::PathBuf;
    use std::process::ExitStatus;

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
    fn branch_exists_from_status_maps_exists_and_missing() {
        assert_eq!(
            branch_exists_from_status("main", exit_status(0)).unwrap(),
            true
        );
        assert_eq!(
            branch_exists_from_status("main", exit_status(2)).unwrap(),
            false
        );
    }

    #[test]
    fn branch_exists_from_status_rejects_other_exit_codes() {
        let err = branch_exists_from_status("main", exit_status(1)).unwrap_err();

        assert!(
            err.to_string()
                .contains("git show-ref --exists failed for branch main with exit code 1")
        );
    }

    #[cfg(unix)]
    fn exit_status(code: i32) -> ExitStatus {
        ExitStatusExt::from_raw(code << 8)
    }
}
