//! The diff of one file, to read and comment line by line (`f`, then Enter). The comments go to the agent of its
//! worktree as one prompt, left in its input without Enter: the person reads it before sending.
use crate::{i18n::tr, theme, *};
use ratatui::{
    crossterm::event::KeyCode,
    prelude::*,
    widgets::Paragraph,
};
use std::{collections::BTreeMap, path::Path, process::Command};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Head,
    Hunk,
    Add,
    Del,
    Same,
}

/// A line of the diff, and its number in the file: the new one, or the old one for a removed line.
#[derive(Debug, PartialEq)]
pub struct Row {
    pub kind: Kind,
    pub text: String,
    pub line: Option<u32>,
}

/// A unified diff, line by line. A file without changes is all of it, as context.
pub fn rows(diff: &str) -> Vec<Row> {
    let (mut old, mut new, mut body) = (0, 0, false);
    diff.lines().map(|l| {
        if let Some(h) = l.strip_prefix("@@ ") {
            // "@@ -3,4 +5,6 @@ fn x"
            let start = |sign: char| h.split(' ').find_map(|w| w.strip_prefix(sign)).and_then(|w| w.split(',').next()?.parse().ok()).unwrap_or(1);
            (old, new, body) = (start('-'), start('+'), true);
            return Row { kind: Kind::Hunk, text: l.into(), line: None };
        }
        let (kind, line) = match l.chars().next() {
            _ if !body => (Kind::Head, None),
            Some('+') => { new += 1; (Kind::Add, Some(new - 1)) }
            Some('-') => { old += 1; (Kind::Del, Some(old - 1)) }
            Some(' ') => { (old, new) = (old + 1, new + 1); (Kind::Same, Some(new - 1)) }
            _ => (Kind::Head, None), // "\\ No newline at end of file"
        };
        let text = if matches!(kind, Kind::Head) { l.to_string() } else { l.get(1..).unwrap_or("").to_string() };
        Row { kind, text, line }
    }).collect()
}

fn whole(text: &str) -> Vec<Row> {
    text.lines().enumerate().map(|(i, l)| Row { kind: Kind::Same, text: l.into(), line: Some(i as u32 + 1) }).collect()
}

/// What changed in a file against the last commit; a new file is all added.
fn read(dir: &Path, path: &str) -> Vec<Row> {
    let out = |args: &[&str]| Command::new("git").args(args).current_dir(dir).output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
    let tracked = Command::new("git").args(["ls-files", "--error-unmatch", "--", path]).current_dir(dir).output().is_ok_and(|o| o.status.success());
    let diff = if tracked { out(&["diff", "--no-color", "HEAD", "--", path]) } else { out(&["diff", "--no-color", "--no-index", "--", "/dev/null", path]) };
    if diff.trim().is_empty() {
        return whole(&std::fs::read_to_string(dir.join(path)).unwrap_or_default());
    }
    // the lines of git before the first change say what the title says
    rows(&diff).into_iter().skip_while(|r| r.kind == Kind::Head).collect()
}

/// The comments as one prompt for the agent, in the order of the file.
pub fn prompt(path: &str, rows: &[Row], notes: &BTreeMap<usize, String>) -> String {
    let mut s = (tr().review_intro)(path);
    for (i, note) in notes {
        let Some(r) = rows.get(*i) else { continue };
        let code = r.text.trim();
        let at = match (r.kind, r.line) {
            (Kind::Del, Some(n)) => (tr().review_removed)(n),
            (_, Some(n)) => (tr().review_line)(n),
            _ => String::new(),
        };
        let code = if code.is_empty() { String::new() } else { format!(" `{}`", code.chars().take(80).collect::<String>()) };
        s += &format!("\n- {at}{code}: {note}");
    }
    s
}

