use crate::{agent::{self, Screen}, control::{self, News}, i18n::tr, prs::{self, Cache, Detail, Group, Pr, State}, *};
use ratatui::{
    crossterm::{event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind}, execute},
    prelude::*,
    widgets::{Block, Paragraph, Wrap},
};
use std::{
    collections::{HashMap, VecDeque},
    hash::{DefaultHasher, Hash, Hasher},
    io::stdout,
    sync::{atomic::{AtomicBool, Ordering}, mpsc},
    time::Instant,
};

const ABYSS: Color = Color::from_u32(theme::ABYSS);
const HULL: Color = Color::from_u32(theme::HULL);
const WAKE: Color = Color::from_u32(theme::WAKE);
const LINE: Color = Color::from_u32(theme::LINE);
const FOAM: Color = Color::from_u32(theme::FOAM);
const MUTED: Color = Color::from_u32(theme::MUTED);
const CURRENT: Color = Color::from_u32(theme::CURRENT);
const SIGNAL: Color = Color::from_u32(theme::SIGNAL);
const CORAL: Color = Color::from_u32(theme::CORAL);
const KELP: Color = Color::from_u32(theme::KELP);

/// Size of the small dialogs, borders included: fits a wrapped git error.
const DIALOG: (u32, u32) = (64, 10);

fn look(s: State) -> (&'static str, Color) {
    match s {
        State::Failed => ("✕", CORAL),
        State::Waiting => ("◆", SIGNAL),
        State::Asking => ("?", SIGNAL),
        State::Broken => ("!", CORAL),
        State::Finished => ("■", KELP),
        State::Working => ("●", CURRENT),
        State::Idle => ("○", MUTED),
        State::Done => ("✓", KELP),
    }
}

pub fn look_hex(s: State) -> (&'static str, u32) {
    let color = match s {
        State::Failed | State::Broken => theme::CORAL,
        State::Waiting | State::Asking => theme::SIGNAL,
        State::Working => theme::CURRENT,
        State::Idle => theme::MUTED,
        State::Done | State::Finished => theme::KELP,
    };
    (look(s).0, color)
}

fn fg<'a>(s: impl Into<String>, c: Color) -> Span<'a> {
    Span::styled(s.into(), Style::new().fg(c))
}

/// `s`, cut with … if it is wider than `room`.
fn fit(s: &str, room: usize) -> String {
    if Span::raw(s).width() <= room {
        return s.into();
    }
    let mut cut = String::new();
    for c in s.chars() {
        if Span::raw(format!("{cut}{c}…")).width() > room {
            break;
        }
        cut.push(c);
    }
    if room > 0 {
        cut.push('…');
    }
    cut
}

/// What runs in a terminal. Claude Code names its process after its version ("2.1.332").
pub fn program(cmd: &str) -> &str {
    if cmd.contains('.') && cmd.chars().all(|c| c.is_ascii_digit() || c == '.') { "claude" } else { cmd }
}

/// One full-width line: `left` cut with … if it does not fit, `right` flush right.
fn lr<'a>(width: u16, left: Vec<Span<'a>>, right: Vec<Span<'a>>) -> Line<'a> {
    let rw: usize = right.iter().map(Span::width).sum();
    let room = (width as usize).saturating_sub(rw);
    let (mut out, mut used) = (vec![], 0);
    for s in left {
        if used + s.width() <= room {
            used += s.width();
            out.push(s);
            continue;
        }
        let cut = fit(&s.content, room - used);
        used += Span::raw(cut.as_str()).width();
        out.push(Span::styled(cut, s.style));
        break;
    }
    out.push(Span::raw(" ".repeat(room.saturating_sub(used))));
    out.extend(right);
    Line::from(out)
}

/// Draws blocks of lines, scrolled so that the selected one is visible. The selection is a soft fill; without
/// the focus it stays, dimmer. Colors of the text are kept.
fn blocks(f: &mut Frame, area: Rect, items: Vec<(Vec<Line>, bool)>, active: bool) -> Vec<(u16, u16)> {
    let (mut lines, mut end, mut spans) = (vec![], 0, vec![]);
    for (block, selected) in items {
        let start = lines.len();
        for mut l in block {
            if selected {
                let bg = if active { WAKE } else { DIM };
                l.spans.iter_mut().for_each(|s| s.style.bg = s.style.bg.or(Some(bg)));
            }
            lines.push(l);
        }
        if selected {
            end = lines.len();
        }
        spans.push((start, lines.len()));
    }
    let top = end.saturating_sub(area.height as usize);
    f.render_widget(Paragraph::new(lines).scroll((top as u16, 0)), area);
    let row = |l: usize| area.y + (l.saturating_sub(top) as u16).min(area.height);
    spans.into_iter().map(|(a, b)| (row(a), row(b))).collect() // where each block is on screen: [from, to)
}

/// The selection of a pane without the focus.
const DIM: Color = Color::from_u32(0x1f2329);

fn keys<'a>(pairs: &[(&'a str, &'a str)]) -> Line<'a> {
    Line::from(pairs.iter().flat_map(|(k, v)| [" ".into(), fg(*k, FOAM), fg(format!(" {v} "), MUTED)]).collect::<Vec<Span>>())
}

/// " ×3" for a worktree with three tabs; nothing for one.
fn times(tabs: usize) -> String {
    if tabs > 1 { format!(" ×{tabs}") } else { String::new() }
}

/// Whether this process's pane has the focus.
fn active() -> bool {
    let me = env::var("TMUX_PANE").unwrap_or_default();
    tmux(&["display-message", "-p", "-t", &me, "#{pane_active}"]).is_ok_and(|s| s == "1")
}

/// What wakes a pane that stays open. Nothing is asked to tmux over and over: it tells.
enum Wake {
    Input(Input),
    /// panes, their marks or what they print
    Tmux,
    /// some pane took the focus
    Focus,
    Resize,
    /// that connection
    Closed(u32),
}

struct Wakes {
    tx: mpsc::Sender<Wake>,
    rx: mpsc::Receiver<Wake>,
    /// what came together with news of tmux
    later: VecDeque<Wake>,
    /// which connection it is, and the last try
    link: u32,
    tried: Instant,
    ticked: Instant,
    files: u64,
    trees: (Instant, u64),
}

impl Wakes {
    fn start() -> Wakes {
        let (tx, rx) = mpsc::channel();
        let keys = tx.clone();
        thread::spawn(move || while let Ok(e) = event::read() {
            let wake = match e {
                Event::Resize(..) => Wake::Resize,
                e => match input(e) {
                    Some(i) => Wake::Input(i),
                    None => continue,
                },
            };
            if keys.send(wake).is_err() {
                break;
            }
        });
        let mut w = Wakes { tx, rx, later: VecDeque::new(), link: 0, tried: Instant::now(), ticked: Instant::now(), files: 0, trees: (Instant::now(), 0) };
        w.connect();
        w.changed();
        w
    }

    fn connect(&mut self) {
        self.link += 1;
        let (tx, link) = (self.tx.clone(), self.link);
        self.tried = Instant::now();
        control::connect(move |n| {
            tx.send(match n {
                News::Changed => Wake::Tmux,
                News::Focus => Wake::Focus,
                News::Closed => Wake::Closed(link),
            }).ok();
        });
    }

    /// What happened, or None after a second without news. Several news of tmux count as one.
    fn next(&mut self) -> Option<Wake> {
        let first = match self.later.pop_front() {
            Some(w) => w,
            None => self.rx.recv_timeout(Duration::from_secs(1)).ok()?,
        };
        if matches!(first, Wake::Closed(n) if n == self.link) {
            control::close();
        }
        if matches!(first, Wake::Tmux) {
            self.later.extend(self.rx.try_iter().filter(|w| !matches!(w, Wake::Tmux)));
        }
        Some(first)
    }

    /// Once a second at most.
    fn tick(&mut self) -> bool {
        let due = self.ticked.elapsed().as_millis() >= 950;
        if due {
            self.ticked = Instant::now();
            if !control::connected() && self.tried.elapsed().as_secs() >= 5 {
                self.connect();
            }
        }
        due
    }

    /// Whether a state file changed, or the branch or the folder of a worktree (looked at less often).
    fn changed(&mut self) -> bool {
        let mut h = DefaultHasher::new();
        for f in ["worktrees.tsv", "repos", "folded", "lang", "icons"] {
            fs::metadata(state_dir().join(f)).and_then(|m| m.modified()).ok().hash(&mut h);
        }
        fs::metadata(prs::cache_file()).and_then(|m| m.modified()).ok().hash(&mut h);
        let files = h.finish();
        let mut trees = self.trees.1;
        if self.trees.0.elapsed().as_secs() >= 5 || trees == 0 {
            let mut h = DefaultHasher::new();
            load().iter().for_each(|w| (w.gone(), w.branch()).hash(&mut h));
            trees = h.finish();
            self.trees = (Instant::now(), self.trees.1);
        }
        let new = (files, trees) != (self.files, self.trees.1);
        (self.files, self.trees.1) = (files, trees);
        new
    }
}

enum Row {
    /// where some repos are, its name and how many they are
    Folder(PathBuf, String, usize),
    /// and how many worktrees it has
    Repo(PathBuf, usize),
    /// and whether it is the last of its siblings, at every level
    Wt(Wt, Vec<bool>),
}

/// What is known of a terminal's screen. Times are of the clock, as those of tmux and of the marks.
#[derive(Default)]
struct Seen {
    visible: bool,
    /// since when it is visible, or hidden
    since: u64,
    hash: Option<u64>,
    /// last time that it changed while visible
    moved: Option<u64>,
    /// last activity while hidden
    activity: u64,
    /// the mark whose dialog was seen on screen
    dialog: u64,
}

/// An interface is on screen: nothing else may be printed.
pub static DRAWING: AtomicBool = AtomicBool::new(false);
/// Whether a person has the session open, for the thread that fetches PRs.
static WATCHED: AtomicBool = AtomicBool::new(true);

struct App {
    rows: Vec<Row>,
    sel: usize,
    /// last-resort error line, for when not even a popup can be opened
    msg: String,
    /// what is known of each terminal's screen
    seen: HashMap<String, Seen>,
    /// the last mark of each terminal that was told to the person
    told: HashMap<String, u64>,
    keeper: restore::Keeper,
    started: bool,
    /// to notice that the clock jumped or the computer slept
    clock: (Instant, u64),
    wts: Vec<Wt>,
    /// worktree id -> its tabs, for those that have a terminal
    live: HashMap<String, Vec<Pane>>,
    prs: Cache,
    active: bool,
    /// pane -> last @orcux_busy sent to tmux
    busy: HashMap<String, bool>,
    /// last worktree on screen, and whether the last refresh found no terminal at all
    visible: String,
    missing: bool,
    /// last tab bar sent to tmux
    bar: String,
    /// last status bar sent to tmux
    status: (String, String),
}

