use crate::{prs::{self, Cache, Group, Pr, State}, *};
use ratatui::{
    crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    prelude::*,
    widgets::{Block, Paragraph, Wrap},
};
use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    time::Instant,
};

// Design system "orcux", theme abismo. Each status color has one meaning and always goes with its glyph and a word.
const ABYSS: Color = Color::Rgb(0x0b, 0x11, 0x14);
const HULL: Color = Color::Rgb(0x13, 0x1d, 0x22);
const WAKE: Color = Color::Rgb(0x1d, 0x35, 0x40);
const FOAM: Color = Color::Rgb(0xe8, 0xf0, 0xf2);
const MUTED: Color = Color::Rgb(0x93, 0xa8, 0xb0);
const CURRENT: Color = Color::Rgb(0x62, 0xc6, 0xe8); // working
const SIGNAL: Color = Color::Rgb(0xf4, 0xb9, 0x42); // waiting for the user
const CORAL: Color = Color::Rgb(0xff, 0x8a, 0x66); // failed
const KELP: Color = Color::Rgb(0xa9, 0xdc, 0xb8); // done

/// Size of the small dialogs, borders included: fits a wrapped git error.
const DIALOG: (u32, u32) = (64, 10);

fn look(s: State) -> (&'static str, Color) {
    match s {
        State::Failed => ("✕", CORAL),
        State::Waiting => ("◆", SIGNAL),
        State::Working => ("●", CURRENT),
        State::Idle => ("○", MUTED),
        State::Done => ("✓", KELP),
    }
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
fn program(cmd: &str) -> &str {
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

/// Draws blocks of lines, scrolled so that the selected one is visible. The selection is the only white
/// patch on screen: without the focus it is dimmed.
fn blocks(f: &mut Frame, area: Rect, items: Vec<(Vec<Line>, bool)>, active: bool) {
    let (mut lines, mut end) = (vec![], 0);
    for (block, selected) in items {
        for mut l in block {
            if selected {
                let st = if active { Style::new().fg(ABYSS).bg(FOAM) } else { Style::new().fg(FOAM).bg(WAKE) };
                l.spans.iter_mut().for_each(|s| s.style = s.style.patch(st));
            }
            lines.push(l);
        }
        if selected {
            end = lines.len();
        }
    }
    let top = end.saturating_sub(area.height as usize) as u16;
    f.render_widget(Paragraph::new(lines).scroll((top, 0)), area);
}

fn keys<'a>(pairs: &[(&'a str, &'a str)]) -> Line<'a> {
    Line::from(pairs.iter().flat_map(|(k, v)| [" ".into(), k.bold().fg(FOAM), fg(format!(" {v} "), MUTED)]).collect::<Vec<Span>>())
}

/// Whether this process's pane has the focus.
fn active() -> bool {
    let me = env::var("TMUX_PANE").unwrap_or_default();
    tmux(&["display-message", "-p", "-t", &me, "#{pane_active}"]).is_ok_and(|s| s == "1")
}

enum Row {
    Repo(PathBuf),
    Wt(Wt),
}

struct App {
    rows: Vec<Row>,
    sel: usize,
    /// last-resort error line, for when not even a popup can be opened
    msg: String,
    /// id -> (hash of visible screen, last time it changed)
    seen: HashMap<String, (u64, Option<Instant>)>,
    /// id -> current command, for worktrees whose terminal is alive
    live: HashMap<String, String>,
    prs: Cache,
    active: bool,
    /// id -> last @orcux_busy sent to tmux
    busy: HashMap<String, bool>,
    /// last status bar sent to tmux
    status: (String, String),
}

impl App {
    fn refresh(&mut self) {
        let wts = load();
        self.rows.clear();
        for r in repos() {
            self.rows.push(Row::Repo(r.clone()));
            self.rows.extend(wts.iter().filter(|w| w.repo == r).cloned().map(Row::Wt));
        }
        self.sel = self.sel.min(self.rows.len().saturating_sub(1));
        self.live.clear();
        // ponytail: "working" = screen changed in the last 3s (agent spinners redraw). Use agent hooks if this misfires.
        for (pane, id, cmd) in panes() {
            if id.is_empty() || id == SIDEBAR || id == RAIL {
                continue;
            }
            let mut h = DefaultHasher::new();
            tmux(&["capture-pane", "-p", "-t", &pane]).unwrap_or_default().hash(&mut h);
            let h = h.finish();
            let e = self.seen.entry(id.clone()).or_insert((h, None));
            if e.0 != h {
                *e = (h, Some(Instant::now()));
            }
            self.live.insert(id.clone(), cmd);
            let now = self.working(&id);
            if self.busy.insert(id, now) != Some(now) {
                set_busy(&pane, now);
            }
        }
        self.prs = prs::cached();
        sync_rail().ok(); // no room for it: stay without
        self.update_status(&wts);
    }

    fn working(&self, id: &str) -> bool {
        self.live.contains_key(id) && self.seen.get(id).and_then(|s| s.1).is_some_and(|t| t.elapsed().as_secs() < 3)
    }

    fn update_status(&mut self, wts: &[Wt]) {
        let esc = |s: &str| s.replace('#', "##").replace('%', "%%");
        let mut left = String::new();
        match self.prs.prs.iter().filter(|p| p.status().0 == Group::Yours).count() {
            0 => {}
            1 => left += "#[bg=#f4b942,fg=#0b1114] 1 PR te toca #[default]",
            n => left += &format!("#[bg=#f4b942,fg=#0b1114] {n} PRs te tocan #[default]"),
        }
        left += &match wts.iter().filter(|w| self.working(&w.id())).count() {
            0 => " #[fg=#93a8b0]○ 0 trabajando".to_string(),
            n => format!(" #[fg=#62c6e8]● {n} trabajando"),
        };
        let shown = terminal_pane().map(|p| p.1).unwrap_or_default();
        let right = wts.iter().find(|w| w.id() == shown)
            .map(|w| format!("{} #[fg=#93a8b0]:{} ", esc(&shown), w.port())).unwrap_or_default();
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
            Row::Repo(p) => Some(p.clone()),
            Row::Wt(w) => Some(w.repo.clone()),
        }
    }

    fn wt_lines<'a>(&self, w: &Wt, width: u16) -> Vec<Line<'a>> {
        let id = w.id();
        let (dot, col) = look(if self.working(&id) { State::Working } else { State::Idle });
        let pr = self.prs.prs.iter().find(|p| p.is_of(w));
        let second = match (pr, self.live.get(&id)) {
            _ if !w.path().exists() => fg("la carpeta ya no existe", CORAL),
            (Some(p), _) => {
                let (_, s, text) = p.status();
                let (g, c) = look(s);
                fg(format!("{g} #{} {text}", p.number), c)
            }
            (None, None) => fg("sin abrir", MUTED),
            (None, Some(cmd)) if self.working(&id) => fg(format!("trabajando, {}", program(cmd)), CURRENT),
            (None, Some(cmd)) => fg(format!("parado, {}", program(cmd)), MUTED),
        };
        vec![
            lr(width, vec![" ".into(), fg(dot, col), " ".into(), w.name.clone().bold().fg(FOAM)], vec![fg(format!(" {} ", w.port()), MUTED)]),
            lr(width, vec!["   ".into(), second], vec![" ".into()]),
        ]
    }

    fn draw(&self, f: &mut Frame) {
        f.render_widget(Block::new().style(Style::new().bg(HULL).fg(FOAM)), f.area());
        let [head, list, foot] = Layout::vertical([Constraint::Length(2), Constraint::Fill(1), Constraint::Length(3)]).areas(f.area());
        let w = f.area().width;
        let n = self.rows.iter().filter(|r| matches!(r, Row::Wt(_))).count();
        let count = if n == 1 { "1 worktree ".into() } else { format!("{n} worktrees ") };
        f.render_widget(lr(w, vec![" orcux".bold()], vec![fg(count, MUTED)]), head);
        let mut items = vec![];
        for (i, r) in self.rows.iter().enumerate() {
            let sel = i == self.sel;
            match r {
                Row::Repo(p) => {
                    let has_wts = |j: usize| matches!(self.rows.get(j), Some(Row::Wt(_)));
                    if i > 0 && (has_wts(i - 1) || has_wts(i + 1)) {
                        items.push((vec![Line::default()], false));
                    }
                    let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
                    items.push((vec![lr(w, vec![fg(format!(" {name}"), MUTED)], vec![])], sel));
                }
                Row::Wt(wt) => items.push((self.wt_lines(wt, w), sel)),
            }
        }
        blocks(f, list, items, self.active);
        if self.msg.is_empty() {
            let lines = vec![
                keys(&[("⏎", "abrir"), ("n", "nuevo"), ("d", "borrar")]),
                keys(&[("p", "PRs"), ("tab", "panel"), ("q", "salir")]),
                keys(&[("?", "comandos"), ("Opt+w", "cambiar")]),
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
        self.popup("Error", DIALOG, &["dialog", "error", text]);
    }
}

/// The next key press within 300 ms.
fn poll_key() -> Option<KeyCode> {
    if !event::poll(Duration::from_millis(300)).unwrap_or(false) {
        return None;
    }
    match event::read() {
        Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => Some(k.code),
        _ => None,
    }
}

pub fn run() -> Res<()> {
    style_ui().ok();
    thread::spawn(|| loop {
        prs::refresh().ok(); // errors go into the cache, the rail shows them
        thread::sleep(Duration::from_secs(60));
    });
    let mut term = ratatui::init();
    let mut app = App {
        rows: vec![], sel: 1, msg: String::new(), seen: HashMap::new(), live: HashMap::new(),
        prs: Cache::default(), active: true, status: Default::default(), busy: HashMap::new(),    };
    let mut last = Instant::now();
    app.refresh();
    let res = loop {
        if last.elapsed().as_millis() >= 1000 {
            app.refresh();
            last = Instant::now();
        }
        app.active = active();
        if let Err(e) = term.draw(|f| app.draw(f)) {
            break Err(e.to_string());
        }
        let Some(k) = poll_key() else { continue };
        app.msg.clear();
        match k {
            KeyCode::Char('q') => { tmux(&["detach-client"]).ok(); }
            KeyCode::Char('j') | KeyCode::Down => app.sel = (app.sel + 1).min(app.rows.len().saturating_sub(1)),
            KeyCode::Char('k') | KeyCode::Up => app.sel = app.sel.saturating_sub(1),
            KeyCode::Char('?') => app.popup("Comandos", (96, 42), &["commands"]),
            KeyCode::Char('p') => if let Err(e) = toggle_rail() {
                app.error(&e);
            },
            KeyCode::Tab => { focus(RAIL).ok(); }
            KeyCode::Char('n') => match app.repo_of_selected() {
                Some(r) => app.popup("Nuevo worktree", DIALOG, &["dialog", "new", &r.to_string_lossy()]),
                None => app.error("Sin repos: ejecuta `orcux repo add <ruta>`"),
            },
            KeyCode::Char('d') => if let Some(Row::Wt(w)) = app.selected() {
                let (id, title) = (w.id(), if w.at.is_some() { "Quitar de orcux" } else { "Borrar worktree" });
                app.popup(title, DIALOG, &["dialog", "rm", &id]);
            },
            KeyCode::Enter => if let Some(Row::Wt(w)) = app.selected() {
                if let Err(e) = show(&w.clone()) {
                    app.error(&e);
                }
            },
            _ => {}
        }
    };
    ratatui::restore();
    res
}

// ---------- switcher: the list of worktrees behind Opt+w (`orcux switch`) ----------

pub fn switch() -> Res<()> {
    let visible = terminal_pane().map(|p| p.1).unwrap_or_default();
    let (wts, prs, busy) = (by_recent(&visible), prs::cached().prs, busy_ids());
    let live: Vec<String> = panes().into_iter().map(|p| p.1).collect();
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
            f.render_widget(Line::from(vec![fg(" Buscar: ", MUTED), fg(format!("{query}▏"), FOAM)]), input);
            let mut items = vec![];
            for (i, wt) in hits.iter().enumerate() {
                let id = wt.id();
                let (g, c) = look(if busy.contains(&id) { State::Working } else { State::Idle });
                let mut left = vec![" ".into(), fg(g, c), " ".into(), fg(format!("{}/", wt.repo_name()), MUTED), wt.name.clone().bold().fg(FOAM)];
                if let Some(p) = pr_of(wt) {
                    let (_, s, t) = p.status();
                    left.push(fg(format!("  {} #{} {t}", look(s).0, p.number), look(s).1));
                } else if !live.contains(&id) {
                    left.push(fg("  sin abrir", MUTED));
                }
                let here = if id == visible { "visible  " } else { "" };
                items.push((vec![lr(w, left, vec![fg(format!(" {here}{} ", wt.port()), MUTED)])], i == cur));
            }
            if hits.is_empty() {
                let t = if wts.is_empty() { " Sin worktrees: créalos con n en el sidebar" } else { " Ninguno coincide" };
                items.push((vec![Line::from(fg(t, MUTED))], false));
            }
            blocks(f, list, items, true);
            if err.is_empty() {
                f.render_widget(keys(&[("↑/↓", "mover"), ("⏎", "cambiar"), ("esc", "cerrar")]), foot);
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
    let mut left = vec!["   ".into()];
    if state == State::Working && p.total > 0 {
        let done = (p.ok + p.failed).min(p.total) * 10 / p.total;
        left.push(Span::raw(" ".repeat(done as usize)).bg(CURRENT));
        left.push(Span::raw(" ".repeat(10 - done as usize)).bg(WAKE));
        left.push(" ".into());
    }
    left.push(fg(if state == State::Idle { format!("{text}, {}", prs::ago(age)) } else { text }, c));
    // the status goes whole; the repo name takes what is left
    let room = (w as usize).saturating_sub(left.iter().map(Span::width).sum::<usize>() + 2);
    let repo = fit(p.repo_name(), room);
    vec![
        lr(w, vec![" ".into(), fg(g, c), format!(" #{} ", p.number).bold().fg(FOAM), fg(p.title.clone(), FOAM)], vec![" ".into()]),
        lr(w, left, vec![fg(format!(" {repo} "), MUTED)]),
    ]
}

fn draw_rail(f: &mut Frame, c: &Cache, wts: &[Wt], sel: usize, active: bool, busy: bool, msg: &str) {
    f.render_widget(Block::new().style(Style::new().bg(HULL).fg(FOAM)), f.area());
    let [head, list, foot] = Layout::vertical([Constraint::Length(3), Constraint::Fill(1), Constraint::Length(2)]).areas(f.area());
    let w = f.area().width;
    let since = prs::now().saturating_sub(c.at);
    let when = match c.at {
        _ if busy => "actualizando… ".into(),
        0 if c.error.is_empty() => "cargando… ".into(),
        0 => String::new(),
        _ => format!("hace {} ", prs::ago(since)),
    };
    let review = c.prs.iter().filter(|p| p.review).count();
    f.render_widget(Paragraph::new(vec![
        lr(w, vec![" Tus PRs".bold()], vec![fg(when, MUTED)]),
        match c.error.as_str() {
            "" => lr(w, vec![fg(format!(" {} tuyas, {review} por revisar", c.prs.len() - review), MUTED)], vec![]),
            e => lr(w, vec![fg(format!(" {e}"), CORAL)], vec![]), // below it, the last PRs that were read
        },
    ]), head);
    let mut items = vec![];
    for (group, name, color) in [(Group::Yours, "Te toca", SIGNAL), (Group::Going, "En curso", CURRENT), (Group::Ready, "Listas", KELP)] {
        let of: Vec<(usize, &Pr)> = c.prs.iter().enumerate().filter(|p| p.1.status().0 == group).collect();
        if of.is_empty() {
            continue;
        }
        items.push((vec![Line::from(vec![" ".into(), format!(" {name} {} ", of.len()).fg(ABYSS).bg(color)]), Line::default()], false));
        for (i, p) in of {
            items.push((pr_lines(p, p.age + since, w), i == sel));
            items.push((vec![Line::default()], false));
        }
    }
    if c.prs.is_empty() && c.at > 0 {
        items.push((vec![Line::from(fg(" Sin PRs abiertas", MUTED))], false));
    }
    blocks(f, list, items, active);
    if !msg.is_empty() {
        f.render_widget(Paragraph::new(format!(" {msg}")).wrap(Wrap { trim: true }).fg(CORAL), foot);
        return;
    }
    let first = match c.prs.get(sel) {
        Some(p) if p.worktree(wts).is_some() => vec![("⏎", "ir al worktree"), ("o", "web")],
        Some(_) => vec![("w", "crear worktree"), ("o", "web")],
        None => vec![],
    };
    f.render_widget(Paragraph::new(vec![keys(&first), keys(&[("p", "ocultar"), ("r", "actualizar"), ("tab", "volver")])]), foot);
}

pub fn rail() -> Res<()> {
    let mut term = ratatui::init();
    let (mut sel, mut msg, mut busy) = (0usize, String::new(), None::<thread::JoinHandle<()>>);
    let res = loop {
        let (c, wts, focused) = (prs::cached(), load(), active());
        sel = sel.min(c.prs.len().saturating_sub(1));
        if busy.as_ref().is_some_and(|b| b.is_finished()) {
            busy = None;
        }
        if let Err(e) = term.draw(|f| draw_rail(f, &c, &wts, sel, focused, busy.is_some(), &msg)) {
            break Err(e.to_string());
        }
        let Some(k) = poll_key() else { continue };
        msg.clear();
        let pr = c.prs.get(sel);
        let done = match (k, pr) {
            (KeyCode::Char('q'), _) => tmux(&["detach-client"]).map(|_| ()),
            (KeyCode::Char('j') | KeyCode::Down, _) => { sel += 1; Ok(()) }
            (KeyCode::Char('k') | KeyCode::Up, _) => { sel = sel.saturating_sub(1); Ok(()) }
            (KeyCode::Tab, _) => focus(SIDEBAR),
            (KeyCode::Char('p'), _) => toggle_rail(), // closes this pane
            (KeyCode::Char('?'), _) => popup("Comandos", (96, 42), &["commands"]),
            (KeyCode::Char('r'), _) if busy.is_none() => { busy = Some(thread::spawn(|| { prs::refresh().ok(); })); Ok(()) }
            (KeyCode::Char('o'), Some(p)) => open_url(&p.url),
            (KeyCode::Enter, Some(p)) => p.worktree(&wts).map_or(Ok(()), show),
            (KeyCode::Char('w'), Some(p)) if p.worktree(&wts).is_some() => Ok(()),
            (KeyCode::Char('w'), Some(p)) if p.fork => Err("PR de un fork: su rama no está en origin".into()),
            (KeyCode::Char('w'), Some(p)) => {
                popup("Crear worktree", DIALOG, &["dialog", "pr", &p.path.to_string_lossy(), &p.branch, &format!("#{} {}", p.number, p.title)])
            }
            _ => Ok(()),
        };
        match done {
            Err(e) if !e.ends_with(": ") => msg = e,
            _ => {}
        }
    };
    ratatui::restore();
    res
}

/// The commands dialog (`orcux commands`, shown in a tmux popup by the sidebar's `?`).
pub fn commands() -> Res<()> {
    let mut term = ratatui::init();
    let mut top = 0u16;
    let res = loop {
        let mut max = 0;
        let drawn = term.draw(|f| {
            let [body, foot] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(f.area());
            let wide = body.width >= 90;
            let mut lines: Vec<Line> = vec![];
            for (group, cmds) in COMMANDS {
                if !lines.is_empty() {
                    lines.push(Line::default());
                }
                lines.push(Line::from(*group).bold().fg(MUTED));
                for (usage, desc) in *cmds {
                    if wide {
                        lines.push(Line::from(vec![format!(" {usage:<40} ").fg(FOAM), desc.fg(MUTED)]));
                    } else {
                        lines.push(Line::from(format!(" {usage}")).fg(FOAM));
                        lines.push(Line::from(format!("   {desc}")).fg(MUTED));
                    }
                }
            }
            max = (lines.len() as u16).saturating_sub(body.height);
            top = top.min(max);
            f.render_widget(Paragraph::new(lines).scroll((top, 0)), body);
            f.render_widget(Line::from(if max > 0 { "j/k desplazar  esc cerrar" } else { "esc cerrar" }).fg(MUTED), foot);
        });
        if let Err(e) = drawn {
            break Err(e.to_string());
        }
        match event::read() {
            Err(e) => break Err(e.to_string()),
            Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => match k.code {
                KeyCode::Esc | KeyCode::Char('q' | '?') => break Ok(()),
                KeyCode::Char('j') | KeyCode::Down => top = (top + 1).min(max),
                KeyCode::Char('k') | KeyCode::Up => top = top.saturating_sub(1),
                _ => {}
            },
            Ok(_) => {} // e.g. resize: just redraw
        }
    };
    ratatui::restore();
    res
}

// ---------- small dialogs: each one is an `orcux dialog ...` process inside a tmux popup ----------

fn dialog(body: impl FnOnce(&mut ratatui::DefaultTerminal) -> Res<()>) -> Res<()> {
    let mut term = ratatui::init();
    let res = body(&mut term);
    ratatui::restore();
    res
}

fn draw_dialog(term: &mut ratatui::DefaultTerminal, lines: Vec<Line>, hint: &str) -> Res<()> {
    term.draw(|f| {
        let [body, foot] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(f.area());
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

/// Drops the echoed command line from a git error so that the reason fits in the dialog.
fn short(e: &str) -> String {
    e.rsplit("fatal: ").next().unwrap_or(e).into()
}

pub fn dialog_new(repo: &Path) -> Res<()> {
    dialog(|term| {
        let repo_name = repo.file_name().unwrap_or_default().to_string_lossy().to_string();
        let (mut name, mut err) = (String::new(), String::new());
        loop {
            let lines = |status: Line<'static>| vec![
                Line::from(vec!["Repo:   ".fg(MUTED), repo_name.clone().bold().fg(FOAM)]),
                Line::from(vec!["Nombre: ".fg(MUTED), format!("{name}▏").into()]),
                Line::default(),
                status,
            ];
            draw_dialog(term, lines(Line::from(err.clone()).fg(CORAL)), "⏎ crear  esc cancelar")?;
            match key()? {
                Some(KeyCode::Esc) => return Ok(()),
                Some(KeyCode::Backspace) => { name.pop(); }
                Some(KeyCode::Char(c)) if !c.is_whitespace() => name.push(c),
                Some(KeyCode::Enter) if !name.is_empty() => {
                    draw_dialog(term, lines(Line::from("Creando…").fg(CURRENT)), "")?;
                    match create(repo, &name, None).and_then(|w| show(&w)) {
                        Ok(()) => return Ok(()),
                        Err(e) => err = short(&e), // stay open to fix the name
                    }
                }
                _ => {}
            }
        }
    })
}

pub fn dialog_pr(repo: &Path, branch: &str, label: &str) -> Res<()> {
    dialog(|term| {
        let repo_name = repo.file_name().unwrap_or_default().to_string_lossy().to_string();
        let port = 10000 + free_slot(&load()) * 100;
        let mut err = String::new();
        loop {
            let field = |k: &'static str, v: String| Line::from(vec![k.fg(MUTED), v.fg(FOAM)]);
            let lines = |status: Line<'static>| vec![
                field("PR:     ", label.to_string()),
                field("Repo:   ", repo_name.clone()),
                field("Rama:   ", branch.to_string()),
                field("Puerto: ", port.to_string()),
                status,
            ];
            draw_dialog(term, lines(Line::from(err.clone()).fg(CORAL)), "⏎ crear y abrir  esc cancelar")?;
            match key()? {
                Some(KeyCode::Esc) => return Ok(()),
                Some(KeyCode::Enter) => {
                    draw_dialog(term, lines(Line::from("Descargando la rama y creando…").fg(CURRENT)), "")?;
                    match create_for_pr(repo, branch).and_then(|w| show(&w)) {
                        Ok(()) => return Ok(()),
                        Err(e) => err = short(&e),
                    }
                }
                _ => {}
            }
        }
    })
}

pub fn dialog_rm(w: &Wt) -> Res<()> {
    dialog(|term| {
        let mut err = String::new();
        loop {
            let (ask, kept) = if w.at.is_some() {
                ("¿Quitar de orcux ", "La carpeta y la rama se conservan. Se cierra su terminal.")
            } else {
                ("¿Borrar ", "La rama se conserva.")
            };
            let lines = |status: Line<'static>| vec![
                Line::from(vec![ask.into(), w.id().bold().fg(FOAM), "?".into()]),
                Line::from(kept).fg(MUTED),
                Line::default(),
                status,
            ];
            let hint = if w.at.is_some() { "y sí  otra tecla: no" } else if err.is_empty() { "y sí  D forzar  otra tecla: no" } else { "D forzar (pierde los cambios)  otra tecla: no" };
            draw_dialog(term, lines(Line::from(err.clone()).fg(CORAL)), hint)?;
            let force = match key()? {
                None => continue,
                Some(KeyCode::Char('y')) if err.is_empty() => false,
                Some(KeyCode::Char('D')) if w.at.is_none() => true,
                Some(_) => return Ok(()),
            };
            draw_dialog(term, lines(Line::from("Borrando…").fg(CURRENT)), "")?;
            match remove(w, force) {
                Ok(()) => return Ok(()),
                Err(e) => err = short(&e), // e.g. dirty worktree: offer to force
            }
        }
    })
}

pub fn dialog_error(text: &str) -> Res<()> {
    dialog(|term| loop {
        draw_dialog(term, short(text).lines().map(|l| Line::from(l.to_string()).fg(CORAL)).collect(), "pulsa una tecla para cerrar")?;
        if key()?.is_some() {
            return Ok(());
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_cuts() {
        assert_eq!((program("2.1.332"), program("zsh"), program("python3.12")), ("claude", "zsh", "python3.12"));
        assert_eq!((fit("abc", 3), fit("abcd", 3), fit("abcd", 0)), ("abc".into(), "ab…".into(), String::new()));
    }
}
