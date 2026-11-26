//! From a new worktree to its merge, and what comes back when tmux ends.
mod lab;

use lab::Lab;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
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
    lab.run("git", &["init", "-q", "-b", "main", "."]);
    for (k, v) in [("user.name", "t"), ("user.email", "t@t")] {
        lab.run("git", &["config", k, v]);
    }
    fs::write(lab.dir.join("repo/a.txt"), "one\ntwo\nthree\n").unwrap();
    lab.run("git", &["add", "."]);
    lab.run("git", &["commit", "-qm", "i"]);
    Some(lab)
}

fn any(lab: &Lab, args: &[&str]) -> Output {
    lab.cmd(ORCUX).args(args).stdin(std::process::Stdio::null()).output().unwrap()
}

fn says(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

fn until(what: &str, f: impl Fn() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed().as_secs() < 15, "never: {what}");
        sleep(Duration::from_millis(100));
    }
}

fn wt(lab: &Lab, name: &str) -> PathBuf {
    lab.dir.join("home/orcux/repo").join(name)
}

fn git(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git").args(args).current_dir(dir).output().unwrap();
    assert!(o.status.success(), "git {args:?}: {}", says(&o));
    String::from_utf8_lossy(&o.stdout).trim().into()
}

fn commit(dir: &Path, file: &str, text: &str) {
    fs::write(dir.join(file), text).unwrap();
    git(dir, &["add", file]);
    git(dir, &["commit", "-qm", file]);
}

/// Types an answer to `orcux trust` in a terminal, as a person would.
fn trust(lab: &Lab, answer: &str) {
    let cmd = format!("{ORCUX} trust {} > /dev/tty; echo; echo code=$?; sleep 30", lab.dir.join("repo").display());
    lab.tmux(&["-L", "person", "-f", "/dev/null", "new-session", "-d", "-s", "p", "-x", "120", "-y", "30", &cmd]);
    let screen = || lab.tmux(&["-L", "person", "capture-pane", "-p", "-t", "p"]);
    until("it asks", || screen().contains("[y/N]"));
    lab.tmux(&["-L", "person", "send-keys", "-t", "p", answer, "Enter"]);
    until("it ends", || screen().contains("code="));
    lab.tmux(&["-L", "person", "kill-server"]);
}

#[test]
fn a_repo_says_what_its_worktrees_need_once_approved() {
    let Some(lab) = ready() else { return };
    let repo = lab.dir.join("repo");
    fs::create_dir(repo.join("node_modules")).unwrap();
    fs::write(repo.join("node_modules/lib.js"), "x").unwrap();
    fs::write(repo.join(".git/info/exclude"), "/node_modules/\n").unwrap();
    let out = lab.dir.join("torn");
    let config = format!("setup = echo $DB_PORT > made.txt\nsetup = echo \"$DB_URL\" >> made.txt\nteardown = test ! -e keep && basename \"$PWD\" >> {}\nlink = node_modules\nport DB_PORT = 7\nenv DB_URL = pg://localhost:${{DB_PORT}}/${{ORCUX_NAME}}\n", out.display());
    commit(&repo, ".orcux", &config);

    // not approved: nothing of it is used, and an agent cannot approve it
    let o = any(&lab, &["new", "w0"]);
    assert!(o.status.success() && says(&o).contains("orcux trust"), "{}", says(&o));
    sleep(Duration::from_secs(1));
    assert!(!wt(&lab, "w0").join("made.txt").exists() && !wt(&lab, "w0").join("node_modules").exists());
    assert!(!says(&any(&lab, &["env", "w0"])).contains("DB_PORT"));
    assert!(!any(&lab, &["trust"]).status.success());
    trust(&lab, "n");
    assert!(!says(&any(&lab, &["env", "w0"])).contains("DB_PORT"));

    trust(&lab, "y");
    lab.orcux(&["new", "fix-1"]);
    let made = wt(&lab, "fix-1").join("made.txt");
    until("it is prepared", || fs::read_to_string(&made).is_ok_and(|t| t.lines().count() == 2));
    assert_eq!(fs::read_to_string(&made).unwrap(), "10207\npg://localhost:10207/repo_fix_1\n");
    assert!(wt(&lab, "fix-1").join("node_modules/lib.js").exists());
    assert!(!git(&wt(&lab, "fix-1"), &["status", "--porcelain"]).contains("node_modules"), "the link would be committed");
    assert!(says(&any(&lab, &["ports", "fix-1"])).contains("DB_PORT"));
    fs::remove_file(&made).unwrap();

    // its end
    fs::write(wt(&lab, "fix-1").join("keep"), "").unwrap();
    git(&wt(&lab, "fix-1"), &["add", "keep"]);
    git(&wt(&lab, "fix-1"), &["commit", "-qm", "keep"]);
    assert!(!any(&lab, &["rm", "fix-1"]).status.success() && wt(&lab, "fix-1").exists());
    // refused for its changes: nothing is taken down
    git(&wt(&lab, "fix-1"), &["rm", "-q", "keep"]);
    assert!(!any(&lab, &["rm", "fix-1"]).status.success() && !out.exists());
    lab.orcux(&["rm", "fix-1", "--force"]);
    fs::remove_file(&out).ok();
    lab.orcux(&["new", "fix-2"]);
    until("it is prepared", || wt(&lab, "fix-2").join("made.txt").exists());
    sleep(Duration::from_millis(500));
    fs::remove_file(wt(&lab, "fix-2").join("made.txt")).unwrap();
    lab.orcux(&["rm", "fix-2"]);
    assert_eq!(fs::read_to_string(&out).unwrap(), "fix-2\n");
    assert!(repo.join("node_modules/lib.js").exists(), "what it linked to must stay");

    // a change needs to be read again
    commit(&repo, ".orcux", "setup = echo other > other.txt\n");
    let o = any(&lab, &["new", "w3"]);
    assert!(says(&o).contains("orcux trust"));
    sleep(Duration::from_secs(1));
    assert!(!wt(&lab, "w3").join("other.txt").exists());
}

