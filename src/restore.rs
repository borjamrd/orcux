//! The terminals that were open when tmux ended (a restart, a crash), and the conversations of their agents. The sidebar
//! keeps the list; with another tmux, those of the last one can be opened again.
use crate::*;

/// A tab: worktree, number, whether it was the active one, and its agent (program|session|folder of settings|model|mode).
#[derive(Clone, PartialEq, Debug)]
pub struct Open {
    pub wt: String,
    pub tab: u32,
    pub on: bool,
    pub session: String,
}

fn parse(text: &str) -> (String, Vec<Open>) {
    let mut lines = text.lines();
    let server = lines.next().unwrap_or("").to_string();
    let all = lines.filter_map(|l| {
        let [wt, tab, on, session] = l.split('\t').collect::<Vec<_>>()[..] else { return None };
        Some(Open { wt: wt.into(), tab: tab.parse().ok()?, on: on == "1", session: session.into() })
    });
    (server, all.collect())
}

fn text(server: &str, all: &[Open]) -> String {
    let rows: String = all.iter().map(|o| format!("{}\t{}\t{}\t{}\n", o.wt, o.tab, u8::from(o.on), o.session)).collect();
    format!("{server}\n{rows}")
}

/// Which tmux this is.
fn server() -> String {
    tmux(&["display-message", "-p", "#{pid} #{start_time}"]).unwrap_or_default()
}

/// For the sidebar: keeps the list. What another tmux left is put aside, for `restore`.
pub struct Keeper {
    server: String,
    last: String,
}

impl Keeper {
    pub fn start() -> Keeper {
        let server = server();
        let old = read_state("open.tsv").unwrap_or_default();
        let (was, all) = parse(&old);
        if !server.is_empty() && !all.is_empty() && was != server {
            // beside what an earlier one left and nobody brought back
            let _lock = lock();
            let mut lost = parse(&read_state("lost.tsv").unwrap_or_default()).1;
            lost.retain(|o| !o.session.is_empty() && !all.iter().any(|n| key(n) == key(o)));
            lost.extend(all);
            write_state("lost.tsv", &text("", &lost)).ok();
        }
        Keeper { server, last: String::new() }
    }

    pub fn keep(&mut self, panes: &[&Pane]) {
        if self.server.is_empty() {
            return;
        }
        let mut all: Vec<Open> = panes.iter().map(|p| Open {
            wt: p.wt.clone(), tab: p.tab, on: p.on, session: session_of(p),
        }).collect();
        all.sort_by(|a, b| (&a.wt, a.tab).cmp(&(&b.wt, b.tab)));
        let now = text(&self.server, &all);
        if now != self.last && write_state("open.tsv", &now).is_ok() {
            self.last = now;
        }
    }
}

/// Not open now: a conversation, by itself; a plain terminal, if its worktree has none.
fn missing(all: Vec<Open>, live: &[Pane]) -> Vec<Open> {
    let wts = load();
    all.into_iter().filter(|o| wts.iter().any(|w| w.id() == o.wt && w.path().exists())).filter(|o| {
        if o.session.is_empty() { !live.iter().any(|p| p.wt == o.wt) } else { !live.iter().any(|p| key(&Open { wt: p.wt.clone(), tab: p.tab, on: p.on, session: session_of(p) }) == key(o)) }
    }).collect()
}

/// Program and conversation, without how it was open. "The last one of its folder" is one per worktree.
fn key(o: &Open) -> String {
    let id = o.session.split('|').take(2).collect::<Vec<_>>().join("|");
    if id.ends_with("|last") { format!("{id}|{}", o.wt) } else { id }
}

/// The conversation of a tab. Claude Code and OpenCode tell theirs. For agents without hooks: Antigravity's is read
/// from its own history (never written), and Codex and Gemini CLI go on with the last one of their folder, where they
/// keep them.
fn session_of(p: &Pane) -> String {
    if p.shell {
        return String::new();
    }
    if !p.session.is_empty() {
        return p.session.clone();
    }
    match tui::program(&p.cmd) {
        "agy" => find(Some(&p.wt)).ok().and_then(|w| agy_conversation(&w.path())).map(|id| format!("agy|{id}|||")).unwrap_or_default(),
        name @ ("codex" | "gemini") => format!("{name}|last|||"),
        _ => String::new(),
    }
}

/// The last conversation of Antigravity in a folder, from the end of its history: one line per prompt, with its
/// folder and its conversation. Read again only when the file changes.
fn agy_conversation(dir: &Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    type Seen = Option<(std::time::SystemTime, Vec<(String, String)>)>;
    static SEEN: std::sync::Mutex<Seen> = std::sync::Mutex::new(None);
    let path = home().join(".gemini/antigravity-cli/history.jsonl");
    let at = fs::metadata(&path).and_then(|m| m.modified()).ok()?;
    let mut seen = SEEN.lock().ok()?;
    if seen.as_ref().map(|s| s.0) != Some(at) {
        let mut f = fs::File::open(&path).ok()?;
        let len = f.metadata().ok()?.len();
        f.seek(SeekFrom::Start(len.saturating_sub(256 * 1024))).ok()?;
        let mut text = String::new();
        f.take(256 * 1024).read_to_string(&mut text).ok()?;
        *seen = Some((at, conversations(&text)));
    }
    let dir = dir.to_string_lossy();
    seen.as_ref()?.1.iter().rev().find(|(w, _)| *w == dir).map(|(_, id)| id.clone())
}

/// (folder, conversation) of each line of Antigravity's history, in order.
fn conversations(text: &str) -> Vec<(String, String)> {
    text.lines().filter_map(|l| {
        let (w, id) = (agent::field(l, "workspace")?, agent::field(l, "conversationId")?);
        is_word(id).then(|| (w.to_string(), id.to_string()))
    }).collect()
}

