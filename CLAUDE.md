# orcux

A lightweight worktree manager for coding agents, in Rust. It does only three things:
1. worktree management
2. a port block per worktree
3. a CLI to talk between worktrees (create, delete, send/read terminal)

It does not emulate terminals: **ratatui draws the sidebar and tmux runs the terminals**, so agents survive when the UI is closed.

## Commands

- `cargo build --release`: the binary is `target/release/orcux`, linked from `~/.local/bin/orcux`.
- `cargo test`: PR classification and parsing, agent states, language choice, text widths per language.
- `cargo test` also runs `tests/parallel.rs` (several orcux at once) and `tests/agents.rs` in a tmux of its own (`tests/lab`); without tmux it is skipped.
- `cargo test --release --test perf -- --ignored --nocapture`: what 50 terminals cost, in a tmux of its own. Limits: no tmux process at rest, 0.5 % of a core per open pane, `orcux ls` under 50 ms, `orcux show` under 100 ms, start under 1 s. Run it after touching the loops of `tui.rs` or `control.rs`.
- `orcux doctor` (`src/doctor.rs`): checks tmux, git, gh, the locale, the state and broken links of worktrees. tmux 3.3 is the oldest that could work (popup styles), untested below 3.6; options that an older one lacks are skipped.
- `packaging/`: `install.sh` (checks the checksum) and `formula.sh` (Homebrew). `.github/workflows`: `ci.yml` (macOS and Linux) and `release.yml` (a tag `v*` builds four binaries and leaves a draft).
- `docs/demo.gif` is made of real screens of an isolated orcux; before recording, set a neutral prompt: the default one shows the user and the computer.
- `orcux help`: lists the subcommands. The same table (`commands` in `src/i18n/*.rs`) feeds the sidebar's `?` dialog (`orcux commands` in a tmux popup): every new command or shortcut goes there, in every language.

## Architecture (1 dependency: ratatui)

