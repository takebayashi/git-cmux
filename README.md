# git-cmux

`git-cmux` is a Git external subcommand that integrates Git operations with [cmux](https://github.com/manaflow-ai/cmux) workspaces, enabling seamless terminal-based Git workflows.

## Features

- **Worktree management**: Interactively create, browse, and open Git worktrees in cmux workspaces

## Requirements

- Git 2.43.0 or later
- `cmux` with a reachable `CMUX_SOCKET_PATH`

## Installation

```bash
cargo install git-cmux
```

## Usage

### Worktree

```bash
# Launch interactive picker to select a worktree and open it in a cmux workspace
git cmux worktree

# Open or create worktree for branch in a cmux workspace (e.g., .worktrees/feature-foo)
git cmux worktree feature/foo
```

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.
