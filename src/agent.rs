//! What an agent says about itself, as a mark on its pane (`@orcux_state`). The user's settings are not touched.
//! - Claude Code and OpenCode load a small plugin, through environment variables of orcux's terminals, that runs
//!   `orcux hook`. What OpenCode says is exact; Claude Code does not tell everything, and `settle` fills the gaps.
//! - Those that tell in the title of their terminal (Gemini CLI, Codex) are followed by the sidebar.
//! - For the rest, "working" means that their screen moves.
use crate::*;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Agent {
    Working,
    Question,
    Permission,
    Failed,
    Done,
    Idle,
}

const NAMES: [(&str, Agent); 6] = [
    ("working", Agent::Working),
    ("question", Agent::Question),
    ("permission", Agent::Permission),
    ("failed", Agent::Failed),
    ("done", Agent::Done),
    ("idle", Agent::Idle),
];

impl Agent {
    /// "question 1764097000": what, and since when. A third word tells where it comes from, if not a hook of Claude.
    pub fn parse(mark: &str) -> Option<(Agent, u64)> {
        let mut words = mark.split(' ');
        let (name, at) = (words.next()?, words.next()?);
        Some((NAMES.iter().find(|n| n.0 == name)?.1, at.parse().ok()?))
    }

    pub fn name(self) -> &'static str {
        NAMES.iter().find(|n| n.1 == self).unwrap().0
    }

    pub fn waits(self) -> bool {
        matches!(self, Agent::Question | Agent::Permission)
    }

    pub fn text(self) -> &'static str {
        match self {
            Agent::Question => tr().agent_asks,
            Agent::Permission => tr().agent_permission,
            Agent::Failed => tr().agent_failed,
            _ => tr().agent_done,
        }
    }
}

/// In the footer of the dialogs of Claude Code.
pub const DIALOG: &str = "Esc to cancel";

/// What a pane looks like from outside.
pub struct Screen {
    /// seconds since it last changed, if it did since it is watched
    pub still: Option<u64>,
    /// None: not looked at
    pub text: Option<String>,
    /// its dialog was seen on screen since it asked
    pub dialog: bool,
    /// in front of the user
    pub visible: bool,
}

/// Whether the agent works, and its new state if what it last said is out of date: no hook tells that a question
/// was answered or rejected, that a turn was interrupted, or that the user has seen the result.
pub fn settle(mark: Option<(Agent, u64)>, exact: bool, screen: &Screen, now: u64) -> (bool, Option<Agent>) {
    let moved_within = |secs| screen.still.is_some_and(|s| s < secs);
    let Some((state, at)) = mark else { return (moved_within(3), None) };
    let age = now.saturating_sub(at);
    match state {
        Agent::Working if exact => (true, None),
        Agent::Question | Agent::Permission if exact => (false, None),
        // Claude draws something all the time while it works (a clock); after an interruption it stops. Without the
        // new mark, the screen moving again because the user types would look like work.
        // ponytail: a still view of Claude (its transcript) looks interrupted until its next hook.
        Agent::Working if age >= 6 && screen.still.is_some_and(|s| s >= 6) => (false, Some(Agent::Idle)),
        Agent::Working => (moved_within(6), None),
        // its dialog is gone: it was there, or the screen moved after it was drawn, and its footer is not
        Agent::Question | Agent::Permission
            if age > 2 && (screen.dialog || moved_within(age - 2)) && screen.text.as_ref().is_some_and(|t| !t.contains(DIALOG)) =>
        {
            (true, Some(Agent::Working))
        }
        Agent::Done | Agent::Failed if screen.visible => (false, Some(Agent::Idle)),
        _ => (false, None),
    }
}

/// Changes the mark of a pane, unless a hook changed it first.
pub fn replace(p: &Pane, state: Agent) {
    let same = format!("#{{==:#{{@orcux_state}},{}}}", p.mark);
    let source = p.mark.split(' ').nth(2).map(|s| format!(" {s}")).unwrap_or_default();
    let set = format!("set-option -p -t '{}' @orcux_state '{} {}{source}'", p.pane, state.name(), prs::now());
    tmux(&["if-shell", "-F", "-t", &p.pane, &same, &set]).ok();
}