#[test]
fn preparing_is_already_work() {
    let Some(lab) = ready() else { return };
    commit(&lab.dir.join("repo"), ".orcux", "setup = sleep 5\n");
    trust(&lab, "y");
    // not a shell script: a shell in front means that there is no agent
    let fake = lab.dir.join("bin/fake");
    lab::write_exe(&fake, &format!("#!/usr/bin/perl\nsystem('{ORCUX} hook working exact'); sleep 1; system('{ORCUX} hook done exact'); sleep 60;\n"));
    lab.orcux(&["agents", "set", "fake", &fake.display().to_string()]);
    lab.orcux(&["run", "a", "task", "--agent", "fake"]);
    let o = any(&lab, &["wait", "a-task", "--timeout", "20"]);
    assert_eq!((o.status.code(), says(&o).trim()), (Some(0), "a-task done"));
    // a preparation that fails is the end of it
    commit(&lab.dir.join("repo"), ".orcux", "setup = exit 3\n");
    trust(&lab, "y");
    lab.orcux(&["run", "b", "task", "--agent", "fake"]);
    assert_eq!(any(&lab, &["wait", "b-task", "--timeout", "20"]).status.code(), Some(5));
}

#[test]
fn work_is_brought_back() {
    let Some(lab) = ready() else { return };
    let repo = lab.dir.join("repo");
    lab.orcux(&["new", "stay"]); // or tmux would end with the last terminal
    // checked out in the main checkout
    lab.orcux(&["new", "one"]);
    commit(&wt(&lab, "one"), "b.txt", "b");
    fs::write(wt(&lab, "one").join("dirty"), "").unwrap();
    let o = any(&lab, &["merge", "one"]);
    assert!(!o.status.success() && wt(&lab, "one").exists(), "{}", says(&o));
    fs::remove_file(wt(&lab, "one").join("dirty")).unwrap();
    lab.orcux(&["merge", "one", "--check"]);
    assert!(wt(&lab, "one").exists());
    lab.orcux(&["merge", "one"]);
    assert!(repo.join("b.txt").exists() && !wt(&lab, "one").exists());
    assert_eq!(git(&repo, &["branch", "--list", "one"]), "");
    assert!(!lab.tmux(&["list-panes", "-a", "-F", "#{@orcux}"]).contains("repo/one"));

    // nothing is guessed
    lab.orcux(&["new", "strict"]);
    commit(&wt(&lab, "strict"), "s.txt", "s");
    for extra in [&["--dry-run"][..], &["--into=main"][..], &["main"][..], &["--into"][..]] {
        assert!(!any(&lab, &[&["merge", "strict"][..], extra].concat()).status.success(), "{extra:?}");
    }
    assert!(wt(&lab, "strict").exists() && !repo.join("s.txt").exists());

    // a clash changes and deletes nothing
    lab.orcux(&["new", "two"]);
    commit(&wt(&lab, "two"), "a.txt", "one\n2\nthree\n");
    commit(&repo, "a.txt", "one\nTWO\nthree\n");
    let before = git(&repo, &["rev-parse", "main", "two"]);
    let o = any(&lab, &["merge", "two"]);
    assert!(!o.status.success() && says(&o).contains("a.txt"), "{}", says(&o));
    assert_eq!((git(&repo, &["rev-parse", "main", "two"]), git(&repo, &["status", "--porcelain"])), (before, String::new()));
    assert!(wt(&lab, "two").exists());

    // a branch that is checked out nowhere, and that moved meanwhile
    lab.orcux(&["new", "three"]);
    commit(&wt(&lab, "three"), "c.txt", "c");
    commit(&repo, "d.txt", "d");
    git(&repo, &["checkout", "-q", "-b", "aside"]);
    lab.orcux(&["merge", "three"]);
    assert_eq!(git(&repo, &["ls-tree", "--name-only", "main"]), "a.txt\nb.txt\nc.txt\nd.txt");
    assert_eq!(git(&repo, &["rev-list", "--count", "--merges", "main"]), "1");
    assert!(!repo.join("c.txt").exists(), "this checkout is of another branch");

    // a child goes into its parent
    lab.orcux(&["new", "kid", "--parent", "two"]);
    commit(&wt(&lab, "kid"), "e.txt", "e");
    lab.orcux(&["merge", "kid"]);
    assert!(wt(&lab, "two").join("e.txt").exists() && !wt(&lab, "kid").exists());

    // an agent at work is not taken its ground away
    lab.orcux(&["new", "busy"]);
    commit(&wt(&lab, "busy"), "f.txt", "f");
    lab.orcux(&["send", "busy", &format!("{ORCUX} hook working exact; exec sleep 60")]);
    until("it works", || says(&any(&lab, &["ls"])).lines().any(|l| l.contains("repo/busy") && l.contains("working")));
    assert!(!any(&lab, &["merge", "busy"]).status.success() && wt(&lab, "busy").exists());
    lab.orcux(&["merge", "busy", "--force"]);
    assert!(!wt(&lab, "busy").exists());
}

