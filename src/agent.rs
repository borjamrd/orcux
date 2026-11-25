//! What a Claude Code agent says about itself. orcux's terminals load a small plugin (through an environment
//! variable: the user's settings are not touched) whose hooks run `orcux hook`, which marks the agent's pane with
//! `@orcux_state`. Other agents have no marks: for them, "working" means that their screen moves.
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
    /// "question 1764097000": what, and since when.
    pub fn parse(mark: &str) -> Option<(Agent, u64)> {
        let (name, at) = mark.split_once(' ')?;
        Some((NAMES.iter().find(|n| n.0 == name)?.1, at.parse().ok()?))
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

/// What a pane looks like from outside.
pub struct Screen {
    /// seconds since it last changed, if it did since it is watched
    pub still: Option<u64>,
    pub text: String,
    /// in front of the user
    pub visible: bool,
}

/// Whether the agent works, and its new state if what it last said is out of date: no hook tells that a question
/// was answered or rejected, that a turn was interrupted, or that the user has seen the result.
pub fn settle(mark: Option<(Agent, u64)>, screen: &Screen, now: u64) -> (bool, Option<Agent>) {
    let moved_within = |secs| screen.still.is_some_and(|s| s < secs);
    let Some((state, at)) = mark else { return (moved_within(3), None) };
    let age = now.saturating_sub(at);
    match state {
        // Claude draws something all the time while it works (a clock); after an interruption it stops. Without the
        // new mark, the screen moving again because the user types would look like work.
        // ponytail: a still view of Claude (its transcript) looks interrupted until its next hook.
        Agent::Working if age >= 6 && screen.still.is_some_and(|s| s >= 6) => (false, Some(Agent::Idle)),
        Agent::Working => (moved_within(6), None),
        // its dialog is gone: it moved after it was drawn, and the footer of the dialog is not there
        Agent::Question | Agent::Permission if age > 2 && moved_within(age - 2) && !screen.text.contains("Esc to cancel") => {
            (true, Some(Agent::Working))
        }
        Agent::Done | Agent::Failed if screen.visible => (false, Some(Agent::Idle)),
        _ => (false, None),
    }
}

/// Changes the mark of a pane, unless a hook changed it first.
pub fn replace(pane: &str, was: (Agent, u64), state: Agent) {
    let name = |s: Agent| NAMES.iter().find(|n| n.1 == s).unwrap().0;
    let same = format!("#{{==:#{{@orcux_state}},{} {}}}", name(was.0), was.1);
    let set = format!("set-option -p -t '{pane}' @orcux_state '{} {}'", name(state), prs::now());
    tmux(&["if-shell", "-F", "-t", pane, &same, &set]).ok();
}

/// `orcux hook <event>`, run by Claude Code. Says nothing and never fails: its output would reach the agent.
pub fn hook(event: &str) {
    let input = std::io::read_to_string(std::io::stdin()).unwrap_or_default();
    let env = |k: &str| env::var(k).unwrap_or_default();
    // a `claude -p` that the agent itself runs is not the agent
    if env("TMUX_PANE").is_empty() || !matches!(env("CLAUDE_CODE_ENTRYPOINT").as_str(), "" | "cli") {
        return;
    }
    let state = match event {
        "ask" if ["AskUserQuestion", "ExitPlanMode"].iter().any(|t| input.contains(&format!("\"tool_name\":\"{t}\""))) => "question",
        "ask" => "permission",
        e => e,
    };
    let pane = env("TMUX_PANE");
    let set = match state {
        "end" => format!("set-option -p -u -t '{pane}' @orcux_state"),
        _ if NAMES.iter().any(|n| n.0 == state) => format!("set-option -p -t '{pane}' @orcux_state '{state} {}'", prs::now()),
        _ => return,
    };
    tmux(&["if-shell", "-F", "-t", &pane, "#{@orcux}", &set]).ok(); // only orcux's panes
}

/// The plugin, written again when this binary moves or changes it.
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
    let manifest = "{\"name\":\"orcux\",\"version\":\"1.0.0\",\"description\":\"Tells orcux when this agent works, asks or ends\"}\n";
    for (path, text) in [(".claude-plugin/plugin.json", manifest.to_string()), ("hooks/hooks.json", format!("{{\n  \"hooks\": {{\n{}\n  }}\n}}\n", hooks.join(",\n")))] {
        let file = dir.join(path);
        if fs::read_to_string(&file).is_ok_and(|t| t == text) {
            continue;
        }
        fs::create_dir_all(file.parent()?).ok()?;
        let tmp = file.with_extension(std::process::id().to_string());
        fs::write(&tmp, text).and_then(|_| fs::rename(&tmp, &file)).ok()?;
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
    fn settles() {
        let s = |still: Option<u64>, text: &str, visible: bool| Screen { still, text: text.into(), visible };
        let menu = "1. Tea\nEnter to select · Esc to cancel";
        let at = |a: Agent| Some((a, 100));
        assert_eq!(settle(None, &s(Some(1), "", false), 200), (true, None));
        assert_eq!(settle(None, &s(Some(5), "", false), 200), (false, None));
        assert_eq!(settle(at(Agent::Working), &s(Some(5), "", false), 200), (true, None));
        assert_eq!(settle(at(Agent::Working), &s(Some(9), "", false), 200), (false, Some(Agent::Idle))); // interrupted
        assert_eq!(settle(at(Agent::Working), &s(Some(9), "", false), 103), (false, None)); // just told to work
        assert_eq!(settle(at(Agent::Working), &s(None, "", false), 200), (false, None)); // not watched yet
        assert_eq!(settle(at(Agent::Idle), &s(Some(0), "half a prompt", false), 200), (false, None)); // typing after Esc
        assert_eq!(settle(at(Agent::Question), &s(Some(50), menu, false), 200), (false, None)); // moving in the menu
        assert_eq!(settle(at(Agent::Question), &s(Some(50), "", false), 200), (true, Some(Agent::Working))); // answered
        assert_eq!(settle(at(Agent::Question), &s(Some(100), "", false), 200), (false, None)); // only its dialog was drawn
        assert_eq!(settle(at(Agent::Permission), &s(None, "", false), 200), (false, None));
        assert_eq!(settle(at(Agent::Done), &s(Some(1), "", false), 200), (false, None));
        assert_eq!(settle(at(Agent::Done), &s(Some(1), "", true), 200), (false, Some(Agent::Idle))); // seen
        assert_eq!(settle(at(Agent::Idle), &s(Some(0), "", true), 200), (false, None)); // the user types
    }
}
