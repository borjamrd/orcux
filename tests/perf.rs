//! What orcux costs with 50 terminals open, against a tmux of its own: never the one of the person who runs it.
//! Slow, so it is asked for: `cargo test --release --test perf -- --ignored --nocapture`.
mod lab;

use lab::Lab;
use std::{
    fs,
    process::Command,
    thread::sleep,
    time::{Duration, Instant},
};

const TERMINALS: usize = 50;
const WATCH: u64 = 20;
/// Share of one core for each pane that stays open.
const CPU: f64 = 0.005;

/// Seconds of CPU used so far.
fn cpu(pid: u32) -> f64 {
    if let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) {
        let f: Vec<f64> = stat.rsplit(')').next().unwrap().split_whitespace().map(|x| x.parse().unwrap_or(0.0)).collect();
        return (f[11] + f[12]) / 100.0;
    }
    let out = Command::new("ps").env("LC_ALL", "C").args(["-o", "time=", "-p", &pid.to_string()]).output().unwrap().stdout;
    String::from_utf8_lossy(&out).trim().split(':').fold(0.0, |all, x| all * 60.0 + x.parse::<f64>().unwrap())
}

/// How many times slower than a computer of one's own this one may be.
fn room() -> u32 {
    std::env::var("ORCUX_PERF_ROOM").ok().and_then(|r| r.parse().ok()).unwrap_or(1)
}

#[test]
#[ignore]
fn fifty_terminals_cost_nothing_at_rest() {
    let lab = Lab::new();
    lab.run("git", &["init", "-q", "."]);
    lab.run("git", &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "i"]);
    for i in 0..10 {
        lab.orcux(&["new", &format!("w{i}")]);
        for _ in 1..TERMINALS / 10 {
            lab.orcux(&["tab", "new", &format!("w{i}")]);
        }
    }

    let started = Instant::now();
    let app = format!("env -u TMUX '{}'", env!("CARGO_BIN_EXE_orcux"));
    lab.tmux(&["-L", "outer", "-f", "/dev/null", "new-session", "-d", "-s", "o", "-x", "160", "-y", "42", &app]);
    while !lab.tmux(&["-L", "outer", "capture-pane", "-p", "-t", "o"]).contains("w9") {
        assert!(started.elapsed().as_secs() < 10, "it did not start");
        sleep(Duration::from_millis(20));
    }
    let start = started.elapsed();
    let panes = lab.tmux(&["list-panes", "-a"]).lines().count();
    assert!(panes >= TERMINALS, "{panes} panes");

    sleep(Duration::from_secs(8));
    let pids = [lab.pid("__sidebar"), lab.pid("__prs")];
    let before = pids.map(cpu);
    fs::remove_file(lab.dir.join("log")).ok();
    sleep(Duration::from_secs(WATCH));
    let calls = fs::read_to_string(lab.dir.join("log")).unwrap_or_default();
    let used: Vec<f64> = pids.iter().zip(before).map(|(p, b)| (cpu(*p) - b) / WATCH as f64).collect();

    let mut ls: Vec<Duration> = (0..9).map(|_| { let t = Instant::now(); lab.orcux(&["ls"]); t.elapsed() }).collect();
    ls.sort();
    let mut show: Vec<Duration> = (0..9).map(|i| { let t = Instant::now(); lab.orcux(&["show", &format!("w{i}")]); t.elapsed() }).collect();
    show.sort();
    println!("{panes} panes, `orcux show` {:?}, start {start:?}, `orcux ls` {:?}, CPU sidebar {:.2} % rail {:.2} %, {} tmux processes in {WATCH} s", show[4], ls[4], used[0] * 100.0, used[1] * 100.0, calls.lines().count());
    assert!(calls.is_empty(), "tmux processes at rest:\n{calls}");
    assert!(used.iter().all(|u| *u <= CPU * f64::from(room())), "CPU at rest: {used:?}");
    assert!(ls[4] < Duration::from_millis(50) * room(), "`orcux ls` took {:?}", ls[4]);
    assert!(show[4] < Duration::from_millis(100) * room(), "`orcux show` took {:?}", show[4]);
    assert!(start < Duration::from_secs(1) * room(), "it took {start:?} to start");
}

#[test]
#[ignore]
fn the_window_costs_nothing_at_rest() {
    let lab = Lab::new();
    lab.run("git", &["init", "-q", "."]);
    lab.run("git", &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "i"]);
    for i in 0..10 {
        lab.orcux(&["new", &format!("w{i}")]);
        for _ in 1..TERMINALS / 10 {
            lab.orcux(&["tab", "new", &format!("w{i}")]);
        }
    }
    let started = Instant::now();
    // another session of the same server plays the person's terminal
    lab.tmux(&["new-session", "-d", "-s", "outer", "-x", "160", "-y", "42", env!("CARGO_BIN_EXE_orcux"), "window"]);
    while !lab.tmux(&["capture-pane", "-p", "-t", "=outer:"]).contains("w9") {
        assert!(started.elapsed().as_secs() < 10, "it did not start");
        sleep(Duration::from_millis(20));
    }
    let start = started.elapsed();
    sleep(Duration::from_secs(8));
    let pid: u32 = lab.tmux(&["display-message", "-p", "-t", "=outer:", "#{pane_pid}"]).trim().parse().unwrap();
    let before = cpu(pid);
    fs::remove_file(lab.dir.join("log")).ok();
    sleep(Duration::from_secs(WATCH));
    let calls = fs::read_to_string(lab.dir.join("log")).unwrap_or_default();
    let used = (cpu(pid) - before) / WATCH as f64;
    println!("window: start {start:?}, CPU {:.2} %, {} tmux processes in {WATCH} s", used * 100.0, calls.lines().count());
    assert!(calls.is_empty(), "tmux processes at rest:\n{calls}");
    assert!(used <= CPU * f64::from(room()), "CPU at rest: {used}");
    assert!(start < Duration::from_secs(1) * room(), "it took {start:?} to start");
}
