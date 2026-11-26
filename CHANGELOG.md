# Changelog

## 0.2.0

- What agents without hooks do, read from their screen: Antigravity, Cursor, Codex and others now tell when they work, ask or need permission. The tabs of `orcux window` show it too.
- Opt+r: with one agent waiting, straight to it; with more, or with pull requests of yours, a list of what waits for you, oldest first.
- `P` on a worktree, or `orcux pr`: push its branch and open its pull request, after a yes that starts on no.
- The diff of a file (`f`, then Enter) is drawn by orcux: comment lines with `c`, and the comments go to the agent of the worktree as one prompt, without sending it.
- `e` or `orcux rename`: rename a worktree, with its folder and, if it was never pushed, its branch.
- Opt+↑ / Opt+↓, or K / J: move a worktree, repo or folder in the sidebar.
- `orcux skill install`: the skill for every agent, not only Claude Code, in `~/.agents/skills`.
- The sidebar shows the branch of the selected worktree; the README has a line to paste into a coding agent to install orcux, and compares its speed with another TUI and another app.
- After `orcux lang`, the sidebar and the PR panel come back as themselves.

## 0.1.0

- orcux keeps the terminals itself: tmux is not needed. Installs that already used tmux go on with it; `orcux engine` changes it.
- Install with one line: `curl … | sh`, which checks the checksum.

- Worktrees with a branch, a folder, a terminal and 100 ports of their own; nested and imported ones.
- Tabs per worktree, and a switcher by name or pull request.
- What agents do: work, ask, need permission, fail, are done. Claude Code and OpenCode by plugins, Gemini CLI and Codex by their title.
- `orcux run` hands a task to an agent in a worktree of its own, `wait` waits for it, `ls --json` tells how all are, and a skill tells agents about it.
- Notifications, and Opt+r to go to the agent that waits.
- `.orcux`: setup, teardown, shared folders, named ports and variables per repo, used once approved.
- `orcux merge` and `orcux conflicts`. Nothing is pushed.
- After a restart, terminals and conversations come back.
- `f` in the sidebar: the files of a worktree and what changed in each, with its diff.
- Click in the sidebar to select a worktree.
- The logo waves in the commands dialog.
- The interface drawn by orcux itself, with a full frame, clickable tabs, history with the wheel and copy with the mouse.
- Opt+o: every command from any pane. Opt+w shows which worktrees have an agent open.
- Your pull requests, read-only, with what blocks each one.
- A CLI to create worktrees and to type into and read their terminals, safe to run several times at once.
- `orcux doctor`.
- English and Spanish.