/// What an agent shows at the start of the title of its terminal: the signs of Gemini CLI, or a spinner (Codex).
fn from_title(title: &str) -> Option<Agent> {
    match title.chars().next()? {
        '✦' => Some(Agent::Working),
        '✋' => Some(Agent::Permission),
        '◇' => Some(Agent::Idle),
        '\u{2801}'..='\u{28ff}' => Some(Agent::Working),
        _ => None,
    }
}

/// The new mark of an agent that is followed by its title, if it changed: what it was and what the title tells.
fn next_by_title(was: Option<Agent>, title: &str) -> Option<Agent> {
    let new = match (was, from_title(title)) {
        (_, Some(a @ (Agent::Working | Agent::Permission))) => a,
        (Some(Agent::Working | Agent::Permission), _) => Agent::Done,
        (None, Some(a)) => a,
        _ => return None,
    };
    (was != Some(new)).then_some(new)
}

/// Whether a mark is the one of a new terminal, and whether it is too old to be believed: its agent has no hooks, or waits
/// for something before them.
pub fn begins(mark: &str) -> (bool, bool) {
    let limit = match mark.split(' ').nth(2) {
        Some("starting") => 1800, // its preparation
        Some("launched") => 15,
        _ => return (false, false),
    };
    (true, Agent::parse(mark).is_none_or(|a| prs::now().saturating_sub(a.1) > limit))
}

/// Follows the title of a pane that no plugin marks, and takes off the mark of an agent that left.
pub fn follow(p: &mut Pane) {
    let (begins, over) = begins(&p.mark);
    if begins {
        if over {
            let same = format!("#{{==:#{{@orcux_state}},{}}}", p.mark);
            tmux(&["if-shell", "-F", "-t", &p.pane, &same, &format!("set-option -p -u -t '{}' @orcux_state", p.pane)]).ok();
            (p.agent, p.mark) = (None, String::new());
        } else if p.shell || from_title(&p.title).is_none() {
            return;
        }
    }
    if p.shell {
        let old = p.mark.split(' ').nth(1).and_then(|at| at.parse().ok()).is_some_and(|at: u64| at + 3 < prs::now());
        if old {
            let same = format!("#{{==:#{{@orcux_state}},{}}}", p.mark);
            tmux(&["if-shell", "-F", "-t", &p.pane, &same, &format!("set-option -p -u -t '{}' @orcux_state", p.pane)]).ok();
        }
        return;
    }
    if !p.mark.is_empty() && !p.mark.ends_with(" title") && !begins {
        return;
    }
    if let Some(new) = next_by_title(p.agent.map(|a| a.0), &p.title) {
        let mark = format!("{} {} title", new.name(), prs::now());
        let same = format!("#{{==:#{{@orcux_state}},{}}}", p.mark);
        tmux(&["if-shell", "-F", "-t", &p.pane, &same, &format!("set-option -p -t '{}' @orcux_state '{mark}'", p.pane)]).ok();
        (p.agent, p.mark) = (Some((new, prs::now())), mark);
    }
}

/// `orcux hook <event>`, run by Claude Code, or with `exact` by OpenCode. Says nothing and never fails: its output
/// would reach the agent.
pub fn hook(event: &str, rest: &[&str]) {
    let exact = rest.first() == Some(&"exact");
    let input = if exact { String::new() } else { std::io::read_to_string(std::io::stdin()).unwrap_or_default() };
    let env = |k: &str| env::var(k).unwrap_or_default();
    // a `claude -p` that the agent itself runs is not the agent
    if env("TMUX_PANE").is_empty() || !matches!(env("CLAUDE_CODE_ENTRYPOINT").as_str(), "" | "cli") {
        return;
    }
    let pane = env("TMUX_PANE");
    let only_ours = |set: &str| { tmux(&["if-shell", "-F", "-t", &pane, "#{@orcux}", set]).ok(); };
    let state = match event {
        "session" => {
            if let Some(id) = rest.get(1).filter(|id| plain(id)) {
                only_ours(&format!("set-option -p -t '{pane}' @orcux_session 'opencode|{id}|'"));
            }
            return;
        }
        "ask" if ["AskUserQuestion", "ExitPlanMode"].iter().any(|t| field(&input, "tool_name") == Some(t)) => "question",
        "ask" => "permission",
        e => e,
    };
    let set = match state {
        // its preparation ended; the agent, if any, is next
        "launched" => format!("set-option -p -t '{pane}' @orcux_state 'working {} launched'", prs::now()),
        "end" => {
            // closed by the person, not by a tmux that ends: there is nothing to go on with
            if matches!(field(&input, "reason"), Some("prompt_input_exit" | "logout")) {
                only_ours(&format!("set-option -p -u -t '{pane}' @orcux_session"));
            }
            format!("set-option -p -u -t '{pane}' @orcux_state")
        }
        _ if NAMES.iter().any(|n| n.0 == state) => format!("set-option -p -t '{pane}' @orcux_state '{state} {}{}'", prs::now(), if exact { " exact" } else { "" }),
        _ => return,
    };
    only_ours(&set); // only orcux's panes
    // which conversation it is, and how: only what Claude Code says in events where nothing else is written by others
    if !exact && matches!(event, "idle" | "working") && field(&input, "hook_event_name").is_some_and(|e| ["SessionStart", "UserPromptSubmit"].contains(&e))
        && let Some(id) = field(&input, "session_id").filter(|id| plain(id))
    {
        let dir = env("CLAUDE_CONFIG_DIR");
        if !dir.contains(['\'', '|']) && !dir.chars().any(char::is_control) {
            only_ours(&format!("set-option -p -t '{pane}' @orcux_session 'claude|{id}|{dir}'"));
        }
        for (name, option) in [("model", "@orcux_model"), ("permission_mode", "@orcux_mode")] {
            if let Some(v) = field(&input, name).filter(|v| restore::is_word(v)) {
                only_ours(&format!("set-option -p -t '{pane}' {option} '{v}'"));
            }
        }
    }
}

