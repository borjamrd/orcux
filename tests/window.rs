//! `orcux window`: orcux draws the session itself. It runs here in another session of the same lab server, which
//! plays the person's terminal: what it shows is what the person would see.
mod lab;

use lab::Lab;
use std::{
    fs,
    thread::sleep,
    time::{Duration, Instant},
};

const ORCUX: &str = env!("CARGO_BIN_EXE_orcux");

fn until(what: &str, f: impl Fn() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed().as_secs() < 15, "never: {what}");
        sleep(Duration::from_millis(100));
    }
}

/// What the person sees.
fn screen(lab: &Lab) -> Vec<String> {
    lab.tmux(&["capture-pane", "-p", "-t", "=outer:"]).lines().map(|l| l.trim_end().to_string()).collect()
}

/// A click of the left button, at a column and row of the screen, from 0.
fn click(lab: &Lab, col: usize, row: usize) {
    for end in ['M', 'm'] {
        lab.tmux(&["send-keys", "-t", "=outer:", "-l", &format!("\x1b[<0;{};{}{end}", col + 1, row + 1)]);
    }
    sleep(Duration::from_millis(400));
}

fn keys(lab: &Lab, text: &str) {
    lab.tmux(&["send-keys", "-t", "=outer:", "-l", text]);
    sleep(Duration::from_millis(300));
}

fn pane_of(lab: &Lab, mark: &str) -> String {
    let all = lab.tmux(&["list-panes", "-a", "-F", "#{pane_id} #{@orcux}"]);
    all.lines().find_map(|l| l.strip_suffix(&format!(" {mark}"))).unwrap_or_else(|| panic!("no {mark}")).to_string()
}