#[test]
fn clashes_are_seen_in_time() {
    let Some(lab) = ready() else { return };
    for (w, text) in [("x", "one\nX\nthree\n"), ("y", "one\nY\nthree\n"), ("z", "one\ntwo\nthree\nfour\n")] {
        lab.orcux(&["new", w]);
        fs::write(wt(&lab, w).join("a.txt"), text).unwrap(); // not even a commit
    }
    fs::write(wt(&lab, "x").join("new.txt"), "1").unwrap();
    fs::write(wt(&lab, "y").join("new.txt"), "2").unwrap();
    git(&wt(&lab, "x"), &["add", "a.txt"]);
    let state = || ["x", "y", "z"].map(|w| (git(&wt(&lab, w), &["status", "--porcelain"]), git(&wt(&lab, w), &["rev-parse", "HEAD"])));
    let before = state();
    let o = any(&lab, &["conflicts"]);
    assert_eq!(o.status.code(), Some(1), "{}", says(&o));
    assert!(says(&o).contains("repo/x ↔ repo/y: ") && says(&o).contains("a.txt") && says(&o).contains("new.txt"), "{}", says(&o));
    assert_eq!(state(), before, "looking must change nothing");
    assert!(!fs::read_dir(lab.dir.join("home/.local/state/orcux")).unwrap().flatten().any(|f| f.file_name().to_string_lossy().starts_with("index.")));

    fs::write(wt(&lab, "y").join("a.txt"), "one\ntwo\nthree\n").unwrap();
    fs::remove_file(wt(&lab, "y").join("new.txt")).unwrap();
    assert_eq!(any(&lab, &["conflicts", "y"]).status.code(), Some(0));
    commit(&lab.dir.join("repo"), "a.txt", "one\nM\nthree\n");
    assert!(says(&any(&lab, &["conflicts"])).contains("repo/x ↔ main: a.txt"));
}