fn plain(s: &str) -> bool {
    s.starts_with(|c: char| c.is_ascii_alphanumeric()) && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
}

/// A text field of what Claude Code sends, which is one line of JSON.
fn field<'a>(input: &'a str, name: &str) -> Option<&'a str> {
    input.split(&format!("\"{name}\":\"")).nth(1)?.split('"').next()
}

/// Replaces a file of a plugin, if it is not like that already.
fn put(file: &Path, text: &str) -> Option<()> {
    if fs::read_to_string(file).is_ok_and(|t| t == text) {
        return Some(());
    }
    fs::create_dir_all(file.parent()?).ok()?;
    let tmp = file.with_extension(std::process::id().to_string());
    fs::write(&tmp, text).and_then(|_| fs::rename(&tmp, file)).ok()
}

/// What OpenCode loads: a file, and the configuration that names it, for an environment variable.
pub fn opencode() -> Option<String> {
    let exe = env::current_exe().ok()?.to_string_lossy().to_string();
    let file = state_dir().join("opencode/orcux.js");
    let path = file.to_string_lossy().to_string();
    if exe.contains(['"', '\\']) || path.contains(['"', '\\']) {
        return None;
    }
    put(&file, &include_str!("opencode.js").replace("ORCUX_EXE", &exe))?;
    Some(format!("{{\"plugin\":[\"file://{path}\"]}}"))
}

/// How an agent uses orcux, for the agents.
pub const SKILL: &str = include_str!("skill.md");

