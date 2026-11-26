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

/// Memory in use, in MB.
fn rss(pid: u32) -> f64 {
    let out = Command::new("ps").env("LC_ALL", "C").args(["-o", "rss=", "-p", &pid.to_string()]).output().unwrap().stdout;
    String::from_utf8_lossy(&out).trim().parse::<f64>().unwrap_or(0.0) / 1024.0
}

/// The server that keeps the terminals: tmux's, or orcux's own engine.
fn server(lab: &Lab) -> u32 {
    lab.tmux(&["display-message", "-p", "#{pid}"]).trim().parse().unwrap()
}

fn engine() -> &'static str {
    if lab::own() { "orcux's own engine" } else { "tmux" }
}

/// Rows of a table for the summary of a CI run (ORCUX_PERF_SUMMARY, the file of GITHUB_STEP_SUMMARY).
fn summary(title: &str, rows: &[(&str, String)]) {
    let Ok(path) = std::env::var("ORCUX_PERF_SUMMARY") else { return };
    let mut text = format!("\n#### {title}, {}\n\n| | |\n| --- | --- |\n", engine());
    for (k, v) in rows {
        text += &format!("| {k} | {v} |\n");
    }
    use std::io::Write;
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(path) {
        f.write_all(text.as_bytes()).ok();
    }
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
    let pids = [lab.pid("__sidebar"), lab.pid("__prs"), server(&lab)];
    let before = pids.map(cpu);
    fs::remove_file(lab.dir.join("log")).ok();
    sleep(Duration::from_secs(WATCH));
    let calls = fs::read_to_string(lab.dir.join("log")).unwrap_or_default();
    let used: Vec<f64> = pids.iter().zip(before).map(|(p, b)| (cpu(*p) - b) / WATCH as f64).collect();

    let mut ls: Vec<Duration> = (0..9).map(|_| { let t = Instant::now(); lab.orcux(&["ls"]); t.elapsed() }).collect();
    ls.sort();
    let mut show: Vec<Duration> = (0..9).map(|i| { let t = Instant::now(); lab.orcux(&["show", &format!("w{i}")]); t.elapsed() }).collect();
    show.sort();
    let mem: Vec<f64> = pids.iter().map(|p| rss(*p)).collect();
    println!("{panes} panes ({}), `orcux show` {:?}, start {start:?}, `orcux ls` {:?}, CPU sidebar {:.2} % rail {:.2} % server {:.2} %, memory sidebar {:.1} MB rail {:.1} MB server {:.1} MB, {} tmux processes in {WATCH} s",
        engine(), show[4], ls[4], used[0] * 100.0, used[1] * 100.0, used[2] * 100.0, mem[0], mem[1], mem[2], calls.lines().count());
    summary(&format!("{panes} terminals: sidebar, PR panel and server"), &[
        ("start", format!("{} ms", start.as_millis())),
        ("`orcux ls`", format!("{} ms", ls[4].as_millis())),
        ("`orcux show`", format!("{} ms", show[4].as_millis())),
        ("CPU at rest: sidebar, panel, server", format!("{:.2} %, {:.2} %, {:.2} %", used[0] * 100.0, used[1] * 100.0, used[2] * 100.0)),
        ("memory: sidebar, panel, server", format!("{:.1}, {:.1}, {:.1} MB", mem[0], mem[1], mem[2])),
        ("processes started at rest", calls.lines().count().to_string()),
    ]);
    assert!(calls.is_empty(), "tmux processes at rest:\n{calls}");
    assert!(used[..2].iter().all(|u| *u <= CPU * f64::from(room())), "CPU at rest: {used:?}");
    // the server keeps every terminal: at rest it still waits, whatever their number
    assert!(used[2] <= CPU * 2.0 * f64::from(room()), "CPU of the server at rest: {}", used[2]);
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
    let mem = rss(pid);
    println!("window ({}): start {start:?}, CPU {:.2} %, memory {mem:.1} MB, {} tmux processes in {WATCH} s", engine(), used * 100.0, calls.lines().count());
    summary("50 terminals, the window drawn by orcux", &[
        ("start", format!("{} ms", start.as_millis())),
        ("CPU at rest", format!("{:.2} %", used * 100.0)),
        ("memory", format!("{mem:.1} MB")),
        ("processes started at rest", calls.lines().count().to_string()),
    ]);
    assert!(calls.is_empty(), "tmux processes at rest:\n{calls}");
    assert!(used <= CPU * f64::from(room()), "CPU at rest: {used}");
    assert!(start < Duration::from_secs(1) * room(), "it took {start:?} to start");
}
