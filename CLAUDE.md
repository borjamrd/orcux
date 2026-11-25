# orcux

A lightweight worktree manager for coding agents, in Rust. It does only three things:
1. worktree management
2. a port block per worktree
3. a CLI to talk between worktrees (create, delete, send/read terminal)

It does not emulate terminals: **ratatui draws the sidebar and tmux runs the terminals**, so agents survive when the UI is closed.

## Commands

- `cargo build --release`: the binary is `target/release/orcux`, linked from `~/.local/bin/orcux`.
- `cargo test`: PR classification and parsing, agent states, language choice, text widths per language.
- `orcux help`: lists the subcommands. The same table (`commands` in `src/i18n/*.rs`) feeds the sidebar's `?` dialog (`orcux commands` in a tmux popup): every new command or shortcut goes there, in every language.

## Architecture (1 dependency: ratatui)

- `src/main.rs`: CLI, git, tmux and state.
- `src/tui.rs`: the sidebar (`orcux sidebar`, the UI's left pane), the PR rail (`orcux rail`, right pane) and the dialogs. Colors come from `src/theme.rs` (design system "orcux"), also for tmux styles.
- `src/i18n/`: every text the user sees, one file per language (`en`, `es`). Never write a user-facing literal elsewhere: add a field to `Text` (a missing translation does not compile) and use `tr().field`; texts with data are functions. Language: `ORCUX_LANG`, then `orcux lang <code>` (saved in `lang`), then the system, then English. The `every_language_fits` test checks widths.
- `src/orc.rs`: the animated orc of the commands dialog (from 144 columns and 25 rows). Pixel layers with lower half blocks `▄`: `▀` leaves stripes in fonts that draw it shorter than the cell.
- `src/agent.rs`: what Claude Code agents say about themselves (working, question, permission, failed, done, idle), through hooks.
- `src/prs.rs`: open PRs of the registered repos, read-only, through `gh` (JSON reduced to TSV with `--jq`, so no JSON crate).

Model:
- tmux session `orcux` holds 1 window: sidebar (36 columns), visible terminal, and the PR rail (44 columns). The rail is shown by default from 140 columns; `p` overrides that in the session option `@orcux_prs`.
- Only the sidebar process opens the rail on its own, fetches PRs (every 60 s, into `~/.local/state/orcux/prs.2.tsv`) and writes the tmux status bar. Session styling is limited to the `orcux` session.
- tmux session `orcux-bg` holds one window per hidden tab. A worktree has one pane per tab: `@orcux_tab` is its number (stable, lowest free) and `@orcux_on=1` marks the active one. The tabs of the visible worktree are drawn in the tmux border above the terminal (`@orcux_bar`). `<wt>:<n>` targets a tab in `show`, `send` and `read`.
- If the visible terminal ends (`exit`), the sidebar shows another of its tabs or the placeholder (`heal`).
- Look: soft fills and gaps (`pane-border-lines spaces`), no lines; rounded outlines only in popups. Badges have rounded ends with `orcux icons nerd` (Powerline glyphs); never half blocks, which Terminal.app draws shorter than the cell.
- Each worktree pane carries the pane option `@orcux=<repo>/<name>`; the sidebar carries `@orcux=__sidebar` and the rail `@orcux=__prs`. Panes are always found through that marker, never by index.
- Showing a worktree = `swap-pane` between its pane and the right-hand one. If the one leaving is the placeholder (empty `@orcux`), it is killed.
- State in plain files in `~/.local/state/orcux/`: `worktrees.tsv` (repo, name, slot), `repos`, `folded` (folders and repos folded in the sidebar), `recent` (ids by last `show`) and `prs.2.tsv` (cache).
- Opt+t/a/d/q: new, previous, next and close tab. Only letters whose Option character is the same on the Spanish and US layouts (Opt+x on Spanish types `∑`, which is Opt+w on US).
- Opt+w opens `orcux switch` in a popup. Terminal.app cannot send Cmd keys, and without Option as Meta it types a character (`æ` on the Spanish layout), so those characters are bound in the root table (`SWITCH_KEYS`, set by the sidebar at start). tmux key tables are global: the binding checks the session name and passes the key on elsewhere. The sidebar marks working panes with `@orcux_busy`, for processes that do not watch screens.
- Agent states: each worktree terminal gets `CLAUDE_CODE_PLUGIN_DIRS` with a plugin that orcux writes in `~/.local/state/orcux/claude-plugin` (the user's `settings.json` is never touched). Its hooks run `orcux hook <state>` (internal), which sets the pane option `@orcux_state="<state> <since>"`. The sidebar corrects what no hook tells (`agent::settle`): answered or rejected question, interrupted turn, result already seen. Glyph of an agent that waits for the user: `?` (`State::Asking`); `◆` is only for PRs. Panes without a mark (other agents, or opened before) fall back to "screen changed in the last 3s".
- Prompts and errors are popups, never sidebar text: the sidebar calls `popup()`, which runs `orcux dialog new|rm|error` (internal) in a centered `display-popup` and blocks until it closes.
- Worktrees go in `~/orcux/<repo>/<name>`. `orcux import` registers existing ones where they are (4th column of `worktrees.tsv`): they are named after their folder, never moved, and `rm`/`d` only unregisters them, even with `--force`. PRs match a worktree by its checked-out branch, read from git's files.
- `worktrees.tsv` columns: repo, name, slot, path if imported, parent's name if it is a child. A child starts at its parent's last commit and is drawn under it; its folder is not inside, and removing the parent moves it up one level. Each pane gets `ORCUX_PORT=10000+slot*100`, `ORCUX_ID`, `ORCUX_SLOT` and `COMPOSE_PROJECT_NAME` via `tmux -e`.
- `.worktreeinclude` in the main checkout is copied into the new worktree (literal paths only, globs skipped).
- New branch from `origin/HEAD` with `--no-track`. If the branch exists locally or on origin, it is reused.

## Gotchas already paid for

- Session targets always use `=` (exact match) and no window index: `=orcux:`. Plain `orcux` prefix-matches `orcux-bg`. The user's `~/.tmux.conf` has `base-index 1`, so `:0` breaks.
- `run()` does `trim_end()`, which eats the trailing `\t` of the last line when `#{@orcux}` is empty. Parse with a default, never with `split_once(...)?`.
- `send` waits 150 ms between the text and Enter: TUIs like claude treat text+Enter in one burst as a paste.
- `display-popup` fails if it is larger than the client (no clamping), so `popup()` caps it to the client size. A detach closes the popup with a non-zero exit and empty stderr: not an error.
- tmux scales panes proportionally when the client attaches or resizes: `ensure_ui` pins the sidebar to 36 columns with a `window-resized` hook (dragging the border by hand still works).
- `orcux dialog view <url>` is the dialog of one PR (Enter or click in the rail): it loads its details in a thread. Read-only towards GitHub. `a` pastes a one-line prompt into the worktree's agent WITHOUT Enter and as a bracketed paste: keys could answer an open permission menu. Text by other people is stripped of control characters and quoted as data.
- GitHub's CI summary (`statusCheckRollup.state`) counts every run, also a cancelled one that was run again, and checks that block nothing. `recheck` asks again about the red PRs: the last run of each check, and only required ones make CI fail (all of them if none is required). A failing optional check is shown in the PR dialog only.
- Conflicting files are not in GitHub's API: both branches are fetched and merged in memory (`git merge-tree`).
- The sidebar and the rail replace themselves (exec) when the binary changes, so a build needs no restart. A mix of versions once made PRs vanish: the old sidebar kept rewriting the cache in its format. Bump the number in `cache_file()` when the format changes.
- A `gh` account that fails keeps its previous PRs. If its token cannot be read it counts as failed: without it gh answers as another account, with no PRs and no error.
- PRs are asked once per logged-in `gh` account (token passed by environment, the active account is never switched) because repos belong to different accounts.
- `w` on a PR fetches its branch before creating: an unknown branch would start from `origin/HEAD`.
- Claude Code has no hook for an interruption (Esc) or a rejected permission: `Stop` does not fire. A working agent whose screen is still for 6 s becomes idle; leaving the mark made typing look like work.
- A `claude -p` run by the agent inherits `TMUX_PANE` and the plugin: `orcux hook` ignores it by `CLAUDE_CODE_ENTRYPOINT` (`sdk-cli`, not `cli`). The hook prints nothing: its output would reach the agent.
- `rm` runs `git worktree remove` BEFORE killing the pane, so that a dirty worktree fails without losing the terminal.

## Testing without touching the user's real tmux/worktrees

Isolated HOME and tmux server. The tmux socket has a ~104-char limit on macOS, so `TMUX_TMPDIR` must be short (e.g. `~/.cache/orcux-test`, deleted at the end):

```sh
unset TMUX TMUX_PANE                       # FIRST, in a subshell: see below
export TMUX_TMPDIR=~/.cache/orcux-test; mkdir -p $TMUX_TMPDIR; export HOME=$(mktemp -d)
case $(tmux display-message -p '#{socket_path}' 2>/dev/null) in ""|*orcux-test*) ;; *) echo 'real tmux server: stop'; exit 1;; esac
git init -q demo && cd demo && echo hi > a && git add a && git -c user.name=t -c user.email=t@t commit -qm i
orcux new feat-a && orcux send feat-a 'echo $ORCUX_PORT' && sleep 1 && orcux read feat-a 3
orcux show feat-a && tmux capture-pane -p -t "$(tmux list-panes -t =orcux: -F '#{pane_id} #{@orcux}' | awk '$2=="__sidebar"{print $1}')"
tmux send-keys -t <sidebar_pane> n; tmux send-keys -t <sidebar_pane> -l feat-b; tmux send-keys -t <sidebar_pane> Enter
tmux kill-server; rm -rf ~/.cache/orcux-test
```

**Never run `tmux kill-server` before `unset TMUX`.** Inside tmux (an agent running in orcux), `$TMUX` wins over `TMUX_TMPDIR`, and so does the default socket if the `TMUX_TMPDIR` folder does not exist: on 2026-11-25 a test killed the user's real server and every terminal in it.

## Known pending items (not done on purpose)

- `orcux ls --json`, for scripts.
- Docker Compose with fixed ports still collides; those ports would have to read `$ORCUX_PORT`.
- Agents other than Claude Code have no hooks: working/idle = "screen changed in the last 3s".
- Globs in `.worktreeinclude` (`git ls-files -oi --exclude-from`).
- A `wait <wt>` command (wait until an agent is idle).

Deliberate simplifications are marked with `ponytail:` comments in the code.