/// What can be opened again: its worktree is still there.
pub fn lost() -> Vec<Open> {
    missing(parse(&read_state("lost.tsv").unwrap_or_default()).1, &panes())
}

pub fn forget() {
    let _lock = lock();
    fs::remove_file(state_dir().join("lost.tsv")).ok();
}

/// A model or a mode: starts like a word, and is not an option.
pub fn is_word(s: &str) -> bool {
    s.starts_with(|c: char| c.is_ascii_alphanumeric()) && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_.[]".contains(c))
}

/// What to type to go on with a conversation. Only what is known to be plain: it goes to a shell.
pub fn resume(session: &str) -> Option<String> {
    let [program, id, dir, model, mode] = session.split('|').collect::<Vec<_>>()[..] else { return None };
    if !is_word(id) {
        return None;
    }
    match program {
        "claude" => {
            let dir = if dir.is_empty() { String::new() } else { format!("CLAUDE_CONFIG_DIR={} ", config::sh(dir)) };
            let option = |name: &str, v: &str| if is_word(v) { format!(" {name} {}", config::sh(v)) } else { String::new() };
            // "command": an alias could be another account
            Some(format!("{dir}command claude{}{} --resume {id}", option("--model", model), option("--permission-mode", mode)))
        }
        "opencode" => Some(format!("opencode --session {id}")),
        "agy" => Some(format!("agy --conversation {id}")),
        "codex" if id == "last" => Some("codex resume --last".into()),
        "gemini" if id == "last" => Some("gemini --resume latest".into()),
        _ => None,
    }
}

/// Opens them again, with the conversations that can go on. Returns how many. What fails is kept for later.
pub fn restore() -> Res<usize> {
    let all = {
        let _lock = lock(); // taken at once: two must not open the same
        let all = lost();
        fs::remove_file(state_dir().join("lost.tsv")).ok();
        all
    };
    let (mut done, mut failed, mut why) = (0, vec![], String::new());
    for o in all {
        match open(&o) {
            Ok(()) => done += 1,
            Err(e) => {
                failed.push(o);
                why = e;
            }
        }
    }
    if failed.is_empty() {
        return Ok(done);
    }
    let _lock = lock();
    let mut left = parse(&read_state("lost.tsv")?).1;
    left.extend(failed);
    write_state("lost.tsv", &text("", &left))?;
    Err(why)
}

fn open(o: &Open) -> Res<()> {
    let w = load().into_iter().find(|w| w.id() == o.wt).ok_or_else(|| (tr().no_worktree)(&o.wt))?;
    let pane = spawn(&w)?;
    if o.on {
        for p in tabs(&o.wt) {
            tmux(&["set-option", "-p", "-t", &p.pane, "@orcux_on", if p.pane == pane { "1" } else { "0" }]).ok();
        }
    }
    match resume(&o.session) {
        Some(c) => {
            // so that it counts as open before the agent tells
            tmux(&["set-option", "-p", "-t", &pane, "@orcux_session", &o.session]).ok();
            type_into(&pane, &c)
        }
        None => Ok(()),
    }
}

pub fn cmd(args: &[&str]) -> Res<()> {
    match args {
        ["--list"] => lost().iter().for_each(|o| println!("{}:{:<3} {}", o.wt, o.tab, o.session.split('|').next().unwrap_or(""))),
        ["--forget"] => forget(),
        [] => println!("{}", (tr().restored)(restore()?)),
        _ => return Err(tr().restore_usage.into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_list() {
        let all = vec![
            Open { wt: "web/a".into(), tab: 1, on: true, session: "claude|1-2|/x|m|plan".into() },
            Open { wt: "web/a".into(), tab: 2, on: false, session: String::new() },
        ];
        assert_eq!(parse(&text("12 34", &all)), ("12 34".to_string(), all));
        assert_eq!(parse(""), (String::new(), vec![]));
    }

    #[test]
    fn resumes() {
        assert_eq!(resume("claude|ab-1||claude-opus[1m]|plan").unwrap(), "command claude --model 'claude-opus[1m]' --permission-mode 'plan' --resume ab-1");
        assert_eq!(resume("claude|ab-1|/it's||").unwrap(), "CLAUDE_CONFIG_DIR='/it'\\''s' command claude --resume ab-1");
        assert_eq!(resume("opencode|ses_1|||").unwrap(), "opencode --session ses_1");
        assert_eq!(resume("agy|34055f3e-4fcf|||").unwrap(), "agy --conversation 34055f3e-4fcf");
        assert_eq!((resume("codex|last|||").unwrap().as_str(), resume("gemini|last|||").unwrap().as_str()), ("codex resume --last", "gemini --resume latest"));
        assert_eq!(resume("codex|abc|||"), None);
        let history = "{\"display\":\"say \\\"workspace\\\":\\\"/x\\\"\",\"timestamp\":1,\"workspace\":\"/w/a\",\"conversationId\":\"c-1\"}\n{\"workspace\":\"/w/b\",\"conversationId\":\"c-2\"}\n{\"workspace\":\"/w/a\",\"conversationId\":\"c;3\"}\nhalf a li";
        assert_eq!(conversations(history), [("/w/a".to_string(), "c-1".to_string()), ("/w/b".to_string(), "c-2".to_string())]);
        for bad in ["claude|a;b|||", "claude||||", "vim|1|||", "", "claude|$(id)|||", "claude|--dangerously-skip-permissions|||", "claude|a|||--x y|z"] {
            assert_eq!(resume(bad), None, "{bad}");
        }
        assert_eq!(resume("claude|a||--x|-y").unwrap(), "command claude --resume a");
    }
}
