//! Handing tasks to agents and following them: `orcux run`, `wait`, `next`, `ls --json`, and the notifications.
use crate::*;
use std::sync::mpsc;

/// Agents that orcux knows how to start with a task. Those of the user, in the file `agents`, come first.
const AGENTS: [(&str, &str); 6] = [
    ("claude", "claude {prompt}"),
    ("codex", "codex {prompt}"),
    ("gemini", "gemini -i {prompt}"),
    ("opencode", "opencode --prompt {prompt}"),
    ("cursor", "cursor-agent {prompt}"),
    ("agy", "agy -i {prompt}"),
];

/// (name, command), without repeated names.
pub fn agents() -> Vec<(String, String)> {
    let own = fs::read_to_string(state_dir().join("agents")).unwrap_or_default();
    let own = own.lines().filter_map(|l| l.split_once('\t')).map(|(n, c)| (n.to_string(), c.to_string()));
    let mut all: Vec<(String, String)> = vec![];
    for a in own.chain(AGENTS.map(|(n, c)| (n.into(), c.into()))) {
        if !all.iter().any(|x| x.0 == a.0) {
            all.push(a);
        }
    }
    all
}

/// Whether its program is in the PATH. A shell function or an alias is not found: the person writes their own.
pub fn installed(cmd: &str) -> bool {
    let program = cmd.split_whitespace().next().unwrap_or_default();
    env::var_os("PATH").is_some_and(|p| env::split_paths(&p).any(|d| d.join(program).is_file()))
}

/// The person chose one, or wrote their own.
pub fn agent_chosen() -> bool {
    state_dir().join("agent").exists()
}

pub fn default_agent() -> String {
    let saved = fs::read_to_string(state_dir().join("agent")).unwrap_or_default().trim().to_string();
    if saved.is_empty() { AGENTS[0].0.into() } else { saved }
}

pub fn agents_cmd(args: &[&str]) -> Res<()> {
    match args {
        [] => {
            let default = default_agent();
            for (name, cmd) in agents() {
                println!("{} {name:<12} {cmd}", if name == default { "*" } else { " " });
            }
            Ok(())
        }
        ["use", name] if agents().iter().any(|a| a.0 == *name) => write_state("agent", name),
        ["use", name] => Err((tr().unknown_agent)(name)),
        ["set", name, cmd @ ..] if !cmd.is_empty() && !name.contains(char::is_whitespace) => {
            let _lock = lock();
            let own = read_state("agents")?;
            let kept = own.lines().filter(|l| l.split('\t').next() != Some(name)).map(|l| format!("{l}\n")).collect::<String>();
            write_state("agents", &format!("{name}\t{}\n{kept}", cmd.join(" ").replace(['\t', '\n'], " ")))
        }
        _ => Err(tr().agents_usage.into()),
    }
}

/// A branch name out of the first words of a task.
fn slug(task: &str) -> String {
    let plain = |c: char| match c {
        'á' | 'à' | 'ä' | 'â' => 'a',
        'é' | 'è' | 'ë' | 'ê' => 'e',
        'í' | 'ì' | 'ï' | 'î' => 'i',
        'ó' | 'ò' | 'ö' | 'ô' => 'o',
        'ú' | 'ù' | 'ü' | 'û' => 'u',
        'ñ' => 'n',
        'ç' => 'c',
        c => c,
    };
    let text: String = task.to_lowercase().chars().map(plain).map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' }).collect();
    let mut out = String::new();
    for w in text.split_whitespace().take(5) {
        if !out.is_empty() && out.len() + w.len() > 32 {
            break;
        }
        out += if out.is_empty() { "" } else { "-" };
        out += &w[..w.len().min(32)];
    }
    if out.is_empty() { "task".into() } else { out }
}

/// What is typed in the terminal: the prompt goes in a file, so that nothing in it means anything to the shell.
fn command(template: &str, prompt: &Path) -> String {
    let file = format!("\"$(cat '{}')\"", prompt.display().to_string().replace('\'', "'\\''"));
    if template.contains("{prompt}") { template.replace("{prompt}", &file) } else { format!("{template} {file}") }
}