#[test]
fn the_window_is_drawn_by_orcux() {
    if !lab::has_tmux() {
        eprintln!("skipped: tmux is needed");
        return;
    }
    let lab = Lab::new();
    lab.run("git", &["init", "-q", "."]);
    lab.run("git", &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "i"]);
    // the clipboard of the test, never the computer's
    let copied = lab.dir.join("copied");
    fs::write(lab.dir.join("bin/pbcopy"), format!("#!/bin/sh\ncat > {}\n", copied.display())).unwrap();
    lab.run("chmod", &["+x", &lab.dir.join("bin/pbcopy").display().to_string()]);
    lab.orcux(&["new", "feat-a"]);
    lab.orcux(&["new", "feat-b"]);
    lab.tmux(&["new-session", "-d", "-s", "outer", "-x", "160", "-y", "40", &format!("{ORCUX} window; sleep 30")]);
    until("the window attached", || !lab.tmux(&["show-option", "-gqv", "@orcux_window"]).trim().is_empty());
    until("the sidebar drawn", || screen(&lab).iter().any(|l| l.contains("feat-b")));
    let s = screen(&lab);
    assert!(s[0].contains("orcux") && s[0].contains('╭') && s[0].contains('╮'), "{}", s[0]);
    assert!(s[39].contains('╰') && s[39].contains('╯'), "the frame closes below: {}", s[39]);

    // a terminal: what orcux draws is what tmux has
    lab.orcux(&["show", "feat-a"]);
    let term = pane_of(&lab, "repo/feat-a");
    until("the terminal focused", || lab.tmux(&["display-message", "-p", "-t", "=orcux:", "#{@orcux}"]).trim() == "repo/feat-a");
    keys(&lab, "echo hi-window\r");
    until("typed through the window", || lab.tmux(&["capture-pane", "-p", "-t", &term]).contains("hi-window\n"));
    let geo = lab.tmux(&["display-message", "-p", "-t", &term, "#{pane_left} #{pane_width} #{pane_height}"]);
    let g: Vec<usize> = geo.split_whitespace().map(|x| x.parse().unwrap()).collect();
    let region = |s: &[String]| -> Vec<String> {
        s[1..=g[2]].iter().map(|l| l.chars().skip(g[0]).take(g[1]).collect::<String>().trim_end().to_string()).collect()
    };
    let truth: Vec<String> = lab.tmux(&["capture-pane", "-p", "-t", &term]).lines().map(|l| l.trim_end().to_string()).collect();
    until("the same screen", || region(&screen(&lab))[..truth.len().min(g[2])] == truth[..truth.len().min(g[2])]);

    // the mouse selects and copies in a terminal that does not read it
    let row = region(&screen(&lab)).iter().position(|l| l == "hi-window").unwrap() + 1;
    for (kind, col) in [("0", g[0]), ("32", g[0] + 4), ("0", g[0] + 8)] {
        let end = if col == g[0] + 8 { 'm' } else { 'M' };
        lab.tmux(&["send-keys", "-t", "=outer:", "-l", &format!("\x1b[<{kind};{};{}{end}", col + 1, row + 1)]);
        sleep(Duration::from_millis(200));
    }
    until("copied", || fs::read_to_string(&copied).is_ok_and(|t| t == "hi-window"));

    // a paste is text for the terminal, whatever it holds: a line of it is never a command for tmux
    let pwned = lab.dir.join("pwned");
    keys(&lab, &format!("\x1b[200~echo pasted\nrun-shell 'touch {}'\n\x1b[201~", pwned.display()));
    until("pasted", || lab.tmux(&["capture-pane", "-p", "-t", &term]).contains("pasted"));
    sleep(Duration::from_millis(500));
    assert!(!pwned.exists(), "a pasted line ran as a tmux command");
    keys(&lab, "\x03");

    // inside a pane of orcux it would draw itself without end: it says so instead
    keys(&lab, &format!("{ORCUX} window\r"));
    until("refused inside orcux", || lab.tmux(&["capture-pane", "-p", "-t", &term]).contains("a terminal of its own"));

    // the wheel goes back in the history
    keys(&lab, "seq 1 150\r");
    until("printed", || lab.tmux(&["capture-pane", "-p", "-t", &term]).contains("150"));
    for _ in 0..5 {
        lab.tmux(&["send-keys", "-t", "=outer:", "-l", &format!("\x1b[<64;{};10M", g[0] + 5)]);
    }
    until("scrolled back", || screen(&lab).iter().any(|l| l.contains("↑ 15")));
    keys(&lab, "x");
    until("live again", || !screen(&lab).iter().any(|l| l.contains("↑ ")));
    keys(&lab, "\x7f");

    // tabs on the frame: Opt+t opens one, a click shows another
    keys(&lab, "\x1bt");
    until("two tabs", || lab.tmux(&["list-panes", "-a", "-F", "#{@orcux}"]).lines().filter(|l| *l == "repo/feat-a").count() == 2);
    until("both on the frame", || screen(&lab)[0].contains(" 2 "));
    let top = screen(&lab)[0].clone();
    let at = top.chars().collect::<Vec<_>>().windows(3).position(|w| w == [' ', '1', ' ']).unwrap();
    click(&lab, at + 1, 0);
    let on = || lab.tmux(&["list-panes", "-a", "-F", "#{@orcux} #{@orcux_tab} #{@orcux_on}"]);
    until("tab 1 shown", || on().lines().any(|l| l == "repo/feat-a 1 1"));

    // a dialog of the sidebar is drawn over everything, and gets the keys
    click(&lab, 5, 30);
    until("the sidebar focused", || lab.tmux(&["display-message", "-p", "-t", "=orcux:", "#{@orcux}"]).trim() == "__sidebar");
    keys(&lab, "n");
    until("a popup", || screen(&lab).iter().any(|l| l.contains("╭ New worktree")));
    keys(&lab, "from-popup\r");
    until("created from the popup", || fs::read_to_string(lab.dir.join("home/.local/state/orcux/worktrees.tsv")).is_ok_and(|t| t.contains("from-popup")));
    until("the popup gone", || !screen(&lab).iter().any(|l| l.contains("New worktree")));

    // q in the sidebar leaves the window; the session and its terminals stay
    click(&lab, 5, 30);
    until("the sidebar focused again", || lab.tmux(&["display-message", "-p", "-t", "=orcux:", "#{@orcux}"]).trim() == "__sidebar");
    keys(&lab, "q");
    until("the window left", || lab.tmux(&["show-option", "-gqv", "@orcux_window"]).trim().is_empty());
    assert!(lab.tmux(&["has-session", "-t", "=orcux"]).is_empty());
    assert!(!pane_of(&lab, "repo/feat-a").is_empty());
}
