<h1>orcux</h1>

Run several coding agents at once, each in a git worktree of its own. Ultra fast, ultra light, in the terminal you already use.

![orcux: worktrees with what their agents do on the left, the agent's terminal in the middle, your pull requests on the right; Opt+r straight to the agent that asks, a diff commented line by line and handed to its agent, and its pull request one key away](docs/demo.gif)

- **A worktree per task**: branch, folder, terminal and 100 ports of its own, with one key.
- **See who needs you**: working `●`, asking `?`, failed `!`, done `■`. One key takes you there.
- **Agents go on without you**: close orcux and come back.
- **Bring the work back**: files and diffs, conflicts, merge, and your pull requests beside it.
- **Fast and light**: one binary under 2 MB, no tmux needed.

| 10 worktrees, a terminal each, at rest | orcux | another TUI (a Rust multiplexer) | another app (Electron) | difference |
| --- | --- | --- | --- | --- |
| memory, all its processes | 24 MB | 55 MB | ≈ 1.2 GB | 2.3× and ≈ 49× less |
| CPU at rest | 0.10 % | 0.60 % | 1.4 % | 6× and 14× less |
| list the worktrees from the CLI | 20 ms | 55 ms | 171 ms | 2.8× and 8.6× faster |
| size installed | 2.0 MB | 22.5 MB | 773 MB | 11× and 380× smaller |

Same machine, same 10 worktrees: [how, and 50 terminals](docs/speed.md).

## Install

Ask your coding agent:

```text
Install orcux from https://github.com/borjamrd/orcux with its install script (curl -fsSL https://raw.githubusercontent.com/borjamrd/orcux/master/packaging/install.sh | sh). Make sure ~/.local/bin is in my PATH, then run orcux doctor and tell me what it reports.
```

Or do it yourself:

```sh
curl -fsSL https://raw.githubusercontent.com/borjamrd/orcux/master/packaging/install.sh | sh
```

macOS and Linux. Needs git; [gh](https://cli.github.com) adds your pull requests.

## Use

```sh
cd your-repo
orcux
```

`n` new worktree · `⏎` open it · `Opt+o` every command

More in the [guide](docs/guide.md). [MIT](LICENSE).