/// `orcux run <task...> [--agent A] [--repo P] [--parent W] [--name N]`: a worktree with an agent at work in it.
pub fn run_task(args: &[&str]) -> Res<()> {
    let (mut words, mut agent, mut repo, mut parent, mut name) = (vec![], None, None, None, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match *a {
            "--agent" => agent = it.next().copied(),
            "--repo" => repo = it.next().copied(),
            "--parent" => parent = it.next().copied(),
            "--name" => name = it.next().copied(),
            "--" => words.extend(it.by_ref()),
            w if w.starts_with("--") => return Err(tr().run_usage.into()),
            w => words.push(w),
        }
    }
    let task = words.join(" ");
    if task.trim().is_empty() {
        return Err(tr().run_usage.into());
    }
    let agent = agent.map_or_else(default_agent, String::from);
    let (_, template) = agents().into_iter().find(|a| a.0 == agent).ok_or_else(|| (tr().unknown_agent)(&agent))?;
    let parent = parent.map(|q| find(Some(q))).transpose()?;
    let repo = match (&parent, repo) {
        (Some(p), _) => p.repo.clone(),
        (None, Some(r)) => PathBuf::from(r),
        (None, None) => env::current_dir().map_err(|e| e.to_string())?,
    };
    let start = |w: &Wt| -> Res<Option<String>> {
        let file = state_dir().join("prompts").join(format!("{}.md", w.slot));
        fs::create_dir_all(file.parent().unwrap()).map_err(|e| e.to_string())?;
        private(&file, &task)?;
        Ok(Some(command(&template, &file)))
    };
    let w = match name {
        Some(n) => create_with(&repo, n, parent.as_ref(), start)?,
        None => {
            // a name of its own, even if others ask for the same at once
            let base = slug(&task);
            let branch = |n: &str| git(&repo, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{n}")]).is_ok();
            let free = |n: &String| !branch(n) && !load().iter().any(|w| w.name == *n);
            let mut names = (1..).map(|i| if i == 1 { base.clone() } else { format!("{base}-{i}") }).filter(free);
            let mut tries = 0;
            loop {
                match create_with(&repo, &names.next().unwrap(), parent.as_ref(), start) {
                    // only because another took the name meanwhile
                    Err(e) if tries < 4 && (e.contains("already exists") || load().iter().any(|w| e == (tr().exists)(&w.name) || e == (tr().id_taken)(&w.id()))) => tries += 1,
                    r => break r?,
                }
            }
        }
    };
    tell_untrusted(&w);
    println!("{}  {}  ORCUX_PORT={}", w.id(), w.path().display(), w.port());
    Ok(())
}

/// Only for the person: a task can hold anything.
fn private(file: &Path, text: &str) -> Res<()> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    fs::remove_file(file).ok();
    let mut f = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(file).map_err(|e| e.to_string())?;
    f.write_all(text.as_bytes()).map_err(|e| e.to_string())
}

/// What an agent does, for scripts.
fn state(p: &Pane) -> &'static str {
    match p.agent {
        Some((a, _)) => a.name(),
        None if p.shell => "shell",
        None if p.busy => "working",
        None => "open",
    }
}

fn json(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out += "\\\"",
            '\\' => out += "\\\\",
            c if c < ' ' => out += &format!("\\u{:04x}", c as u32),
            c => out.push(c),
        }
    }
    out + "\""
}

