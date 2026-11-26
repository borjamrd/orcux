//! Prototype: orcux draws the terminal of a worktree itself, inside a frame with its tabs, instead of showing tmux.
//! tmux still runs the terminal; this process is not one of its clients. It reads what the terminal prints through a
//! control-mode connection, keeps the screen with an emulator (`vt100`), and gives tmux the keys by name, so tmux
//! encodes them as the program in the terminal asked. `orcux window <wt>[:<tab>]`; Ctrl+] leaves.
//!
//! The emulator only follows: whenever it could be wrong (start, resize, leaving the alternate screen, output that tmux
//! paused) it is seeded again from tmux's own screen (`capture-pane`). That is why it needs no reflow of its own.
use crate::*;
use ratatui::{
    crossterm::{
        event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
        execute,
    },
    prelude::*,
    widgets::{Block, BorderType},
};
use std::{
    io::{stdout, BufRead, BufReader, Write},
    num::NonZeroU16,
    process::{Child, ChildStdin, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

enum Msg {
    /// what the terminal printed
    Output(Vec<u8>),
    /// the answer to a command of ours, line by line
    Reply(bool, Vec<Vec<u8>>),
    /// tmux paused the output for us: we were slow
    Paused,
    Gone,
    Input(Event),
}

/// The connection to tmux, attached to a session that holds only the window of this terminal.
struct Link {
    child: Child,
    stdin: ChildStdin,
}

impl Link {
    fn send(&mut self, args: &[&str]) -> Res<()> {
        self.send_all(&[args])
    }

    /// Several commands in one line: tmux runs them one after the other, with nothing read from the terminal in
    /// between, and answers them in one reply.
    fn send_all(&mut self, cmds: &[&[&str]]) -> Res<()> {
        let quote = |a: &&str| format!("'{}'", a.replace('\'', "'\"'\"'"));
        let line = cmds.iter().map(|c| c.iter().map(quote).collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join(" ; ");
        writeln!(self.stdin, "{line}").and_then(|_| self.stdin.flush()).map_err(|e| e.to_string())
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
    }
}

/// tmux escapes what is not printable, and the backslash, as octal \ooo.
fn unescape(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() + 1 && b.get(i + 1..i + 4).is_some_and(|d| d.iter().all(|c| (b'0'..=b'7').contains(c))) {
            out.push((b[i + 1] - b'0') * 64 + (b[i + 2] - b'0') * 8 + (b[i + 3] - b'0'));
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

/// One line of control mode: what it means for us, if anything. `block` gathers the lines of a reply.
/// A reply of tmux is a block: %begin time number flags, its lines, %end (or %error) with the same three. Flags 1:
/// a command of this client; others (the attach itself) are not answers we wait for.
fn read_line(line: &[u8], pane: &str, block: &mut Option<(Vec<u8>, Vec<Vec<u8>>)>) -> Option<Msg> {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    if let Some((head, lines)) = block {
        let end = line.strip_prefix(b"%end ").map(|h| (true, h)).or(line.strip_prefix(b"%error ").map(|h| (false, h)));
        if let Some((ok, h)) = end && h == head.as_slice() {
            let ours = h.ends_with(b" 1");
            let lines = std::mem::take(lines);
            *block = None;
            return ours.then_some(Msg::Reply(ok, lines));
        }
        lines.push(line.to_vec());
        return None;
    }
    if let Some(head) = line.strip_prefix(b"%begin ") {
        *block = Some((head.to_vec(), vec![]));
        return None;
    }
    // %output %1 value, or with flow control: %extended-output %1 age ... : value
    let (rest, extended) = match (line.strip_prefix(b"%output "), line.strip_prefix(b"%extended-output ")) {
        (Some(r), _) => (r, false),
        (_, Some(r)) => (r, true),
        _ => {
            return if line.starts_with(b"%pause ") { Some(Msg::Paused) } else if line.starts_with(b"%exit") { Some(Msg::Gone) } else { None };
        }
    };
    let sp = rest.iter().position(|c| *c == b' ')?;
    if &rest[..sp] != pane.as_bytes() {
        return None;
    }
    let value = if extended {
        let at = rest.windows(3).position(|w| w == b" : ")?;
        &rest[at + 3..]
    } else {
        &rest[sp + 1..]
    };
    Some(Msg::Output(unescape(value)))
}

/// A key as tmux names it, or text to send as it is.
fn key_name(k: &KeyEvent) -> Option<(bool, String)> {
    let (ctrl, alt, shift) = (k.modifiers.contains(KeyModifiers::CONTROL), k.modifiers.contains(KeyModifiers::ALT), k.modifiers.contains(KeyModifiers::SHIFT));
    let named = |n: &str| {
        let mut s = String::new();
        if ctrl { s += "C-" }
        if alt { s += "M-" }
        if shift && !matches!(k.code, KeyCode::BackTab) { s += "S-" }
        Some((false, s + n))
    };
    match k.code {
        KeyCode::Char(c) if ctrl || alt => {
            let mut s = String::new();
            if ctrl { s += "C-" }
            if alt { s += "M-" }
            Some((false, format!("{s}{c}")))
        }
        KeyCode::Char(c) => Some((true, c.to_string())),
        KeyCode::Enter => named("Enter"),
        KeyCode::Tab => named("Tab"),
        KeyCode::BackTab => named("BTab"),
        KeyCode::Backspace => named("BSpace"),
        KeyCode::Esc => named("Escape"),
        KeyCode::Up => named("Up"),
        KeyCode::Down => named("Down"),
        KeyCode::Left => named("Left"),
        KeyCode::Right => named("Right"),
        KeyCode::Home => named("Home"),
        KeyCode::End => named("End"),
        KeyCode::PageUp => named("PPage"),
        KeyCode::PageDown => named("NPage"),
        KeyCode::Delete => named("DC"),
        KeyCode::Insert => named("IC"),
        KeyCode::F(n) => named(&format!("F{n}")),
        _ => None,
    }
}

fn color(c: vt100::Color) -> Color {
    match c {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// Copies the emulated screen into the frame. A wide character fills its cell and the next.
fn paint(screen: &vt100::Screen, buf: &mut Buffer, area: Rect) {
    let (rows, cols) = screen.size();
    let (rows, cols) = (rows.get(), cols.get());
    for r in 0..rows.min(area.height) {
        for c in 0..cols.min(area.width) {
            let Some(cell) = screen.cell(r, c) else { continue };
            if cell.is_wide_continuation() {
                continue;
            }
            let mut style = Style::new().fg(color(cell.fgcolor())).bg(color(cell.bgcolor()));
            for (on, m) in [(cell.bold(), Modifier::BOLD), (cell.dim(), Modifier::DIM), (cell.italic(), Modifier::ITALIC), (cell.underline(), Modifier::UNDERLINED), (cell.inverse(), Modifier::REVERSED)] {
                if on {
                    style = style.add_modifier(m);
                }
            }
            let text = if cell.has_contents() { cell.contents() } else { " " };
            if let Some(b) = buf.cell_mut((area.x + c, area.y + r)) {
                b.set_symbol(text).set_style(style);
            }
        }
    }
}

/// What the terminal shows now, as tmux has it: its screen, where its cursor is, and whether it is on the alternate
/// screen. The lines come in the reply to capture-pane; the rest in the reply to display-message.
fn seeded(rows: u16, cols: u16, lines: &[Vec<u8>], state: &str) -> vt100::Parser {
    let nz = |n: u16| NonZeroU16::new(n.max(1)).unwrap_or(NonZeroU16::MIN);
    let mut p = vt100::Parser::new(nz(rows), nz(cols), 0);
    let f: Vec<u16> = state.split(' ').filter_map(|x| x.parse().ok()).collect();
    let (x, y, visible, alternate) = (f.first().copied().unwrap_or(0), f.get(1).copied().unwrap_or(0), f.get(2) != Some(&0), f.get(3) == Some(&1));
    if alternate {
        p.process(b"\x1b[?1049h");
    }
    for (i, l) in lines.iter().take(rows as usize).enumerate() {
        p.process(format!("\x1b[{};1H\x1b[0m", i + 1).as_bytes());
        p.process(l);
    }
    p.process(format!("\x1b[0m\x1b[{};{}H", y + 1, x + 1).as_bytes());
    if !visible {
        p.process(b"\x1b[?25l");
    }
    p
}

/// Whether these bytes leave the alternate screen: the main one has to be asked to tmux again.
fn leaves_alternate(b: &[u8]) -> bool {
    [&b"\x1b[?1049l"[..], b"\x1b[?1047l", b"\x1b[?47l"].iter().any(|s| b.windows(s.len()).any(|w| w == *s))
}

/// The last of "begin" or "end" of a synchronized update in these bytes, if any.
fn sync_state(b: &[u8]) -> Option<bool> {
    let (on, off) = (b"\x1b[?2026h", b"\x1b[?2026l");
    let last = |s: &[u8]| b.windows(s.len()).rposition(|w| w == s);
    match (last(on), last(off)) {
        (Some(a), Some(b)) => Some(a > b),
        (Some(_), None) => Some(true),
        (None, Some(_)) => Some(false),
        (None, None) => None,
    }
}

pub fn run(q: &str) -> Res<()> {
    let (w, tab) = target(q)?;
    let pane = pane_at(&w, tab)?;
    let info = tmux(&["display-message", "-p", "-t", &pane, "#{session_name}\t#{window_id}"])?;
    let (session, window) = info.split_once('\t').ok_or(tr().no_pane)?;
    if session == UI_NAME {
        return Err(tr().window_visible.into()); // the prototype does not take it from the other interface
    }
    // a session of its own, with only that window: control mode tells what every pane of the session prints
    let view = format!("orcux-view-{}", std::process::id());
    let first = tmux(&["new-session", "-d", "-s", &view, "-P", "-F", "#{window_id}", "sleep 99999"])?;
    let undo = || { tmux(&["kill-session", "-t", &format!("={view}")]).ok(); };
    let linked = tmux(&["link-window", "-d", "-s", window, "-t", &format!("={view}:")]).and_then(|_| tmux(&["kill-window", "-t", first.trim()]));
    if let Err(e) = linked {
        undo();
        return Err(e);
    }
    tmux(&["set-option", "-w", "-t", window, "window-size", "manual"]).ok();
    let res = show(&w, &pane, window, &view);
    undo(); // the window stays where it was
    res
}

fn show(w: &Wt, pane: &str, window: &str, view: &str) -> Res<()> {
    let (tx, rx) = mpsc::channel();
    let mut waiting: std::collections::VecDeque<&str> = ["option"].into();
    let mut child = Command::new("tmux").args(["-u", "-C", "attach-session", "-t", &format!("={view}"), "-f", "ignore-size,pause-after=2"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e| e.to_string())?;
    let (Some(stdin), Some(out)) = (child.stdin.take(), child.stdout.take()) else { return Err(tr().no_pane.into()) };
    let mut link = Link { child, stdin };
    // gone with its last client, even if this process is killed: the window stays where it was
    link.send(&["set-option", "-t", &format!("={view}"), "destroy-unattached", "on"])?;
    let (feed, id) = (tx.clone(), pane.to_string());
    std::thread::spawn(move || {
        let mut block = None;
        for line in BufReader::new(out).split(b'\n') {
            let Ok(line) = line else { break };
            if let Some(m) = read_line(&line, &id, &mut block) && feed.send(m).is_err() {
                break;
            }
        }
        feed.send(Msg::Gone).ok();
    });
    let keys = tx;
    std::thread::spawn(move || while let Ok(e) = event::read() {
        if keys.send(Msg::Input(e)).is_err() {
            break;
        }
    });

    let mut term = ratatui::init();
    execute!(stdout(), EnableBracketedPaste).ok();
    let res = (|| -> Res<()> {
        let mut screen: Option<vt100::Parser> = None;
        let (mut dirty, mut drawn, mut synced_since) = (true, Instant::now() - Duration::from_secs(1), None::<Instant>);
        let mut size = (0u16, 0u16);
        // the screen and the cursor at the same moment: one line, so tmux reads nothing from the terminal in
        // between; each command still has its own reply
        let resync = |link: &mut Link, waiting: &mut std::collections::VecDeque<&str>| -> Res<()> {
            link.send_all(&[&["capture-pane", "-p", "-e", "-N", "-t", pane], &["display-message", "-p", "-t", pane, "#{cursor_x} #{cursor_y} #{cursor_flag} #{alternate_on}"]])?;
            waiting.extend(["capture", "state"]);
            Ok(())
        };
        let mut capture: Vec<Vec<u8>> = vec![];
        loop {
            // the size of the frame's inside is the size of the terminal
            let area = term.size().map_err(|e| e.to_string())?;
            let inside = (area.height.saturating_sub(2).max(1), area.width.saturating_sub(2).max(1));
            if inside != size {
                size = inside;
                tmux(&["resize-window", "-t", window, "-x", &size.1.to_string(), "-y", &size.0.to_string()])?;
                screen = None;
                resync(&mut link, &mut waiting)?;
            }
            let syncing = synced_since.is_some_and(|t| t.elapsed() < Duration::from_millis(200));
            if dirty && !syncing && drawn.elapsed() >= Duration::from_millis(16) {
                let tabs = tabs(&w.id());
                term.draw(|f| {
                    let mut title = vec![Span::raw(" ")];
                    for t in &tabs {
                        let name = format!(" {} {} ", t.tab, tui::program(&t.cmd));
                        title.push(if t.pane == pane { name.bold().fg(Color::from_u32(theme::FOAM)).bg(Color::from_u32(theme::WAKE)) } else { name.fg(Color::from_u32(theme::MUTED)) });
                        title.push(Span::raw(" "));
                    }
                    let block = Block::bordered().border_type(BorderType::Rounded).border_style(Style::new().fg(Color::from_u32(theme::CURRENT))).title(Line::from(title));
                    let inner = block.inner(f.area());
                    f.render_widget(block, f.area());
                    if let Some(p) = &screen {
                        paint(p.screen(), f.buffer_mut(), inner);
                        let (r, c) = p.screen().cursor_position();
                        if !p.screen().hide_cursor() {
                            f.set_cursor_position((inner.x + c, inner.y + r));
                        }
                    }
                }).map_err(|e| e.to_string())?;
                (dirty, drawn) = (false, Instant::now());
            }
            // nothing to do: wait without a timer
            let wait = if dirty { Duration::from_millis(16).saturating_sub(drawn.elapsed()).max(Duration::from_millis(1)) } else { Duration::from_secs(3600) };
            let msg = match rx.recv_timeout(wait) {
                Ok(m) => m,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => return Ok(()),
            };
            match msg {
                Msg::Output(b) => {
                    if let Some(on) = sync_state(&b) {
                        synced_since = on.then(Instant::now);
                    }
                    if let Some(p) = &mut screen {
                        p.process(&b);
                        if leaves_alternate(&b) {
                            resync(&mut link, &mut waiting)?;
                        }
                    }
                    // before the first seed: tmux's screen will already have it
                    dirty = true;
                }
                Msg::Reply(ok, lines) => match waiting.pop_front() {
                    Some("capture") => capture = if ok { lines } else { vec![] },
                    Some("state") if ok => {
                        let state = lines.first().map(|l| String::from_utf8_lossy(l).into_owned()).unwrap_or_default();
                        screen = Some(seeded(size.0, size.1, &capture, &state));
                        dirty = true;
                    }
                    _ => {}
                },
                Msg::Paused => {
                    link.send(&["refresh-client", "-A", &format!("{pane}:continue")])?;
                    waiting.push_back("continue");
                    resync(&mut link, &mut waiting)?;
                }
                Msg::Gone => return Ok(()),
                Msg::Input(Event::Key(k)) if k.kind != KeyEventKind::Release => {
                    // Ctrl+] is the byte 0x1d, which crossterm reads as Ctrl+5
                    if matches!(k.code, KeyCode::Char(']' | '5')) && k.modifiers.contains(KeyModifiers::CONTROL) {
                        return Ok(());
                    }
                    if let Some((literal, key)) = key_name(&k) {
                        let args: &[&str] = if literal { &["send-keys", "-l", "-t", pane, &key] } else { &["send-keys", "-t", pane, &key] };
                        link.send(args)?;
                        waiting.push_back("key");
                    }
                }
                Msg::Input(Event::Paste(text)) => {
                    // tmux wraps it as the program asked (bracketed paste)
                    link.send(&["set-buffer", "-b", "orcux-paste", &text])?;
                    link.send(&["paste-buffer", "-p", "-d", "-b", "orcux-paste", "-t", pane])?;
                    waiting.extend(["key", "key"]);
                }
                Msg::Input(_) => dirty = true, // a resize: the next turn sees it
            }
        }
    })();
    execute!(stdout(), DisableBracketedPaste).ok();
    ratatui::restore();
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_lines() {
        assert_eq!(unescape(b"a\\033[1mb\\134c\\015\\012"), b"a\x1b[1mb\\c\r\n");
        let mut block = None;
        assert!(matches!(read_line(b"%output %3 hi\\012", "%3", &mut block), Some(Msg::Output(b)) if b == b"hi\n"));
        assert!(read_line(b"%output %4 other", "%3", &mut block).is_none());
        assert!(matches!(read_line(b"%extended-output %3 12 : x : y", "%3", &mut block), Some(Msg::Output(b)) if b == b"x : y"));
        assert!(read_line(b"%begin 1 2 1", "%3", &mut block).is_none());
        assert!(read_line(b"line \x1b[1mone", "%3", &mut block).is_none());
        assert!(read_line(b"%end 9 9 1", "%3", &mut block).is_none()); // another block's end is a line of this one
        assert!(matches!(read_line(b"%end 1 2 1", "%3", &mut block), Some(Msg::Reply(true, l)) if l.len() == 2));
        // the block of the attach is not an answer
        assert!(read_line(b"%begin 5 6 0", "%3", &mut block).is_none());
        assert!(read_line(b"%end 5 6 0", "%3", &mut block).is_none());
        assert!(matches!(read_line(b"%pause %3", "%3", &mut block), Some(Msg::Paused)));
    }

    #[test]
    fn keys_and_screens() {
        let k = |code, m| key_name(&KeyEvent::new(code, m));
        assert_eq!(k(KeyCode::Char('a'), KeyModifiers::NONE), Some((true, "a".into())));
        assert_eq!(k(KeyCode::Char('c'), KeyModifiers::CONTROL), Some((false, "C-c".into())));
        assert_eq!(k(KeyCode::Enter, KeyModifiers::NONE), Some((false, "Enter".into())));
        assert_eq!(k(KeyCode::Up, KeyModifiers::SHIFT), Some((false, "S-Up".into())));
        assert_eq!(sync_state(b"\x1b[?2026hxx\x1b[?2026l"), Some(false));
        assert_eq!(sync_state(b"\x1b[?2026l..\x1b[?2026h"), Some(true));
        assert!(leaves_alternate(b"bye\x1b[?1049l"));
        let p = seeded(3, 10, &[b"\x1b[31mred".to_vec(), b"two".to_vec()], "4 1 1 0");
        assert_eq!(p.screen().contents(), "red\ntwo");
        assert_eq!(p.screen().cursor_position(), (1, 4));
        assert_eq!(p.screen().cell(0, 0).map(|c| c.fgcolor()), Some(vt100::Color::Idx(1)));
    }
}
