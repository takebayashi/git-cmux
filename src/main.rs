mod cmux;
mod git;
mod tui;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "git-cmux",
    about = "Git subcommands with cmux workspace integration"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Manage git worktrees
    Worktree {
        /// Branch name: open existing worktree or create new one
        branch: Option<String>,
    },
}

fn main() {
    if let Err(e) = run() {
        eprintln!("git-cmux: {:#}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Worktree { branch: None } => worktree_app::pick_and_open()?,
        Commands::Worktree {
            branch: Some(branch),
        } => worktree_app::open_or_create(&branch)?,
    }
    Ok(())
}

mod worktree_app {
    use anyhow::Result;
    use std::fs;
    use std::path::{Path, PathBuf};

    use crate::cmux;
    use crate::git::{self, Worktree};
    use crate::tui::{self, PickerItemKind, PickerRow};

    pub fn open_or_create(branch: &str) -> Result<()> {
        let worktrees = git::list_worktrees()?;
        if let Some(path) = find_path(&worktrees, branch) {
            open_path_in_cmux(path)
        } else {
            let path = worktree_destination(branch)?;
            if path.exists() {
                anyhow::bail!("Directory already exists at {}", path.display());
            }
            let parent = path
                .parent()
                .expect("worktree destination must have a parent");
            fs::create_dir_all(parent)?;
            git::add_worktree(&path, branch)?;
            println!("Created worktree at {}", path.display());
            open_path_in_cmux(&path)
        }
    }

    pub fn pick_and_open() -> Result<()> {
        let worktrees = git::list_worktrees()?;
        let rows = worktree_picker_rows(&worktrees);
        let default = default_worktree_selection(&worktrees);

        loop {
            let Some(selection) = tui::pick_row("Select a worktree", &rows, default)? else {
                return Ok(());
            };

            if selection == 0 {
                if let Some(branch) = tui::prompt_text("Branch name")? {
                    open_or_create(&branch)?;
                    return Ok(());
                }
                continue;
            }

            if let Some(path) = selection_to_worktree_path(&worktrees, selection) {
                open_path_in_cmux(path)?;
            }
            return Ok(());
        }
    }

    fn open_path_in_cmux(path: &Path) -> Result<()> {
        let workspace_id = cmux::create_workspace(path)?;
        cmux::select_workspace(&workspace_id)
    }

    fn worktree_destination(branch: &str) -> Result<PathBuf> {
        let repo_root = git::repo_root()?;
        Ok(worktree_destination_from_root(&repo_root, branch))
    }

    fn find_path<'a>(worktrees: &'a [Worktree], branch: &str) -> Option<&'a Path> {
        worktrees
            .iter()
            .find(|wt| wt.branch.as_deref() == Some(branch))
            .map(|wt| wt.path.as_path())
    }

    fn default_worktree_selection(worktrees: &[Worktree]) -> usize {
        usize::from(!worktrees.is_empty())
    }

    fn selection_to_worktree_path(worktrees: &[Worktree], selection: usize) -> Option<&Path> {
        worktrees
            .get(selection.checked_sub(1)?)
            .map(|wt| wt.path.as_path())
    }

    fn worktree_picker_rows(worktrees: &[Worktree]) -> Vec<PickerRow> {
        let mut rows = Vec::with_capacity(worktrees.len() + 1);
        rows.push(PickerRow {
            primary: "Create new worktree...".to_string(),
            secondary: None,
            kind: PickerItemKind::Action,
        });
        rows.extend(worktrees.iter().map(|worktree| PickerRow {
            primary: worktree_display_name(worktree).to_string(),
            secondary: Some(worktree.path.display().to_string()),
            kind: PickerItemKind::Item,
        }));
        rows
    }

    fn worktree_destination_from_root(repo_root: &Path, branch: &str) -> PathBuf {
        repo_root.join(".worktrees").join(branch.replace('/', "-"))
    }

    fn worktree_display_name(worktree: &Worktree) -> &str {
        if let Some(ref branch) = worktree.branch {
            branch.as_str()
        } else {
            &worktree.head[..worktree.head.len().min(8)]
        }
    }

    #[cfg(test)]
    mod tests {
        use super::{
            default_worktree_selection, selection_to_worktree_path, worktree_destination_from_root,
            worktree_picker_rows,
        };
        use crate::git::Worktree;
        use crate::tui::PickerItemKind;
        use std::path::{Path, PathBuf};

        fn sample_worktree(branch: Option<&str>, path: &str) -> Worktree {
            Worktree {
                path: PathBuf::from(path),
                head: "0123456789abcdef".to_string(),
                branch: branch.map(str::to_string),
            }
        }

        #[test]
        fn worktree_destination_uses_repo_local_worktrees_directory() {
            let repo_root = Path::new("/tmp/myapp");

            let path = worktree_destination_from_root(repo_root, "feature/login");

            assert_eq!(path, PathBuf::from("/tmp/myapp/.worktrees/feature-login"));
        }

        #[test]
        fn worktree_destination_replaces_all_branch_separators() {
            let repo_root = Path::new("/tmp/myapp");

            let path = worktree_destination_from_root(repo_root, "feature/foo/bar");

            assert_eq!(path, PathBuf::from("/tmp/myapp/.worktrees/feature-foo-bar"));
        }

        #[test]
        fn worktree_picker_rows_prepends_create_action() {
            let rows = worktree_picker_rows(&[sample_worktree(Some("main"), "/repo")]);

            assert_eq!(rows[0].primary, "Create new worktree...");
            assert_eq!(rows[0].kind, PickerItemKind::Action);
            assert_eq!(rows[1].primary, "main");
            assert_eq!(rows[1].secondary.as_deref(), Some("/repo"));
            assert_eq!(rows[1].kind, PickerItemKind::Item);
        }

        #[test]
        fn default_worktree_selection_prefers_first_worktree() {
            assert_eq!(
                default_worktree_selection(&[sample_worktree(Some("main"), "/repo")]),
                1
            );
            assert_eq!(default_worktree_selection(&[]), 0);
        }

        #[test]
        fn selection_to_worktree_path_maps_picker_index() {
            let worktrees = [sample_worktree(Some("main"), "/repo")];

            assert_eq!(
                selection_to_worktree_path(&worktrees, 1),
                Some(Path::new("/repo"))
            );
            assert_eq!(selection_to_worktree_path(&worktrees, 0), None);
            assert_eq!(selection_to_worktree_path(&worktrees, 2), None);
        }
    }
}
