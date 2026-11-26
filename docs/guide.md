# orcux guide

Everything beyond the [README](../README.md). `orcux help` and `Opt+o` list every command and key.

## Keys

| Key | |
| --- | --- |
| `n` / `N` | new worktree / new one from the selected one |
| `⏎` | open the selected worktree |
| `f` | the files of the selected worktree and their changes; Enter shows a diff, where `c` comments a line and the comments go to its agent on closing, without sending them |
| `m` | merge the selected worktree and delete it |
| `e` | rename the selected worktree |
| `a` | the agent that `orcux run` starts: one that orcux knows, or your own command |
| `P` | push its branch and open its pull request |
| `Opt+↑` / `Opt+↓`, `K` / `J` | move the selected worktree, repo or folder |
| `Opt+w` | switch worktree, by name or pull request |
| `Opt+r` | what waits for you: with one agent, straight to it; with more, or with pull requests of yours, the list (oldest first) |
| `Opt+t`, `Opt+a`, `Opt+d`, `Opt+q` | new, previous, next and close tab |
| `Opt+o` | every command and key (`?` in the sidebar) |
| `tab` | go to the pull requests |
| `q` | leave; the terminals stay alive |

| Mouse | |
| --- | --- |
| click | select a row, or the pane you work in; again on a row, open it |
| click on a tab, on `+` | show that tab, open a new one |
| wheel | the terminal's history; any key goes back |
| drag | select and copy; with Shift in programs that use the mouse |

## The first time

`orcux` inside a repo adds it as your first project, and the empty screen lists the first steps, each with its key: add a project (`A`), choose your agent (`a`), teach orcux to every agent (`s`), create your first worktree (`n`). Each one is ticked by what is there, and the list goes away with your first worktree, or with `x`.

## What runs the terminals

orcux keeps the terminals alive in a small server of its own: close orcux, or lose the connection, and come back to them. If you already used orcux with tmux, it goes on with tmux; `orcux engine own` or `orcux engine tmux` changes it. With tmux there is also the classic interface, drawn by tmux: `orcux ui classic`.

On a Mac the server starts as a service of macOS, so that the terminals outlive the app that opened orcux. So when a program in one of them asks for something macOS guards (your photos, contacts, the camera…), the question comes in the name of orcux, as it would come in the name of your terminal app without orcux. orcux itself asks for none of that: answer as you would for that program. What you allowed or not is under System Settings › Privacy & Security, as orcux.

## On another machine

```sh
orcux ssh my-server        # as for ssh: a host, user@host or an alias of ~/.ssh/config
```

orcux runs on that machine, in this terminal: its worktrees, agents and ports are there. Close the connection, or lose it, and they go on; the same command comes back to them. If orcux is not there yet, it offers to install it with its install script. Over SSH a click on a web address copies it to this computer's clipboard instead of opening a browser on the other one.

## For agents and scripts

```sh
orcux run "fix the login redirect, with a test"  # worktree, ports, and an agent at that task
orcux wait fix-the-login-redirect --timeout 600  # until it stops working
orcux read fix-the-login-redirect 40             # the end of its terminal
orcux send fix-the-login-redirect "go on"        # types it, with Enter
orcux ls --json                                  # every worktree and what its agents do
orcux rm fix-the-login-redirect                  # keeps the branch
orcux rename fix-the-login-redirect login        # its folder and, if never pushed, its branch
```

A rename waits until only shells run in its terminals (they start again in the new folder): an agent would go on in the old path. What its `.orcux` made with the old name, such as a database, keeps it.

`wait` ends with 0 when the agent is done, 2 when it asks something or needs permission, 3 when it failed, 4 on timeout and 5 when there is no agent. Several commands can run at once.

Claude Code agents get this as a skill in the terminals of orcux. For the others (Antigravity, Codex, Gemini CLI…), `orcux skill install` puts it in `~/.agents/skills/orcux/`, where they look, and orcux keeps it up to date; `orcux skill remove` takes it away. `orcux skill` prints it.