impl App {
    fn refresh(&mut self) {
        self.wts = load();
        let wts = self.wts.clone();
        let key = |r: &Row| match r {
            Row::Folder(p, ..) | Row::Repo(p, _) => p.to_string_lossy().to_string(),
            Row::Wt(w, _) => w.id(),
        };
        let was = self.selected().map(key);
        self.rows.clear();
        let (folded, folders) = (folded(), by_folder(&repos()));
        let many = folders.len() > 1;
        for (dir, name, repos) in folders {
            if many {
                self.rows.push(Row::Folder(dir.clone(), name, repos.len()));
                if folded.contains(&dir) {
                    continue;
                }
            }
            self.add_repos(&repos, &wts, &folded);
        }
        // the same row stays selected when others come and go
        if let Some(i) = was.and_then(|k| self.rows.iter().position(|r| key(r) == k)) {
            self.sel = i;
        }
        self.sel = self.sel.min(self.rows.len().saturating_sub(1));
        self.live.clear();
        let front = terminal_pane().map(|p| p.0);
        let watched = viewer().is_some();
        WATCHED.store(watched, Ordering::Relaxed);
        self.active = active();
        let now = prs::now();
        let jumped = now.saturating_sub(self.clock.1) > self.clock.0.elapsed().as_secs() + 5;
        self.clock = (Instant::now(), now);
        let mode = work::notify_mode();
        for mut p in panes() {
            if p.wt == SIDEBAR || p.wt == RAIL {
                continue;
            }
            let visible = front.as_ref() == Some(&p.pane);
            agent::follow(&mut p);
            let exact = p.mark.split(' ').nth(2).is_some();
            let s = self.seen.entry(p.pane.clone()).or_insert_with(|| Seen { visible, ..Seen::default() });
            if jumped {
                (s.since, s.moved) = (now, s.moved.map(|_| now));
            }
            if s.visible != visible {
                // a hidden terminal tells by its window, and it has just got one with a past of its own
                *s = Seen { visible, since: now, moved: visible.then_some(s.activity), dialog: s.dialog, ..Seen::default() };
            }
            if !visible {
                s.activity = p.activity;
            }
            let asks = p.agent.filter(|a| a.0.waits());
            let working = p.agent.is_some_and(|a| a.0 == Agent::Working);
            let quiet = |at: u64| now.saturating_sub(at);
            // how long a hidden one has been still; the visible one is captured, if its agent does not tell what it needs
            let hidden = quiet(if working { p.activity.max(s.since) } else { p.activity });
            let moved = !visible && asks.is_some_and(|(_, at)| hidden + 2 < quiet(at));
            let unseen = asks.is_some_and(|(_, at)| s.dialog != at);
            let look = !exact && if visible { p.agent.is_none() || working || asks.is_some() } else { moved || unseen };
            let text = look.then(|| tmux(&["capture-pane", "-p", "-t", &p.pane]).ok()).flatten();
            if visible && let Some(text) = &text {
                let mut h = DefaultHasher::new();
                text.hash(&mut h);
                let h = Some(h.finish());
                if s.hash.is_some() && s.hash != h {
                    s.moved = Some(now);
                }
                s.hash = h;
            }
            if let (Some((_, at)), Some(text)) = (asks, &text) && text.contains(agent::DIALOG) {
                s.dialog = at;
            }
            let still = if visible { s.moved.map(quiet) } else { Some(hidden) };
            let dialog = asks.is_some_and(|(_, at)| s.dialog == at);
            let (busy, new) = agent::settle(p.agent, exact, &Screen { still, text, dialog, visible: visible && watched }, now);
            p.busy = busy;
            if let (Some(_), Some(new)) = (p.agent, new) {
                agent::replace(&p, new);
                p.agent = Some((new, now));
            }
            if self.busy.insert(p.pane.clone(), p.busy) != Some(p.busy) {
                set_busy(&p.pane, p.busy);
            }
            // told once, and only if it is not in front of the person; those that were there at the start are known
            if let Some((a, at)) = p.agent && self.told.insert(p.pane.clone(), at) != Some(at) && self.started && !(visible && watched) && work::worth(a, &mode) {
                work::notify(&p.wt, a.text());
            }
            self.live.entry(p.wt.clone()).or_default().push(p);
        }
        self.started = true;
        self.keeper.keep(&self.live.values().flatten().collect::<Vec<_>>());
        self.heal(&wts);
        let bar = tab_bar();
        if self.bar != bar {
            set_tab_bar(&bar);
            self.bar = bar;
        }
        self.prs = prs::cached();
        sync_rail().ok(); // no room for it: stay without
        self.update_status(&wts);
    }

    /// The visible terminal ended (`exit`): shows another tab of its worktree, or the placeholder. Waits for a
    /// second refresh, because another orcux process may be halfway through a change.
    fn heal(&mut self, wts: &[Wt]) {
        match terminal_pane() {
            Some((_, id)) => (self.visible, self.missing) = (id, false),
            None if !self.missing => self.missing = true,
            None => {
                let w = wts.iter().find(|w| w.id() == self.visible && self.live.contains_key(&self.visible));
                w.map_or_else(ensure_ui, show).ok();
                self.missing = false;
            }
        }
    }

    fn working(&self, id: &str) -> bool {
        self.live.get(id).is_some_and(|t| t.iter().any(|p| p.busy))
    }

    /// The tab that tells how a worktree is.
    fn foremost(&self, id: &str) -> Option<&Pane> {
        foremost(self.live.get(id)?.iter())
    }

    fn waits(&self, id: &str) -> bool {
        self.foremost(id).is_some_and(|p| p.look() == State::Asking)
    }

    /// Something that will change by itself: an agent that stops, or a terminal that is missing.
    fn pending(&self) -> bool {
        let asked = |p: &Pane| p.agent.is_some_and(|a| a.0.waits()) && self.seen.get(&p.pane).is_some_and(|s| s.visible);
        self.missing || self.live.values().flatten().any(|p| p.busy || asked(p))
    }

    fn update_status(&mut self, wts: &[Wt]) {
        let esc = |s: &str| s.replace('#', "##").replace('%', "%%");
        let mut left = String::new();
        match wts.iter().filter(|w| self.waits(&w.id())).count() {
            0 => {}
            n => left += &format!(" #[fg={}]{} {} ", hex(theme::SIGNAL), look(State::Asking).0, (tr().agents_wait)(n)),
        }
        match self.prs.prs.iter().filter(|p| p.status().0 == Group::Yours).count() {
            0 => left += " ",
            n => left += &format!(" #[fg={}]◆ {}  ", hex(theme::SIGNAL), (tr().prs_yours)(n)),
        }
        left += &match wts.iter().filter(|w| self.working(&w.id())).count() {
            0 => format!("#[fg={}]○ {}", hex(theme::MUTED), (tr().n_working)(0)),
            n => format!("#[fg={}]● {}", hex(theme::CURRENT), (tr().n_working)(n)),
        };
        let shown = terminal_pane().map(|p| p.1).unwrap_or_default();
        let right = wts.iter().find(|w| w.id() == shown)
            .map(|w| format!("{} #[fg={}]:{} ", esc(&shown), hex(theme::MUTED), w.port())).unwrap_or_default();
        if self.status != (left.clone(), right.clone()) {
            set_status(&left, &right);
            self.status = (left, right);
        }
    }

    fn selected(&self) -> Option<&Row> {
        self.rows.get(self.sel)
    }

    fn repo_of_selected(&self) -> Option<PathBuf> {
        match self.selected()? {
            Row::Folder(..) => None,
            Row::Repo(p, _) => Some(p.clone()),
            Row::Wt(w, _) => Some(w.repo.clone()),
        }
    }

    fn add_repos(&mut self, repos: &[PathBuf], wts: &[Wt], folded: &[PathBuf]) {
        for r in repos {
            let of: Vec<&Wt> = wts.iter().filter(|w| w.repo == *r).collect();
            self.rows.push(Row::Repo(r.clone(), of.len()));
            if !folded.contains(r) {
                self.rows.extend(tree(&of).into_iter().map(|(w, trail)| Row::Wt(w, trail)));
            }
        }
    }

    /// What is going on in some repos: how many PRs need the user and, if they are folded, how many agents work or
    /// wait for the user (unfolded, each worktree tells).
    fn summary<'a>(&self, within: impl Fn(&Path) -> bool, folded: bool) -> Vec<Span<'a>> {
        let mut out = vec![];
        let yours = self.prs.prs.iter().filter(|x| within(&x.path) && x.status().0 == Group::Yours).count();
        if yours > 0 {
            out.push(fg(format!(" ◆ {yours}"), SIGNAL));
        }
        if folded {
            let ids: Vec<String> = self.wts.iter().filter(|w| within(&w.repo)).map(Wt::id).collect();
            let waiting = ids.iter().filter(|id| self.waits(id)).count();
            if waiting > 0 {
                out.insert(0, fg(format!(" {}", (tr().agents_short)(waiting)), SIGNAL));
            }
            let busy = ids.iter().filter(|id| self.working(id)).count();
            if busy > 0 {
                out.push(fg(format!(" ● {busy}"), CURRENT));
            }
        }
        out.push(" ".into());
        out
    }

    /// One line, hung from its repo; the selected one also tells how it is and its port.
    fn wt_lines<'a>(&self, w: &Wt, width: u16, trail: &[bool], selected: bool) -> Vec<Line<'a>> {
        let id = w.id();
        let tabs = self.live.get(&id).map_or(0, Vec::len);
        let pr = self.prs.prs.iter().find(|p| p.is_of(w)).map(|p| (p.number.clone(), p.status()));
        let gone = w.gone();
        let mut right = vec![fg(times(tabs), MUTED), " ".into()];
        if let Some((_, (group, s, _))) = &pr && (*group != Group::Going || *s == State::Failed) {
            right.push(fg(format!("{} ", look(*s).0), look(*s).1));
        }
        if gone {
            right.push(fg("✕ ", CORAL));
        } else if let Some(s) = self.foremost(&id).map(Pane::look).filter(|s| *s != State::Idle) {
            right.push(fg(format!("{} ", look(s).0), look(s).1));
        }
        let name = if tabs > 0 || selected { fg(w.name.clone(), FOAM) } else { fg(w.name.clone(), MUTED) };
        let (last, above) = trail.split_last().unwrap_or((&true, &[]));
        let stem: String = above.iter().map(|l| if *l { "    " } else { "│   " }).collect();
        let branch = format!(" {stem}{}", if *last { "╰── " } else { "├── " });
        let mut lines = vec![lr(width, vec![fg(branch, LINE), name], right)];
        if selected {
            let text = match (pr, self.foremost(&id)) {
                _ if gone => fg(tr().gone, CORAL),
                (_, Some(p)) if matches!(p.look(), State::Asking | State::Broken | State::Finished) => {
                    let (a, at) = p.agent.unwrap_or((Agent::Done, prs::now()));
                    let ago = (tr().ago)(&prs::ago(prs::now().saturating_sub(at)));
                    fg(format!("{}, {ago}", a.text()), look(p.look()).1)
                }
                (Some((number, (_, s, text))), _) => fg(format!("#{number} {text}"), look(s).1),
                (None, None) => fg(tr().unopened, MUTED),
                (None, Some(p)) if p.busy => fg((tr().working)(program(&p.cmd)), CURRENT),
                (None, Some(p)) => fg((tr().idle)(program(&p.cmd)), MUTED),
            };
            let branch = format!(" {stem}{}", if *last { "    " } else { "│   " });
            lines.push(lr(width, vec![fg(branch, LINE), text], vec![fg(format!(" {} ", w.port()), MUTED)]));
        }
        lines
    }