- `src/main.rs`: CLI, git, tmux and state.
- `src/control.rs`: a lasting connection to tmux (control mode) for the sidebar, the rail and the placeholder. `tmux()` goes through it when there is one; commands that ask many things (`show`, `tab`, `switch`) borrow one; elsewhere, and with an old tmux, it is a process.
- `src/tui.rs`: the sidebar (`orcux sidebar`, the UI's left pane), the PR rail (`orcux rail`, right pane) and the dialogs. Colors come from `src/theme.rs` (design system "orcux"), also for tmux styles.
- `src/i18n/`: every text the user sees, one file per language (`en`, `es`). Never write a user-facing literal elsewhere: add a field to `Text` (a missing translation does not compile) and use `tr().field`; texts with data are functions. Language: `ORCUX_LANG`, then `orcux lang <code>` (saved in `lang`), then the system, then English. The `every_language_fits` test checks widths.
- `src/orc.rs`: the animated orc of the commands dialog, at its left (from 134 columns and 22 rows): only the bust, without a background.
- `src/mark.rs`: the logo, the "o" of Fraunces Bold with an "x" as subscript, in braille dots with a gradient (`DEEP`, `CURRENT`, `FOAM`). Shown by `orcux welcome` (internal), the placeholder pane when no worktree is shown: it passes its keys to the sidebar.
- `src/pixels.rs`: draws the orc, in shades of the text color, with lower half blocks `▄`: `▀` leaves stripes in fonts that draw it shorter than the cell.
- `src/agent.rs`: what agents say about themselves (working, question, permission, failed, done, idle): Claude Code through hooks, OpenCode through `src/opencode.js`, Gemini CLI and Codex by the title of their terminal (documented, not tested with the real ones).
- `src/config.rs`: `.orcux` in the main checkout (setup, teardown, link, port, env). Used only as approved with `orcux trust`, kept whole in `trusted/`, never through a hash. `trust` asks at a terminal.
- `src/merge.rs`: `merge` (merge-tree first; merges exactly the commit it checked, then removes; never pushes) and `conflicts` (snapshots of uncommitted work through a temporary index: the agent's index is not touched).
- `src/restore.rs`: the sidebar keeps `open.tsv` (tabs and conversations, with the id of the tmux server); with another server, they go to `lost.tsv` for `restore`. Conversations come from `@orcux_session`, `@orcux_model` and `@orcux_mode`, set from hook fields only: the command line can hold the text of a prompt.
- `src/work.rs`: `run`, `agents`, `wait`, `next`, `ls --json` and notifications. `src/skill.md` is the skill, loaded with the Claude plugin.
- `src/prs.rs`: open PRs of the registered repos, read-only, through `gh` (JSON reduced to TSV with `--jq`, so no JSON crate).

Model:
- tmux session `orcux` holds 1 window: sidebar (36 columns), visible terminal, and the PR rail (44 columns). The rail is shown by default from 140 columns; `p` overrides that in the session option `@orcux_prs`.
- Only the sidebar process opens the rail on its own, fetches PRs (every 60 s, or 10 min if nobody has the session open, into `~/.local/state/orcux/prs.2.tsv`) and writes the tmux status bar. Session styling is limited to the `orcux` session.
- tmux session `orcux-bg` holds one window per hidden tab. A worktree has one pane per tab: `@orcux_tab` is its number (stable, lowest free) and `@orcux_on=1` marks the active one. The tabs of the visible worktree are drawn in the tmux border above the terminal (`@orcux_bar`). `<wt>:<n>` targets a tab in `show`, `send` and `read`.
- If the visible terminal ends (`exit`), the sidebar shows another of its tabs or the placeholder (`heal`).
- Logo: `⢎⡱ orcux` in the tmux border above the sidebar (an "o" of braille dots). A terminal cannot choose fonts.
- Look: soft fills and gaps (`pane-border-lines spaces`), no lines; rounded outlines only in popups. Badges have rounded ends with `orcux icons nerd` (Powerline glyphs); never half blocks, which Terminal.app draws shorter than the cell.
- Each worktree pane carries the pane option `@orcux=<repo>/<name>`; the sidebar carries `@orcux=__sidebar` and the rail `@orcux=__prs`. Panes are always found through that marker, never by index.
- Showing a worktree = `swap-pane` between its pane and the right-hand one. If the one leaving is the placeholder (empty `@orcux`), it is killed.
- State in plain files in `~/.local/state/orcux/`: `worktrees.tsv` (repo, name, slot), `repos`, `folded` (folders and repos folded in the sidebar), `recent` (ids by last `show`) and `prs.2.tsv` (cache).
- Opt+t/a/d/q: new, previous, next and close tab. Opt+r: the agent that waits. Only letters whose Option character is the same on the Spanish and US layouts (Opt+x on Spanish types `∑`, which is Opt+w on US).
- Opt+w opens `orcux switch` in a popup. Terminal.app cannot send Cmd keys, and without Option as Meta it types a character (`æ` on the Spanish layout), so those characters are bound in the root table (`SWITCH_KEYS`, set by the sidebar at start). tmux key tables are global: the binding checks the session name and passes the key on elsewhere. The sidebar marks working panes with `@orcux_busy`, for processes that do not watch screens.
- Agent states: each worktree terminal gets `CLAUDE_CODE_PLUGIN_DIRS` with a plugin that orcux writes in `~/.local/state/orcux/claude-plugin` (the user's `settings.json` is never touched). Its hooks run `orcux hook <state>` (internal), which sets the pane option `@orcux_state="<state> <since> [exact|title]"`: no third word is Claude, whose gaps `settle` fills; `exact` (OpenCode, through `OPENCODE_CONFIG_CONTENT`, never `OPENCODE_CONFIG_DIR`, where it installs packages) and `title` are believed. The sidebar corrects what no hook tells (`agent::settle`): answered or rejected question, interrupted turn, result already seen. Glyphs of an agent: `?` waits for the user, `!` failed, `■` is done (`State::Asking`, `Broken`, `Finished`); `◆`, `✕` and `✓` are only for PRs, because both are drawn on one row. Panes without a mark (other agents, or opened before) fall back to "something printed in the last 3s": the activity of their window if hidden, the captured screen if visible.
- Prompts and errors are popups, never sidebar text: the sidebar calls `popup()`, which runs `orcux dialog new|rm|error` (internal) in a centered `display-popup` and blocks until it closes.
- Worktrees go in `~/orcux/<repo>/<name>`. `orcux import` registers existing ones where they are (4th column of `worktrees.tsv`): they are named after their folder, never moved, and `rm`/`d` only unregisters them, even with `--force`. PRs match a worktree by its checked-out branch, read from git's files.
- `worktrees.tsv` columns: repo, name, slot, path if imported, parent's name if it is a child. A child starts at its parent's last commit and is drawn under it; its folder is not inside, and removing the parent moves it up one level. Each pane gets `ORCUX_PORT=10000+slot*100`, `ORCUX_ID`, `ORCUX_SLOT` and `COMPOSE_PROJECT_NAME` via `tmux -e`.
- `.worktreeinclude` in the main checkout is copied into the new worktree (literal paths only, globs skipped).
- New branch from `origin/HEAD` with `--no-track`. If the branch exists locally or on origin, it is reused.

## Gotchas already paid for

- Inside a tmux conditional (`#{?…}`) a comma in a style breaks the whole format, without an error: use two `#[…]` or `#,`.
- Speed: the panes that stay open never poll. tmux tells what changes (`refresh-client -B`, once a second, and `%window-pane-changed`) and state files are looked at by date. A hidden terminal is not captured, only dated (`window_activity`). The visible one shares its window with the sidebar and the rail, whose own drawing must not count as news: it would redraw for ever.
- The connections are clients of the session `orcux` too: whoever needs the person's client (popups, detach, "is somebody looking") must ask `viewer()`. They attach with `ignore-size`, or they would resize the window.
- A command through the connection can answer in several blocks (`if-shell`), so each one is followed by a marker. Commands with control characters, popups and those about the current client go as processes.
- Several agents run orcux at once: state changes go through `change()` / `lock()`, and tmux changes that read and then act (`spawn`, `ensure_ui`, `show_tab`, `close_tab`) take `lock()` too. Never hold it while git works. A worktree is registered before git creates it, so its name and ports are taken. State files are replaced whole, and one that cannot be read is an error, not an empty list.
- `swap-pane` gives a pane the window of another one, with the activity of that window: without remembering since when a pane is hidden, a working agent looked idle.
- `rm` refuses a worktree that holds another one: git would delete it with everything inside.
- A new terminal with something to run is marked `working <t> starting` (its `.orcux` setup) or `launched`, so that `wait` does not see "no agent" before the agent tells; they expire (30 min, 15 s).
- `remove` checks everything that would stop git (changes, lock, submodules) before the teardown: a teardown may delete a database.
- `run` types `agent "$(cat file)"`: the task goes in a file of mode 0600, so nothing in it means anything to the shell, and aliases still work.
- The plugin of OpenCode starts `orcux hook` one after another: two at once could arrive in the wrong order.
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

- Docker Compose with fixed ports still collides; those ports would have to read `$ORCUX_PORT`.
- Cursor and Antigravity tell nothing that orcux can read without changing the user's files: working = their screen moves.
- Globs in `.worktreeinclude` (`git ls-files -oi --exclude-from`).

Deliberate simplifications are marked with `ponytail:` comments in the code.
