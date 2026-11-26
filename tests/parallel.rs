//! Several orcux at once, as several agents would run it.
mod lab;

use lab::Lab;
use std::{collections::BTreeSet, fs, process::Output};

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

fn at_once(lab: &Lab, all: &[Vec<String>]) -> Vec<Output> {
    let running: Vec<_> = all.iter().map(|a| {
        let mut c = lab.cmd(env!("CARGO_BIN_EXE_orcux"));
        c.args(a).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap()
    }).collect();
    running.into_iter().map(|c| c.wait_with_output().unwrap()).collect()
}

fn rows(lab: &Lab) -> Vec<Vec<String>> {
    let text = fs::read_to_string(lab.dir.join("home/.local/state/orcux/worktrees.tsv")).unwrap_or_default();
    text.lines().map(|l| l.split('\t').map(String::from).collect()).collect()
}

fn marks(lab: &Lab, format: &str) -> Vec<String> {
    lab.tmux(&["list-panes", "-a", "-F", format]).lines().map(String::from).collect()
}

#[test]
fn nothing_is_lost() {
    let Some(lab) = ready() else { return };
    let call = |words: &[&str]| words.iter().map(|w| w.to_string()).collect::<Vec<_>>();
    let name = |i: usize| format!("w{i}");

    let out = at_once(&lab, &(0..8).map(|i| call(&["new", &name(i)])).collect::<Vec<_>>());
    for o in &out {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }
    assert_eq!(rows(&lab).len(), 8);
    assert_eq!(rows(&lab).iter().map(|r| r[2].clone()).collect::<BTreeSet<_>>().len(), 8, "ports must differ");
    assert_eq!(marks(&lab, "#{@orcux}").len(), 8);

    at_once(&lab, &(0..5).map(|_| call(&["tab", "new", "w0"])).collect::<Vec<_>>());
    let tabs: BTreeSet<String> = marks(&lab, "#{@orcux} #{@orcux_tab}").into_iter().filter(|m| m.starts_with("repo/w0 ")).collect();
    assert_eq!(tabs.len(), 6, "{tabs:?}");

    // one name, several times: one wins
    let out = at_once(&lab, &(0..4).map(|_| call(&["new", "same"])).collect::<Vec<_>>());
    assert_eq!(out.iter().filter(|o| o.status.success()).count(), 1);
    assert_eq!(rows(&lab).iter().filter(|r| r[1] == "same").count(), 1);

    // some come while others go
    let mix: Vec<Vec<String>> = (0..8).map(|i| if i < 4 { call(&["rm", &name(i), "--force"]) } else { call(&["new", &format!("n{i}")]) }).collect();
    at_once(&lab, &mix);
    let left: BTreeSet<String> = rows(&lab).iter().map(|r| r[1].clone()).collect();
    assert_eq!(left, ["n4", "n5", "n6", "n7", "same", "w4", "w5", "w6", "w7"].map(String::from).into());
}

#[test]
fn names_are_only_names() {
    let Some(lab) = ready() else { return };
    // a branch name can hold a tmux format, even one that runs commands
    lab.orcux(&["new", "x#(touch${IFS}PWNED)"]);
    std::thread::sleep(std::time::Duration::from_secs(2));
    let found = planted(&lab);
    assert!(found.is_empty(), "a branch name ran a command: {found}");
    lab.orcux(&["new", "fix-#Solo"]);
    // the folder of a new terminal is known a moment after it starts (on Linux, from its process)
    let start = std::time::Instant::now();
    while !marks(&lab, "#{pane_current_path}").iter().any(|p| p.ends_with("/fix-#Solo")) {
        assert!(start.elapsed().as_secs() < 5, "{:?}", marks(&lab, "#{pane_current_path}"));
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    lab.orcux(&["new", "t"]);
    lab.orcux(&["send", "t", "echo one; echo two;"]);
    lab.orcux(&["send", "t", "-n; echo three;"]);
    std::thread::sleep(std::time::Duration::from_secs(1));
    let screen = String::from_utf8_lossy(&lab.orcux(&["read", "t"]).stdout).to_string();
    assert!(screen.contains("echo one; echo two;") && screen.contains("-n; echo three;"), "{screen}");
}

fn planted(lab: &Lab) -> String {
    let out = std::process::Command::new("find").arg(&lab.dir).args(["-name", "PWNED"]).output().unwrap();
    let here = std::path::Path::new("PWNED").exists();
    format!("{}{}", String::from_utf8_lossy(&out.stdout), if here { "PWNED here" } else { "" })
}

#[test]
fn a_worktree_inside_another_is_kept() {
    let Some(lab) = ready() else { return };
    lab.orcux(&["new", "outer"]);
    let outer = lab.dir.join("home/orcux/repo/outer");
    fs::write(lab.dir.join("repo/.git/info/exclude"), "inner\n").unwrap();
    lab.run("git", &["worktree", "add", "-q", "-b", "inner", outer.join("inner").to_str().unwrap()]);
    for args in [&["rm", "outer"][..], &["rm", "outer", "--force"][..]] {
        assert!(!lab.cmd(env!("CARGO_BIN_EXE_orcux")).args(args).output().unwrap().status.success());
    }
    assert!(outer.join("inner").exists());
}