#[test]
fn terminals_come_back() {
    let Some(lab) = ready() else { return };
    let ui = |what: &str| {
        lab.tmux(&["-L", "outer", "-f", "/dev/null", "new-session", "-d", "-s", "o", "-x", "160", "-y", "46", &format!("env -u TMUX '{ORCUX}'")]);
        until(what, || lab.tmux(&["-L", "outer", "capture-pane", "-p", "-t", "o"]).contains("beta"));
    };
    let said = lab.dir.join("claude-args");
    lab::write_exe(&lab.dir.join("bin/claude"), &format!("#!/bin/sh\necho \"$CLAUDE_CONFIG_DIR|$*\" >> {}\nexec sleep 100\n", said.display()));
    lab.orcux(&["new", "alpha"]);
    lab.orcux(&["new", "beta"]);
    lab.orcux(&["tab", "new", "alpha"]);
    // what Claude Code sends when a conversation starts
    let start = format!("printf '{{\"session_id\":\"abc-1\",\"hook_event_name\":\"SessionStart\",\"model\":\"m-1\"}}' | CLAUDE_CONFIG_DIR=/tmp/acc {ORCUX} hook idle; exec sleep 300");
    lab.orcux(&["send", "alpha:2", &start]);
    ui("the interface");
    let state = lab.dir.join("home/.local/state/orcux");
    until("the list is kept", || fs::read_to_string(state.join("open.tsv")).is_ok_and(|t| t.contains("repo/alpha\t2\t") && t.contains("claude|abc-1|/tmp/acc|")));
    assert_eq!(says(&any(&lab, &["restore", "--list"])).trim(), "");

    // tmux ends, with everything in it
    lab.kill_servers();
    sleep(Duration::from_secs(1));
    ui("the interface again");
    until("it knows what was lost", || says(&any(&lab, &["restore", "--list"])).lines().count() == 3);
    until("it is offered", || lab.tmux(&["-L", "outer", "capture-pane", "-p", "-t", "o"]).contains("3 terminals"));
    // opening something first must not lose what was there
    lab.orcux(&["send", "alpha", "echo first"]);
    assert!(says(&any(&lab, &["restore", "--list"])).contains("claude"));
    lab.orcux(&["restore"]);
    let marks = lab.tmux(&["list-panes", "-a", "-F", "#{@orcux}:#{@orcux_tab}"]);
    for m in ["repo/alpha:1", "repo/alpha:2", "repo/beta:1"] {
        assert!(marks.lines().any(|l| l == m), "{marks}");
    }
    until("the conversation goes on", || fs::read_to_string(&said).is_ok_and(|t| t == "/tmp/acc|--model m-1 --resume abc-1\n"));
    assert_eq!(says(&any(&lab, &["restore", "--list"])).trim(), "");
}

#[test]
fn a_new_language_keeps_the_sidebar() {
    let Some(lab) = ready() else { return };
    lab.orcux(&["new", "alpha"]);
    lab.tmux(&["-L", "outer", "-f", "/dev/null", "new-session", "-d", "-s", "o", "-x", "160", "-y", "40", &format!("env -u TMUX '{ORCUX}'")]);
    // what each one draws: a shell in their place would draw a prompt (its program is not a sure sign: on Linux tmux
    // names the sh that started it)
    let screen = |mark: &str| {
        let all = lab.tmux(&["list-panes", "-a", "-F", "#{pane_id} #{@orcux}"]);
        all.lines().find_map(|l| l.strip_suffix(&format!(" {mark}"))).map(|p| lab.tmux(&["capture-pane", "-p", "-t", p])).unwrap_or_default()
    };
    until("the interface", || screen("__sidebar").contains("alpha") && screen("__prs").contains("to review"));
    lab.orcux(&["lang", "es"]);
    // restarted in the new language, as themselves
    until("the sidebar and the panel again", || screen("__sidebar").contains("alpha") && screen("__prs").contains("por revisar"));
}

