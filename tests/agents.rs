//! Handing tasks to agents and following them. The agents are scripts that say what real ones would.
mod lab;

use lab::Lab;
use std::{
    process::Output,
    thread::sleep,
    time::{Duration, Instant},
};

const ORCUX: &str = env!("CARGO_BIN_EXE_orcux");

fn ready() -> Option<Lab> {
    if !lab::has_tmux() {
        eprintln!("skipped: tmux is needed");
        return None;
    }
    let lab = Lab::new();
    lab.run("git", &["init", "-q", "."]);
    lab.run("git", &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "i"]);
    Some(lab)
}

/// An agent that keeps its task in a file, works for a second, and ends up as it is told.
fn agent(lab: &Lab, name: &str, end: &str) {
    let script = format!("sh -c 'printf %s \"$1\" > task.txt; {ORCUX} hook working exact; sleep 1; {ORCUX} hook {end} exact; exec sleep 60' -- {{prompt}}");
    lab.orcux(&["agents", "set", name, &script]);
}

fn any(lab: &Lab, args: &[&str]) -> Output {
    lab.cmd(ORCUX).args(args).output().unwrap()
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

fn until(what: &str, f: impl Fn() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed().as_secs() < 20, "never: {what}");
        sleep(Duration::from_millis(100));
    }
}

#[test]
fn a_task_is_handed_over_and_followed() {
    let Some(lab) = ready() else { return };
    for end in ["done", "question", "failed"] {
        agent(&lab, end, end);
    }
    let task = "Fix \"it\": $HOME `date` $(id) 'x' \\ ; !! *\nsecond line";
    assert!(out(&lab.orcux(&["run", task, "--agent", "done"])).starts_with("repo/fix-it-home-date-id "));
    let w = any(&lab, &["wait", "fix-it-home-date-id", "--timeout", "15"]);
    assert_eq!((w.status.code(), out(&w).as_str()), (Some(0), "fix-it-home-date-id done"));
    assert_eq!(std::fs::read_to_string(lab.dir.join("home/orcux/repo/fix-it-home-date-id/task.txt")).unwrap(), task);

    lab.orcux(&["agents", "use", "question"]);
    lab.orcux(&["run", "ask", "me"]);
    let w = any(&lab, &["wait", "ask-me", "--timeout", "15"]);
    assert_eq!((w.status.code(), out(&w).as_str()), (Some(2), "ask-me question"));
    // same words: another name
    assert!(out(&lab.orcux(&["run", "ask", "me", "--agent", "failed"])).starts_with("repo/ask-me-2 "));
    let w = any(&lab, &["wait", "fix-it-home-date-id", "ask-me-2", "--timeout", "15"]);
    assert_eq!(w.status.code(), Some(3), "{}", out(&w)); // the worst of both
    assert!(out(&w).contains("ask-me-2 failed") && out(&w).contains("fix-it-home-date-id done"));

    lab.orcux(&["new", "empty"]);
    assert_eq!(any(&lab, &["wait", "empty", "--timeout", "8"]).status.code(), Some(5));
    lab.orcux(&["agents", "set", "slow", &format!("sh -c '{ORCUX} hook working exact; exec sleep 60' --")]);
    lab.orcux(&["run", "slow", "--agent", "slow"]);
    sleep(Duration::from_millis(800));
    assert_eq!(any(&lab, &["wait", "slow", "--timeout", "1"]).status.code(), Some(4));
    assert!(!any(&lab, &["run", "x", "--agent", "nobody"]).status.success());
    assert!(!any(&lab, &["wait"]).status.success());

    let json = out(&lab.orcux(&["ls", "--json"]));
    assert!(json.contains(r#""name":"ask-me","#) && json.contains(r#""state":"question""#), "{json}");
    let check = lab.cmd("python3").args(["-c", "import json,sys; d=json.loads(sys.argv[1]); print(len(d), d[0]['tabs'][0]['state'])", &json]).output();
    if let Ok(o) = check {
        assert_eq!(out(&o), "5 done", "{json}");
    }
    // the prompts are only for the person
    use std::os::unix::fs::PermissionsExt;
    for f in std::fs::read_dir(lab.dir.join("home/.local/state/orcux/prompts")).unwrap().flatten() {
        assert_eq!(f.metadata().unwrap().permissions().mode() & 0o777, 0o600);
    }
}

#[test]
fn titles_are_followed_and_the_one_that_waits_is_shown() {
    let Some(lab) = ready() else { return };
    let script = lab.dir.join("bin/titled");
    let perl = r#"#!/usr/bin/perl
$| = 1;
sub title { print "\e]2;$_[0]\a" }
title("\xe2\x9c\xa6 Working (repo)"); sleep 4;
title("\xe2\x9c\x8b Action Required (repo)"); sleep 9;
title("\xe2\x97\x87 Ready (repo)"); sleep 60;
"#;
    lab::write_exe(&script, perl);
    lab.orcux(&["agents", "set", "titled", &script.display().to_string()]);
    lab.orcux(&["new", "other"]);
    lab.tmux(&["-L", "outer", "-f", "/dev/null", "new-session", "-d", "-s", "o", "-x", "160", "-y", "42", &format!("env -u TMUX '{ORCUX}'")]);
    until("the interface", || lab.tmux(&["-L", "outer", "capture-pane", "-p", "-t", "o"]).contains("other"));
    lab.orcux(&["run", "titled", "--agent", "titled"]);
    let state = || out(&lab.orcux(&["ls", "--json"])).split("\"name\":\"titled\"").nth(1).unwrap_or("").split("\"tabs\"").next().unwrap_or("").to_string();
    until("works", || state().contains("working"));
    until("needs permission", || state().contains("permission"));

    lab.orcux(&["show", "other"]);
    lab.orcux(&["next"]);
    assert_eq!(lab.tmux(&["list-panes", "-t", "=orcux:", "-F", "#{@orcux}"]).lines().find(|l| !l.starts_with("__")), Some("repo/titled"));
    lab.orcux(&["show", "other"]);
    until("is done", || state().contains("done"));
    // seen, so nobody waits
    lab.orcux(&["next"]);
    until("is seen", || state().contains("idle"));
    lab.orcux(&["show", "other"]);
    assert!(!any(&lab, &["next"]).status.success());
}