    fn repo_line<'a>(&self, p: &Path, width: u16, wts: usize, folded: bool) -> Line<'a> {
        let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
        let initial = name.chars().next().unwrap_or('?').to_uppercase().to_string();
        let mut h = DefaultHasher::new();
        name.hash(&mut h);
        let color = if wts > 0 { Color::from_u32(theme::BADGES[h.finish() as usize % theme::BADGES.len()]) } else { WAKE };
        let ink = if wts > 0 { ABYSS } else { MUTED };
        let right = self.summary(|x| x == p, folded);
        let end = |s: &'static str| if s.is_empty() { Span::raw(" ").bg(color) } else { fg(s, color) };
        let (open, close) = theme::caps().unwrap_or_default();
        let mut left = vec![end(open), initial.bold().fg(ink).bg(color), end(close), " ".into()];
        left.push(if wts > 0 { name.bold().fg(FOAM) } else { fg(name, MUTED) });
        if wts > 0 {
            left.push(fg(format!("  {wts}{}", if folded { " ▸" } else { "" }), MUTED));
        }
        lr(width, left, right)
    }

    fn draw(&self, f: &mut Frame) {
        f.render_widget(Block::new().style(Style::new().bg(ABYSS).fg(FOAM)), f.area());
        let [_, list, _, foot] = Layout::vertical([Constraint::Length(1), Constraint::Fill(1), Constraint::Length(1), Constraint::Length(2)]).areas(f.area());
        let w = f.area().width;
        let is_wt = |j: usize| matches!(self.rows.get(j), Some(Row::Wt(..)));
        let mut items = vec![];
        for (i, r) in self.rows.iter().enumerate() {
            let sel = i == self.sel;
            match r {
                Row::Folder(p, name, n) => {
                    if i > 0 {
                        items.push((vec![Line::default()], false));
                    }
                    let open = matches!(self.rows.get(i + 1), Some(Row::Repo(r, _)) if r.parent() == Some(p));
                    let right = if open { vec![" ".into()] } else { self.summary(|x| x.parent() == Some(p), true) };
                    let left = vec![fg(if open { " ▾ " } else { " ▸ " }, MUTED), fg(name.clone(), MUTED), fg(if open { String::new() } else { format!("  {n}") }, LINE)];
                    items.push((vec![lr(w, left, right)], sel));
                }
                Row::Repo(p, wts) => {
                    let after_folder = i > 0 && matches!(self.rows[i - 1], Row::Folder(..));
                    if i > 0 && !after_folder && (is_wt(i - 1) || is_wt(i + 1)) {
                        items.push((vec![Line::default()], false));
                    }
                    items.push((vec![self.repo_line(p, w, *wts, *wts > 0 && !is_wt(i + 1))], sel));
                }
                Row::Wt(wt, trail) => items.push((self.wt_lines(wt, w, trail, sel), sel)),
            }
        }
        blocks(f, list, items, self.active);
        if self.msg.is_empty() {
            let lines = vec![
                keys(&[("n", tr().k_new), ("A", tr().k_add), ("d", tr().k_delete)]),
                keys(&[("?", tr().k_commands), ("Opt+w", tr().k_switch)]),
            ];
            f.render_widget(Paragraph::new(lines), foot);
        } else {
            f.render_widget(Paragraph::new(self.msg.as_str()).wrap(Wrap { trim: true }).fg(CORAL), foot);
        }
    }

    fn popup(&mut self, title: &str, size: (u32, u32), args: &[&str]) {
        match popup(title, size, args) {
            Err(e) if !e.ends_with(": ") => self.msg = e, // empty stderr = popup closed by a detach, not a tmux error
            _ => {}
        }
        self.refresh();
    }

    fn error(&mut self, text: &str) {
        self.popup(tr().title_error, DIALOG, &["dialog", "error", text]);
    }
}

enum Input {
    Key(KeyCode),
    Click { col: u16, row: u16 },
    Wheel { down: bool },
}

/// The next key press or mouse action within `ms`. Ctrl-C counts as Esc.
fn poll_input(ms: u64) -> Option<Input> {
    if !event::poll(Duration::from_millis(ms)).unwrap_or(false) {
        return None;
    }
    input(event::read().ok()?)
}

fn input(e: Event) -> Option<Input> {
    match e {
        Event::Key(k) if k.kind == KeyEventKind::Press => {
            Some(Input::Key(if k.modifiers.contains(KeyModifiers::CONTROL) { KeyCode::Esc } else { k.code }))
        }
        Event::Mouse(m) => match m.kind {
            MouseEventKind::Down(MouseButton::Left) => Some(Input::Click { col: m.column, row: m.row }),
            MouseEventKind::ScrollDown => Some(Input::Wheel { down: true }),
            MouseEventKind::ScrollUp => Some(Input::Wheel { down: false }),
            _ => None,
        },
        _ => None,
    }
}

pub fn run() -> Res<()> {
    let mut term = ratatui::init();
    let mut wakes = Wakes::start();
    style_ui().ok();
    thread::spawn(|| loop {
        let last = Instant::now();
        prs::refresh().ok(); // errors go into the cache, the rail shows them
        // nobody looks: less often
        while last.elapsed().as_secs() < if WATCHED.load(Ordering::Relaxed) { 60 } else { 600 } {
            thread::sleep(Duration::from_secs(5));
        }
    });
    let mut app = App {
        rows: vec![], sel: 1, msg: String::new(), seen: HashMap::new(), told: HashMap::new(), keeper: restore::Keeper::start(), started: false, clock: (Instant::now(), prs::now()), live: HashMap::new(),
        prs: Cache::default(), active: true, status: Default::default(), busy: HashMap::new(),
        visible: String::new(), missing: false, bar: String::new(), wts: vec![],
    };
    let (mut last, binary) = (Instant::now(), Binary::now());
    app.refresh();
    let mut draw = true;
    let res = loop {
        if draw && let Err(e) = term.draw(|f| app.draw(f)) {
            break Err(e.to_string());
        }
        let wake = wakes.next();
        if wakes.tick() && binary.replaced() {
            ratatui::restore();
            break Err(restart_self());
        }
        let k = match wake {
            Some(Wake::Input(Input::Key(k))) => k,
            Some(Wake::Focus) => {
                let on = active();
                draw = app.active != on;
                app.active = on;
                continue;
            }
            w => {
                // half a minute: what tells how long ago something was
                draw = matches!(w, Some(Wake::Tmux | Wake::Resize)) || wakes.changed() || app.pending() || !control::connected() || last.elapsed().as_secs() >= 30;
                if draw {
                    app.refresh();
                    last = Instant::now();
                }
                continue;
            }
        };
        draw = true;
        app.msg.clear();
        match k {
            KeyCode::Char('q') => { detach().ok(); }
            KeyCode::Char('j') | KeyCode::Down => app.sel = (app.sel + 1).min(app.rows.len().saturating_sub(1)),
            KeyCode::Char('k') | KeyCode::Up => app.sel = app.sel.saturating_sub(1),
            KeyCode::Char('?') => app.popup(tr().title_commands, (136, 34), &["commands"]),
            KeyCode::Char('p') => if let Err(e) = toggle_rail() {
                app.error(&e);
            },
            KeyCode::Char('r') => { work::next().ok(); } // nobody waits: stay
            KeyCode::Char('R') => if let Err(e) = restore::restore() {
                app.error(&e);
            },
            KeyCode::Char('m') => if let Some(Row::Wt(w, _)) = app.selected() {
                let id = w.id();
                app.popup(tr().title_merge, (76, 14), &["dialog", "merge", &id]);
            },
            KeyCode::Tab => { focus(RAIL).ok(); }
            KeyCode::Char('N') => if let Some(Row::Wt(w, _)) = app.selected() {
                let id = w.id();
                app.popup(tr().title_child, DIALOG, &["dialog", "child", &id]);
            },
            KeyCode::Char('t') => if let Some(Row::Wt(w, _)) = app.selected()
                && let Err(e) = open_tab(&w.clone()) {
                    app.error(&e);
                },
            KeyCode::Char('h') | KeyCode::Left => { step_tab(false).ok(); }
            KeyCode::Char('l') | KeyCode::Right => { step_tab(true).ok(); }
            KeyCode::Char('n') => match app.repo_of_selected() {
                Some(r) => app.popup(tr().title_new, DIALOG, &["dialog", "new", &r.to_string_lossy()]),
                None if app.rows.is_empty() => app.error(tr().no_repos),
                None => {}
            },
            KeyCode::Char('A') => {
                // new folders and clones go beside the selected repo
                let base = match app.selected() {
                    Some(Row::Folder(p, ..)) => p.clone(),
                    _ => app.repo_of_selected().and_then(|r| Some(r.parent()?.to_path_buf())).unwrap_or_else(home),
                };
                app.popup(tr().title_add, (76, 11), &["dialog", "add", &base.to_string_lossy()]);
            }
            KeyCode::Char('d') => match app.selected() {
                Some(Row::Wt(w, _)) => {
                    let (id, title) = (w.id(), if w.at.is_some() { tr().title_unregister } else { tr().title_rm });
                    app.popup(title, DIALOG, &["dialog", "rm", &id]);
                }
                Some(Row::Repo(r, _)) => {
                    let r = r.to_string_lossy().to_string();
                    app.popup(tr().title_unregister, DIALOG, &["dialog", "rmrepo", &r]);
                }
                _ => {}
            },
            KeyCode::Enter | KeyCode::Char(' ') => match app.selected() {
                Some(Row::Folder(p, ..) | Row::Repo(p, _)) => {
                    toggle_fold(&p.clone());
                    app.refresh();
                }
                Some(Row::Wt(w, _)) if k == KeyCode::Enter => if let Err(e) = show(&w.clone()) {
                    app.error(&e);
                },
                _ => {}
            },
            _ => {}
        }
        if !matches!(k, KeyCode::Char('j' | 'k') | KeyCode::Down | KeyCode::Up) {
            app.refresh();
        }
    };
    ratatui::restore();
    res
}

// ---------- switcher: the list of worktrees behind Opt+w (`orcux switch`) ----------

