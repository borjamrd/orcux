# Speed

orcux is meant to be ultra fast and ultra light: it waits without a timer, starts no processes while nothing changes, and answers in milliseconds. These numbers come from its own test, which anyone can run:

```sh
cargo test --release --test perf -- --ignored --nocapture                       # on tmux
ORCUX_ENGINE=own cargo test --release --test perf -- --ignored --nocapture      # on orcux's own engine
```

It opens 50 terminals (10 worktrees with 5 tabs each), starts the interface, waits 8 seconds and then watches for 20. CI runs it on every push, on macOS and Linux, and puts the numbers in the summary of each run; there the machines are shared, so times get six times the room, but processes at rest must stay at zero.

## 50 terminals, on Apple M4 Max, orcux 0.1.0

| | orcux's own engine | on tmux |
| --- | --- | --- |
| `orcux ls` | 14 ms | 41 ms |
| `orcux show` (switch worktree) | 24 ms | 33 ms |
| start of the interface | 0.4 s | 0.3 s |
| CPU at rest: the server that keeps the terminals | 0.05 % | 0.90 % |
| CPU at rest: window, sidebar, PR panel | 0.00 %, 0.05 %, 0.05 % | 0.00 %, 0.05 %, 0.10 % |
| memory: server, window, sidebar, PR panel | 7.8, 4.3, 4.0, 3.2 MB | 5.6, 4.5, 4.2, 3.3 MB |
| processes started at rest, in 20 s | 0 | 0 |
| binary | 1.9 MB | 1.9 MB |

CPU is a share of one core. At rest, all of orcux with 50 terminals costs under 0.2 % of one core and under 20 MB.

## Against other tools

The same work on each, on the same Apple M4 Max: one repo, 10 worktrees with a terminal each (an idle shell), the interface open, then 8 seconds to settle and 20 to watch.

| | orcux 0.1 (own engine) | another TUI: a terminal multiplexer for agents, in Rust | another app: a desktop app for agents, on Electron |
| --- | --- | --- | --- |
| memory, every process of it (not the shells) | 24 MB, 7 processes | 55 MB, 2 processes | ≈ 1.2 GB (see below) |
| CPU at rest, every process of it | 0.10 % | 0.60 % | 1.4 % |
| list the worktrees from its CLI (median of 11) | 20 ms | 55 ms | 171 ms |
| size installed | 2.0 MB (one binary) | 22.5 MB (one binary) | 773 MB (the app) |

The TUI ran in a home of its own, like orcux. The app was the copy that was already open, with projects of its own: 1.1 GB for the app itself plus 7 MB per terminal (each one gets a process of its own), so ≈ 1.2 GB with 10 terminals; its CPU was measured twice over 60 s. CPU is a share of one core.

## How it stays this way

- Nothing polls: the server says what changed, and state files are looked at by their date.
- A hidden terminal is not drawn, only kept.
- One thread with `poll()` holds every terminal in the engine; no thread per terminal.
- Two dependencies (ratatui, vt100) and libc.
- A change to the loops of the interface or the engine runs this test before it is merged.