pub fn ls(as_json: bool) -> Res<()> {
    let all = panes();
    let wts = load();
    let mine = |w: &Wt| { let id = w.id(); let mut t: Vec<&Pane> = all.iter().filter(|p| p.wt == id).collect(); t.sort_by_key(|p| p.tab); t };
    if !as_json {
        for w in &wts {
            let what = foremost(mine(w).into_iter()).map_or("-", state);
            println!("{:<40} :{:<6} {what:<10} {}", w.id(), w.port(), w.path().display());
        }
        return Ok(());
    }
    let text = |s: Option<&str>| s.map_or("null".into(), json);
    let rows: Vec<String> = wts.iter().map(|w| {
        let tabs: Vec<String> = mine(w).iter().map(|p| {
            let since = p.agent.map_or("null".into(), |a| a.1.to_string());
            format!("{{\"tab\":{},\"active\":{},\"program\":{},\"state\":{},\"since\":{since}}}", p.tab, p.on, json(tui::program(&p.cmd)), json(state(p)))
        }).collect();
        let overall = foremost(mine(w).into_iter()).map(state);
        format!(
            "{{\"id\":{},\"repo\":{},\"name\":{},\"path\":{},\"branch\":{},\"port\":{},\"slot\":{},\"parent\":{},\"state\":{},\"tabs\":[{}]}}",
            json(&w.id()), json(&w.repo.to_string_lossy()), json(&w.name), json(&w.path().to_string_lossy()), text(w.branch().as_deref()),
            w.port(), w.slot, text(w.parent.as_deref()), text(overall), tabs.join(",")
        )
    }).collect();
    println!("[{}]", rows.join(",\n "));
    Ok(())
}

/// How `orcux wait` ends.
const DONE: i32 = 0;
const ASKS: i32 = 2;
const FAILED: i32 = 3;
const TIMEOUT: i32 = 4;
const NO_AGENT: i32 = 5;

/// None: it works.
fn outcome(p: Option<&Pane>) -> Option<(i32, &'static str)> {
    let Some(p) = p else { return Some((NO_AGENT, "closed")) };
    match p.agent.map(|a| a.0) {
        Some(Agent::Working) => None,
        Some(a) if a.waits() => Some((ASKS, a.name())),
        Some(Agent::Failed) => Some((FAILED, "failed")),
        Some(a) => Some((DONE, a.name())),
        None if p.busy => None,
        None => Some((NO_AGENT, state(p))),
    }
}

/// `orcux wait <wt>[:n]... [--timeout s]`: until none of them works. Prints how each one is and ends with the code of
/// the one that matters most. An agent that was just told something needs a moment to start.
pub fn wait(args: &[&str]) -> Res<i32> {
    let limit = flag(args, "--timeout").map(|s| s.parse::<u64>().map_err(|_| tr().wait_usage.to_string())).transpose()?;
    let names: Vec<&str> = args.iter().enumerate().filter(|(i, a)| **a != "--timeout" && !(*i > 0 && args[i - 1] == "--timeout")).map(|a| *a.1).collect();
    if names.is_empty() || names.iter().any(|n| n.starts_with('-')) {
        return Err(tr().wait_usage.into());
    }
    let targets: Vec<(Wt, Option<u32>)> = names.iter().map(|q| target(q)).collect::<Res<_>>()?;
    let (tx, rx) = mpsc::channel();
    control::connect(move |_| { tx.send(()).ok(); });
    let start = std::time::Instant::now();
    loop {
        let all = panes();
        let now: Vec<Option<(i32, &str)>> = targets.iter().map(|(w, tab)| {
            let id = w.id();
            outcome(match tab {
                Some(n) => all.iter().find(|p| p.wt == id && p.tab == *n),
                None => foremost_at_work(all.iter().filter(|p| p.wt == id)),
            })
        }).collect();
        let grace = start.elapsed().as_secs() < 3 && now.iter().flatten().any(|o| matches!(o.0, DONE | NO_AGENT));
        let late = limit.is_some_and(|s| start.elapsed().as_secs() >= s);
        if late || (!grace && now.iter().all(Option::is_some)) {
            for (q, o) in names.iter().zip(&now) {
                println!("{q} {}", o.map_or("working", |o| o.1));
            }
            // what needs the person first
            let codes: Vec<i32> = now.iter().map(|o| o.map_or(TIMEOUT, |o| o.0)).collect();
            return Ok([ASKS, FAILED, TIMEOUT, NO_AGENT, DONE].into_iter().find(|c| codes.contains(c)).unwrap_or(DONE));
        }
        // tmux tells; without the connection, ask now and then
        rx.recv_timeout(Duration::from_secs(1)).ok();
        while rx.try_recv().is_ok() {}
    }
}