pub fn switch() -> Res<()> {
    let visible = terminal_pane().map(|p| p.1).unwrap_or_default();
    let (wts, prs, live) = (by_recent(&visible), prs::cached().prs, panes());
    let tabs_of = |id: &str| live.iter().filter(|p| p.wt == id).count();
    let pr_of = |w: &Wt| prs.iter().find(|p| p.is_of(w));
    let text = |w: &Wt| format!("{} {}", w.id(), pr_of(w).map(|p| format!("#{} {}", p.number, p.title)).unwrap_or_default()).to_lowercase();
    let (mut query, mut sel, mut err) = (String::new(), None, String::new());
    dialog(|term| loop {
        let words = query.to_lowercase();
        let hits: Vec<&Wt> = wts.iter().filter(|w| words.split_whitespace().all(|q| text(w).contains(q))).collect();
        // start on the previous one, so that Opt+w + Enter goes back and forth
        let cur = sel.unwrap_or_else(|| hits.iter().position(|w| w.id() != visible).unwrap_or(0)).min(hits.len().saturating_sub(1));
        term.draw(|f| {
            let [input, list, foot] = Layout::vertical([Constraint::Length(2), Constraint::Fill(1), Constraint::Length(1)]).areas(f.area());
            let w = f.area().width;
            f.render_widget(Line::from(vec![fg(format!(" {}: ", tr().search), MUTED), fg(format!("{query}▏"), FOAM)]), input);
            let mut items = vec![];
            for (i, wt) in hits.iter().enumerate() {
                let id = wt.id();
                let mine = foremost(live.iter().filter(|p| p.wt == id));
                let (g, c) = look(mine.map_or(State::Idle, Pane::look));
                let mut left = vec![" ".into(), fg(g, c), " ".into(), fg(format!("{}/", wt.repo_name()), MUTED), wt.name.clone().bold().fg(FOAM)];
                if let Some((a, _)) = mine.filter(|p| matches!(p.look(), State::Asking | State::Broken | State::Finished)).and_then(|p| p.agent) {
                    left.push(fg(format!("  {}", a.text()), c));
                } else if let Some(p) = pr_of(wt) {
                    let (_, s, t) = p.status();
                    left.push(fg(format!("  {} #{} {t}", look(s).0, p.number), look(s).1));
                } else if tabs_of(&id) == 0 {
                    left.push(fg(format!("  {}", tr().unopened), MUTED));
                }
                let here = if id == visible { format!("{}  ", tr().visible) } else { String::new() };
                items.push((vec![lr(w, left, vec![fg(format!(" {here}{} {} ", times(tabs_of(&id)), wt.port()), MUTED)])], i == cur));
            }
            if hits.is_empty() {
                let t = if wts.is_empty() { tr().no_worktrees } else { tr().no_match };
                items.push((vec![Line::from(fg(format!(" {t}"), MUTED))], false));
            }
            blocks(f, list, items, true);
            if err.is_empty() {
                f.render_widget(keys(&[("↑/↓", tr().k_move), ("⏎", tr().k_switch), ("esc", tr().k_close)]), foot);
            } else {
                f.render_widget(Line::from(fg(format!(" {err}"), CORAL)), foot);
            }
        }).map_err(|e| e.to_string())?;
        match key()? {
            Some(KeyCode::Esc) => return Ok(()),
            Some(KeyCode::Down) => sel = Some(cur + 1),
            Some(KeyCode::Up) => sel = Some(cur.saturating_sub(1)),
            Some(KeyCode::Backspace) => {
                query.pop();
                sel = None;
            }
            Some(KeyCode::Char(c)) if SWITCH_KEYS.contains(&c.to_string().as_str()) => return Ok(()), // again: close
            Some(KeyCode::Char(c)) => {
                query.push(c);
                sel = None;
            }
            Some(KeyCode::Enter) => if let Some(w) = hits.get(cur) {
                match show(w) {
                    Ok(()) => return Ok(()),
                    Err(e) => err = short(&e),
                }
            },
            _ => {}
        }
    })
}

// ---------- rail: the right pane with the pull requests (`orcux rail`) ----------

fn pr_lines<'a>(p: &Pr, age: u64, w: u16) -> Vec<Line<'a>> {
    let (_, state, text) = p.status();
    let (g, c) = look(state);
    let mut left = vec!["    ".into()];
    if state == State::Working && p.total > 0 {
        let done = ((p.ok + p.failed).min(p.total) * 10 / p.total) as usize;
        left.push(fg("━".repeat(done), CURRENT));
        left.push(fg("━".repeat(10 - done), LINE));
        left.push(" ".into());
    }
    left.push(fg(if state == State::Idle { format!("{text}, {}", prs::ago(age)) } else { text }, MUTED));
    vec![
        lr(w, vec!["  ".into(), fg(g, c), " ".into(), fg(p.title.clone(), FOAM)], vec![fg(format!(" #{}  ", p.number), MUTED)]),
        lr(w, left, vec![" ".into()]),
    ]
}

/// Returns where each PR is on screen: (from, to, its index).
fn draw_rail(f: &mut Frame, c: &Cache, wts: &[Wt], sel: usize, active: bool, busy: bool, msg: &str) -> Vec<(u16, u16, usize)> {
    f.render_widget(Block::new().style(Style::new().bg(HULL).fg(FOAM)), f.area());
    let [head, list, _, foot] = Layout::vertical([Constraint::Length(3), Constraint::Fill(1), Constraint::Length(1), Constraint::Length(2)]).areas(f.area());
    let w = f.area().width;
    let since = prs::now().saturating_sub(c.at);
    let when = match c.at {
        _ if busy => format!("{}  ", tr().updating),
        0 if c.error.is_empty() => format!("{}  ", tr().loading),
        0 => String::new(),
        _ => format!("{}  ", (tr().ago)(&prs::ago(since))),
    };
    let review = c.prs.iter().filter(|p| p.review).count();
    f.render_widget(Paragraph::new(vec![
        Line::default(),
        match c.error.as_str() {
            "" => lr(w, vec![fg(format!("  {}", (tr().counts)(c.prs.len() - review, review)), MUTED)], vec![fg(format!(" {when}"), MUTED)]),
            e => lr(w, vec![fg(format!("  {e}"), CORAL)], vec![fg(format!(" {when}"), MUTED)]), // below it, the last PRs that were read
        },
    ]), head);
    let (mut items, mut which) = (vec![], vec![]); // which: the PR of each item
    let mut push = |lines: Vec<Line<'static>>, i: Option<usize>| {
        items.push((lines, i == Some(sel)));
        which.push(i);
    };
    for (group, name, state) in [(Group::Yours, tr().group_yours, State::Waiting), (Group::Going, tr().group_going, State::Working), (Group::Ready, tr().group_ready, State::Done)] {
        let of: Vec<(usize, &Pr)> = c.prs.iter().enumerate().filter(|p| p.1.status().0 == group).collect();
        if of.is_empty() {
            continue;
        }
        push(vec![Line::from(vec!["  ".into(), name.bold().fg(look(state).1), fg(format!("  {}", of.len()), MUTED)])], None);
        let mut repo = "";
        for (i, p) in of {
            if p.repo != repo {
                repo = &p.repo;
                push(vec![lr(w, vec![fg(format!("  {}", p.repo_name()), MUTED)], vec![]), Line::default()], None);
            }
            push(pr_lines(p, p.age + since, w), Some(i));
            push(vec![Line::default()], None);
        }
    }
    if c.prs.is_empty() && c.at > 0 {
        push(vec![Line::from(fg(format!("  {}", tr().no_prs), MUTED))], None);
    }
    let rows = blocks(f, list, items, active);
    if msg.is_empty() {
        let first = match c.prs.get(sel) {
            Some(p) if p.worktree(wts).is_some() => vec![("⏎", tr().k_view), ("o", tr().k_web)],
            Some(_) => vec![("⏎", tr().k_view), ("w", tr().k_create), ("o", tr().k_web)],
            None => vec![],
        };
        f.render_widget(Paragraph::new(vec![keys(&first), keys(&[("p", tr().k_hide), ("r", tr().k_refresh), ("tab", tr().k_back)])]), foot);
    } else {
        f.render_widget(Paragraph::new(format!(" {msg}")).wrap(Wrap { trim: true }).fg(CORAL), foot);
    }
    rows.into_iter().zip(which).filter_map(|((a, b), i)| Some((a, b, i?))).collect()
}

/// Keys typed in the placeholder are for the sidebar.
fn pass_on(k: KeyCode) {
    let Some(side) = ui_pane(SIDEBAR) else { return };
    focus(SIDEBAR).ok();
    match k {
        KeyCode::Char(c) => tmux(&["send-keys", "-l", "-t", &side, "--", &c.to_string()]).ok(),
        KeyCode::Enter => tmux(&["send-keys", "-t", &side, "Enter"]).ok(),
        KeyCode::Up => tmux(&["send-keys", "-t", &side, "Up"]).ok(),
        KeyCode::Down => tmux(&["send-keys", "-t", &side, "Down"]).ok(),
        _ => None,
    };
}

/// What is shown when no worktree is: the logo and what to do next. Keys go to the sidebar.
pub fn welcome() -> Res<()> {
    let mut term = ratatui::init();
    let (mut wakes, binary, mut draw) = (Wakes::start(), Binary::now(), true);
    let res = loop {
        if wakes.tick() && binary.replaced() {
            ratatui::restore();
            break Err(restart_self());
        }
        if !draw {
            draw = match wakes.next() {
                Some(Wake::Input(Input::Key(k))) => {
                    pass_on(k);
                    false
                }
                Some(Wake::Tmux | Wake::Resize) => true,
                _ => !control::connected(),
            };
            continue;
        }
        draw = false;
        let (all, wts) = (panes(), load());
        let waits = |w: &&Wt| foremost(all.iter().filter(|p| p.wt == w.id())).is_some_and(|p| p.look() == State::Asking);
        let waiting = wts.iter().filter(waits).count();
        let drawn = term.draw(|f| {
            f.render_widget(Block::new().style(Style::new().bg(HULL).fg(FOAM)), f.area());
            let mut lines = vec![];
            if f.area().height >= mark::H + 5 && f.area().width >= mark::W + 2 {
                lines.extend(mark::lines());
                lines.extend([Line::default(), Line::default()]);
            }
            if waiting > 0 {
                let (g, c) = look(State::Asking);
                lines.push(Line::from(fg(format!("{g} {}", (tr().agents_wait)(waiting)), c)));
                lines.push(Line::default());
            }
            let lost = restore::lost().len();
            if lost > 0 {
                lines.push(Line::from(vec![fg("R", FOAM), fg(format!(" {}", (tr().restore_offer)(lost)), MUTED)]));
                lines.push(Line::default());
            }
            lines.push(keys(&[("n", tr().k_new), ("A", tr().k_add), ("?", tr().k_commands)]));
            let [_, middle, _] = Layout::vertical([Constraint::Fill(1), Constraint::Length(lines.len() as u16), Constraint::Fill(1)]).areas(f.area());
            f.render_widget(Paragraph::new(lines).centered(), middle);
        });
        if let Err(e) = drawn {
            break Err(e.to_string());
        }
    };
    ratatui::restore();
    res
}

