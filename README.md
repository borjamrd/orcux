<h1>orcux</h1>

Run several coding agents at once, each in a git worktree of its own. Ultra fast, ultra light, in the terminal you already use.

![orcux: worktrees on the left, the terminal of an agent in the middle, pull requests on the right](docs/demo.gif)

- **A worktree per task**: branch, folder, terminal and 100 ports of its own, with one key.
- **See who needs you**: working `●`, asking `?`, failed `!`, done `■`. One key takes you there.
- **Agents go on without you**: close orcux and come back.
- **Bring the work back**: files and diffs, conflicts, merge, and your pull requests beside it.
- **Fast and light**: one binary under 2 MB, no tmux needed.

| 50 terminals open | |
| --- | --- |
| `orcux ls` | 14 ms |
| switch worktree | 24 ms |
| CPU at rest, all of orcux | under 0.2 % of one core |
| memory, all of orcux | under 20 MB |

Measured by its own test, on every push: [how](docs/speed.md).

## Install

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