/// `orcux dialog diff <dir> <path>`.
pub fn show(dir: &Path, path: &str) -> Res<()> {
    let rows = read(dir, path);
    let (fg, muted, add, del, hunk, note, hull) = (
        Color::from_u32(theme::FOAM), Color::from_u32(theme::MUTED), Color::from_u32(theme::KELP), Color::from_u32(theme::CORAL),
        Color::from_u32(theme::CURRENT), Color::from_u32(theme::SIGNAL), Color::from_u32(theme::HULL),
    );
    // start on the first change
    let mut sel = rows.iter().position(|r| matches!(r.kind, Kind::Add | Kind::Del)).unwrap_or(0);
    let (mut top, mut notes, mut typing) = (0usize, BTreeMap::<usize, String>::new(), None::<String>);
    tui::dialog(|term| {
        loop {
            term.draw(|f| {
                let [head, body, foot] = Layout::vertical([Constraint::Length(2), Constraint::Fill(1), Constraint::Length(1)]).areas(f.area());
                let count = if notes.is_empty() { String::new() } else { format!("  {}", (tr().review_count)(notes.len())) };
                f.render_widget(Line::from(vec![Span::raw(" "), Span::styled(path.to_string(), Style::new().fg(fg).bold()), Span::styled(count, Style::new().fg(note))]), head);
                // what fits from `top`, a comment under its line; the selected line always in view
                let height = body.height as usize;
                let shown = |top: usize| {
                    let mut out = vec![];
                    for (i, _) in rows.iter().enumerate().skip(top) {
                        out.push((i, false));
                        if notes.contains_key(&i) || (i == sel && typing.is_some()) { out.push((i, true)); }
                        if out.len() >= height { break; }
                    }
                    out.truncate(height);
                    out
                };
                if sel < top { top = sel; }
                while top < sel && !shown(top).iter().any(|(i, n)| *i == sel && (!*n || typing.is_none())) { top += 1; }
                let lines: Vec<Line> = shown(top).into_iter().map(|(i, is_note)| {
                    let r = &rows[i];
                    if is_note {
                        let text = if i == sel && let Some(t) = &typing { format!("{t}▏") } else { notes.get(&i).cloned().unwrap_or_default() };
                        return Line::from(vec![Span::raw("        ✎ "), Span::styled(text, Style::new().fg(note))]);
                    }
                    let (sign, color) = match r.kind {
                        Kind::Add => ("+", add),
                        Kind::Del => ("-", del),
                        Kind::Hunk => ("", hunk),
                        Kind::Head => ("", muted),
                        Kind::Same => (" ", fg),
                    };
                    let num = r.line.map(|n| format!("{n:>6} ")).unwrap_or_else(|| " ".repeat(7));
                    let mark = if notes.contains_key(&i) { "✎" } else { " " };
                    let mut l = Line::from(vec![Span::styled(num, Style::new().fg(muted)), Span::styled(mark, Style::new().fg(note)), Span::styled(format!("{sign}{}", r.text), Style::new().fg(color))]);
                    if i == sel { l = l.bg(hull); }
                    l
                }).collect();
                f.render_widget(Paragraph::new(lines), body);
                let hint = if typing.is_some() { tr().review_typing } else { tr().review_keys };
                f.render_widget(Line::from(Span::styled(format!(" {hint}"), Style::new().fg(muted))), foot);
            }).map_err(|e| e.to_string())?;
            let Some(k) = tui::key()? else { continue };
            if let Some(t) = &mut typing {
                match k {
                    KeyCode::Enter => {
                        let t = t.trim().to_string();
                        if t.is_empty() { notes.remove(&sel); } else { notes.insert(sel, t); }
                        typing = None;
                    }
                    KeyCode::Esc => typing = None,
                    KeyCode::Backspace => { t.pop(); }
                    KeyCode::Char(c) => t.push(c),
                    _ => {}
                }
                continue;
            }
            let page = term.size().map(|s| s.height as usize).unwrap_or(20).saturating_sub(4);
            let last = rows.len().saturating_sub(1);
            match k {
                KeyCode::Down | KeyCode::Char('j') => sel = (sel + 1).min(last),
                KeyCode::Up | KeyCode::Char('k') => sel = sel.saturating_sub(1),
                KeyCode::PageDown | KeyCode::Char(' ') => sel = (sel + page).min(last),
                KeyCode::PageUp => sel = sel.saturating_sub(page),
                KeyCode::Char('g') => sel = 0,
                KeyCode::Char('G') => sel = last,
                // the next and the previous change
                KeyCode::Char('n') => sel = (sel + 1..rows.len()).find(|i| rows[*i].kind == Kind::Hunk).unwrap_or(sel),
                KeyCode::Char('N') => sel = (0..sel).rev().find(|i| rows[*i].kind == Kind::Hunk).unwrap_or(sel),
                KeyCode::Char('c') | KeyCode::Enter if rows[sel].line.is_some() => typing = Some(notes.get(&sel).cloned().unwrap_or_default()),
                KeyCode::Esc | KeyCode::Char('q') => break,
                _ => {}
            }
        }
        if notes.is_empty() {
            return Ok(());
        }
        send(term, dir, &prompt(path, &rows, &notes), notes.len())
    })
}