/// Waits until the rail has something to do: a key, or None to draw again.
fn rail_input(wakes: &mut Wakes, binary: &Binary, focused: &mut bool, busy: bool) -> Option<Input> {
    let since = Instant::now();
    loop {
        match wakes.next() {
            Some(Wake::Input(i)) => return Some(i),
            Some(Wake::Focus) => {
                let was = std::mem::replace(focused, active());
                if was != *focused {
                    return None;
                }
            }
            Some(Wake::Resize) => return None,
            _ => {}
        }
        // the age of the PRs is told in steps of ten seconds
        if wakes.tick() && (busy || wakes.changed() || binary.replaced() || since.elapsed().as_secs() >= 10) {
            *focused = active();
            return None;
        }
    }
}

pub fn rail() -> Res<()> {
    let mut term = ratatui::init();
    execute!(stdout(), event::EnableMouseCapture).ok();
    let (mut sel, mut msg, mut busy) = (0usize, String::new(), None::<thread::JoinHandle<()>>);
    let (mut wakes, binary) = (Wakes::start(), Binary::now());
    let mut focused = active();
    let res = loop {
        if binary.replaced() {
            execute!(stdout(), event::DisableMouseCapture).ok();
            ratatui::restore();
            break Err(restart_self());
        }
        let (c, wts) = (prs::cached(), load());
        sel = sel.min(c.prs.len().saturating_sub(1));
        if busy.as_ref().is_some_and(|b| b.is_finished()) {
            busy = None;
        }
        let mut rows = vec![];
        if let Err(e) = term.draw(|f| rows = draw_rail(f, &c, &wts, sel, focused, busy.is_some(), &msg)) {
            break Err(e.to_string());
        }
        let view = |p: &Pr| popup(&format!("#{} {}", p.number, p.repo_name()), (100, 34), &["dialog", "view", &p.url]);
        let k = match rail_input(&mut wakes, &binary, &mut focused, busy.is_some()) {
            None => continue,
            Some(Input::Key(k)) => k,
            Some(Input::Wheel { down }) => if down { KeyCode::Down } else { KeyCode::Up },
            Some(Input::Click { row, .. }) => match rows.iter().find(|r| (r.0..r.1).contains(&row)) {
                Some(r) => {
                    sel = r.2;
                    KeyCode::Enter
                }
                None => continue,
            },
        };
        msg.clear();
        let pr = c.prs.get(sel);
        let done = match (k, pr) {
            (KeyCode::Char('q'), _) => detach(),
            (KeyCode::Char('j') | KeyCode::Down, _) => { sel += 1; Ok(()) }
            (KeyCode::Char('k') | KeyCode::Up, _) => { sel = sel.saturating_sub(1); Ok(()) }
            (KeyCode::Tab, _) => focus(SIDEBAR),
            (KeyCode::Char('p'), _) => toggle_rail(), // closes this pane
            (KeyCode::Char('?'), _) => popup(tr().title_commands, (136, 34), &["commands"]),
            (KeyCode::Char('r'), _) if busy.is_none() => { busy = Some(thread::spawn(|| { prs::refresh().ok(); })); Ok(()) }
            (KeyCode::Char('o'), Some(p)) => open_url(&p.url),
            (KeyCode::Enter, Some(p)) => view(p),
            (KeyCode::Char('w'), Some(p)) if p.worktree(&wts).is_some() => Ok(()),
            (KeyCode::Char('w'), Some(p)) if p.fork => Err(tr().fork.into()),
            (KeyCode::Char('w'), Some(p)) => {
                popup(tr().title_pr, DIALOG, &["dialog", "pr", &p.path.to_string_lossy(), &p.branch, &format!("#{} {}", p.number, p.title)])
            }
            _ => Ok(()),
        };
        match done {
            Err(e) if !e.ends_with(": ") => msg = e,
            _ => {}
        }
    };
    execute!(stdout(), event::DisableMouseCapture).ok();
    ratatui::restore();
    res
}

// ---------- the dialog of one PR (`orcux dialog view`) ----------

/// `s` in lines of at most `w` columns, broken at spaces where possible.
fn wrap(s: &str, w: usize) -> Vec<String> {
    let (mut out, mut line) = (vec![], String::new());
    for word in s.split(' ') {
        let mut word = word;
        if !line.is_empty() && Span::raw(format!("{line} {word}")).width() > w {
            out.push(std::mem::take(&mut line));
        } else if !line.is_empty() {
            line.push(' ');
        }
        while Span::raw(word).width() > w.max(1) {
            let (i, _) = word.char_indices().nth(w.max(1) / 2).unwrap_or((word.len(), ' ')); // half: wide characters fit too
            let cut = word.char_indices().map(|c| c.0).rfind(|c| Span::raw(&word[..*c]).width() <= w).filter(|c| *c > 0).unwrap_or(i.max(1));
            out.push(std::mem::take(&mut line) + &word[..cut]);
            word = &word[cut..];
        }
        line.push_str(word);
    }
    out.push(line);
    out
}

/// The end of `s`, which is what tells paths apart.
fn fit_end(s: &str, room: usize) -> String {
    let rev: String = s.chars().rev().collect();
    fit(&rev, room).chars().rev().collect()
}

/// One fold of the dialog of a PR: its heading with a summary, and what it shows when open.
struct Fold {
    title: &'static str,
    summary: Vec<Span<'static>>,
    lines: Vec<Line<'static>>,
    /// it blocks the PR: opened from the start
    blocks: bool,
}

/// The files by folder, so that each long path is written once, with their changes in columns.
fn file_tree(files: &[(String, u32, u32)], w: u16) -> Vec<Line<'static>> {
    let mut files: Vec<(&str, &str, u32, u32)> = files.iter().map(|(p, a, d)| {
        let (dir, name) = p.rsplit_once('/').unwrap_or(("", p));
        (dir, name, *a, *d)
    }).collect();
    files.sort();
    let mut out = vec![];
    for (i, (dir, name, a, d)) in files.iter().enumerate() {
        if i == 0 || files[i - 1].0 != *dir {
            out.push(Line::from(fg(format!("     {}", fit_end(if dir.is_empty() { "/" } else { dir }, w as usize - 6)), MUTED)));
        }
        let last = files.get(i + 1).is_none_or(|n| n.0 != *dir);
        let green = (a * 5).checked_div(a + d).unwrap_or(0) as usize;
        out.push(lr(w, vec![fg(if last { "     ╰── " } else { "     ├── " }, LINE), fg(name.to_string(), FOAM)], vec![
            fg(format!("  {:>6}", format!("+{a}")), KELP), fg(format!(" {:>6}  ", format!("−{d}")), CORAL),
            fg("■".repeat(green), KELP), fg("■".repeat(5 - green), if a + d > 0 { CORAL } else { LINE }), " ".into(),
        ]));
    }
    out
}

/// What blocks the PR goes first; for a review request, what it is about.
fn folds(p: &Pr, d: &Detail, w: u16) -> Vec<Fold> {
    let room = w as usize - 10;
    let item = |g: &str, c: Color, s: &str| Line::from(vec![fg(format!("     {g}  "), c), fg(fit(s, room), FOAM)]);
    let note = |s: &str| Line::from(fg(format!("     {s}"), MUTED));
    let count = |g: &str, n: usize, c: Color| (n > 0).then(|| fg(format!("{g} {n}  "), c));
    let conflicting = p.mergeable == "CONFLICTING";
    let conflicts = Fold {
        title: tr().s_conflicts,
        summary: vec![match p.mergeable.as_str() {
            "CONFLICTING" => fg(format!("✕ {}", if d.conflicts.is_empty() { String::new() } else { d.conflicts.len().to_string() }), CORAL),
            "MERGEABLE" => fg(format!("✓ {}", tr().none), KELP),
            _ => fg(tr().unknown, MUTED),
        }],
        lines: match conflicting {
            true if d.conflicts.is_empty() => vec![note(tr().no_conflict_files)],
            true => d.conflicts.iter().map(|s| item("✕", CORAL, &fit_end(s, room))).collect(),
            false => vec![],
        },
        blocks: conflicting,
    };
    let checks = Fold {
        title: tr().s_checks,
        summary: [count("✕", d.failing.len(), CORAL), count("●", d.pending.len(), CURRENT), count("○", d.optional.len(), MUTED), count("✓", p.ok as usize, KELP)].into_iter().flatten().collect(),
        lines: d.failing.iter().map(|s| item("✕", CORAL, s)).chain(d.pending.iter().map(|s| item("●", CURRENT, s)))
            .chain(d.optional.iter().map(|s| item("○", MUTED, &(tr().optional)(s)))).collect(),
        blocks: !d.failing.is_empty(),
    };
    let mut lines = vec![];
    for t in &d.threads {
        lines.push(Line::from(vec![fg("     ◆  ", SIGNAL), fg(fit_end(&format!("{}:{}", t.path, t.line), room), FOAM)]));
        lines.extend(wrap(&format!("{}: {}", t.author, t.text), room).into_iter().map(|l| Line::from(fg(format!("        {l}"), MUTED))));
    }
    let threads = Fold {
        title: tr().s_threads,
        summary: vec![match d.threads.len() { 0 => fg(format!("✓ {}", tr().none), KELP), n => fg(format!("◆ {}", (tr().n_unresolved)(n)), SIGNAL) }],
        lines,
        blocks: !d.threads.is_empty(),
    };
    let of = |s: &str| d.reviews.iter().filter(|r| r.0 == s).count();
    let mut summary: Vec<Span> = [count("✓", of("APPROVED"), KELP), count("◆", of("CHANGES_REQUESTED"), SIGNAL)].into_iter().flatten().collect();
    if of("PENDING") > 0 {
        summary.push(fg(format!("○ {}", (tr().n_missing)(of("PENDING"))), MUTED));
    }
    let reviews = Fold {
        title: tr().d_reviews,
        summary: if d.reviews.is_empty() { vec![fg(format!("○ {}", tr().r_nobody), MUTED)] } else { summary },
        lines: d.reviews.iter().map(|(state, who)| {
            let who = if who.is_empty() { tr().a_team } else { who };
            match state.as_str() {
                "APPROVED" => item("✓", KELP, &(tr().r_approved)(who)),
                "CHANGES_REQUESTED" => item("◆", SIGNAL, &(tr().r_changes)(who)),
                "PENDING" => item("○", MUTED, &(tr().r_pending)(who)),
                _ => item("○", MUTED, &(tr().r_commented)(who)),
            }
        }).collect(),
        blocks: p.decision == "CHANGES_REQUESTED",
    };
    let mut lines = file_tree(&d.files, w);
    if d.changed > d.files.len() {
        lines.push(note(&(tr().more)(d.changed - d.files.len())));
    }
    let files = Fold {
        title: tr().d_files,
        summary: vec![fg(format!("{}   ", d.changed), MUTED), fg(format!("+{} ", d.added), KELP), fg(format!("−{}", d.deleted), CORAL)],
        lines,
        blocks: false,
    };
    let mut text: Vec<&str> = d.body.iter().map(|l| l.trim_end()).skip_while(|l| l.is_empty()).collect();
    while text.last().is_some_and(|l| l.is_empty()) {
        text.pop();
    }
    let body = Fold {
        title: tr().d_body,
        summary: vec![fg((tr().n_lines)(text.len()), MUTED)],
        lines: text.iter().flat_map(|l| wrap(l, room)).map(|l| Line::from(fg(format!("     {l}"), FOAM))).collect(),
        blocks: p.review,
    };
    if p.review {
        vec![body, files, checks, threads, reviews, conflicts]
    } else if conflicting {
        vec![conflicts, checks, threads, reviews, files, body]
    } else {
        vec![checks, threads, reviews, conflicts, files, body]
    }
}