## Agents

`orcux run` knows Claude Code, Codex, Gemini CLI, OpenCode and Cursor. `orcux agents` lists them, `orcux agents use codex` changes the default and `orcux agents set mine "my-agent --task {prompt}"` adds or replaces one.

| What they do is known | |
| --- | --- |
| Claude Code | from its hooks |
| OpenCode | from a plugin, exactly |
| Antigravity | from its screen: when it works, asks permission or a question |
| Gemini CLI, Codex, Cursor | from their title and their screen, as documented; not tested with the real ones yet |
| others | from their screen if it shows the usual signs (`esc to interrupt`, `[y/N]`…), else only whether it moves |

Nothing in your settings is changed. `orcux notify off` turns notifications off; `all` also tells when an agent is done.

## Each repo, its needs

A `.orcux` file in the main checkout says what its worktrees need:

```
setup = npm ci
link = node_modules
port DB_PORT = 1
env DATABASE_URL = postgres://localhost:${DB_PORT}/${ORCUX_NAME}
compose = docker-compose.yml
teardown = docker compose down -v
```

`setup` runs in the terminal of every new worktree, `teardown` before one is deleted, `link` shares a folder of the main checkout, `port` names one of its 100 ports and `env` adds variables. `compose` lets several worktrees run the same Docker Compose file at once: each one publishes its ports on its own block (from `ORCUX_PORT` + 50, so 5432 becomes 10150 in one and 10250 in the next), through an override that `COMPOSE_FILE` names, so plain `docker compose up` works in its terminals; `PORT_<SERVICE>_<PORT>` tells where each one went (`${PORT_DB_5432}` in an `env` line). It needs Compose 2.24 or later. It can run commands, so nothing in it is used until you read and approve it with `orcux trust`, and again whenever it changes. `orcux ports` lists the ports and which are in use.

## Ports

Worktrees share your computer, so two servers on port 3000 collide. Each terminal gets its own block:

| Variable | |
| --- | --- |
| `ORCUX_PORT` | the first of 100 ports: 10100, 10200… |
| `ORCUX_SLOT`, `ORCUX_ID` | the number and the name of the worktree |
| `COMPOSE_PROJECT_NAME` | so that Docker Compose keeps them apart |

Use them in your scripts: `vite --port $ORCUX_PORT`. Files that git ignores and you need in every worktree, such as `.env`, go in a `.worktreeinclude`: paths, or patterns as in `.gitignore` (`*.env`, `config/**/local.*`, `!keep.env`), copied when the worktree is made.

## Bringing work back

```sh
orcux conflicts                  # which worktrees clash, also with uncommitted work
orcux merge fix-login --check    # what would happen
orcux merge fix-login            # merges its branch, then deletes the worktree and the branch
```

A child goes into its parent, the others into the main branch. Nothing is pushed. It refuses when there are uncommitted changes, when its agent works or when its pull request is open.

## After a restart

orcux remembers which terminals were open and which conversations their agents had. Press `R` on the empty screen, or run `orcux restore`, and Claude Code, OpenCode and Antigravity go on with their conversation; Codex and Gemini CLI with the last one of their folder.

## Pull requests

With [gh](https://cli.github.com) logged in, the right panel shows your open pull requests: whose turn it is, what fails, and a key to hand it to the agent.

`P` on a worktree, or `orcux pr <wt>`, pushes its branch and opens its pull request: into its parent's branch for a child already on GitHub, else into the main one. Its title and description come from its commits (one commit: its message; several: the branch name and their list), as the account of gh that can push there. It asks first, starting on no; one already open is shown instead. `orcux pr <wt> --check` tells what it would open.

## Languages

English and Spanish, after your system or with `orcux lang`.

## Development

```sh
cargo test                                                  # on tmux
ORCUX_ENGINE=own cargo test                                 # on orcux's own engine
cargo test --release --test perf -- --ignored --nocapture   # what 50 terminals cost: docs/speed.md
```

Tests use servers of their own, never yours.
