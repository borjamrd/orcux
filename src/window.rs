//! The window of orcux drawn by orcux (`orcux window`), instead of tmux's. tmux keeps running everything as before:
//! the sidebar, the terminals and the PR panel are the same panes of the session `orcux`, and the classic interface
//! (`orcux`) can be attached at any time. This process is a control-mode client of that session: tmux tells it what each
//! pane prints and how they are laid out, it keeps each screen with an emulator (`vt100`) and draws them, with a
//! frame around the terminal, clickable tabs and popups of its own. Keys go to the active pane by name, so tmux encodes
//! them as the program in it asked; clicks go to the pane under them, as the program asked too.
//!
//! An emulator only follows: whenever it could be wrong (a new pane, a resize, leaving the alternate screen, output that
//! tmux paused) it is seeded again from tmux's own screen (`capture-pane`). So it needs no reflow of its own.
use crate::*;
use ratatui::{
    crossterm::{
        event::{self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
        execute,
    },
    prelude::*,
};
use std::{
    collections::{HashMap, VecDeque},
    io::{stdout, BufRead, BufReader, Write},
    num::NonZeroU16,
    process::{Child, ChildStdin, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

/// The option where this window says which client it is, so that `viewer()` finds it.
pub const CLIENT: &str = "@orcux_window";

enum Msg {
    /// what a pane printed
    Output(String, Vec<u8>),
    /// the answer to a command of ours, line by line
    Reply(bool, Vec<Vec<u8>>),
    /// tmux paused a pane's output for us: we were slow
    Paused(String),
    /// panes, windows or their options changed
    Layout,
    Gone,
    Input(Event),
}

struct Link {
    child: Child,
    stdin: ChildStdin,
}

impl Link {
    fn send(&mut self, args: &[&str]) -> Res<()> {
        self.send_all(&[args])
    }

    /// Several commands in one line: tmux runs them one after the other, with nothing read from the terminals in
    /// between. Each one still has its own reply.
    fn send_all(&mut self, cmds: &[&[&str]]) -> Res<()> {
        // a line ends a command, an empty one detaches: nothing here may hold one (pasted text goes through a file)
        let quote = |a: &&str| format!("'{}'", a.replace(['\n', '\r'], " ").replace('\'', "'\"'\"'"));
        let line = cmds.iter().map(|c| c.iter().map(quote).collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join(" ; ");
        debug(&format!("> {line}"));
        writeln!(self.stdin, "{line}").and_then(|_| self.stdin.flush()).map_err(|e| e.to_string())
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
    }
}

/// With ORCUX_WINDOW_LOG set to a file: what this window asks and hears, for finding a fault.
fn debug(what: &str) {
    if let Some(path) = env::var_os("ORCUX_WINDOW_LOG") && let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(path) {
        writeln!(f, "{what}").ok();
    }
}

/// tmux escapes what is not printable, and the backslash, as octal \ooo.
fn unescape(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && b.get(i + 1..i + 4).is_some_and(|d| d.iter().all(|c| (b'0'..=b'7').contains(c))) {
            out.push((b[i + 1] - b'0') * 64 + (b[i + 2] - b'0') * 8 + (b[i + 3] - b'0'));
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

/// One line of control mode, and what it means for us. A reply is a block: %begin time number flags, its lines,
/// %end (or %error) with the same three; flags 1 marks a command of this client, others (the attach) are not answers.
fn read_line(line: &[u8], block: &mut Option<(Vec<u8>, Vec<Vec<u8>>)>) -> Option<Msg> {
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
    let word = |r: &[u8]| -> (String, usize) {
        let sp = r.iter().position(|c| *c == b' ').unwrap_or(r.len());
        (String::from_utf8_lossy(&r[..sp]).into_owned(), sp)
    };
    if let Some(r) = line.strip_prefix(b"%output ") {
        let (pane, sp) = word(r);
        return Some(Msg::Output(pane, unescape(r.get(sp + 1..).unwrap_or(&[]))));
    }
    // with flow control: %extended-output %1 age ... : value
    if let Some(r) = line.strip_prefix(b"%extended-output ") {
        let (pane, _) = word(r);
        let at = r.windows(3).position(|w| w == b" : ")?;
        return Some(Msg::Output(pane, unescape(&r[at + 3..])));
    }
    if let Some(r) = line.strip_prefix(b"%pause ") {
        return Some(Msg::Paused(word(r).0));
    }
    if line.starts_with(b"%exit") {
        return Some(Msg::Gone);
    }
    const LAYOUT: [&[u8]; 9] = [b"%layout-change", b"%window-add", b"%window-close", b"%unlinked-window-close", b"%window-pane-changed",
        b"%session-window-changed", b"%subscription-changed", b"%pane-mode-changed", b"%window-renamed"];
    LAYOUT.iter().any(|p| line.starts_with(p)).then_some(Msg::Layout)
}

/// A key as tmux names it, or text to send as it is.
fn key_name(k: &KeyEvent) -> Option<(bool, String)> {
    let (ctrl, alt, shift) = (k.modifiers.contains(KeyModifiers::CONTROL), k.modifiers.contains(KeyModifiers::ALT), k.modifiers.contains(KeyModifiers::SHIFT));
    let prefix = |shifted: bool| format!("{}{}{}", if ctrl { "C-" } else { "" }, if alt { "M-" } else { "" }, if shifted && shift { "S-" } else { "" });
    let named = |n: &str| Some((false, prefix(true) + n));
    match k.code {
        KeyCode::Char(c) if ctrl || alt => Some((false, format!("{}{c}", prefix(false)))),
        KeyCode::Char(c) => Some((true, c.to_string())),
        KeyCode::Enter => named("Enter"),
        KeyCode::Tab => named("Tab"),
        KeyCode::BackTab => Some((false, "BTab".into())),
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

/// The keys that orcux binds in every pane (Opt+w, t, a, d, q, r): with Option as Meta, or the character that Option
/// types on the Spanish and US layouts.
#[derive(Debug, PartialEq, Clone, Copy)]
enum Chord {
    Switch,
    NewTab,
    PrevTab,
    NextTab,
    CloseTab,
    Waiting,
}

fn chord(k: &KeyEvent) -> Option<Chord> {
    let alt = k.modifiers.contains(KeyModifiers::ALT) && !k.modifiers.contains(KeyModifiers::CONTROL);
    let KeyCode::Char(c) = k.code else { return None };
    let c = if alt { c } else { match c { 'æ' | '∑' => 'w', '†' => 't', 'å' => 'a', '∂' => 'd', 'œ' => 'q', '®' => 'r', _ => return None } };
    Some(match c {
        'w' => Chord::Switch,
        't' => Chord::NewTab,
        'a' => Chord::PrevTab,
        'd' => Chord::NextTab,
        'q' => Chord::CloseTab,
        'r' => Chord::Waiting,
        _ => return None,
    })
}

/// A mouse event as the program in a pane asked for it, at that column and row of the pane, both from 0: only the
/// events of its mode, in SGR or in the old encoding of single bytes.
fn mouse_bytes(m: &MouseEvent, col: u16, row: u16, mode: vt100::MouseProtocolMode, encoding: vt100::MouseProtocolEncoding) -> Option<Vec<u8>> {
    use vt100::MouseProtocolMode as M;
    let mods = |b: u8| b + if m.modifiers.contains(KeyModifiers::SHIFT) { 4 } else { 0 } + if m.modifiers.contains(KeyModifiers::ALT) { 8 } else { 0 } + if m.modifiers.contains(KeyModifiers::CONTROL) { 16 } else { 0 };
    let button = |b: MouseButton| match b { MouseButton::Left => 0, MouseButton::Middle => 1, MouseButton::Right => 2 };
    let (code, release) = match m.kind {
        MouseEventKind::Down(b) => (button(b), false),
        MouseEventKind::ScrollUp => (64, false),
        MouseEventKind::ScrollDown => (65, false),
        MouseEventKind::Up(b) if mode != M::Press => (button(b), true),
        MouseEventKind::Drag(b) if matches!(mode, M::ButtonMotion | M::AnyMotion) => (32 + button(b), false),
        MouseEventKind::Moved if mode == M::AnyMotion => (35, false),
        _ => return None,
    };
    if encoding == vt100::MouseProtocolEncoding::Sgr {
        return Some(format!("\x1b[<{};{};{}{}", mods(code), col + 1, row + 1, if release { 'm' } else { 'M' }).into_bytes());
    }
    // one byte each, from 32: a release has no button of its own, and far columns do not fit
    let code = if release { 3 } else { code };
    let (x, y) = (u8::try_from(col + 33).ok()?, u8::try_from(row + 33).ok()?);
    Some(vec![0x1b, b'[', b'M', 32 + mods(code), x, y])
}

fn color(c: vt100::Color, default: Color) -> Color {
    match c {
        vt100::Color::Default => default,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// Copies an emulated screen into the frame; the default colors are those of the pane (a card has its own ground).
fn paint(screen: &vt100::Screen, buf: &mut Buffer, area: Rect, ground: (Color, Color)) {
    let (rows, cols) = screen.size();
    for r in 0..rows.get().min(area.height) {
        for c in 0..cols.get().min(area.width) {
            let Some(cell) = screen.cell(r, c) else { continue };
            if cell.is_wide_continuation() {
                continue;
            }
            let mut style = Style::new().fg(color(cell.fgcolor(), ground.0)).bg(color(cell.bgcolor(), ground.1));
            for (on, m) in [(cell.bold(), Modifier::BOLD), (cell.dim(), Modifier::DIM), (cell.italic(), Modifier::ITALIC), (cell.underline(), Modifier::UNDERLINED), (cell.inverse(), Modifier::REVERSED)] {
                if on {
                    style = style.add_modifier(m);
                }
            }
            if let Some(b) = buf.cell_mut((area.x + c, area.y + r)) {
                b.set_symbol(if cell.has_contents() { cell.contents() } else { " " }).set_style(style);
            }
        }
    }
}

/// What seeded() needs besides the screen, in this order.
const STATE: &str = "#{cursor_x} #{cursor_y} #{cursor_flag} #{alternate_on} #{mouse_standard_flag} #{mouse_button_flag} #{mouse_any_flag} \
    #{mouse_sgr_flag} #{scroll_region_upper} #{scroll_region_lower} #{origin_flag} #{insert_flag} #{wrap_flag} #{history_size}";

/// What a pane shows now, as tmux has it: its screen (the reply to capture-pane) and its cursor and screen (the reply
/// to display-message: x, y, whether the cursor is shown, whether the alternate screen is on).
fn seeded(rows: u16, cols: u16, lines: &[Vec<u8>], state: &str) -> vt100::Parser {
    let nz = |n: u16| NonZeroU16::new(n.max(1)).unwrap_or(NonZeroU16::MIN);
    let mut p = vt100::Parser::new(nz(rows), nz(cols), 0);
    let f: Vec<u16> = state.split(' ').map(|x| x.parse().unwrap_or(0)).collect();
    let at = |i: usize, default: u16| f.get(i).copied().unwrap_or(default);
    let on = |i: usize| at(i, 0) == 1;
    let (x, y) = (at(0, 0), at(1, 0));
    if on(3) {
        p.process(b"\x1b[?1049h");
    }
    for (i, l) in lines.iter().take(rows as usize).enumerate() {
        p.process(format!("\x1b[{};1H\x1b[0m", i + 1).as_bytes());
        p.process(l);
    }
    p.process(b"\x1b[0m");
    // the modes that the program set and that change what comes next: the mouse it reads, its scroll region,
    // origin, insert and wrap
    for (i, mode) in [(6, "1003"), (5, "1002"), (4, "1000")] {
        if on(i) {
            p.process(format!("\x1b[?{mode}h").as_bytes());
            break;
        }
    }
    if on(7) {
        p.process(b"\x1b[?1006h");
    }
    let (upper, lower) = (at(8, 0), at(9, rows.saturating_sub(1)));
    if (upper, lower) != (0, rows.saturating_sub(1)) && upper < lower {
        p.process(format!("\x1b[{};{}r", upper + 1, lower + 1).as_bytes());
    }
    let origin = on(10);
    if origin {
        p.process(b"\x1b[?6h");
    }
    if on(11) {
        p.process(b"\x1b[4h");
    }
    if f.len() > 12 && !on(12) {
        p.process(b"\x1b[?7l");
    }
    let y = if origin { y.saturating_sub(upper) } else { y };
    p.process(format!("\x1b[{};{}H", y + 1, x + 1).as_bytes());
    if f.len() > 2 && !on(2) {
        p.process(b"\x1b[?25l");
    }
    p
}

/// The first bytes after a seed may be the end of a sequence that began before it, which tmux's screen has and a new
/// emulator never saw: "38;2;1;2;3m" would be printed as text. That end is dropped: parameters and a final byte (CSI),
/// or a string up to its BEL or ST (OSC).
fn after_seed(b: &[u8]) -> &[u8] {
    if b.first() == Some(&0x1b) {
        return b;
    }
    let csi = b.strip_prefix(b"[").unwrap_or(b);
    let params = csi.iter().take_while(|c| b"0123456789;:?<=>".contains(c)).count();
    // without its "[", only what cannot be text: several parameters ("12 files" stays)
    if (csi.len() < b.len() || csi[..params].contains(&b';')) && let Some(f) = csi.get(params) && (0x40..=0x7e).contains(f) {
        return &csi[params + 1..];
    }
    // the rest of a title or another string: short, on one line, ended by BEL or ST
    let end = b.iter().take(512).position(|c| *c == 0x07 || *c == b'\n').filter(|i| b[*i] == 0x07)
        .or_else(|| b.windows(2).take(512).position(|w| w == b"\x1b\\").map(|i| i + 1));
    match end {
        Some(i) if !b[..i].contains(&0x1b) || b[..i].ends_with(&[0x1b]) => &b[i + 1..],
        _ => b,
    }
}

fn holds(b: &[u8], s: &[u8]) -> bool {
    b.windows(s.len()).any(|w| w == s)
}

/// Whether these bytes leave the alternate screen: the main one has to be asked to tmux again.
fn leaves_alternate(b: &[u8]) -> bool {
    [&b"\x1b[?1049l"[..], b"\x1b[?1047l", b"\x1b[?47l"].iter().any(|s| holds(b, s))
}

/// The last of "begin" or "end" of a synchronized update in these bytes, if any.
fn sync_state(b: &[u8]) -> Option<bool> {
    let last = |s: &[u8]| b.windows(s.len()).rposition(|w| w == s);
    match (last(b"\x1b[?2026h"), last(b"\x1b[?2026l")) {
        (Some(a), Some(b)) => Some(a > b),
        (Some(_), None) => Some(true),
        (None, Some(_)) => Some(false),
        (None, None) => None,
    }
}

/// A pane of the session, where tmux puts it.
#[derive(Clone, Debug, PartialEq)]
struct Place {
    pane: String,
    window: String,
    /// left, top, width, height, inside the window
    at: (u16, u16, u16, u16),
    active: bool,
    /// the window that the session shows
    shown: bool,
    mark: String,
    /// it reads the mouse, in SGR
    mouse: bool,
    /// title and wanted size of a popup's window
    popup: Option<String>,
}

fn places() -> Vec<Place> {
    let f = "#{pane_id}\t#{window_id}\t#{pane_left}\t#{pane_top}\t#{pane_width}\t#{pane_height}\t#{pane_active}\t#{window_active}\t#{@orcux}\t#{mouse_sgr_flag}\t#{@orcux_popup}";
    let list = tmux(&["list-panes", "-s", "-t", &format!("{UI}:"), "-F", f]).unwrap_or_default();
    list.lines().filter_map(|l| {
        let v: Vec<&str> = l.split('\t').collect();
        let n = |i: usize| v.get(i).and_then(|x| x.parse().ok()).unwrap_or(0u16);
        let s = |i: usize| v.get(i).copied().unwrap_or("").to_string();
        Some(Place {
            pane: s(0), window: s(1), at: (n(2), n(3), n(4), n(5)), active: s(6) == "1", shown: s(7) == "1", mark: s(8),
            mouse: s(9) == "1", popup: Some(s(10)).filter(|t| !t.is_empty()),
        }).filter(|p| !p.pane.is_empty())
    }).collect()
}

/// Each pane's screen, and whether it is still being asked for.
struct Screen {
    parser: Option<vt100::Parser>,
    size: (u16, u16),
    syncing: Option<Instant>,
    /// lines scrolled back into tmux's history, and those lines once they come; 0 is the live screen
    back: u16,
    /// seeded, and nothing printed since: see after_seed
    fresh: bool,
    /// lines of tmux's history, as of the last seed: how far back the wheel goes
    lines: u16,
    history: Option<vt100::Parser>,
}

impl Screen {
    /// What is on screen: the history while scrolled back.
    fn shown(&self) -> Option<&vt100::Parser> {
        if self.back > 0 { self.history.as_ref().or(self.parser.as_ref()) } else { self.parser.as_ref() }
    }
}

/// Text being selected with the mouse, in one pane: where it started and where it is, as (row, column) of the screen.
struct Selection {
    pane: String,
    from: (u16, u16),
    to: (u16, u16),
}

impl Selection {
    fn ordered(&self) -> ((u16, u16), (u16, u16)) {
        if self.from <= self.to { (self.from, self.to) } else { (self.to, self.from) }
    }

    fn holds(&self, r: u16, c: u16) -> bool {
        let ((r0, c0), (r1, c1)) = self.ordered();
        (r0, c0) <= (r, c) && (r, c) <= (r1, c1)
    }
}

/// Pasted text in a file that only this person can read, for tmux's load-buffer.
fn paste_file(text: &str) -> Option<std::path::PathBuf> {
    use std::os::unix::fs::OpenOptionsExt;
    static COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = env::temp_dir().join(format!("orcux-paste-{}-{n}", std::process::id()));
    let mut f = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path).ok()?;
    f.write_all(text.as_bytes()).ok()?;
    Some(path)
}

/// The text goes to the clipboard of the computer: its own program if there is one, else the terminal (OSC 52).
fn copy(text: &str) {
    let tools: [&[&str]; 3] = [&["pbcopy"], &["wl-copy"], &["xclip", "-selection", "clipboard"]];
    for t in tools {
        if let Ok(mut c) = Command::new(t[0]).args(&t[1..]).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            if let Some(mut i) = c.stdin.take() {
                i.write_all(text.as_bytes()).ok();
            }
            if c.wait().is_ok_and(|s| s.success()) {
                return;
            }
        }
    }
    let mut out = stdout();
    write!(out, "\x1b]52;c;{}\x07", base64(text.as_bytes())).ok();
    out.flush().ok();
}

fn base64(b: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    b.chunks(3).map(|c| {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        (0..4).map(|i| if i <= c.len() { T[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' }).collect::<String>()
    }).collect()
}

/// What a click on the top row does.
enum Spot {
    Tab(u32),
    NewTab,
}

pub fn run() -> Res<()> {
    // in a pane of orcux it would draw itself, and resize its own window, without end
    // (TMUX_PANE means something only with TMUX: it is a pane of that server)
    if env::var("TMUX").is_ok() && let Ok(me) = env::var("TMUX_PANE") && let Ok(s) = tmux(&["display-message", "-p", "-t", &me, "#{session_name}"]) && (s == UI_NAME || s == BG) {
        return Err(tr().window_inside.into());
    }
    ensure_ui()?;
    let (tx, rx) = mpsc::channel();
    let mut child = Command::new("tmux").args(["-u", "-C", "attach-session", "-t", UI, "-f", "pause-after=2"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e| e.to_string())?;
    let (Some(stdin), Some(out)) = (child.stdin.take(), child.stdout.take()) else { return Err(tr().no_pane.into()) };
    let mut link = Link { child, stdin };
    let feed = tx.clone();
    std::thread::spawn(move || {
        let mut block = None;
        for line in BufReader::new(out).split(b'\n') {
            let Ok(line) = line else { break };
            if !line.starts_with(b"%output") && !line.starts_with(b"%extended-output") {
                debug(&format!("< {}", String::from_utf8_lossy(&line)));
            }
            if let Some(m) = read_line(&line, &mut block) && feed.send(m).is_err() {
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
    // a panic must not leave the mouse on in the person's shell: ratatui's own hook restores the rest
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        execute!(stdout(), DisableMouseCapture, DisableBracketedPaste).ok();
        hook(info);
    }));
    execute!(stdout(), EnableBracketedPaste, EnableMouseCapture).ok();
    let mut window = Window::new();
    let res = window.run(&mut term, &mut link, &rx);
    execute!(stdout(), DisableMouseCapture, DisableBracketedPaste).ok();
    ratatui::restore();
    // its popups go with it; the option only if it is still ours (another window may have come since)
    for l in tmux(&["list-windows", "-t", UI, "-F", "#{window_id} #{@orcux_popup}"]).unwrap_or_default().lines() {
        if let Some((id, title)) = l.split_once(' ') && !title.is_empty() {
            tmux(&["kill-window", "-t", id]).ok();
        }
    }
    if !window.me.is_empty() && tmux(&["show-option", "-gqv", CLIENT]).is_ok_and(|v| v == window.me) {
        tmux(&["set-option", "-g", "-u", CLIENT]).ok();
    }
    res
}

struct Window {
    screens: HashMap<String, Screen>,
    places: Vec<Place>,
    /// replies come in the order of the commands: what each one is for
    waiting: VecDeque<(&'static str, String)>,
    capture: Vec<Vec<u8>>,
    /// rows above the window: our top row
    top: u16,
    size: (u16, u16),
    spots: Vec<(u16, u16, Spot)>,
    dirty: bool,
    selection: Option<Selection>,
    /// this client's name, once tmux told it
    me: String,
    /// the tabs of the shown worktree: asked when the layout or the tab bar change, not at every frame
    tabs: Vec<Pane>,
}

impl Window {
    fn new() -> Window {
        Window { screens: HashMap::new(), places: vec![], waiting: VecDeque::new(), capture: vec![], top: 1, size: (0, 0), spots: vec![], dirty: true, selection: None, me: String::new(), tabs: vec![] }
    }

    /// The screen of that pane and its cursor at the same moment: one line, so tmux reads nothing in between.
    fn resync(&mut self, link: &mut Link, pane: &str) -> Res<()> {
        link.send_all(&[&["capture-pane", "-p", "-e", "-N", "-t", pane], &["display-message", "-p", "-t", pane, STATE]])?;
        self.waiting.extend([("capture", pane.to_string()), ("state", pane.to_string())]);
        Ok(())
    }

    /// Where the panes are now; the screens of new or resized ones are asked again.
    fn relayout(&mut self, link: &mut Link) -> Res<()> {
        self.places = places();
        let shown = self.places.iter().find(|p| p.shown && p.popup.is_none() && p.mark != "__sidebar" && p.mark != "__prs").map(|p| p.mark.clone());
        self.tabs = shown.filter(|m| !m.is_empty()).map(|m| tabs(&m)).unwrap_or_default();
        let live: Vec<(String, (u16, u16))> = self.places.iter().map(|p| (p.pane.clone(), (p.at.3, p.at.2))).collect();
        self.screens.retain(|id, _| live.iter().any(|l| &l.0 == id));
        for (id, size) in live {
            let stale = self.screens.get(&id).is_none_or(|s| s.size != size);
            if stale {
                self.screens.insert(id.clone(), Screen { parser: None, size, syncing: None, back: 0, history: None, fresh: false, lines: 0 });
                self.resync(link, &id)?;
            }
        }
        self.dirty = true;
        Ok(())
    }

    /// The pane that gets the keys: a popup if there is one, else the active pane of the shown window.
    fn target(&self) -> Option<&Place> {
        self.places.iter().find(|p| p.popup.is_some()).or_else(|| self.places.iter().find(|p| p.shown && p.active))
    }

    /// The size of this terminal, given to tmux as the client's: two rows less, for the top row and the bottom of the frame.
    fn fit(&mut self, link: &mut Link, area: Size) -> Res<()> {
        if (area.width, area.height) != self.size {
            self.size = (area.width, area.height);
            // a control client has no status line: the window gets the whole size, between our top row and the bottom
            // of the frame
            link.send(&["refresh-client", "-C", &format!("{}x{}", area.width.max(10), area.height.saturating_sub(2).max(5))])?;
            self.waiting.push_back(("", String::new()));
        }
        Ok(())
    }

    fn run(&mut self, term: &mut ratatui::DefaultTerminal, link: &mut Link, rx: &mpsc::Receiver<Msg>) -> Res<()> {
        link.send(&["display-message", "-p", "#{client_name}"])?;
        self.waiting.push_back(("me", String::new()));
        link.send(&["refresh-client", "-B", "orcux-window::#{@orcux_bar}#{session_windows}"])?;
        self.waiting.push_back(("", String::new()));
        self.fit(link, term.size().map_err(|e| e.to_string())?)?;
        self.relayout(link)?;
        let mut drawn = Instant::now() - Duration::from_secs(1);
        loop {
            let syncing = self.screens.values().any(|s| s.syncing.is_some_and(|t| t.elapsed() < Duration::from_millis(200)));
            if self.dirty && !syncing && drawn.elapsed() >= Duration::from_millis(16) {
                term.draw(|f| self.draw(f)).map_err(|e| e.to_string())?;
                (self.dirty, drawn) = (false, Instant::now());
            }
            // nothing to draw: wait without a timer
            let wait = if self.dirty { Duration::from_millis(16).saturating_sub(drawn.elapsed()).max(Duration::from_millis(1)) } else { Duration::from_secs(3600) };
            let msg = match rx.recv_timeout(wait) {
                Ok(m) => m,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => return Ok(()),
            };
            match msg {
                Msg::Output(id, b) => {
                    let Some(s) = self.screens.get_mut(&id) else { continue };
                    if let Some(on) = sync_state(&b) {
                        s.syncing = on.then(Instant::now);
                    }
                    if let Some(p) = &mut s.parser {
                        let fresh = std::mem::take(&mut s.fresh);
                        let kept = if fresh { after_seed(&b) } else { &b[..] };
                        // cut in the middle of something (a sequence, a character): what tmux has now is whole
                        let cut = fresh && (kept.len() < b.len() || b.first().is_some_and(|c| (0x80..0xc0).contains(c)));
                        p.process(kept);
                        if leaves_alternate(kept) || cut {
                            self.resync(link, &id)?;
                        }
                    }
                    self.dirty = true;
                }
                Msg::Reply(ok, lines) => match self.waiting.pop_front() {
                    Some(("me", _)) if ok => {
                        let me = lines.first().map(|l| String::from_utf8_lossy(l).into_owned()).unwrap_or_default();
                        // the person is this client now, for popups and for "somebody is looking"
                        tmux(&["set-option", "-g", CLIENT, &me]).ok();
                        self.me = me;
                    }
                    Some(("capture", _)) => self.capture = if ok { lines } else { vec![] },
                    Some(("unlink", path)) => { fs::remove_file(path).ok(); }
                    Some(("hsize", id)) => {
                        if let Some(s) = self.screens.get_mut(&id) {
                            s.lines = lines.first().and_then(|l| String::from_utf8_lossy(l).trim().parse().ok()).unwrap_or(s.lines);
                            s.back = s.back.min(s.lines);
                            if s.back == 0 {
                                s.history = None;
                            }
                            self.dirty = true;
                        }
                    }
                    Some(("history", id)) => {
                        if let Some(s) = self.screens.get_mut(&id) && s.back > 0 {
                            // fewer lines than asked: the top of the history
                            let lines = if ok { lines } else { vec![] };
                            s.history = Some(seeded(s.size.0, s.size.1, &lines, "0 0 0 0"));
                            self.dirty = true;
                        }
                    }
                    Some(("state", id)) if ok => {
                        if let Some(s) = self.screens.get_mut(&id) {
                            let state = lines.first().map(|l| String::from_utf8_lossy(l).into_owned()).unwrap_or_default();
                            s.parser = Some(seeded(s.size.0, s.size.1, &self.capture, &state));
                            s.fresh = true;
                            s.lines = state.split(' ').nth(13).and_then(|n| n.parse().ok()).unwrap_or(u16::MAX);
                            self.dirty = true;
                        }
                    }
                    _ => {}
                },
                Msg::Paused(id) => {
                    link.send(&["refresh-client", "-A", &format!("{id}:continue")])?;
                    self.waiting.push_back(("", String::new()));
                    self.resync(link, &id)?;
                }
                Msg::Layout => self.relayout(link)?,
                Msg::Gone => return Ok(()),
                Msg::Input(Event::Resize(..)) => {
                    self.fit(link, term.size().map_err(|e| e.to_string())?)?;
                    self.dirty = true;
                }
                Msg::Input(Event::Key(k)) if k.kind != KeyEventKind::Release => self.key(link, &k)?,
                Msg::Input(Event::Mouse(m)) => self.mouse(link, &m)?,
                Msg::Input(Event::Paste(text)) => {
                    if let Some(p) = self.target().map(|p| p.pane.clone()) && let Some(file) = paste_file(&text) {
                        // through a file of this person only: text in a command could end it and start another.
                        // tmux wraps it as the program asked (bracketed paste)
                        let path = file.to_string_lossy().to_string();
                        link.send_all(&[&["load-buffer", "-b", "orcux-paste", &path], &["paste-buffer", "-p", "-d", "-b", "orcux-paste", "-t", &p]])?;
                        self.waiting.extend([("unlink", path), ("", String::new())]);
                    }
                }
                Msg::Input(_) => {}
            }
        }
    }

    fn key(&mut self, link: &mut Link, k: &KeyEvent) -> Res<()> {
        let Some(target) = self.target().cloned() else { return Ok(()) };
        // in a popup every key is its own; elsewhere orcux's keys work in every pane, as in the classic interface
        if target.popup.is_none() && let Some(c) = chord(k) {
            std::thread::spawn(move || {
                let r = match c {
                    Chord::Switch => popup(tr().title_switch, (100, 30), &["switch"]),
                    Chord::NewTab => visible().and_then(|(w, _)| open_tab(&w)),
                    Chord::PrevTab => step_tab(false),
                    Chord::NextTab => step_tab(true),
                    Chord::CloseTab => popup(tr().title_close, (64, 10), &["dialog", "close"]),
                    Chord::Waiting => work::next(),
                };
                drop(r); // as with the classic bindings: nothing to say over the screen
            });
            return Ok(());
        }
        // a key goes back to the live screen, as in tmux
        if self.selection.take().is_some() {
            self.dirty = true;
        }
        if let Some(s) = self.screens.get_mut(&target.pane) && s.back > 0 {
            (s.back, s.history, self.dirty) = (0, None, true);
        }
        if let Some((literal, key)) = key_name(k) {
            let args: &[&str] = if literal { &["send-keys", "-l", "-t", &target.pane, &key] } else { &["send-keys", "-t", &target.pane, &key] };
            link.send(args)?;
            self.waiting.push_back(("", String::new()));
        }
        Ok(())
    }

    /// Where a pane is on this screen.
    fn rect(&self, p: &Place) -> Rect {
        match &p.popup {
            Some(_) => {
                let (w, h) = (p.at.2, p.at.3);
                Rect { x: self.size.0.saturating_sub(w) / 2, y: self.size.1.saturating_sub(h) / 2, width: w, height: h }
            }
            None => Rect { x: p.at.0, y: p.at.1 + self.top, width: p.at.2, height: p.at.3 },
        }
    }

    fn mouse(&mut self, link: &mut Link, m: &MouseEvent) -> Res<()> {
        let (col, row) = (m.column, m.row);
        // a selection follows the button wherever it goes, and ends where it is let go
        if matches!(m.kind, MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left))
            && let Some(owner) = self.selection.as_ref().and_then(|s| self.places.iter().find(|p| p.pane == s.pane)).cloned()
        {
            let r = self.rect(&owner);
            let to = (row.clamp(r.y, r.y + r.height.saturating_sub(1)) - r.y, col.clamp(r.x, r.x + r.width.saturating_sub(1)) - r.x);
            if let Some(s) = &mut self.selection {
                s.to = to;
            }
            if matches!(m.kind, MouseEventKind::Up(_)) && let Some(s) = self.selection.take() && s.from != s.to
                && let Some(scr) = self.screens.get(&s.pane).and_then(Screen::shown)
            {
                let ((r0, c0), (r1, c1)) = s.ordered();
                copy(&scr.screen().contents_between(r0, c0, r1, c1 + 1));
            }
            self.dirty = true;
            return Ok(());
        }
        if row == 0 && matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
            let spot = self.spots.iter().find(|s| (s.0..s.1).contains(&col)).map(|s| match s.2 { Spot::Tab(n) => Some(n), Spot::NewTab => None });
            if let Some(spot) = spot {
                std::thread::spawn(move || {
                    let r = match spot {
                        Some(n) => click_tab(&format!("t{n}")),
                        None => click_tab("new"),
                    };
                    drop(r);
                });
            }
            return Ok(());
        }
        // a popup takes the mouse as it takes the keys
        let visible: Vec<Place> = match self.places.iter().find(|p| p.popup.is_some()) {
            Some(p) => vec![p.clone()],
            None => self.places.iter().filter(|p| p.shown).cloned().collect(),
        };
        let Some(p) = visible.iter().find(|p| { let r = self.rect(p); r.contains(Position { x: col, y: row }) }) else { return Ok(()) };
        let r = self.rect(p);
        // the focus goes where the click is; asked every time, as what we know of it may be a moment old
        if matches!(m.kind, MouseEventKind::Down(_)) && p.popup.is_none() {
            link.send(&["select-pane", "-t", &p.pane])?;
            self.waiting.push_back(("", String::new()));
        }
        let (pc, pr) = (col - r.x, row - r.y);
        // what the program in the pane asked for, as its own screen says now
        let modes = self.screens.get(&p.pane).and_then(|s| s.parser.as_ref()).map(|s| (s.screen().mouse_protocol_mode(), s.screen().mouse_protocol_encoding()));
        let wants = modes.is_some_and(|m| m.0 != vt100::MouseProtocolMode::None);
        // with Shift, the mouse is ours (to select), as in most terminals
        if !wants || m.modifiers.contains(KeyModifiers::SHIFT) {
            match m.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    self.selection = Some(Selection { pane: p.pane.clone(), from: (pr, pc), to: (pr, pc) });
                    self.dirty = true;
                }
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => self.scroll(link, p, matches!(m.kind, MouseEventKind::ScrollUp))?,
                _ => {}
            }
            return Ok(());
        }
        if let Some((mode, encoding)) = modes && let Some(bytes) = mouse_bytes(m, pc, pr, mode, encoding) {
            let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
            let mut args = vec!["send-keys", "-H", "-t", &p.pane];
            args.extend(hex.iter().map(String::as_str));
            link.send(&args)?;
            self.waiting.push_back(("", String::new()));
        }
        Ok(())
    }

    /// The wheel over a pane whose program does not read the mouse: its history, asked to tmux, or arrows for a
    /// program on the alternate screen (a pager), as tmux does.
    fn scroll(&mut self, link: &mut Link, p: &Place, up: bool) -> Res<()> {
        let Some(s) = self.screens.get_mut(&p.pane) else { return Ok(()) };
        if s.parser.as_ref().is_some_and(|x| x.screen().alternate_screen()) {
            for _ in 0..3 {
                link.send(&["send-keys", "-t", &p.pane, if up { "Up" } else { "Down" }])?;
                self.waiting.push_back(("", String::new()));
            }
            return Ok(());
        }
        s.back = if up { s.back.saturating_add(3) } else { s.back.saturating_sub(3) };
        if s.back == 0 {
            s.history = None;
            self.dirty = true;
            return Ok(());
        }
        let (h, back) = (i32::from(s.size.0), i32::from(s.back));
        // how much history there is now, with the lines: the wheel stops at its top
        link.send_all(&[&["display-message", "-p", "-t", &p.pane, "#{history_size}"], &["capture-pane", "-p", "-e", "-N", "-S", &(-back).to_string(), "-E", &(h - 1 - back).to_string(), "-t", &p.pane]])?;
        self.waiting.extend([("hsize", p.pane.clone()), ("history", p.pane.clone())]);
        Ok(())
    }

    fn draw(&mut self, f: &mut Frame) {
        let blue = Color::from_u32(theme::CURRENT);
        let (foam, muted, hull) = (Color::from_u32(theme::FOAM), Color::from_u32(theme::MUTED), Color::from_u32(theme::HULL));
        let area = f.area();
        let shown: Vec<Place> = self.places.iter().filter(|p| p.shown && p.popup.is_none()).cloned().collect();
        for p in &shown {
            let ground = if p.mark == "__prs" { (foam, hull) } else { (Color::Reset, Color::Reset) };
            let r = self.rect(p);
            if let Some(scr) = self.screens.get(&p.pane) && let Some(s) = scr.shown() {
                paint(s.screen(), f.buffer_mut(), r, ground);
                if scr.back > 0 {
                    let badge = format!(" ↑ {} ", scr.back);
                    let x = (r.x + r.width).saturating_sub(Span::raw(&badge).width() as u16);
                    f.buffer_mut().set_string(x, r.y, &badge, Style::new().fg(Color::from_u32(theme::ABYSS)).bg(Color::from_u32(theme::SIGNAL)));
                }
            }
            if let Some(sel) = &self.selection && sel.pane == p.pane {
                for row in 0..r.height {
                    for col in 0..r.width {
                        if sel.holds(row, col) && let Some(c) = f.buffer_mut().cell_mut((r.x + col, r.y + row)) {
                            c.modifier.insert(Modifier::REVERSED);
                        }
                    }
                }
            }
        }
        // the frame of the terminal: its sides are on the last column of the sidebar and the first of the rail
        let side = shown.iter().find(|p| p.mark == "__sidebar").map(|p| p.at.0 + p.at.2 - 1);
        let rail = shown.iter().find(|p| p.mark == "__prs").map(|p| p.at.0);
        let terminal = shown.iter().find(|p| p.mark != "__sidebar" && p.mark != "__prs");
        let buf = f.buffer_mut();
        let put = |buf: &mut Buffer, x: u16, y: u16, s: &str, st: Style| { if let Some(c) = buf.cell_mut((x, y)) { c.set_symbol(s).set_style(st); } };
        // the accent while the terminal has the focus
        let focused = terminal.is_some_and(|t| t.active);
        let line = Style::new().fg(if focused { blue } else { Color::from_u32(theme::LINE) });
        let (left, right) = (side.unwrap_or(0), rail.unwrap_or(area.width.saturating_sub(1)));
        let bottom = area.height.saturating_sub(1);
        for x in left..=right {
            put(buf, x, 0, "─", line);
            put(buf, x, bottom, "─", line);
        }
        for (x, y, s) in [(left, 0, "╭"), (right, 0, "╮"), (left, bottom, "╰"), (right, bottom, "╯")] {
            put(buf, x, y, s, line);
        }
        // the sides over the lines that the sidebar and the rail draw, in the color of the rest
        for y in 1..bottom {
            put(buf, left, y, "│", line);
            put(buf, right, y, "│", line);
        }
        // the logo over the sidebar, the tabs on the frame, the title of the rail
        for (x, s, st) in [(2, "⢎", Style::new().fg(blue)), (3, "⡱", Style::new().fg(foam)), (5, "orcux", Style::new().fg(foam).bold())] {
            if x < left { buf.set_string(x, 0, s, st); }
        }
        // under the frame, at the left: what waits and what works is in the sidebar; nothing to draw there
        self.spots.clear();
        if let Some(t) = terminal && !t.mark.is_empty() {
            let mut x = t.at.0 + 1;
            for p in &self.tabs {
                let name = format!(" {} {} ", p.tab, tui::program(&p.cmd));
                let w = Span::raw(&name).width() as u16;
                // no room left before the corner: say that there are more
                if x + w + 6 > right {
                    buf.set_string(x, 0, " … ", Style::new().fg(muted));
                    x += 3;
                    break;
                }
                let start = x;
                if p.pane == t.pane {
                    buf.set_string(x, 0, "┤", line);
                    buf.set_string(x + 1, 0, &name, Style::new().fg(foam).bold());
                    buf.set_string(x + 1 + w, 0, "├", line);
                    x += w + 2;
                } else {
                    buf.set_string(x, 0, &name, Style::new().fg(muted));
                    x += w;
                }
                self.spots.push((start, x, Spot::Tab(p.tab)));
                x += 1;
            }
            buf.set_string(x, 0, " + ", Style::new().fg(muted));
            self.spots.push((x, x + 3, Spot::NewTab));
        }
        if let Some(r) = rail {
            buf.set_string(r + 1, 0, format!("  {}  ", tr().your_prs), Style::new().fg(foam).bg(hull).bold());
        }
        // a popup over everything, in a card with a rounded outline
        let popup = self.places.iter().find(|p| p.popup.is_some()).cloned();
        if let Some(p) = &popup {
            let r = self.rect(p);
            let outline = Rect { x: r.x.saturating_sub(2), y: r.y.saturating_sub(1), width: r.width + 4, height: r.height + 2 }.intersection(area);
            f.render_widget(ratatui::widgets::Clear, outline);
            let title = format!(" {} ", p.popup.clone().unwrap_or_default());
            f.render_widget(ratatui::widgets::Block::bordered().border_type(ratatui::widgets::BorderType::Rounded)
                .border_style(Style::new().fg(Color::from_u32(theme::LINE)).bg(hull)).style(Style::new().bg(hull))
                .title(Span::styled(title, Style::new().fg(foam))), outline);
            if let Some(s) = self.screens.get(&p.pane).and_then(|s| s.parser.as_ref()) {
                paint(s.screen(), f.buffer_mut(), r, (foam, hull));
            }
        }
        // the cursor of the pane that gets the keys
        if let Some(p) = self.target() && let Some(scr) = self.screens.get(&p.pane) && scr.back == 0 && let Some(s) = &scr.parser && !s.screen().hide_cursor() {
            let (r, c) = s.screen().cursor_position();
            let at = self.rect(p);
            f.set_cursor_position((at.x + c, at.y + r));
        }
    }
}

/// Whether `orcux window` is the person's interface now: its client is attached. Its name and size.
pub fn client() -> Option<(String, u32, u32)> {
    let name = tmux(&["show-option", "-gqv", CLIENT]).ok().filter(|n| !n.is_empty())?;
    let clients = tmux(&["list-clients", "-t", UI, "-F", "#{client_name} #{client_width} #{client_height}"]).ok()?;
    clients.lines().find_map(|l| {
        let [n, w, h] = l.split(' ').collect::<Vec<_>>()[..] else { return None };
        // a control client has no height of its own
        (n == name).then(|| (name.clone(), w.parse().unwrap_or(0), h.parse().unwrap_or(0)))
    })
}

/// A popup of `orcux window`: the dialog runs in a window of its own in the session, which the window draws over
/// everything and gives the keys to. Blocks until the dialog ends.
pub fn popup(title: &str, (w, h): (u32, u32), args: &[&str]) -> Res<()> {
    let exe = env::current_exe().map_err(|e| e.to_string())?;
    let cmd = std::iter::once(exe.to_string_lossy().to_string()).chain(args.iter().map(|a| a.to_string())).map(|a| config::sh(&a)).collect::<Vec<_>>().join(" ");
    let mut a = vec!["new-window".to_string(), "-d".into(), "-t".into(), format!("{UI}:"), "-P".into(), "-F".into(), "#{window_id} #{pane_id}".into()];
    for v in ["ORCUX_LANG", "LANG", "LC_ALL"] {
        if let Ok(x) = env::var(v) {
            a.extend(["-e".into(), format!("{v}={x}")]);
        }
    }
    a.push(cmd);
    let ids = tmux(&a.iter().map(String::as_str).collect::<Vec<_>>())?;
    let (window, pane) = ids.split_once(' ').ok_or(tr().no_pane)?;
    // the size of the interface: asked through the sidebar, whose window it is (the session would answer for the new one)
    let main = ui_pane(SIDEBAR).unwrap_or_else(|| UI.to_string());
    let (cw, ch) = tmux(&["display-message", "-p", "-t", &main, "#{window_width} #{window_height}"]).ok()
        .and_then(|s| { let (a, b) = s.split_once(' ')?; Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?)) }).unwrap_or((80, 24));
    let size = ((w - 2).min(cw.saturating_sub(6)), (h - 2).min(ch.saturating_sub(2)));
    for (k, v) in [("window-size", "manual".to_string()), ("@orcux_popup", title.to_string()), ("remain-on-exit", "off".to_string())] {
        tmux(&["set-option", "-w", "-t", window, k, &v]).ok();
    }
    tmux(&["set-option", "-p", "-t", pane, "@orcux", "__popup"]).ok();
    tmux(&["resize-window", "-t", window, "-x", &size.0.to_string(), "-y", &size.1.to_string()]).ok();
    // gone with its program. Looked for in the list: asked by its id, a pane that is gone may be answered for by another
    let alive = format!("{pane} 0");
    while tmux(&["list-panes", "-t", window, "-F", "#{pane_id} #{pane_dead}"]).is_ok_and(|l| l.lines().any(|p| p == alive)) {
        // nobody to answer it any more: it would wait for ever, and whoever opened it with it
        if client().is_none() {
            tmux(&["kill-window", "-t", window]).ok();
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_lines() {
        assert_eq!(unescape(b"a\\033[1mb\\134c\\015\\012"), b"a\x1b[1mb\\c\r\n");
        let mut block = None;
        assert!(matches!(read_line(b"%output %3 hi\\012", &mut block), Some(Msg::Output(p, b)) if p == "%3" && b == b"hi\n"));
        assert!(matches!(read_line(b"%extended-output %3 12 : x : y", &mut block), Some(Msg::Output(_, b)) if b == b"x : y"));
        assert!(read_line(b"%begin 1 2 1", &mut block).is_none());
        assert!(read_line(b"line \x1b[1mone", &mut block).is_none());
        assert!(read_line(b"%end 9 9 1", &mut block).is_none()); // another block's end is a line of this one
        assert!(matches!(read_line(b"%end 1 2 1", &mut block), Some(Msg::Reply(true, l)) if l.len() == 2));
        assert!(read_line(b"%begin 5 6 0", &mut block).is_none());
        assert!(read_line(b"%end 5 6 0", &mut block).is_none()); // the attach's own block is not an answer
        assert!(matches!(read_line(b"%pause %3", &mut block), Some(Msg::Paused(p)) if p == "%3"));
        assert!(matches!(read_line(b"%layout-change @1 abc", &mut block), Some(Msg::Layout)));
        assert!(matches!(read_line(b"%exit", &mut block), Some(Msg::Gone)));
    }

    #[test]
    fn keys_and_mouse() {
        let k = |code, m| key_name(&KeyEvent::new(code, m));
        assert_eq!(k(KeyCode::Char('a'), KeyModifiers::NONE), Some((true, "a".into())));
        assert_eq!(k(KeyCode::Char('c'), KeyModifiers::CONTROL), Some((false, "C-c".into())));
        assert_eq!(k(KeyCode::Up, KeyModifiers::SHIFT), Some((false, "S-Up".into())));
        assert_eq!(k(KeyCode::BackTab, KeyModifiers::SHIFT), Some((false, "BTab".into())));
        let c = |code, m| chord(&KeyEvent::new(code, m));
        assert_eq!(c(KeyCode::Char('w'), KeyModifiers::ALT), Some(Chord::Switch));
        assert_eq!(c(KeyCode::Char('∑'), KeyModifiers::NONE), Some(Chord::Switch));
        assert_eq!(c(KeyCode::Char('œ'), KeyModifiers::NONE), Some(Chord::CloseTab));
        assert_eq!(c(KeyCode::Char('w'), KeyModifiers::NONE), None);
        use vt100::{MouseProtocolEncoding as En, MouseProtocolMode as Mo};
        let m = MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: 9, row: 2, modifiers: KeyModifiers::NONE };
        assert_eq!(mouse_bytes(&m, 3, 1, Mo::PressRelease, En::Sgr), Some(b"\x1b[<0;4;2M".to_vec()));
        assert_eq!(mouse_bytes(&m, 3, 1, Mo::Press, En::Default), Some(vec![0x1b, b'[', b'M', 32, 36, 34]));
        let up = MouseEvent { kind: MouseEventKind::Up(MouseButton::Left), ..m };
        assert_eq!(mouse_bytes(&up, 0, 0, Mo::PressRelease, En::Sgr), Some(b"\x1b[<0;1;1m".to_vec()));
        assert_eq!(mouse_bytes(&up, 0, 0, Mo::PressRelease, En::Default), Some(vec![0x1b, b'[', b'M', 35, 33, 33]));
        assert_eq!(mouse_bytes(&up, 0, 0, Mo::Press, En::Sgr), None); // only presses asked for
        let drag = MouseEvent { kind: MouseEventKind::Drag(MouseButton::Left), ..m };
        assert_eq!(mouse_bytes(&drag, 0, 0, Mo::PressRelease, En::Sgr), None);
        assert_eq!(mouse_bytes(&drag, 0, 0, Mo::ButtonMotion, En::Sgr), Some(b"\x1b[<32;1;1M".to_vec()));
        let wheel = MouseEvent { kind: MouseEventKind::ScrollUp, ..m };
        assert_eq!(mouse_bytes(&wheel, 0, 0, Mo::Press, En::Sgr), Some(b"\x1b[<64;1;1M".to_vec()));
    }

    #[test]
    fn the_end_of_a_sequence_before_a_seed() {
        assert_eq!(after_seed(b"154;161;169;48;2;24;27;32m\xe2\x97\x8b MLDP"), "○ MLDP".as_bytes());
        assert_eq!(after_seed(b"[1;31mred"), b"red");
        assert_eq!(after_seed(b"a title\x07$ "), b"$ ");
        assert_eq!(after_seed(b"\x1b[1mbold"), b"\x1b[1mbold");
        // plain text stays
        assert_eq!(after_seed(b"hello world"), b"hello world");
        assert_eq!(after_seed(b"12 files\n"), b"12 files\n");
        assert_eq!(after_seed(b"line\nmore\x07"), b"line\nmore\x07");
    }

    #[test]
    fn copying() {
        assert_eq!(base64(b"hola"), "aG9sYQ==");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"abc"), "YWJj");
        let s = Selection { pane: "%1".into(), from: (2, 5), to: (1, 3) };
        assert_eq!(s.ordered(), ((1, 3), (2, 5)));
        assert!(s.holds(1, 70) && s.holds(2, 0) && !s.holds(2, 6) && !s.holds(1, 2));
    }

    #[test]
    fn screens() {
        assert_eq!(sync_state(b"\x1b[?2026hxx\x1b[?2026l"), Some(false));
        assert_eq!(sync_state(b"\x1b[?2026l..\x1b[?2026h"), Some(true));
        assert!(leaves_alternate(b"bye\x1b[?1049l"));
        let m = seeded(5, 10, &[], "0 0 1 0 1 0 0 1 1 3 0 0 1 120");
        assert_eq!((m.screen().mouse_protocol_mode(), m.screen().mouse_protocol_encoding()), (vt100::MouseProtocolMode::PressRelease, vt100::MouseProtocolEncoding::Sgr));
        let p = seeded(3, 10, &[b"\x1b[31mred".to_vec(), b"two".to_vec()], "4 1 1 0");
        assert_eq!(p.screen().contents(), "red\ntwo");
        assert_eq!(p.screen().cursor_position(), (1, 4));
        assert_eq!(p.screen().cell(0, 0).map(|c| c.fgcolor()), Some(vt100::Color::Idx(1)));
    }
}