/// What blocks the PR, in one line.
fn verdict(p: &Pr, d: &Detail) -> Line<'static> {
    let mut parts = vec![];
    if !p.review {
        if p.mergeable == "CONFLICTING" {
            parts.push(fg(tr().conflicts, CORAL));
        }
        if !d.failing.is_empty() {
            parts.push(fg((tr().v_checks)(d.failing.len()), CORAL));
        }
        if !d.threads.is_empty() {
            parts.push(fg((tr().d_threads)(d.threads.len()), SIGNAL));
        }
    }
    if parts.is_empty() {
        let (_, s, text) = p.status();
        return Line::from(fg(text, look(s).1).bold());
    }
    let only = parts.len() == 1 && p.status().0 == Group::Yours;
    let mut spans = vec![];
    for (i, s) in parts.into_iter().enumerate() {
        if i > 0 {
            spans.push(fg("  ·  ", MUTED));
        }
        spans.push(s.bold());
    }
    if only {
        spans.push(fg(format!("  {}", tr().v_only), MUTED));
    }
    Line::from(spans)
}

/// What to ask an agent about this PR, in one line (a line break would submit it half-written).
fn prompt(p: &Pr, d: &Detail) -> Option<String> {
    let label = format!("#{} ({})", p.number, p.title);
    if p.review {
        return Some((tr().ask_review)(&label));
    }
    let mut parts = vec![];
    if p.mergeable == "CONFLICTING" {
        parts.push((tr().ask_conflicts)(&d.base, &d.conflicts.join(", ")));
    }
    if !d.failing.is_empty() {
        parts.push((tr().ask_checks)(&p.number, &d.failing.join(", ")));
    }
    if !d.threads.is_empty() {
        let list: Vec<String> = d.threads.iter().map(|t| format!("{}:{} ({}) «{}»", t.path, t.line, t.author, t.text.replace(['«', '»'], "\""))).collect();
        parts.push((tr().ask_threads)(&list.join("; ")));
    }
    (!parts.is_empty()).then(|| format!("{} {} {}", (tr().ask_intro)(&label), parts.join(" "), tr().ask_outro))
}

/// Keys as buttons; returns where each one is: (from column, to column, its key).
fn buttons(f: &mut Frame, area: Rect, all: &[(&str, &str, KeyCode)]) -> Vec<(u16, u16, KeyCode)> {
    let (mut spans, mut at, mut x) = (vec![], vec![], area.x);
    for (k, label, code) in all {
        let width = (Span::raw(*k).width() + Span::raw(*label).width() + 3) as u16;
        spans.push(Span::styled(format!(" {k} "), Style::new().fg(FOAM).bg(WAKE).bold()));
        spans.push(Span::styled(format!("{label} "), Style::new().fg(FOAM).bg(WAKE)));
        spans.push("  ".into());
        at.push((x, x + width, *code));
        x += width + 2;
    }
    f.render_widget(Line::from(spans), area);
    at
}

pub fn dialog_view(url: &str) -> Res<()> {
    let p = prs::cached().prs.into_iter().find(|p| p.url == url).ok_or_else(|| url.to_string())?;
    let (tx, rx) = mpsc::channel();
    let asked = p.clone();
    thread::spawn(move || tx.send(prs::detail(&asked)).ok());
    dialog(|term| {
        execute!(stdout(), event::EnableMouseCapture).ok();
        let (mut detail, mut msg, mut ask) = (None::<Res<Detail>>, String::new(), None::<String>);
        let (mut sel, mut open, mut top) = (0usize, vec![], 0u16);
        let res = loop {
            if let Ok(d) = rx.try_recv() {
                if let Ok(d) = &d {
                    let all = folds(&p, d, 80);
                    open = all.iter().map(|x| x.blocks).collect();
                    sel = open.iter().position(|o| *o).unwrap_or(0);
                }
                detail = Some(d);
            }
            let wts = load();
            let wt = p.worktree(&wts);
            let age = prs::ago(p.age + prs::now().saturating_sub(prs::cached().at));
            let main = if wt.is_some() { ("⏎", tr().k_goto, KeyCode::Enter) } else { ("w", tr().k_create, KeyCode::Char('w')) };
            let all = match &ask {
                Some(_) => vec![("⏎", tr().k_paste, KeyCode::Enter), ("esc", tr().k_back, KeyCode::Esc)],
                None => vec![main, ("a", tr().k_agent, KeyCode::Char('a')), ("o", tr().k_web, KeyCode::Char('o'))],
            };
            // what is where on screen, for the mouse
            let (mut at, mut heads, mut url_row, mut foot_row) = (vec![], vec![], 0, 0);
            let drawn = term.draw(|f| {
                let area = f.area().inner(Margin::new(2, 1));
                let w = area.width;
                let mut head: Vec<Line> = wrap(&p.title, w as usize).into_iter().take(3).map(|l| Line::from(l.bold().fg(FOAM))).collect();
                url_row = area.y + head.len() as u16;
                let link = Span::styled(fit(&p.url, w.saturating_sub(4) as usize), Style::new().fg(CURRENT).underlined());
                head.push(Line::from(link));
                head.push(Line::from(fg(format!("{}, {}", p.author, (tr().ago)(&age)), MUTED)));
                head.push(Line::default());
                let mut status = None;
                if let (None, Some(Ok(d))) = (&ask, &detail) {
                    head.push(verdict(&p, d));
                    status = Some(d);
                } else {
                    let (_, s, text) = p.status();
                    head.push(Line::from(fg(text, look(s).1).bold()));
                }
                let [top_area, _, body, _, foot, hint] = Layout::vertical([
                    Constraint::Length(head.len() as u16), Constraint::Length(1), Constraint::Fill(1),
                    Constraint::Length(1), Constraint::Length(1), Constraint::Length(1),
                ]).areas(area);
                f.render_widget(Paragraph::new(head), top_area);
                let mut lines: Vec<Line> = vec![];
                match (&ask, &detail, status) {
                    (Some(a), ..) => {
                        lines.extend([Line::from(tr().ask_title.bold().fg(SIGNAL)), Line::default()]);
                        lines.extend(wrap(a, w as usize).into_iter().map(|l| Line::from(fg(l, FOAM))));
                    }
                    (_, None, _) => lines.push(Line::from(fg(tr().loading, MUTED))),
                    (_, Some(Err(e)), _) => lines.extend(wrap(e, w as usize).into_iter().map(|l| Line::from(fg(l, CORAL)))),
                    (_, _, Some(d)) => for (i, x) in folds(&p, d, w).into_iter().enumerate() {
                        let mark = match (x.lines.is_empty(), open[i]) { (true, _) => "   ", (_, true) => " ▾ ", _ => " ▸ " };
                        let mut right = x.summary;
                        right.push("  ".into());
                        let mut l = lr(w, vec![fg(mark, MUTED), fg(format!(" {}", x.title), FOAM).bold()], right);
                        if i == sel {
                            l.spans.iter_mut().for_each(|s| s.style.bg = Some(WAKE));
                        }
                        heads.push(lines.len() as u16);
                        lines.push(l);
                        if open[i] && !x.lines.is_empty() {
                            lines.push(Line::default());
                            lines.extend(x.lines);
                        }
                        lines.push(Line::default());
                    },
                    _ => {}
                }
                top = top.min((lines.len() as u16).saturating_sub(body.height));
                f.render_widget(Paragraph::new(lines).scroll((top, 0)), body);
                // from lines to screen rows; the ones out of sight cannot be clicked
                heads = heads.iter().map(|h| if *h >= top && *h < top + body.height { body.y + h - top } else { u16::MAX }).collect();
                foot_row = foot.y;
                at = buttons(f, foot, &all);
                let [tip, close] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(tr().hint_close.len() as u16)]).areas(hint);
                if msg.is_empty() {
                    f.render_widget(Line::from(fg(if ask.is_some() { "" } else { tr().hint_view }, MUTED)), tip);
                } else {
                    f.render_widget(Line::from(fg(msg.as_str(), CORAL)), tip);
                }
                f.render_widget(Line::from(fg(tr().hint_close, MUTED)), close);
            });
            if let Err(e) = drawn {
                break Err(e.to_string());
            }
            let k = match poll_input(150) {
                None => continue,
                Some(Input::Wheel { down }) => KeyCode::Char(if down { 'j' } else { 'k' }),
                Some(Input::Key(k)) => k,
                Some(Input::Click { row, .. }) if row == url_row => KeyCode::Char('o'),
                Some(Input::Click { col, row }) if row == foot_row => match at.iter().find(|b| (b.0..b.1).contains(&col)) {
                    Some(b) => b.2,
                    None => continue,
                },
                Some(Input::Click { row, .. }) => match heads.iter().position(|h| *h == row) {
                    Some(i) => {
                        sel = i;
                        KeyCode::Char(' ')
                    }
                    None => continue,
                },
            };
            msg.clear();
            match k {
                KeyCode::Esc | KeyCode::Char('q') if ask.is_some() => ask = None,
                KeyCode::Esc | KeyCode::Char('q') => break Ok(()),
                KeyCode::Char('j') => top += 1,
                KeyCode::Char('k') => top = top.saturating_sub(1),
                KeyCode::Down if ask.is_none() && !open.is_empty() => sel = (sel + 1).min(open.len() - 1),
                KeyCode::Up if ask.is_none() => sel = sel.saturating_sub(1),
                KeyCode::Char(' ') if ask.is_none() && !open.is_empty() => open[sel] = !open[sel],
                KeyCode::Right if ask.is_none() && !open.is_empty() => open[sel] = true,
                KeyCode::Left if ask.is_none() && !open.is_empty() => open[sel] = false,
                KeyCode::Enter => match (&ask, wt) {
                    (Some(a), Some(w)) => {
                        let done = agent_tab(w).ok_or_else(|| tr().no_agent.to_string())
                            .and_then(|t| show_tab(w, Some(t.tab)).and_then(|_| paste_into(&t.pane, a)));
                        match done {
                            Ok(()) => break Ok(()),
                            Err(e) => msg = short(&e),
                        }
                    }
                    (None, Some(w)) => match show(w) {
                        Ok(()) => break Ok(()),
                        Err(e) => msg = short(&e),
                    },
                    _ => {}
                },
                KeyCode::Char('o') => if let Err(e) = open_url(&p.url) {
                    msg = e;
                },
                KeyCode::Char('w') if wt.is_none() && ask.is_none() => {
                    if p.fork {
                        msg = tr().fork.into();
                    } else if create_view(term, &p.path, &p.branch, &format!("#{} {}", p.number, p.title))? {
                        break Ok(());
                    }
                }
                KeyCode::Char('a') if ask.is_none() => match (wt, &detail) {
                    (_, None | Some(Err(_))) => {}
                    (None, _) => msg = tr().no_wt_agent.into(),
                    (Some(w), _) if agent_tab(w).is_none() => msg = tr().no_agent.into(),
                    (Some(_), Some(Ok(d))) => {
                        ask = prompt(&p, d);
                        top = 0;
                        if ask.is_none() {
                            msg = tr().nothing_to_ask.into();
                        }
                    }
                },
                _ => {}
            }
            // keep the selected heading in sight after moving or folding
            if matches!(k, KeyCode::Down | KeyCode::Up | KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right) && let Some(Ok(d)) = &detail {
                let size = term.size().map_err(|e| e.to_string())?;
                let w = size.width.saturating_sub(4);
                let line: usize = folds(&p, d, w).iter().zip(&open).take(sel)
                    .map(|(x, o)| if *o && !x.lines.is_empty() { x.lines.len() + 3 } else { 2 }).sum();
                let height = size.height.saturating_sub(12);
                let shown = if open[sel] { folds(&p, d, w)[sel].lines.len() as u16 + 3 } else { 2 };
                // all of it if it fits, else from its heading
                top = top.max((line as u16 + shown).saturating_sub(height)).min(line as u16);
            }
        };
        execute!(stdout(), event::DisableMouseCapture).ok();
        res
    })
}