#[test]
fn a_rename_moves_the_folder_and_the_branch_and_keeps_its_children() {
    let Some(lab) = ready() else { return };
    let repo = lab.dir.join("repo");
    for args in [&["new", "a", "--repo"][..], &["new", "b", "--parent", "repo/a", "--repo"]] {
        let o = any(&lab, &[args, &[repo.to_str().unwrap()]].concat());
        assert!(o.status.success(), "{}", says(&o));
    }
    // a program in its terminals: it would go on in a folder that moved
    any(&lab, &["send", "repo/a", "sleep 30"]);
    until("it runs", || lab.tmux(&["list-panes", "-a", "-F", "#{@orcux} #{pane_current_command}"]).contains("repo/a sleep"));
    let o = any(&lab, &["rename", "repo/a", "c"]);
    assert!(!o.status.success() && says(&o).contains("sleep"), "{}", says(&o));
    let pane = lab.tmux(&["list-panes", "-a", "-F", "#{pane_id} #{@orcux}"]);
    let pane = pane.lines().find(|l| l.ends_with(" repo/a")).unwrap().split(' ').next().unwrap().to_string();
    lab.tmux(&["send-keys", "-t", &pane, "C-c"]);
    until("it stops", || !lab.tmux(&["list-panes", "-a", "-F", "#{@orcux} #{pane_current_command}"]).contains("repo/a sleep"));

    let o = any(&lab, &["rename", "repo/a", "c"]);
    assert!(o.status.success(), "{}", says(&o));
    assert!(wt(&lab, "c").exists() && !wt(&lab, "a").exists());
    assert_eq!(git(&wt(&lab, "c"), &["branch", "--show-current"]), "c");
    let state = fs::read_to_string(lab.dir.join("home/.local/state/orcux/worktrees.tsv")).unwrap();
    assert!(state.lines().any(|l| l.contains("\tb\t") && l.ends_with("\tc")), "{state}");
    // its terminal is a new shell, where it is now, with the new name
    any(&lab, &["send", "repo/c", "echo id=$ORCUX_ID at=$PWD"]);
    until("the new shell answers", || says(&any(&lab, &["read", "repo/c", "5"])).contains("id=repo/c at="));
    assert!(says(&any(&lab, &["read", "repo/c", "5"])).contains("/c"));
    // a name that is taken, and the same name, change nothing
    assert!(!any(&lab, &["rename", "repo/c", "b"]).status.success());
    assert!(any(&lab, &["rename", "repo/c", "c"]).status.success());
    assert!(wt(&lab, "c").exists() && wt(&lab, "b").exists());
}

#[test]
fn a_pull_request_is_proposed_from_the_commits_on_top() {
    let Some(lab) = ready() else { return };
    let repo = lab.dir.join("repo");
    let origin = lab.dir.join("origin.git");
    git(&lab.dir, &["init", "-q", "--bare", "-b", "main", origin.to_str().unwrap()]);
    git(&repo, &["branch", "-M", "main"]);
    git(&repo, &["remote", "add", "origin", origin.to_str().unwrap()]);
    git(&repo, &["push", "-q", "origin", "main"]);
    git(&repo, &["remote", "set-head", "origin", "main"]);
    assert!(any(&lab, &["new", "feat", "--repo", repo.to_str().unwrap()]).status.success());
    let check = |w: &str| says(&any(&lab, &["pr", w, "--check"]));
    assert!(check("repo/feat").starts_with("feat → main  0 commits"), "{}", check("repo/feat"));
    assert!(!any(&lab, &["pr", "repo/feat"]).status.success()); // nothing to propose
    commit(&wt(&lab, "feat"), "b.txt", "b");
    assert_eq!(check("repo/feat"), "feat → main  1 commit\nb.txt\n");
    commit(&wt(&lab, "feat"), "c.txt", "c");
    assert_eq!(check("repo/feat"), "feat → main  2 commits\nfeat\n");
    // a child goes into its parent once that one is on origin
    git(&wt(&lab, "feat"), &["push", "-q", "origin", "feat"]);
    assert!(any(&lab, &["new", "kid", "--parent", "repo/feat"]).status.success());
    commit(&wt(&lab, "kid"), "d.txt", "d");
    assert!(check("repo/kid").starts_with("kid → feat  1 commit"), "{}", check("repo/kid"));
}