/// Of several tabs, one that works; else the one that matters most.
fn foremost_at_work<'a>(tabs: impl Iterator<Item = &'a Pane>) -> Option<&'a Pane> {
    let t: Vec<&Pane> = tabs.collect();
    t.iter().find(|p| outcome(Some(p)).is_none()).copied().or_else(|| foremost(t.into_iter()))
}

/// Shows the terminal that waits for the person since longer ago; else one that failed, or is done.
/// Something that waits for the person.
pub enum Wait {
    Agent(Pane),
    Pr(prs::Pr),
}

/// What waits for the person, besides the terminal in front of them: agents that ask, then those that failed, then
/// those that are done, each the one that waits longest first; then the pull requests that are theirs to move.
pub fn waiting() -> Vec<Wait> {
    let here = terminal_pane().map(|p| p.0);
    let rank = |p: &Pane| match p.look() {
        prs::State::Asking => 0,
        prs::State::Broken => 1,
        prs::State::Finished => 2,
        _ => 9,
    };
    let mut agents: Vec<Pane> = panes().into_iter().filter(|p| Some(&p.pane) != here.as_ref() && rank(p) < 9).collect();
    agents.sort_by_key(|p| (rank(p), p.agent.map_or(0, |a| a.1)));
    let prs = prs::cached().prs.into_iter().filter(|p| p.status().0 == prs::Group::Yours);
    agents.into_iter().map(Wait::Agent).chain(prs.map(Wait::Pr)).collect()
}

/// Opt+r: straight to the agent that waits, if it is the only thing; with more, the list of them.
pub fn next() -> Res<()> {
    match waiting().as_slice() {
        [] => Err(tr().nobody_waits.into()),
        [Wait::Agent(p)] => show_tab(&find(Some(&p.wt))?, Some(p.tab)),
        all => popup(tr().title_inbox, (100, (all.len() as u32 + 4).clamp(8, 24)), &["inbox"]),
    }
}

// ---------- notifications ----------

pub const NOTIFY: [&str; 3] = ["off", "ask", "all"];

/// "ask": an agent that asks, needs permission or failed. "all": also one that is done.
pub fn notify_mode() -> String {
    let saved = fs::read_to_string(state_dir().join("notify")).unwrap_or_default();
    NOTIFY.into_iter().find(|n| *n == saved.trim()).unwrap_or("ask").into()
}

pub fn set_notify(mode: &str) -> Res<()> {
    write_state("notify", mode)
}

/// Whether a state deserves a notification.
pub fn worth(a: Agent, mode: &str) -> bool {
    match mode {
        "off" => false,
        "all" => a.waits() || matches!(a, Agent::Failed | Agent::Done),
        _ => a.waits() || a == Agent::Failed,
    }
}

/// Through the system. The texts go as arguments, never inside a script.
pub fn notify(title: &str, text: &str) {
    let (title, text) = (title.to_string(), text.to_string());
    thread::spawn(move || {
        if cfg!(target_os = "macos") {
            let script = "on run a\ndisplay notification (item 2 of a) with title (item 1 of a)\nend run";
            Command::new("osascript").args(["-e", script, &title, &text]).output().ok();
        } else {
            Command::new("notify-send").args(["-a", "orcux", "--", &title, &text]).output().ok();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(slug("Fix the login redirect, with a test"), "fix-the-login-redirect-with");
        assert_eq!(slug("Añade paginación: 50 por página!"), "anade-paginacion-50-por-pagina");
        assert_eq!(slug("¿¡…!?"), "task");
        assert_eq!(slug(&"x".repeat(90)).len(), 32);
    }

    #[test]
    fn prompts_are_only_text() {
        let p = Path::new("/home/o'neil/1.md");
        assert_eq!(command("codex {prompt} --x", p), r#"codex "$(cat '/home/o'\''neil/1.md')" --x"#);
        assert!(command("my agent", p).ends_with("md')\""));
        assert_eq!(json("a\"b\\c\n"), r#""a\"b\\c\u000a""#);
    }
}