/// The plugin of Claude Code, written again when this binary moves or changes it.
pub fn plugin() -> Option<PathBuf> {
    let exe = env::current_exe().ok()?.to_string_lossy().to_string();
    if exe.contains(['"', '\\', '\'']) {
        return None;
    }
    let events = [
        ("SessionStart", "startup|resume|clear|fork", "idle"),
        ("UserPromptSubmit", "", "working"),
        ("PermissionRequest", "*", "ask"),
        ("Elicitation", "*", "question"),
        ("ElicitationResult", "*", "working"),
        ("PostToolBatch", "", "working"),
        ("Stop", "", "done"),
        ("StopFailure", "", "failed"),
        ("SessionEnd", "", "end"),
    ];
    let hooks: Vec<String> = events.iter().map(|(event, matcher, state)| {
        let matcher = if matcher.is_empty() { String::new() } else { format!("\"matcher\":\"{matcher}\",") };
        let cmd = format!("[ ! -x '{exe}' ] || '{exe}' hook {state}");
        format!("    \"{event}\": [{{{matcher}\"hooks\":[{{\"type\":\"command\",\"command\":\"{cmd}\",\"timeout\":5}}]}}]")
    }).collect();
    let dir = state_dir().join("claude-plugin");
    let manifest = "{\"name\":\"orcux\",\"version\":\"1.0.0\",\"description\":\"Tells orcux when this agent works, asks or ends, and how to use orcux\"}\n";
    let hooks = format!("{{\n  \"hooks\": {{\n{}\n  }}\n}}\n", hooks.join(",\n"));
    for (path, text) in [(".claude-plugin/plugin.json", manifest), ("hooks/hooks.json", hooks.as_str()), ("skills/orcux/SKILL.md", SKILL)] {
        put(&dir.join(path), text)?;
    }
    Some(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks() {
        assert_eq!(Agent::parse("question 17"), Some((Agent::Question, 17)));
        assert_eq!((Agent::parse(""), Agent::parse("what 1"), Agent::parse("done")), (None, None, None));
    }

    #[test]
    fn titles() {
        let t = next_by_title;
        assert_eq!(t(None, "◇ Ready (web)"), Some(Agent::Idle));
        assert_eq!(t(Some(Agent::Idle), "✦ Working… (web)"), Some(Agent::Working));
        assert_eq!(t(Some(Agent::Working), "✦ Working… (web)"), None);
        assert_eq!(t(Some(Agent::Working), "✋ Action Required (web)"), Some(Agent::Permission));
        assert_eq!(t(Some(Agent::Permission), "◇ Ready (web)"), Some(Agent::Done));
        assert_eq!(t(None, "⠋ web"), Some(Agent::Working)); // Codex: a spinner, then only the project
        assert_eq!(t(Some(Agent::Working), "web"), Some(Agent::Done));
        assert_eq!((t(None, "vim"), t(None, ""), t(Some(Agent::Done), "web")), (None, None, None));
    }

    #[test]
    fn exact_marks_are_believed() {
        let quiet = Screen { still: Some(900), text: Some(String::new()), dialog: true, visible: false };
        for (a, busy) in [(Agent::Working, true), (Agent::Question, false), (Agent::Permission, false)] {
            assert_eq!(settle(Some((a, 100)), true, &quiet, 2000), (busy, None), "{a:?}");
        }
        assert_eq!(Agent::parse("question 17 exact"), Some((Agent::Question, 17)));
    }

    #[test]
    fn settles() {
        let s = |still: Option<u64>, text: &str, visible: bool| Screen { still, text: Some(text.into()), dialog: false, visible };
        let menu = "1. Tea\nEnter to select · Esc to cancel";
        let at = |a: Agent| Some((a, 100));
        assert_eq!(settle(None, false, &s(Some(1), "", false), 200), (true, None));
        assert_eq!(settle(None, false, &s(Some(5), "", false), 200), (false, None));
        assert_eq!(settle(at(Agent::Working), false, &s(Some(5), "", false), 200), (true, None));
        assert_eq!(settle(at(Agent::Working), false, &s(Some(9), "", false), 200), (false, Some(Agent::Idle))); // interrupted
        assert_eq!(settle(at(Agent::Working), false, &s(Some(9), "", false), 103), (false, None)); // just told to work
        assert_eq!(settle(at(Agent::Working), false, &s(None, "", false), 200), (false, None)); // not watched yet
        assert_eq!(settle(at(Agent::Idle), false, &s(Some(0), "half a prompt", false), 200), (false, None)); // typing after Esc
        assert_eq!(settle(at(Agent::Question), false, &s(Some(50), menu, false), 200), (false, None)); // moving in the menu
        assert_eq!(settle(at(Agent::Question), false, &s(Some(50), "", false), 200), (true, Some(Agent::Working))); // answered
        assert_eq!(settle(at(Agent::Question), false, &s(Some(100), "", false), 200), (false, None)); // only its dialog was drawn
        assert_eq!(settle(at(Agent::Permission), false, &s(None, "", false), 200), (false, None));
        let quick = |text: Option<&str>| Screen { still: Some(98), text: text.map(String::from), dialog: true, visible: true };
        assert_eq!(settle(at(Agent::Permission), false, &quick(Some("rejected at once")), 200), (true, Some(Agent::Working)));
        assert_eq!(settle(at(Agent::Permission), false, &quick(None), 200), (false, None)); // not looked at
        assert_eq!(settle(at(Agent::Done), false, &s(Some(1), "", false), 200), (false, None));
        assert_eq!(settle(at(Agent::Done), false, &s(Some(1), "", true), 200), (false, Some(Agent::Idle))); // seen
        assert_eq!(settle(at(Agent::Idle), false, &s(Some(0), "", true), 200), (false, None)); // the user types
    }
}