/// Asks, starting on yes (nothing is sent: it is left in the agent's input), and pastes it into the agent of the
/// worktree of that folder.
fn send(term: &mut ratatui::DefaultTerminal, dir: &Path, text: &str, n: usize) -> Res<()> {
    let w = find_at(dir);
    let pane = w.as_ref().and_then(|w| agent_tab(w).map(|p| p.pane));
    let mut yes = true;
    loop {
        let mut lines = vec![Line::from((tr().review_ask)(n, &w.as_ref().map(Wt::id).unwrap_or_default())), Line::from(tr().review_kept).fg(Color::from_u32(theme::MUTED)), Line::default()];
        lines.extend(text.lines().take(12).map(|l| Line::from(l.to_string()).fg(Color::from_u32(theme::FOAM))));
        if pane.is_none() {
            lines = vec![Line::from(tr().review_no_agent).fg(Color::from_u32(theme::CORAL))];
            tui::draw_dialog(term, lines, tr().hint_any)?;
            tui::key()?;
            return Ok(());
        }
        tui::draw_choice(term, lines, yes, "")?;
        match tui::key()?.and_then(|k| tui::chosen(k, &mut yes)) {
            None => {}
            Some(false) => return Ok(()),
            Some(true) => return paste_into(pane.as_deref().unwrap_or_default(), text),
        }
    }
}

/// The worktree whose folder this is.
fn find_at(dir: &Path) -> Option<Wt> {
    let dir = dir.canonicalize().ok()?;
    load().into_iter().find(|w| w.path().canonicalize().is_ok_and(|p| p == dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diffs_by_line() {
        let d = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -3,3 +3,4 @@ fn x\n same\n-gone\n+new\n+more\n tail\n\\ No newline at end of file";
        let r = rows(d);
        let got: Vec<(Kind, Option<u32>, &str)> = r.iter().map(|r| (r.kind, r.line, r.text.as_str())).collect();
        assert_eq!(got[3..], [(Kind::Hunk, None, "@@ -3,3 +3,4 @@ fn x"), (Kind::Same, Some(3), "same"), (Kind::Del, Some(4), "gone"),
            (Kind::Add, Some(4), "new"), (Kind::Add, Some(5), "more"), (Kind::Same, Some(6), "tail"), (Kind::Head, None, "\\ No newline at end of file")]);
        assert!(r[..3].iter().all(|r| r.kind == Kind::Head));
        let notes = BTreeMap::from([(6, "why two?".to_string()), (5, "keep it".to_string())]);
        let p = prompt("src/x.rs", &r, &notes);
        assert!(p.starts_with(&(tr().review_intro)("src/x.rs")), "{p}");
        assert!(p.contains(&format!("- {} `gone`: keep it", (tr().review_removed)(4))) && p.ends_with(&format!("- {} `new`: why two?", (tr().review_line)(4))), "{p}");
    }
}
