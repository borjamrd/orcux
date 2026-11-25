# orcux

A lightweight alternative to Orca (orca.dev) in Rust. Of Orca it keeps only three things:
1. worktree management
2. a port block per worktree
3. a CLI to talk between worktrees (create, delete, send/read terminal)

It looks like herdr (herdr.dev: Rust + ratatui + its own VT), but orcux has worktrees and does not emulate terminals. **ratatui draws the sidebar and tmux runs the terminals**, so agents survive when the UI is closed.

## Commands

- `cargo build --release`: the binary is `target/release/orcux`, linked from `~/.local/bin/orcux`.
- `orcux help`: lists the subcommands. Same table (`COMMANDS` in `main.rs`) feeds the sidebar's `?` dialog (`orcux commands` in a tmux popup): every new command or shortcut goes there. UI text is in Spanish for now.

## Architecture (2 files, 1 dependency: ratatui)

- `src/main.rs`: CLI, git, tmux and state.
- `src/tui.rs`: the sidebar (`orcux sidebar`, run inside the UI's left pane).

Model:
- tmux session `orcux` holds 1 window: the sidebar pane on the left and the visible terminal on the right.
- tmux session `orcux-bg` holds one window per worktree, with its terminal.
- Each worktree pane carries the pane option `@orcux=<repo>/<name>`; the sidebar carries `@orcux=__sidebar`. Panes are always found through that marker, never by index.
- Showing a worktree = `swap-pane` between its pane and the right-hand one. If the one leaving is the placeholder (empty `@orcux`), it is killed.
- State in plain files: `~/.local/state/orcux/worktrees.tsv` (repo, name, slot) and `~/.local/state/orcux/repos`.
- Prompts and errors are popups, never sidebar text: the sidebar calls `popup()`, which runs `orcux dialog new|rm|error` (internal) in a centered `display-popup` and blocks until it closes.
- Worktrees go in `~/orcux/<repo>/<name>`. Each pane gets `ORCUX_PORT=10000+slot*100`, `ORCUX_ID`, `ORCUX_SLOT` and `COMPOSE_PROJECT_NAME` via `tmux -e`.
- `.worktreeinclude` in the main checkout is copied into the new worktree (same contract as Orca; literal paths only, globs skipped).
- New branch from `origin/HEAD` with `--no-track`. If the branch exists locally or on origin, it is reused.

## Gotchas already paid for

- Session targets always use `=` (exact match) and no window index: `=orcux:`. Plain `orcux` prefix-matches `orcux-bg`. The user's `~/.tmux.conf` has `base-index 1`, so `:0` breaks.
- `run()` does `trim_end()`, which eats the trailing `\t` of the last line when `#{@orcux}` is empty. Parse with a default, never with `split_once(...)?`.
- `send` waits 150 ms between the text and Enter: TUIs like claude treat text+Enter in one burst as a paste.
- `display-popup` fails if it is larger than the client (no clamping), so `popup()` caps it to the client size. A detach closes the popup with a non-zero exit and empty stderr: not an error.
- `rm` runs `git worktree remove` BEFORE killing the pane, so that a dirty worktree fails without losing the terminal.

## Testing without touching the user's real tmux/worktrees

Isolated HOME and tmux server. The tmux socket has a ~104-char limit on macOS, so `TMUX_TMPDIR` must be short (e.g. `~/.cache/orcux-test`, deleted at the end):

```sh
export HOME=$(mktemp -d) TMUX_TMPDIR=~/.cache/orcux-test; unset TMUX   # in a subshell
git init -q demo && cd demo && echo hi > a && git add a && git -c user.name=t -c user.email=t@t commit -qm i
orcux new feat-a && orcux send feat-a 'echo $ORCUX_PORT' && sleep 1 && orcux read feat-a 3
orcux show feat-a && tmux capture-pane -p -t "$(tmux list-panes -t =orcux: -F '#{pane_id} #{@orcux}' | awk '$2=="__sidebar"{print $1}')"
tmux send-keys -t <sidebar_pane> n; tmux send-keys -t <sidebar_pane> -l feat-b; tmux send-keys -t <sidebar_pane> Enter
tmux kill-server; rm -rf ~/.cache/orcux-test
```

## Known pending items (not done on purpose)

- `orcux ls --json`, for `~/orquestador/estado.py`, which today reads `orca worktree ps --json`.
- Docker Compose with fixed ports (fever2: 8010/8012) still collides; those ports would have to read `$ORCUX_PORT`.
- Working/idle status = "screen changed in the last 3s" (heuristic). Upgrade: Claude Code hooks.
- Globs in `.worktreeinclude` (`git ls-files -oi --exclude-from`).
- Importing existing Orca worktrees (`~/orca/workspaces`).
- A `wait <wt>` command (wait until an agent is idle), like Orca/herdr.

Deliberate simplifications are marked with `ponytail:` comments in the code.