/// Lower case and without accents, to search: "pestana" finds "pestaña".
fn plain(s: &str) -> String {
    s.to_lowercase().chars().map(|c| match c {
        'á' | 'à' | 'ä' | 'â' => 'a',
        'é' | 'è' | 'ë' | 'ê' => 'e',
        'í' | 'ì' | 'ï' | 'î' => 'i',
        'ó' | 'ò' | 'ö' | 'ô' => 'o',
        'ú' | 'ù' | 'ü' | 'û' => 'u',
        'ñ' => 'n',
        'ç' => 'c',
        c => c,
    }).collect()
}

/// The commands that have every word of the search in their group, usage or description.
fn search_commands(query: &str) -> Vec<(&'static str, Vec<(&'static str, &'static str)>)> {
    let query = plain(query);
    tr().commands.iter().map(|(group, cmds)| {
        let hit = |c: &&(&str, &str)| {
            let text = plain(&format!("{group} {} {}", c.0, c.1));
            query.split_whitespace().all(|w| text.contains(w))
        };
        (*group, cmds.iter().filter(hit).copied().collect::<Vec<_>>())
    }).filter(|g| !g.1.is_empty()).collect()
}

/// The commands dialog (`orcux commands`, shown in a tmux popup by the sidebar's `?`).
pub fn commands() -> Res<()> {
    let mut term = ratatui::init();
    execute!(stdout(), event::EnableMouseCapture).ok();
    let (mut query, mut top, start) = (String::new(), 0u16, Instant::now());
    let res = loop {
        let (mut max, mut animated) = (0, false);
        let drawn = term.draw(|f| {
            let mut area = f.area();
            // the orc goes beside the list, if there is room for both
            if area.width as usize >= 94 + 2 + orc::W && area.height as usize >= orc::H / 2 + 4 {
                animated = true;
                let [side, list] = Layout::horizontal([Constraint::Length(orc::W as u16 + 2), Constraint::Fill(1)]).areas(area);
                area = list;
                let tick = (start.elapsed().as_millis() / 120) as usize;
                let mut art = orc::lines(tick);
                // typed letter by letter, then left to read
                let text = tr().orc[tick / 60 % tr().orc.len()];
                let typed: String = text.chars().take(tick % 60).collect();
                let cursor = if (tick / 4).is_multiple_of(2) { "▏" } else { " " };
                art.extend([Line::default(), Line::from(vec![fg("> ", MUTED), fg(typed, FOAM), fg(cursor, FOAM)])]);
                f.render_widget(Paragraph::new(art), side.inner(Margin::new(1, 1)));
            }
            let [_, input, _, body, _, foot] = Layout::vertical([
                Constraint::Length(1), Constraint::Length(1), Constraint::Length(1),
                Constraint::Fill(1), Constraint::Length(1), Constraint::Length(1),
            ]).areas(area);
            f.render_widget(lr(input.width, vec![fg(format!(" {}: ", tr().search), MUTED), fg(format!("{query}▏"), FOAM)], vec![]).bg(WAKE), input);
            let wide = body.width >= 90;
            let mut lines: Vec<Line> = vec![];
            for (group, cmds) in search_commands(&query) {
                if !lines.is_empty() {
                    lines.push(Line::default());
                }
                lines.push(Line::from(format!(" {group}")).bold().fg(MUTED));
                for (usage, desc) in cmds {
                    if wide {
                        lines.push(Line::from(vec![format!(" {usage:<40} ").fg(FOAM), desc.fg(MUTED)]));
                    } else {
                        lines.push(Line::from(format!(" {usage}")).fg(FOAM));
                        lines.push(Line::from(format!("   {desc}")).fg(MUTED));
                    }
                }
            }
            if lines.is_empty() {
                lines.push(Line::from(format!(" {}", tr().no_match)).fg(MUTED));
            }
            max = (lines.len() as u16).saturating_sub(body.height);
            top = top.min(max);
            f.render_widget(Paragraph::new(lines).scroll((top, 0)), body);
            f.render_widget(Line::from(format!(" {}", if max > 0 { tr().hint_scroll } else { tr().hint_close })).fg(MUTED), foot);
        });
        if let Err(e) = drawn {
            break Err(e.to_string());
        }
        match poll_input(if animated { 60 } else { 500 }) {
            Some(Input::Key(KeyCode::Esc)) if !query.is_empty() => query.clear(),
            Some(Input::Key(KeyCode::Esc)) => break Ok(()),
            Some(Input::Key(KeyCode::Char('?'))) if query.is_empty() => break Ok(()), // as it opened
            Some(Input::Key(KeyCode::Char(c))) => {
                query.push(c);
                top = 0;
            }
            Some(Input::Key(KeyCode::Backspace)) => {
                query.pop();
                top = 0;
            }
            Some(Input::Key(KeyCode::Down) | Input::Wheel { down: true }) => top = (top + 1).min(max),
            Some(Input::Key(KeyCode::Up) | Input::Wheel { down: false }) => top = top.saturating_sub(1),
            Some(Input::Key(KeyCode::PageDown)) => top = (top + 10).min(max),
            Some(Input::Key(KeyCode::PageUp)) => top = top.saturating_sub(10),
            _ => {} // next frame, or a resize
        }
    };
    execute!(stdout(), event::DisableMouseCapture).ok();
    ratatui::restore();
    res
}

// ---------- small dialogs: each one is an `orcux dialog ...` process inside a tmux popup ----------

fn dialog(body: impl FnOnce(&mut ratatui::DefaultTerminal) -> Res<()>) -> Res<()> {
    DRAWING.store(true, Ordering::Relaxed);
    let mut term = ratatui::init();
    let res = body(&mut term);
    ratatui::restore();
    res
}

fn draw_dialog(term: &mut ratatui::DefaultTerminal, lines: Vec<Line>, hint: &str) -> Res<()> {
    term.draw(|f| {
        let area = f.area().inner(Margin::new(1, 0));
        let [body, foot] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body);
        f.render_widget(Line::from(hint).fg(MUTED), foot);
    }).map(|_| ()).map_err(|e| e.to_string())
}

/// Next key press, or None if something else happened (resize: redraw). Ctrl-C and Meta keys count as Esc.
fn key() -> Res<Option<KeyCode>> {
    Ok(match event::read().map_err(|e| e.to_string())? {
        Event::Key(k) if k.kind == KeyEventKind::Press => {
            Some(if k.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) { KeyCode::Esc } else { k.code })
        }
        _ => None,
    })
}

/// "Repo:   ", so that the values of a dialog line up.
fn tag(s: &str) -> String {
    format!("{:<8}", format!("{s}:"))
}

/// Drops the echoed command line from a git error so that the reason fits in the dialog.
fn short(e: &str) -> String {
    e.rsplit("fatal: ").next().unwrap_or(e).into()
}

pub fn dialog_new(repo: &Path, parent: Option<&Wt>) -> Res<()> {
    dialog(|term| {
        let repo_name = repo.file_name().unwrap_or_default().to_string_lossy().to_string();
        let (mut name, mut err) = (String::new(), String::new());
        let untrusted = matches!(config::of(repo), Ok(config::Trust::No));
        let dirty = parent.is_some_and(|p| git(&p.path(), &["status", "--porcelain"]).is_ok_and(|s| !s.is_empty()));
        loop {
            let lines = |status: Line<'static>| {
                let mut l = vec![Line::from(vec![tag(tr().label_repo).fg(MUTED), repo_name.clone().bold().fg(FOAM)])];
                if let Some(p) = parent {
                    l.push(Line::from(vec![tag(tr().label_parent).fg(MUTED), p.name.clone().fg(FOAM), format!("  {}", p.branch().unwrap_or_default()).fg(MUTED)]));
                }
                l.push(Line::from(vec![tag(tr().label_name).fg(MUTED), format!("{name}▏").into()]));
                if untrusted {
                    l.push(Line::from((tr().untrusted)(&tilde(repo))).fg(SIGNAL));
                }
                if let (Some(p), true) = (parent, dirty) {
                    l.push(Line::from((tr().parent_dirty)(&p.name)).fg(SIGNAL));
                }
                l.extend([Line::default(), status]);
                l
            };
            draw_dialog(term, lines(Line::from(err.clone()).fg(CORAL)), tr().hint_new)?;
            match key()? {
                Some(KeyCode::Esc) => return Ok(()),
                Some(KeyCode::Backspace) => { name.pop(); }
                Some(KeyCode::Char(c)) if !c.is_whitespace() => name.push(c),
                Some(KeyCode::Enter) if !name.is_empty() => {
                    draw_dialog(term, lines(Line::from(tr().creating).fg(CURRENT)), "")?;
                    match create_from(repo, &name, None, parent).and_then(|w| show(&w)) {
                        Ok(()) => return Ok(()),
                        Err(e) => err = short(&e), // stay open to fix the name
                    }
                }
                _ => {}
            }
        }
    })
}

