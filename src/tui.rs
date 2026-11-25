use crate::*;
use ratatui::{
    crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    prelude::*,
    widgets::{List, ListItem, ListState, Paragraph, Wrap},
};
use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    time::Instant,
};

enum Row {
    Repo(PathBuf),
    Wt(Wt),
}

/// Size of the small dialogs, borders included: fits a wrapped git error.
const DIALOG: (u32, u32) = (64, 10);

struct App {
    rows: Vec<Row>,
    sel: usize,
    /// last-resort error line, for when not even a popup can be opened
    msg: String,
    /// id -> (hash of visible screen, last time it changed)
    seen: HashMap<String, (u64, Option<Instant>)>,
    /// id -> current command, for worktrees whose terminal is alive
    live: HashMap<String, String>,
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
            if id.is_empty() || id == SIDEBAR {
                continue;
            }
            let mut h = DefaultHasher::new();
            tmux(&["capture-pane", "-p", "-t", &pane]).unwrap_or_default().hash(&mut h);
            let h = h.finish();
            let e = self.seen.entry(id.clone()).or_insert((h, None));
            if e.0 != h {
                *e = (h, Some(Instant::now()));
            }
            self.live.insert(id, cmd);
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

    fn draw(&self, f: &mut Frame) {
        let [list, foot, hint] = Layout::vertical([Constraint::Fill(1), Constraint::Length(3), Constraint::Length(1)]).areas(f.area());
        let items: Vec<ListItem> = self.rows.iter().map(|r| match r {
            Row::Repo(p) => ListItem::new(Line::from(p.file_name().unwrap_or_default().to_string_lossy().to_string()).bold().cyan()),
            Row::Wt(w) => {
                let id = w.id();
                let working = self.seen.get(&id).and_then(|s| s.1).is_some_and(|t| t.elapsed().as_secs() < 3);
                let (dot, col) = match self.live.get(&id) {
                    None => ("✕", Color::DarkGray),
                    Some(_) if working => ("●", Color::Green),
                    Some(_) => ("○", Color::Gray),
                };
                let cmd = self.live.get(&id).cloned().unwrap_or_default();
                ListItem::new(Line::from(vec![
                    Span::raw(" "),
                    Span::styled(dot, col),
                    Span::raw(format!(" {} ", w.name)),
                    Span::styled(format!(":{} {cmd}", w.port()), Color::DarkGray),
                ]))
            }
        }).collect();
        let mut st = ListState::default().with_selected(Some(self.sel));
        f.render_stateful_widget(List::new(items).highlight_style(Style::new().reversed()), list, &mut st);
        let text = if self.msg.is_empty() { "⏎ abrir  n nuevo  d borrar\nj/k mover  q salir" } else { &self.msg };
        f.render_widget(Paragraph::new(text).wrap(Wrap { trim: true }).dark_gray(), foot);
        f.render_widget(Line::from(vec!["?".bold().cyan(), " comandos".dark_gray()]), hint);
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

pub fn run() -> Res<()> {
    let mut term = ratatui::init();
    let mut app = App { rows: vec![], sel: 1, msg: String::new(), seen: HashMap::new(), live: HashMap::new() };
    let mut last = Instant::now();
    app.refresh();
    let res = loop {
        if last.elapsed().as_millis() >= 1000 {
            app.refresh();
            last = Instant::now();
        }
        if let Err(e) = term.draw(|f| app.draw(f)) {
            break Err(e.to_string());
        }
        if !event::poll(std::time::Duration::from_millis(300)).unwrap_or(false) {
            continue;
        }
        let Ok(Event::Key(k)) = event::read() else { continue };
        if k.kind != KeyEventKind::Press {
            continue;
        }
        app.msg.clear();
        match k.code {
            KeyCode::Char('q') => { tmux(&["detach-client"]).ok(); }
            KeyCode::Char('j') | KeyCode::Down => app.sel = (app.sel + 1).min(app.rows.len().saturating_sub(1)),
            KeyCode::Char('k') | KeyCode::Up => app.sel = app.sel.saturating_sub(1),
            KeyCode::Char('?') => app.popup("Comandos", (96, 30), &["commands"]),
            KeyCode::Char('n') => match app.repo_of_selected() {
                Some(r) => app.popup("Nuevo worktree", DIALOG, &["dialog", "new", &r.to_string_lossy()]),
                None => app.error("Sin repos: ejecuta `orcux repo add <ruta>`"),
            },
            KeyCode::Char('d') => if let Some(Row::Wt(w)) = app.selected() {
                let id = w.id();
                app.popup("Borrar worktree", DIALOG, &["dialog", "rm", &id]);
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
                lines.push(Line::from(*group).bold().cyan());
                for (usage, desc) in *cmds {
                    if wide {
                        lines.push(Line::from(vec![Span::raw(format!(" {usage:<40} ")), desc.dark_gray()]));
                    } else {
                        lines.push(Line::from(format!(" {usage}")));
                        lines.push(Line::from(format!("   {desc}")).dark_gray());
                    }
                }
            }
            max = (lines.len() as u16).saturating_sub(body.height);
            top = top.min(max);
            f.render_widget(Paragraph::new(lines).scroll((top, 0)), body);
            f.render_widget(Line::from(if max > 0 { "j/k desplazar  esc cerrar" } else { "esc cerrar" }).dark_gray(), foot);
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
        f.render_widget(Line::from(hint).dark_gray(), foot);
    }).map(|_| ()).map_err(|e| e.to_string())
}

/// Next key press, or None if something else happened (resize: redraw). Ctrl-C counts as Esc.
fn key() -> Res<Option<KeyCode>> {
    Ok(match event::read().map_err(|e| e.to_string())? {
        Event::Key(k) if k.kind == KeyEventKind::Press => {
            Some(if k.modifiers.contains(KeyModifiers::CONTROL) { KeyCode::Esc } else { k.code })
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
                Line::from(vec!["Repo:   ".dark_gray(), repo_name.clone().bold().cyan()]),
                Line::from(vec!["Nombre: ".dark_gray(), format!("{name}▏").into()]),
                Line::default(),
                status,
            ];
            draw_dialog(term, lines(Line::from(err.clone()).red()), "⏎ crear  esc cancelar")?;
            match key()? {
                Some(KeyCode::Esc) => return Ok(()),
                Some(KeyCode::Backspace) => { name.pop(); }
                Some(KeyCode::Char(c)) if !c.is_whitespace() => name.push(c),
                Some(KeyCode::Enter) if !name.is_empty() => {
                    draw_dialog(term, lines(Line::from("Creando…").yellow()), "")?;
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

pub fn dialog_rm(w: &Wt) -> Res<()> {
    dialog(|term| {
        let mut err = String::new();
        loop {
            let lines = |status: Line<'static>| vec![
                Line::from(vec!["¿Borrar ".into(), w.id().bold().cyan(), "?".into()]),
                Line::from("La rama se conserva.").dark_gray(),
                Line::default(),
                status,
            ];
            let hint = if err.is_empty() { "y sí  D forzar  otra tecla: no" } else { "D forzar (pierde los cambios)  otra tecla: no" };
            draw_dialog(term, lines(Line::from(err.clone()).red()), hint)?;
            let force = match key()? {
                None => continue,
                Some(KeyCode::Char('y')) if err.is_empty() => false,
                Some(KeyCode::Char('D')) => true,
                Some(_) => return Ok(()),
            };
            draw_dialog(term, lines(Line::from("Borrando…").yellow()), "")?;
            match remove(w, force) {
                Ok(()) => return Ok(()),
                Err(e) => err = short(&e), // e.g. dirty worktree: offer to force
            }
        }
    })
}

pub fn dialog_error(text: &str) -> Res<()> {
    dialog(|term| loop {
        draw_dialog(term, short(text).lines().map(|l| Line::from(l.to_string()).red()).collect(), "pulsa una tecla para cerrar")?;
        if key()?.is_some() {
            return Ok(());
        }
    })
}
