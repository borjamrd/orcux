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

## How it stays this way

- Nothing polls: the server says what changed, and state files are looked at by their date.
- A hidden terminal is not drawn, only kept.
- One thread with `poll()` holds every terminal in the engine; no thread per terminal.
- Two dependencies (ratatui, vt100) and libc.
- A change to the loops of the interface or the engine runs this test before it is merged.