/// The folders that complete a half-written path: their longest common beginning.
fn complete(text: &str) -> Option<String> {
    let (dir, part) = text.rsplit_once('/')?;
    let names: Vec<String> = fs::read_dir(expand(&format!("{dir}/"))).ok()?.flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with(part) && (part.starts_with('.') || !n.starts_with('.')))
        .collect();
    let first = names.first()?;
    let same = first.char_indices().map(|(i, c)| i + c.len_utf8()).take_while(|i| names.iter().all(|n| n.starts_with(&first[..*i]))).last().unwrap_or(0);
    Some(format!("{dir}/{}{}", &first[..same], if names.len() == 1 { "/" } else { "" }))
}

pub fn dialog_add(base: &Path) -> Res<()> {
    dialog(|term| {
        let (mut text, mut err) = (String::new(), String::new());
        loop {
            let what = if text.trim().is_empty() { None } else { Some(project(&text, base)) };
            let preview = match &what {
                None => Line::default(),
                Some(Ok(Project::Register(p))) => Line::from((tr().will_register)(&tilde(p))).fg(MUTED),
                Some(Ok(Project::Clone(_, p))) => Line::from((tr().will_clone)(&tilde(p))).fg(MUTED),
                Some(Ok(Project::Create(p))) => Line::from((tr().will_create)(&tilde(p))).fg(MUTED),
                Some(Err(e)) => Line::from(e.clone()).fg(MUTED),
            };
            let lines = |status: Line<'static>| vec![
                Line::from(format!("{}:", tr().label_project)).fg(MUTED),
                Line::from(format!("{text}▏")).fg(FOAM),
                Line::default(),
                status,
            ];
            let status = if err.is_empty() { preview } else { Line::from(err.clone()).fg(CORAL) };
            draw_dialog(term, lines(status), tr().hint_add)?;
            match key()? {
                Some(KeyCode::Esc) => return Ok(()),
                Some(KeyCode::Backspace) => { text.pop(); err.clear(); }
                Some(KeyCode::Tab) => if let Some(t) = complete(&text) {
                    text = t;
                },
                Some(KeyCode::Char(c)) => { text.push(c); err.clear(); }
                Some(KeyCode::Enter) => if let Some(Ok(p)) = &what {
                    draw_dialog(term, lines(Line::from(tr().cloning).fg(CURRENT)), "")?;
                    match add_project(p) {
                        Ok(_) => return Ok(()),
                        Err(e) => err = short(&e),
                    }
                },
                _ => {}
            }
        }
    })
}

pub fn dialog_rm_repo(repo: &Path) -> Res<()> {
    dialog(|term| {
        let mut err = String::new();
        loop {
            let (ask, name) = (tr().ask_unregister, repo.file_name().unwrap_or_default().to_string_lossy().to_string());
            let lines = vec![
                Line::from(vec![ask.0.into(), name.bold().fg(FOAM), ask.1.into()]),
                Line::from(tr().kept_repo).fg(MUTED),
                Line::default(),
                Line::from(err.clone()).fg(CORAL),
            ];
            draw_dialog(term, lines, tr().hint_unregister)?;
            match key()? {
                None => {}
                Some(KeyCode::Char('y')) if err.is_empty() => match remove_repo(repo) {
                    Ok(()) => return Ok(()),
                    Err(e) => err = e,
                },
                Some(_) => return Ok(()),
            }
        }
    })
}

pub fn dialog_pr(repo: &Path, branch: &str, label: &str) -> Res<()> {
    dialog(|term| create_view(term, repo, branch, label).map(|_| ()))
}

/// Asks and creates the worktree of a PR; whether it did.
fn create_view(term: &mut ratatui::DefaultTerminal, repo: &Path, branch: &str, label: &str) -> Res<bool> {
    let repo_name = repo.file_name().unwrap_or_default().to_string_lossy().to_string();
    let port = 10000 + free_slot(&load()) * 100;
    let mut err = String::new();
    loop {
        let field = |k: &str, v: String| Line::from(vec![tag(k).fg(MUTED), v.fg(FOAM)]);
        let lines = |status: Line<'static>| vec![
            field("PR", label.to_string()),
            field(tr().label_repo, repo_name.clone()),
            field(tr().label_branch, branch.to_string()),
            field(tr().label_port, port.to_string()),
            status,
        ];
        draw_dialog(term, lines(Line::from(err.clone()).fg(CORAL)), tr().hint_pr)?;
        match key()? {
            Some(KeyCode::Esc) => return Ok(false),
            Some(KeyCode::Enter) => {
                draw_dialog(term, lines(Line::from(tr().fetching).fg(CURRENT)), "")?;
                match create_for_pr(repo, branch).and_then(|w| show(&w)) {
                    Ok(()) => return Ok(true),
                    Err(e) => err = short(&e),
                }
            }
            _ => {}
        }
    }
}

pub fn dialog_rm(w: &Wt) -> Res<()> {
    dialog(|term| {
        let mut err = String::new();
        loop {
            let (ask, kept) = if w.at.is_some() {
                (tr().ask_unregister, tr().kept_unregister)
            } else {
                (tr().ask_rm, tr().kept_rm)
            };
            let lines = |status: Line<'static>| vec![
                Line::from(vec![ask.0.into(), w.id().bold().fg(FOAM), ask.1.into()]),
                Line::from(kept).fg(MUTED),
                Line::default(),
                status,
            ];
            let hint = if w.at.is_some() { tr().hint_unregister } else if err.is_empty() { tr().hint_rm } else { tr().hint_force };
            draw_dialog(term, lines(Line::from(err.clone()).fg(CORAL)), hint)?;
            let force = match key()? {
                None => continue,
                Some(KeyCode::Char('y')) if err.is_empty() => false,
                Some(KeyCode::Char('D')) if w.at.is_none() => true,
                Some(_) => return Ok(()),
            };
            draw_dialog(term, lines(Line::from(tr().deleting).fg(CURRENT)), "")?;
            match remove(w, force) {
                Ok(()) => return Ok(()),
                Err(e) => err = short(&e), // e.g. dirty worktree: offer to force
            }
        }
    })
}

/// Merges a worktree and takes it away, after telling what will happen.
pub fn dialog_merge(w: &Wt) -> Res<()> {
    dialog(|term| {
        draw_dialog(term, vec![Line::from(tr().loading).fg(MUTED)], "")?;
        let mut err = String::new();
        let plan = merge::plan(w, None, false);
        let others = merge::with_others(w).unwrap_or_default();
        loop {
            let mut lines = vec![];
            let ready = match &plan {
                Ok(p) => {
                    lines.push(Line::from((tr().merge_plan)(&p.branch, &p.into, p.commits as usize)).fg(FOAM));
                    lines.push(Line::from(tr().merge_then).fg(MUTED));
                    if !p.clashes.is_empty() {
                        lines.push(Line::from((tr().merge_clash)(&p.into, &p.clashes.join(", "))).fg(CORAL));
                    }
                    p.clashes.is_empty()
                }
                Err(e) => {
                    lines.push(Line::from(e.clone()).fg(CORAL));
                    false
                }
            };
            for (other, files) in &others {
                lines.push(Line::from((tr().merge_others)(other, &files.join(", "))).fg(SIGNAL));
            }
            lines.extend([Line::default(), Line::from(err.clone()).fg(CORAL)]);
            draw_dialog(term, lines, if ready && err.is_empty() { tr().hint_merge } else { tr().hint_any })?;
            match (key()?, &plan) {
                (None, _) => {}
                (Some(KeyCode::Char('y')), Ok(p)) if ready && err.is_empty() => match merge::run_plan(w, p) {
                    Ok(()) => return Ok(()),
                    Err(e) => err = short(&e),
                },
                _ => return Ok(()),
            }
        }
    })
}

/// Closes the visible tab (Opt+q), which may be running something.
pub fn dialog_close() -> Res<()> {
    let (w, n) = visible()?;
    let cmd = tabs(&w.id()).into_iter().find(|p| p.tab == n).map(|p| p.cmd).unwrap_or_default();
    dialog(|term| {
        let mut err = String::new();
        loop {
            let ask = tr().ask_close;
            let lines = vec![
                Line::from(vec![ask.0.into(), format!("{n} {}", program(&cmd)).bold().fg(FOAM), ask.1.into()]),
                Line::from(tr().kept_close).fg(MUTED),
                Line::default(),
                Line::from(err.clone()).fg(CORAL),
            ];
            draw_dialog(term, lines, tr().hint_unregister)?;
            match key()? {
                None => {}
                Some(KeyCode::Char('y')) => match close_tab(&w, n) {
                    Ok(()) => return Ok(()),
                    Err(e) => err = short(&e),
                },
                Some(_) => return Ok(()),
            }
        }
    })
}

pub fn dialog_error(text: &str) -> Res<()> {
    dialog(|term| loop {
        draw_dialog(term, short(text).lines().map(|l| Line::from(l.to_string()).fg(CORAL)).collect(), tr().hint_any)?;
        if key()?.is_some() {
            return Ok(());
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_and_a_pr_look_different() {
        // both are drawn on the row of a worktree
        let pr = [State::Failed, State::Waiting, State::Done].map(|s| look(s).0);
        for a in [Agent::Question, Agent::Permission, Agent::Failed, Agent::Done] {
            let p = Pane { pane: "%1".into(), wt: "r/w".into(), tab: 1, on: true, busy: false, cmd: "claude".into(), agent: Some((a, 0)), mark: String::new(), title: String::new(), session: String::new(), shell: false, activity: 0 };
            assert!(!pr.contains(&look(p.look()).0), "{a:?}");
        }
    }

    #[test]
    fn names_and_cuts() {
        assert_eq!((program("2.1.332"), program("zsh"), program("python3.12")), ("claude", "zsh", "python3.12"));
        assert_eq!(plain("Pestaña ÚNICA"), "pestana unica");
        assert!(search_commands("").len() == tr().commands.len() && search_commands("zzz").is_empty());
        let found: Vec<_> = search_commands("IMPORT orcux").into_iter().flat_map(|g| g.1).collect();
        assert!(!found.is_empty() && found.iter().all(|c| plain(&format!("{} {}", c.0, c.1)).contains("import")));
        assert_eq!(wrap("uno dos tres", 7), ["uno dos", "tres"]);
        assert_eq!(wrap("abcdefgh ij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(wrap("", 4), [""]);
        assert_eq!(fit_end("src/lib/a.ts", 6), "…/a.ts");
        assert_eq!((fit("abc", 3), fit("abcd", 3), fit("abcd", 0)), ("abc".into(), "ab…".into(), String::new()));
    }
}