#[test]
fn an_agent_without_hooks_comes_back_too() {
    let Some(lab) = ready() else { return };
    let ui = |what: &str| {
        lab.tmux(&["-L", "outer", "-f", "/dev/null", "new-session", "-d", "-s", "o", "-x", "160", "-y", "46", &format!("env -u TMUX '{ORCUX}'")]);
        until(what, || lab.tmux(&["-L", "outer", "capture-pane", "-p", "-t", "o"]).contains("alpha"));
    };
    // Antigravity: its process is called agy, and it says nothing to orcux; its history says which conversation
    let said = lab.dir.join("agy-args");
    fs::create_dir_all(lab.dir.join("real")).unwrap();
    // a program whose process is called agy: a copy of orcux, waiting in a dialog (a copy of a system binary may not run)
    fs::copy(ORCUX, lab.dir.join("real/agy")).unwrap();
    lab::write_exe(&lab.dir.join("bin/agy"), &format!("#!/bin/sh\necho \"$*\" >> {}\nexec {} dialog error waiting\n", said.display(), lab.dir.join("real/agy").display()));
    lab.orcux(&["new", "alpha"]);
    let history = lab.dir.join("home/.gemini/antigravity-cli/history.jsonl");
    fs::create_dir_all(history.parent().unwrap()).unwrap();
    let line = |dir: &Path, id: &str| format!("{{\"display\":\"hi\",\"timestamp\":1,\"workspace\":\"{}\",\"conversationId\":\"{id}\"}}\n", dir.display());
    fs::write(&history, line(&wt(&lab, "alpha"), "conv-1") + &line(Path::new("/elsewhere"), "conv-2")).unwrap();
    lab.orcux(&["send", "alpha", "agy"]);
    ui("the interface");
    let state = lab.dir.join("home/.local/state/orcux");
    until("the list is kept", || fs::read_to_string(state.join("open.tsv")).is_ok_and(|t| t.contains("repo/alpha\t1\t") && t.contains("\tagy|conv-1|")));
    lab.kill_servers();
    sleep(Duration::from_secs(1));
    ui("the interface again");
    until("it knows what was lost", || says(&any(&lab, &["restore", "--list"])).contains("agy"));
    lab.orcux(&["restore"]);
    until("the conversation goes on", || fs::read_to_string(&said).is_ok_and(|t| t.ends_with("--conversation conv-1\n")));
}

#[test]
fn the_first_time_shows_the_first_steps() {
    let Some(lab) = ready() else { return };
    let repo = lab.dir.join("repo");
    // orcux, the first time, inside a repo: that repo is the project, and the steps are on the empty screen
    lab.tmux(&["-L", "outer", "-f", "/dev/null", "new-session", "-d", "-s", "o", "-x", "150", "-y", "32", "-c", repo.to_str().unwrap(), &format!("env -u TMUX '{ORCUX}'")]);
    let screen = || lab.tmux(&["-L", "outer", "capture-pane", "-p", "-t", "o"]);
    until("the first steps", || screen().contains("✓  A") && screen().contains("○  n"));
    let repos = fs::read_to_string(lab.dir.join("home/.local/state/orcux/repos")).unwrap();
    assert!(repos.lines().any(|l| Path::new(l).ends_with("repo")), "{repos}");
    // hidden with x, for good
    lab.tmux(&["-L", "outer", "send-keys", "-t", "o", "x"]);
    until("they are hidden", || !screen().contains("○  n"));
    assert!(lab.dir.join("home/.local/state/orcux/onboarded").exists());
}

#[test]
fn includes_copy_paths_and_patterns() {
    let Some(lab) = ready() else { return };
    let repo = lab.dir.join("repo");
    fs::write(repo.join(".gitignore"), "*.env\nlocal/\nsecret.txt\n").unwrap();
    for (f, t) in [(".env", "A=1"), ("api.env", "B=2"), ("keep.env", "no"), ("secret.txt", "s"), ("local/x.json", "{}"), ("local/deep/y.json", "{}")] {
        fs::create_dir_all(repo.join(f).parent().unwrap()).unwrap();
        fs::write(repo.join(f), t).unwrap();
    }
    fs::write(repo.join(".worktreeinclude"), "*.env\n!keep.env\nsecret.txt\nlocal/\nmissing.txt\n").unwrap();
    commit(&repo, ".gitignore", "*.env\nlocal/\nsecret.txt\n");
    assert!(any(&lab, &["new", "inc", "--repo", repo.to_str().unwrap()]).status.success());
    let w = wt(&lab, "inc");
    for f in [".env", "api.env", "secret.txt", "local/x.json", "local/deep/y.json"] {
        assert!(w.join(f).exists(), "{f} was not copied");
    }
    assert!(!w.join("keep.env").exists() && !w.join("missing.txt").exists());
}
