<h1>orcux</h1>

Git worktrees, ports and terminals for several coding agents at once, in the terminal you already use.

![orcux: worktrees on the left, the terminal of an agent in the middle, pull requests on the right](docs/demo.gif)

- **A worktree per task.** One key creates the branch, the folder, the terminal and a block of 100 ports of its own.
- **You see who needs you.** Agents tell when they work (`●`), ask or need permission (`?`), fail (`!`) or are done (`■`), the system notifies you, and one key takes you there. Nothing in your settings is changed.
- **Agents go on without you.** tmux runs the terminals: close orcux, or lose the connection, and come back.
- **Your pull requests beside them**, read-only: whose turn it is, what fails, and a key to hand it to the agent.
- **Agents that hand over work.** A CLI and a skill: an agent can start others in worktrees of their own, wait for them and read what they did.
- **Fast.** One binary of about 1 MB. With 50 terminals at rest it starts no processes and uses about 0.1 % of a core per pane.

## Install

You need [tmux](https://github.com/tmux/tmux) 3.6 and git 2.38, or later. tmux 3.3 to 3.5 should work, with lines between panes; it is not tested. [gh](https://cli.github.com) is optional: it shows your pull requests.

From the source:

```sh
cargo install --git https://github.com/borjamrd/orcux
orcux doctor   # checks what orcux needs
```

macOS and Linux.

## Start

```sh
cd your-repo
orcux repo add
orcux
```

Press `n` to create a worktree, `⏎` to open it, and start your agent there. `?` lists everything else.

| Key | |
| --- | --- |
| `n` / `N` | new worktree / new one from the selected one |
| `⏎` | open the selected worktree |
| `Opt+w` | switch worktree, by name or pull request |
| `Opt+r` | go to the agent that waits for you |
| `Opt+t`, `Opt+a`, `Opt+d`, `Opt+q` | new, previous, next and close tab |
| `tab` | go to the pull requests |
| `q` | leave; the terminals stay alive |

## For agents and scripts

```sh
orcux run "fix the login redirect, with a test"  # worktree, ports, and an agent at that task
orcux wait fix-the-login-redirect --timeout 600  # until it stops working
orcux read fix-the-login-redirect 40             # the end of its terminal
orcux send fix-the-login-redirect "go on"        # types it, with Enter
orcux ls --json                                  # every worktree and what its agents do
orcux rm fix-the-login-redirect                  # keeps the branch
```

`wait` ends with 0 when the agent is done, 2 when it asks something or needs permission, 3 when it failed, 4 on timeout and 5 when there is no agent. Several commands can run at once. `orcux help` lists them all.

Claude Code agents get this as a skill in the terminals of orcux. For others, `orcux skill` prints it.

## Agents

`orcux run` knows Claude Code, Codex, Gemini CLI, OpenCode and Cursor. `orcux agents` lists them, `orcux agents use codex` changes the default and `orcux agents set mine "my-agent --task {prompt}"` adds or replaces one.

| What they do is known | |
| --- | --- |
| Claude Code | from its hooks |
| OpenCode | from a plugin, exactly |
| Gemini CLI, Codex | from the title of their terminal, as documented; not tested with the real ones yet |
| others | only whether their screen moves |

`orcux notify off` turns notifications off; `all` also tells when an agent is done.

## A window drawn by orcux (preview)

`orcux window` opens the same interface, drawn by orcux itself instead of tmux: the terminal in a full frame, tabs you can click, dialogs over everything, the wheel for history and the mouse to select and copy. tmux keeps running the terminals, so `orcux` (the classic interface) and `orcux window` can be opened at any time, and agents go on when either is closed. `q` in the sidebar closes it.

## Each repo, its needs

A `.orcux` file in the main checkout says what its worktrees need:

```
setup = npm ci
link = node_modules
port DB_PORT = 1
env DATABASE_URL = postgres://localhost:${DB_PORT}/${ORCUX_NAME}
teardown = docker compose down -v
```

`setup` runs in the terminal of every new worktree, `teardown` before one is deleted, `link` shares a folder of the main checkout, `port` names one of its 100 ports and `env` adds variables. It can run commands, so nothing in it is used until you read and approve it with `orcux trust`, and again whenever it changes. `orcux ports` lists the ports and which are in use.

## Bringing work back

```sh
orcux conflicts                  # which worktrees clash, also with uncommitted work
orcux merge fix-login --check    # what would happen
orcux merge fix-login            # merges its branch, then deletes the worktree and the branch
```

A child goes into its parent, the others into the main branch. Nothing is pushed. It refuses when there are uncommitted changes, when its agent works or when its pull request is open. `m` does it from the sidebar.

## After a restart

If tmux ends, orcux remembers which terminals were open and which conversations their agents had. Press `R` on the empty screen, or run `orcux restore`, and Claude Code and OpenCode go on where they were.

## Ports

Worktrees share your computer, so two servers on port 3000 collide. Each terminal gets its own block:

| Variable | |
| --- | --- |
| `ORCUX_PORT` | the first of 100 ports: 10100, 10200… |
| `ORCUX_SLOT`, `ORCUX_ID` | the number and the name of the worktree |
| `COMPOSE_PROJECT_NAME` | so that Docker Compose keeps them apart |

Use them in your scripts: `vite --port $ORCUX_PORT`. Files that git ignores and you need in every worktree, such as `.env`, go in a `.worktreeinclude`.

## Languages

English and Spanish, after your system or with `orcux lang`.

## License

[MIT](LICENSE)

## Development

```sh
cargo test
cargo test --release --test perf -- --ignored --nocapture   # what 50 terminals cost
```

Tests use a tmux server of their own, never yours.
