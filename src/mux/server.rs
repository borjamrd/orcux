//! The server of orcux's own engine: it owns the terminals (a pseudo-terminal and an emulator each) and answers the
//! commands that orcux sends, in tmux's words. One thread, one poll() over the listener, the clients and the terminals,
//! and a pipe for SIGCHLD: at rest it waits without a timer.
use super::{cmdline, format, keys, proto};
use std::{
    collections::{BTreeMap, HashMap},
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
        unix::net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

type Opts = BTreeMap<String, String>;
/// session, window and pane that a target names
type Target = (Option<u32>, Option<u32>, Option<u32>);

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Variables of whoever started the server that are not for its terminals: of tmux, of a worktree, or the marks of an
/// agent's own session (an agent started in a terminal would think it is a child of that one).
pub fn inherited_only(k: &str) -> bool {
    matches!(k, "TMUX" | "TMUX_PANE" | "ORCUX_MUX" | "ORCUX_ID" | "ORCUX_PORT" | "ORCUX_SLOT" | "ORCUX_NAME" | "CLAUDECODE")
        || (k.starts_with("CLAUDE_CODE_") && k != "CLAUDE_CODE_USE_BEDROCK" && k != "CLAUDE_CODE_USE_VERTEX")
}

/// What a terminal asks of its terminal (where is the cursor, what are you, what colors), answered as a terminal would.
#[derive(Default)]
struct Asks {
    title: String,
    replies: Vec<u8>,
}

impl vt100::Callbacks for Asks {
    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        self.title = String::from_utf8_lossy(title).chars().filter(|c| !c.is_control()).take(256).collect();
    }

    fn unhandled_csi(&mut self, screen: &mut vt100::Screen, i1: Option<u8>, _i2: Option<u8>, params: &[&[u16]], c: char) {
        let first = params.first().and_then(|p| p.first()).copied().unwrap_or(0);
        match (i1, c, first) {
            (None, 'n', 6) => {
                let (r, col) = screen.cursor_position();
                self.replies.extend(format!("\x1b[{};{}R", r + 1, col + 1).as_bytes());
            }
            (None, 'n', 5) => self.replies.extend(b"\x1b[0n"),
            (None, 'c', 0) => self.replies.extend(b"\x1b[?62;22c"),
            (Some(b'>'), 'c', 0) => self.replies.extend(b"\x1b[>41;0;0c"),
            _ => {}
        }
    }

    fn unhandled_osc(&mut self, _: &mut vt100::Screen, params: &[&[u8]]) {
        // the colors of the terminal: a dark one, as orcux's
        match params {
            [b"10", b"?"] => self.replies.extend(b"\x1b]10;rgb:e8e8/eaea/eded\x1b\\"),
            [b"11", b"?"] => self.replies.extend(b"\x1b]11;rgb:0f0f/1111/1414\x1b\\"),
            _ => {}
        }
    }
}

struct Pane {
    id: u32,
    window: u32,
    pid: i32,
    master: OwnedFd,
    parser: vt100::Parser<Asks>,
    options: Opts,
    width: u16,
    height: u16,
    /// what could not be written yet
    pending: Vec<u8>,
    /// where it started, and what it runs (nothing: a shell)
    dir: PathBuf,
    command: Vec<String>,
}

struct Window {
    id: u32,
    session: u32,
    name: String,
    /// left to right, each with its width; one column between them
    panes: Vec<u32>,
    widths: Vec<u16>,
    size: (u16, u16),
    active: u32,
    options: Opts,
    hooks: HashMap<String, String>,
    activity: u64,
}

struct Session {
    id: u32,
    name: String,
    windows: Vec<u32>,
    current: u32,
    options: Opts,
    created: u64,
}

struct Client {
    id: u32,
    stream: UnixStream,
    inbox: Vec<u8>,
    outbox: Vec<u8>,
    session: Option<u32>,
    /// size asked with refresh-client -C, and flags
    size: Option<(u16, u16)>,
    ignore_size: bool,
    no_output: bool,
    activity: u64,
    commands: u64,
    /// name, what, format, last value
    subscriptions: Vec<(String, String, String, Option<String>)>,
    closing: bool,
}

pub struct Server {
    socket: PathBuf,
    started: u64,
    next: u32,
    panes: BTreeMap<u32, Pane>,
    windows: BTreeMap<u32, Window>,
    sessions: BTreeMap<u32, Session>,
    clients: Vec<Client>,
    globals: Opts,
    env: BTreeMap<String, String>,
    buffers: HashMap<String, Vec<u8>>,
    /// the last time subscriptions were looked at
    looked: Instant,
    /// it had sessions: with none left, it ends, as tmux does
    had: bool,
    /// something changed since subscriptions were looked at: at rest they are not
    changed: bool,
}

fn hex(c: u32) -> String {
    format!("%{c}")
}

/// A command's result: what it printed, and whether it worked.
type Out = Result<String, String>;

/// The arguments of a command: flags with and without values, and the rest.
struct Args {
    flags: Vec<char>,
    values: HashMap<char, Vec<String>>,
    rest: Vec<String>,
}

impl Args {
    fn parse(words: &[String], with_value: &str) -> Args {
        let (mut flags, mut values, mut rest) = (vec![], HashMap::<char, Vec<String>>::new(), vec![]);
        let mut i = 0;
        while i < words.len() {
            let w = &words[i];
            if w == "--" {
                rest.extend(words[i + 1..].iter().cloned());
                break;
            }
            if w.len() > 1 && w.starts_with('-') && rest.is_empty() && !w[1..].starts_with(|c: char| c.is_ascii_digit()) {
                let cs: Vec<char> = w[1..].chars().collect();
                for (j, c) in cs.iter().enumerate() {
                    if with_value.contains(*c) {
                        let v = if j + 1 < cs.len() { cs[j + 1..].iter().collect() } else { i += 1; words.get(i).cloned().unwrap_or_default() };
                        values.entry(*c).or_default().push(v);
                        break;
                    }
                    flags.push(*c);
                }
            } else {
                rest.push(w.clone());
            }
            i += 1;
        }
        Args { flags, values, rest }
    }
    fn has(&self, c: char) -> bool {
        self.flags.contains(&c)
    }
    fn get(&self, c: char) -> Option<&str> {
        self.values.get(&c).and_then(|v| v.last()).map(String::as_str)
    }
    fn all(&self, c: char) -> Vec<String> {
        self.values.get(&c).cloned().unwrap_or_default()
    }
}

/// Where a format is evaluated.
struct Ctx<'a> {
    s: &'a Server,
    client: Option<u32>,
    session: Option<u32>,
    window: Option<u32>,
    pane: Option<u32>,
}

impl format::Scope for Ctx<'_> {
    fn get(&self, name: &str) -> Option<String> {
        let s = self.s;
        let pane = self.pane.and_then(|p| s.panes.get(&p));
        let window = self.window.and_then(|w| s.windows.get(&w));
        let session = self.session.and_then(|x| s.sessions.get(&x));
        let client = self.client.and_then(|c| s.clients.iter().find(|x| x.id == c));
        let b = |v: bool| if v { "1" } else { "0" }.to_string();
        if let Some(opt) = name.strip_prefix('@') {
            let key = format!("@{opt}");
            return pane.and_then(|p| p.options.get(&key)).or_else(|| window.and_then(|w| w.options.get(&key)))
                .or_else(|| session.and_then(|x| x.options.get(&key))).or_else(|| s.globals.get(&key)).cloned().or(Some(String::new()));
        }
        let screen = pane.map(|p| p.parser.screen());
        Some(match name {
            "pane_id" => hex(pane?.id),
            "pane_pid" => pane?.pid.to_string(),
            "pane_left" => s.left_of(pane?.id).to_string(),
            "pane_top" => "0".into(),
            "pane_width" => pane?.width.to_string(),
            "pane_height" => pane?.height.to_string(),
            "pane_active" => b(window.is_some_and(|w| w.active == pane.map_or(0, |p| p.id))),
            "pane_current_command" => command_of(pane?),
            "pane_title" => pane?.parser.callbacks().title.clone(),
            "pane_dead" => "0".into(),
            "pane_current_path" => cwd_of(pane?).display().to_string(),
            "cursor_x" => screen?.cursor_position().1.to_string(),
            "cursor_y" => screen?.cursor_position().0.to_string(),
            "cursor_flag" => b(!screen?.hide_cursor()),
            "alternate_on" => b(screen?.alternate_screen()),
            "mouse_standard_flag" => b(screen?.mouse_protocol_mode() != vt100::MouseProtocolMode::None),
            "mouse_button_flag" => b(matches!(screen?.mouse_protocol_mode(), vt100::MouseProtocolMode::ButtonMotion)),
            "mouse_any_flag" => b(matches!(screen?.mouse_protocol_mode(), vt100::MouseProtocolMode::AnyMotion)),
            "mouse_sgr_flag" => b(screen?.mouse_protocol_encoding() == vt100::MouseProtocolEncoding::Sgr),
            "scroll_region_upper" | "origin_flag" | "insert_flag" => "0".into(),
            "scroll_region_lower" => pane?.height.saturating_sub(1).to_string(),
            "wrap_flag" => "1".into(),
            "history_size" => screen?.scrollback_len().to_string(),
            "history_bytes" => (screen?.scrollback_len() * pane?.width as usize).to_string(),
            "window_id" => format!("@{}", window?.id),
            "window_name" => window?.name.clone(),
            "window_active" => b(session.is_some_and(|x| x.current == window.map_or(0, |w| w.id))),
            "window_width" => window?.size.0.to_string(),
            "window_height" => window?.size.1.to_string(),
            "window_activity" => window?.activity.to_string(),
            "window_panes" => window?.panes.len().to_string(),
            "session_name" => session?.name.clone(),
            "session_id" => format!("${}", session?.id),
            "session_windows" => session?.windows.len().to_string(),
            "session_attached" => s.clients.iter().filter(|c| c.session == session.map(|x| x.id)).count().to_string(),
            "session_created" => session?.created.to_string(),
            "client_name" => format!("client-{}", client?.id),
            "client_width" => client?.size.map(|x| x.0.to_string()).unwrap_or_default(),
            "client_height" => String::new(),
            "client_control_mode" => "1".into(),
            "client_activity" => client?.activity.to_string(),
            "pid" => std::process::id().to_string(),
            "start_time" => s.started.to_string(),
            "socket_path" => s.socket.display().to_string(),
            "version" => "orcux".into(),
            _ => String::new(),
        })
    }

    fn each(&self, kind: char) -> Vec<Box<dyn format::Scope + '_>> {
        let s = self.s;
        let ctx = |session: Option<u32>, window: Option<u32>, pane: Option<u32>| -> Box<dyn format::Scope + '_> {
            Box::new(Ctx { s, client: self.client, session, window, pane })
        };
        match kind {
            'S' => s.sessions.values().map(|x| ctx(Some(x.id), Some(x.current), s.windows.get(&x.current).map(|w| w.active))).collect(),
            'W' => self.session.and_then(|x| s.sessions.get(&x)).map(|x| x.windows.iter().map(|w| ctx(Some(x.id), Some(*w), s.windows.get(w).map(|w| w.active))).collect()).unwrap_or_default(),
            'P' => self.window.and_then(|w| s.windows.get(&w)).map(|w| w.panes.iter().map(|p| ctx(Some(w.session), Some(w.id), Some(*p))).collect()).unwrap_or_default(),
            _ => vec![],
        }
    }
}

/// Where what runs in front works now; where it started, if that cannot be known.
fn cwd_of(p: &Pane) -> PathBuf {
    let pgid = unsafe { libc::tcgetpgrp(p.master.as_raw_fd()) };
    let pid = if pgid > 0 { pgid } else { p.pid };
    #[cfg(not(target_os = "macos"))]
    if let Ok(d) = std::fs::read_link(format!("/proc/{pid}/cwd")) {
        return d;
    }
    let _ = pid;
    p.dir.clone()
}

/// The name of what runs in front in a terminal: the process group that has it.
fn command_of(p: &Pane) -> String {
    let pgid = unsafe { libc::tcgetpgrp(p.master.as_raw_fd()) };
    let pid = if pgid > 0 { pgid } else { p.pid };
    name_of(pid).unwrap_or_default()
}

#[cfg(target_os = "macos")]
fn name_of(pid: i32) -> Option<String> {
    let mut buf = [0u8; 256];
    let n = unsafe { libc::proc_name(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    (n > 0).then(|| String::from_utf8_lossy(&buf[..n as usize]).into_owned())
}

#[cfg(not(target_os = "macos"))]
fn name_of(pid: i32) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/comm")).ok().map(|s| s.trim().to_string())
}

/// Rows of a screen as text, one per line. -N keeps the spaces at their end; -J joins a row that the terminal wrapped
/// with the next one, as one line, as it was printed.
fn plain_rows(screen: &vt100::Screen, row: u16, take: u16, cols: u16, keep: bool, join: bool) -> Vec<u8> {
    let mut out = vec![];
    for (i, r) in screen.rows(0, cols).enumerate().skip(row as usize).take(take as usize) {
        let wrapped = join && screen.row_wrapped(i as u16);
        out.extend(if keep || wrapped { r } else { r.trim_end().to_string() }.as_bytes());
        if !wrapped {
            out.push(b'\n');
        }
    }
    out
}

/// tmux escapes what is not printable, and the backslash, as octal.
#[cfg(test)]
fn escape(b: &[u8]) -> String {
    let mut out = String::with_capacity(b.len());
    for &c in b {
        if c < 0x20 || c == b'\\' || c == 0x7f {
            out += &format!("\\{c:03o}");
        } else {
            out.push(c as char);
        }
    }
    // non-ASCII bytes go as they are: the line is bytes
    out
}

fn escape_bytes(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len());
    for &c in b {
        if c < 0x20 || c == b'\\' || c == 0x7f {
            out.extend(format!("\\{c:03o}").as_bytes());
        } else {
            out.push(c);
        }
    }
    out
}

static mut CHILD_PIPE: RawFd = -1;

extern "C" fn on_child(_: libc::c_int) {
    unsafe {
        let fd = CHILD_PIPE;
        if fd >= 0 {
            libc::write(fd, b"c".as_ptr().cast(), 1);
        }
    }
}

impl Server {
    pub fn new(socket: &Path) -> Server {
        let env: BTreeMap<String, String> = std::env::vars().filter(|(k, _)| !inherited_only(k)).collect();
        Server {
            socket: socket.into(), started: now(), next: 0, panes: BTreeMap::new(), windows: BTreeMap::new(), sessions: BTreeMap::new(),
            clients: vec![], globals: Opts::new(), env, buffers: HashMap::new(), looked: Instant::now(), had: false, changed: true,
        }
    }

    fn id(&mut self) -> u32 {
        self.next += 1;
        self.next
    }

    fn left_of(&self, pane: u32) -> u16 {
        let Some(w) = self.panes.get(&pane).and_then(|p| self.windows.get(&p.window)) else { return 0 };
        let mut x = 0;
        for (p, width) in w.panes.iter().zip(&w.widths) {
            if *p == pane {
                return x;
            }
            x += width + 1;
        }
        x
    }

    pub fn run(mut self, listener: UnixListener) {
        listener.set_nonblocking(true).ok();
        let mut fds = [0; 2];
        unsafe {
            libc::pipe(fds.as_mut_ptr());
            for fd in fds {
                libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK);
                libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
            }
            CHILD_PIPE = fds[1];
            libc::signal(libc::SIGCHLD, on_child as *const () as libc::sighandler_t);
            libc::signal(libc::SIGPIPE, libc::SIG_IGN);
            libc::signal(libc::SIGHUP, libc::SIG_IGN);
        }
        let children = fds[0];
        loop {
            // what to watch: the listener, the child pipe, each client and each terminal
            let mut polls = vec![
                libc::pollfd { fd: listener.as_raw_fd(), events: libc::POLLIN, revents: 0 },
                libc::pollfd { fd: children, events: libc::POLLIN, revents: 0 },
            ];
            let clients: Vec<u32> = self.clients.iter().map(|c| c.id).collect();
            for c in &self.clients {
                let out = if c.outbox.is_empty() { 0 } else { libc::POLLOUT };
                polls.push(libc::pollfd { fd: c.stream.as_raw_fd(), events: libc::POLLIN | out, revents: 0 });
            }
            let panes: Vec<u32> = self.panes.keys().copied().collect();
            for p in self.panes.values() {
                let out = if p.pending.is_empty() { 0 } else { libc::POLLOUT };
                polls.push(libc::pollfd { fd: p.master.as_raw_fd(), events: libc::POLLIN | out, revents: 0 });
            }
            // subscriptions are looked at once a second at most, and only if there are any
            let subs = self.changed && self.clients.iter().any(|c| !c.subscriptions.is_empty());
            let timeout = if subs { 1000i32.saturating_sub(self.looked.elapsed().as_millis() as i32).max(0) } else if self.had { -1 } else { 1000 };
            let n = unsafe { libc::poll(polls.as_mut_ptr(), polls.len() as libc::nfds_t, timeout) };
            if n < 0 {
                continue; // EINTR
            }
            if polls[1].revents != 0 {
                let mut buf = [0u8; 64];
                while unsafe { libc::read(children, buf.as_mut_ptr().cast(), buf.len()) } > 0 {}
                self.reap();
            }
            if polls[0].revents != 0 {
                while let Ok((stream, _)) = listener.accept() {
                    stream.set_nonblocking(true).ok();
                    let id = self.id();
                    self.clients.push(Client {
                        id, stream, inbox: vec![], outbox: vec![], session: None, size: None, ignore_size: false, no_output: false,
                        activity: now(), commands: 0, subscriptions: vec![], closing: false,
                    });
                }
            }
            for (i, id) in clients.iter().enumerate() {
                let r = polls[2 + i].revents;
                if r & libc::POLLOUT != 0 {
                    self.flush(*id);
                }
                if r & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
                    self.read_client(*id);
                }
            }
            let base = 2 + clients.len();
            for (i, id) in panes.iter().enumerate() {
                let r = polls[base + i].revents;
                if r & libc::POLLOUT != 0 {
                    self.write_pane(*id, &[]);
                }
                if r & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
                    self.read_pane(*id);
                }
            }
            if subs && self.looked.elapsed().as_millis() >= 1000 {
                self.looked = Instant::now();
                self.subscriptions();
            }
            self.clients.retain(|c| !(c.closing && c.outbox.is_empty()));
            for c in &mut self.clients {
                if c.closing && !c.outbox.is_empty() {
                    let _ = c.stream.write(&c.outbox).map(|n| { c.outbox.drain(..n); });
                    if c.outbox.is_empty() {
                        let _ = c.stream.shutdown(std::net::Shutdown::Both);
                    }
                }
            }
            self.had |= !self.sessions.is_empty();
            // nothing left, or started for nothing
            let idle = self.sessions.is_empty() && self.panes.is_empty() && self.clients.is_empty();
            if idle && (self.had || now().saturating_sub(self.started) > 5) {
                std::fs::remove_file(&self.socket).ok();
                return;
            }
        }
    }

    fn reap(&mut self) {
        loop {
            let mut status = 0;
            let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
            if pid <= 0 {
                break;
            }
            if let Some(id) = self.panes.values().find(|p| p.pid == pid).map(|p| p.id) {
                self.read_pane(id); // what it printed last
                self.close_pane(id);
            }
        }
    }

    fn read_pane(&mut self, id: u32) {
        let Some(p) = self.panes.get_mut(&id) else { return };
        let mut buf = [0u8; 65536];
        let mut got = vec![];
        loop {
            let n = unsafe { libc::read(p.master.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
            if n <= 0 {
                break;
            }
            got.extend_from_slice(&buf[..n as usize]);
            if got.len() > 1 << 20 {
                break;
            }
        }
        if got.is_empty() {
            return;
        }
        p.parser.process(&got);
        self.changed = true;
        let replies = std::mem::take(&mut p.parser.callbacks_mut().replies);
        let window = p.window;
        if !replies.is_empty() {
            self.write_pane(id, &replies);
        }
        if let Some(w) = self.windows.get_mut(&window) {
            w.activity = now();
        }
        let session = self.windows.get(&window).map(|w| w.session);
        let mut line = format!("%output {} ", hex(id)).into_bytes();
        line.extend(escape_bytes(&got));
        line.push(b'\n');
        for c in self.clients.iter_mut().filter(|c| c.session == session && !c.no_output && !c.closing) {
            c.outbox.extend_from_slice(&line);
        }
    }

    fn write_pane(&mut self, id: u32, bytes: &[u8]) {
        let Some(p) = self.panes.get_mut(&id) else { return };
        p.pending.extend_from_slice(bytes);
        while !p.pending.is_empty() {
            let n = unsafe { libc::write(p.master.as_raw_fd(), p.pending.as_ptr().cast(), p.pending.len()) };
            if n <= 0 {
                break;
            }
            p.pending.drain(..n as usize);
        }
    }

    fn flush(&mut self, id: u32) {
        let Some(c) = self.clients.iter_mut().find(|c| c.id == id) else { return };
        while !c.outbox.is_empty() {
            match c.stream.write(&c.outbox) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    c.outbox.drain(..n);
                }
            }
        }
    }

    fn read_client(&mut self, id: u32) {
        let Some(c) = self.clients.iter_mut().find(|c| c.id == id) else { return };
        let mut buf = [0u8; 65536];
        let mut gone = false;
        loop {
            match c.stream.read(&mut buf) {
                Ok(0) => {
                    gone = true;
                    break;
                }
                Ok(n) => c.inbox.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => {
                    gone = true;
                    break;
                }
            }
        }
        if c.session.is_none() {
            // a single command, or the start of a control client
            if let Some((req, used)) = proto::Request::decode(&c.inbox) {
                c.inbox.drain(..used);
                self.request(id, req);
            }
        }
        // a control client: its lines, also those that came with its request
        if self.clients.iter().any(|c| c.id == id && c.session.is_some()) {
            // control mode: one command line at a time
            while let Some(nl) = self.clients.iter().find(|c| c.id == id).and_then(|c| c.inbox.iter().position(|b| *b == b'\n')) {
                let line = {
                    let c = self.clients.iter_mut().find(|c| c.id == id).unwrap_or_else(|| unreachable!());
                    let line: Vec<u8> = c.inbox.drain(..=nl).collect();
                    c.activity = now();
                    String::from_utf8_lossy(&line[..line.len() - 1]).trim_end_matches('\r').to_string()
                };
                if line.is_empty() {
                    self.detach(id); // an empty line detaches, as in tmux
                    return;
                }
                for cmd in cmdline::parse(&line) {
                    let out = self.exec(&cmd, Some(id), None);
                    self.block(id, out);
                }
            }
        }
        if gone && let Some(c) = self.clients.iter_mut().find(|c| c.id == id) {
            c.closing = true;
            c.outbox.clear();
        }
    }

    /// The reply of a control client's command, as a block.
    fn block(&mut self, id: u32, out: Out) {
        let Some(c) = self.clients.iter_mut().find(|c| c.id == id) else { return };
        c.commands += 1;
        let head = format!("{} {} 1", now(), c.commands);
        let (word, text) = match &out {
            Ok(t) => ("%end", t.clone()),
            Err(e) => ("%error", e.clone()),
        };
        c.outbox.extend(format!("%begin {head}\n").as_bytes());
        if !text.is_empty() {
            c.outbox.extend(text.as_bytes());
            if !text.ends_with('\n') {
                c.outbox.push(b'\n');
            }
        }
        c.outbox.extend(format!("{word} {head}\n").as_bytes());
    }

    fn request(&mut self, id: u32, req: proto::Request) {
        if req.attach {
            // a control client: attach-session -t target -f flags
            let a = Args::parse(&req.argv, "tfc");
            let target = a.get('t').unwrap_or("").to_string();
            let session = self.resolve(&target, Some(id), req.caller.as_deref()).ok().and_then(|t| t.0);
            let Some(c) = self.clients.iter_mut().find(|c| c.id == id) else { return };
            let Some(session) = session else {
                c.outbox.extend(b"%exit no session\n");
                c.closing = true;
                return;
            };
            c.session = Some(session);
            for f in a.get('f').unwrap_or("").split(',') {
                match f {
                    "ignore-size" => c.ignore_size = true,
                    "no-output" => c.no_output = true,
                    _ => {}
                }
            }
            c.outbox.extend(format!("%begin {} 0 0\n%end {} 0 0\n", now(), now()).as_bytes());
            return;
        }
        let mut out = String::new();
        let mut ok = true;
        for cmd in &req.commands() {
            match self.exec(cmd, None, req.caller.as_deref()) {
                Ok(t) => out += &t,
                Err(e) => {
                    ok = false;
                    out = e;
                    break;
                }
            }
        }
        if let Some(c) = self.clients.iter_mut().find(|c| c.id == id) {
            c.outbox.extend(proto::reply(ok, &out));
            c.closing = true;
        }
    }

    fn detach(&mut self, id: u32) {
        if let Some(c) = self.clients.iter_mut().find(|c| c.id == id) {
            c.outbox.extend(b"%exit\n");
            c.closing = true;
        }
        self.sizes();
    }

    /// session, window, pane of a target; the caller's pane, or the client's session, when there is none.
    fn resolve(&self, t: &str, client: Option<u32>, caller: Option<&str>) -> Result<Target, String> {
        let of_pane = |p: u32| -> Target {
            let w = self.panes.get(&p).map(|p| p.window);
            (w.and_then(|w| self.windows.get(&w)).map(|w| w.session), w, Some(p))
        };
        let of_window = |w: u32| -> Target {
            let win = self.windows.get(&w);
            (win.map(|w| w.session), Some(w), win.map(|w| w.active))
        };
        if t.is_empty() {
            if let Some(p) = caller.and_then(|c| c.strip_prefix('%')).and_then(|n| n.parse().ok()) && self.panes.contains_key(&p) {
                return Ok(of_pane(p));
            }
            if let Some(s) = client.and_then(|c| self.clients.iter().find(|x| x.id == c)).and_then(|c| c.session) {
                return Ok(self.sessions.get(&s).map_or((Some(s), None, None), |x| of_window(x.current)));
            }
            return self.sessions.values().last().map(|x| of_window(x.current)).ok_or_else(|| "no current session".into());
        }
        if let Some(n) = t.strip_prefix('%') {
            let p: u32 = n.parse().map_err(|_| format!("can't find pane: {t}"))?;
            return if self.panes.contains_key(&p) { Ok(of_pane(p)) } else { Err(format!("can't find pane: {t}")) };
        }
        if let Some(n) = t.strip_prefix('@') {
            let w: u32 = n.trim_end_matches(':').parse().map_err(|_| format!("can't find window: {t}"))?;
            return if self.windows.contains_key(&w) { Ok(of_window(w)) } else { Err(format!("can't find window: {t}")) };
        }
        let (session, rest) = t.split_once(':').unwrap_or((t, ""));
        let name = session.strip_prefix('=').unwrap_or(session);
        let exact = session.starts_with('=');
        let s = self.sessions.values().find(|x| x.name == name).or_else(|| (!exact).then(|| self.sessions.values().find(|x| x.name.starts_with(name))).flatten())
            .ok_or_else(|| format!("can't find session: {name}"))?;
        if rest.is_empty() {
            return Ok(of_window(s.current));
        }
        let w = rest.strip_prefix('@').and_then(|n| n.parse().ok()).or_else(|| rest.parse::<usize>().ok().and_then(|i| s.windows.get(i).copied()))
            .ok_or_else(|| format!("can't find window: {rest}"))?;
        Ok(of_window(w))
    }

    fn ctx(&self, t: Target, client: Option<u32>) -> Ctx<'_> {
        Ctx { s: self, client, session: t.0, window: t.1, pane: t.2 }
    }

    fn spawn(&mut self, window: u32, a: &Args, size: (u16, u16)) -> Result<u32, String> {
        let id = self.id();
        let (cols, rows) = (size.0.max(2), size.1.max(2));
        let mut ws = libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 };
        let (mut master, mut slave) = (0, 0);
        // *const on Linux, *mut on macOS: a raw pointer cast fits both
        if unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null_mut(), &raw mut ws as _) } != 0 {
            return Err(format!("openpty: {}", std::io::Error::last_os_error()));
        }
        unsafe {
            libc::fcntl(master, libc::F_SETFL, libc::O_NONBLOCK);
            libc::fcntl(master, libc::F_SETFD, libc::FD_CLOEXEC);
        }
        let master = unsafe { OwnedFd::from_raw_fd(master) };
        // tmux expands formats in the folder and the name: "##" is a "#"
        let plain = |s: &str| format::expand(s, &Ctx { s: self, client: None, session: None, window: None, pane: None });
        let dir = a.get('c').map(|c| PathBuf::from(plain(c))).filter(|d| d.is_dir()).unwrap_or_else(|| PathBuf::from(self.env.get("HOME").cloned().unwrap_or("/".into())));
        let shell = self.env.get("SHELL").cloned().unwrap_or("/bin/sh".into());
        let line = a.rest.join(" ");
        let mut env = self.env.clone();
        for e in a.all('e') {
            if let Some((k, v)) = e.split_once('=') {
                env.insert(k.into(), v.into());
            }
        }
        env.insert("TMUX_PANE".into(), hex(id));
        env.insert("ORCUX_MUX".into(), self.socket.display().to_string());
        env.insert("TERM".into(), "xterm-256color".into());
        env.insert("COLORTERM".into(), "truecolor".into());
        let mut cmd = if line.is_empty() {
            // a login shell, as a terminal starts it
            let mut c = std::process::Command::new(&shell);
            let name = Path::new(&shell).file_name().map(|n| format!("-{}", n.to_string_lossy())).unwrap_or("-sh".into());
            std::os::unix::process::CommandExt::arg0(&mut c, name);
            c
        } else {
            let mut c = std::process::Command::new("/bin/sh");
            c.arg("-c").arg(&line);
            c
        };
        cmd.env_clear().envs(&env).current_dir(&dir);
        let slave_fd = slave;
        unsafe {
            use std::os::unix::process::CommandExt;
            cmd.pre_exec(move || {
                if libc::setsid() < 0 || libc::ioctl(slave_fd, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                for fd in 0..3 {
                    libc::dup2(slave_fd, fd);
                }
                if slave_fd > 2 {
                    libc::close(slave_fd);
                }
                for sig in [libc::SIGCHLD, libc::SIGPIPE, libc::SIGHUP, libc::SIGINT, libc::SIGQUIT, libc::SIGTERM] {
                    libc::signal(sig, libc::SIG_DFL);
                }
                let empty: libc::sigset_t = std::mem::zeroed();
                libc::sigprocmask(libc::SIG_SETMASK, &empty, std::ptr::null_mut());
                Ok(())
            });
        }
        let child = cmd.spawn();
        unsafe { libc::close(slave) };
        let child = child.map_err(|e| format!("{}: {e}", if line.is_empty() { shell.clone() } else { line.clone() }))?;
        let nz = |n: u16| std::num::NonZeroU16::new(n).unwrap_or(std::num::NonZeroU16::MIN);
        let parser = vt100::Parser::new_with_callbacks(nz(rows), nz(cols), 2000, Asks::default());
        self.panes.insert(id, Pane { id, window, pid: child.id() as i32, master, parser, options: Opts::new(), width: cols, height: rows, pending: vec![], dir, command: a.rest.clone() });
        std::mem::forget(child); // reaped by SIGCHLD
        Ok(id)
    }

    fn resize_pane(&mut self, id: u32, w: u16, h: u16) {
        let Some(p) = self.panes.get_mut(&id) else { return };
        if (p.width, p.height) == (w, h) {
            return;
        }
        (p.width, p.height) = (w.max(1), h.max(1));
        let ws = libc::winsize { ws_row: p.height, ws_col: p.width, ws_xpixel: 0, ws_ypixel: 0 };
        unsafe { libc::ioctl(p.master.as_raw_fd(), libc::TIOCSWINSZ, &ws) };
        let nz = |n: u16| std::num::NonZeroU16::new(n).unwrap_or(std::num::NonZeroU16::MIN);
        p.parser.screen_mut().set_size(nz(p.height), nz(p.width));
    }

    /// Gives each pane of a window its width (the widest takes what changes) and tells the panes and the clients.
    fn layout(&mut self, window: u32) {
        let Some(w) = self.windows.get_mut(&window) else { return };
        let gaps = w.panes.len().saturating_sub(1) as u16;
        let total: u16 = w.widths.iter().sum::<u16>();
        let room = w.size.0.saturating_sub(gaps).max(w.widths.len() as u16);
        if total != room && !w.widths.is_empty() {
            // in proportion, as tmux does: a hook (window-resized) pins the panes that must not change
            let mut given = 0u16;
            let n = w.widths.len();
            for (i, width) in w.widths.iter_mut().enumerate() {
                *width = if i + 1 == n { room.saturating_sub(given).max(1) } else { ((*width as u32 * room as u32 + total as u32 / 2) / total.max(1) as u32).max(1) as u16 };
                given += *width;
            }
        }
        let (h, list): (u16, Vec<(u32, u16)>) = (w.size.1, w.panes.iter().copied().zip(w.widths.iter().copied()).collect());
        for (p, width) in list {
            self.resize_pane(p, width, h);
        }
        self.notify_window(window, &format!("%layout-change @{window} x x x\n"));
    }

    fn notify_window(&mut self, window: u32, line: &str) {
        let session = self.windows.get(&window).map(|w| w.session);
        for c in self.clients.iter_mut().filter(|c| c.session.is_some() && c.session == session && !c.closing) {
            c.outbox.extend(line.as_bytes());
        }
    }

    fn notify_session(&mut self, session: u32, line: &str) {
        for c in self.clients.iter_mut().filter(|c| c.session == Some(session) && !c.closing) {
            c.outbox.extend(line.as_bytes());
        }
    }

    /// The size of each session's windows: the latest client that gives one, unless the window was sized by hand.
    fn sizes(&mut self) {
        let mut want: HashMap<u32, (u16, u16)> = HashMap::new();
        let mut by: Vec<&Client> = self.clients.iter().filter(|c| !c.ignore_size && !c.closing && c.size.is_some()).collect();
        by.sort_by_key(|c| c.activity);
        for c in by {
            if let (Some(s), Some(size)) = (c.session, c.size) {
                want.insert(s, size);
            }
        }
        let changed: Vec<u32> = self.windows.values_mut().filter_map(|w| {
            let size = *want.get(&w.session)?;
            (w.options.get("window-size").map(String::as_str) != Some("manual") && w.size != size).then(|| {
                w.size = size;
                w.id
            })
        }).collect();
        for w in changed {
            self.layout(w);
            self.hook(w, "window-resized");
        }
    }

    fn hook(&mut self, window: u32, name: &str) {
        let Some(line) = self.windows.get(&window).and_then(|w| w.hooks.get(name)).cloned() else { return };
        let target = self.windows.get(&window).map(|w| hex(w.active));
        for cmd in cmdline::parse(&line) {
            let _ = self.exec(&cmd, None, target.as_deref());
        }
    }

    fn close_pane(&mut self, id: u32) {
        let Some(p) = self.panes.remove(&id) else { return };
        unsafe { libc::kill(-p.pid, libc::SIGHUP) };
        let window = p.window;
        let Some(w) = self.windows.get_mut(&window) else { return };
        if let Some(i) = w.panes.iter().position(|x| *x == id) {
            w.panes.remove(i);
            let freed = w.widths.remove(i) + 1;
            if !w.widths.is_empty() {
                let j = i.min(w.widths.len() - 1);
                w.widths[j] += freed;
            }
            if w.active == id {
                w.active = *w.panes.get(i.min(w.panes.len().saturating_sub(1))).unwrap_or(&0);
            }
        }
        if w.panes.is_empty() {
            self.close_window(window);
        } else {
            self.layout(window);
            let active = self.windows.get(&window).map_or(0, |w| w.active);
            self.notify_window(window, &format!("%window-pane-changed @{window} {}\n", hex(active)));
        }
    }

    fn close_window(&mut self, window: u32) {
        let Some(w) = self.windows.remove(&window) else { return };
        for p in w.panes.clone() {
            if let Some(p) = self.panes.remove(&p) {
                unsafe { libc::kill(-p.pid, libc::SIGHUP) };
            }
        }
        self.notify_session(w.session, &format!("%window-close @{window}\n"));
        let Some(s) = self.sessions.get_mut(&w.session) else { return };
        s.windows.retain(|x| *x != window);
        if s.current == window {
            s.current = *s.windows.last().unwrap_or(&0);
            let (sid, cur) = (s.id, s.current);
            self.notify_session(sid, &format!("%session-window-changed ${sid} @{cur}\n"));
        }
        if self.sessions.get(&w.session).is_some_and(|s| s.windows.is_empty()) {
            self.kill_session(w.session);
        }
    }

    fn kill_session(&mut self, session: u32) {
        let Some(s) = self.sessions.remove(&session) else { return };
        for w in s.windows {
            self.close_window(w);
        }
        for c in self.clients.iter_mut().filter(|c| c.session == Some(session)) {
            c.outbox.extend(b"%exit\n");
            c.closing = true;
        }
    }

    fn new_window(&mut self, session: u32, a: &Args, size: (u16, u16)) -> Result<(u32, u32), String> {
        let wid = self.id();
        let pane = self.spawn(wid, a, size)?;
        let name = a.get('n').map(|n| format::expand(n, &Ctx { s: self, client: None, session: None, window: None, pane: None })).unwrap_or_else(|| "orcux".into());
        self.windows.insert(wid, Window { id: wid, session, name, panes: vec![pane], widths: vec![size.0], size, active: pane, options: Opts::new(), hooks: HashMap::new(), activity: now() });
        if let Some(s) = self.sessions.get_mut(&session) {
            s.windows.push(wid);
            if !a.has('d') || s.windows.len() == 1 {
                s.current = wid;
            }
        }
        self.notify_session(session, &format!("%window-add @{wid}\n"));
        Ok((wid, pane))
    }

    fn print(&self, a: &Args, t: Target, default: &str) -> String {
        if a.has('P') {
            format::expand(a.get('F').unwrap_or(default), &self.ctx(t, None)) + "\n"
        } else {
            String::new()
        }
    }

    /// One command, from a control client or from a single request. A fault in one is that command's error: the server,
    /// and every terminal in it, go on.
    fn exec(&mut self, words: &[String], client: Option<u32>, caller: Option<&str>) -> Out {
        let me = std::panic::AssertUnwindSafe(&mut *self);
        std::panic::catch_unwind(move || { let me = me; me.0.run_command(words, client, caller) })
            .unwrap_or_else(|_| Err(format!("internal error in {}", words.first().map_or("", |w| w.as_str()))))
    }

    fn run_command(&mut self, words: &[String], client: Option<u32>, caller: Option<&str>) -> Out {
        let Some(name) = words.first() else { return Ok(String::new()) };
        // what only reads changes nothing that a subscription could show
        let reads = ["list-panes", "lsp", "list-windows", "lsw", "list-sessions", "ls", "list-clients", "lsc", "display-message", "display",
            "show-options", "show", "show-option", "capture-pane", "capturep", "has-session", "has"];
        if !reads.contains(&name.as_str()) {
            self.changed = true;
        }
        let rest = &words[1..];
        let target = |s: &Server, a: &Args| s.resolve(a.get('t').unwrap_or(""), client, caller);
        match name.as_str() {
            "new-session" | "new" => {
                let a = Args::parse(rest, "scxyFeEn");
                let name = a.get('s').map(String::from).unwrap_or_else(|| self.next.to_string());
                if self.sessions.values().any(|s| s.name == name) {
                    return Err(format!("duplicate session: {name}"));
                }
                let size = (a.get('x').and_then(|x| x.parse().ok()).unwrap_or(80), a.get('y').and_then(|y| y.parse().ok()).unwrap_or(24));
                let sid = self.id();
                self.sessions.insert(sid, Session { id: sid, name, windows: vec![], current: 0, options: Opts::new(), created: now() });
                match self.new_window(sid, &a, size) {
                    Ok((w, p)) => Ok(self.print(&a, (Some(sid), Some(w), Some(p)), "#{session_name}:")),
                    Err(e) => {
                        self.sessions.remove(&sid);
                        Err(e)
                    }
                }
            }
            "new-window" | "neww" => {
                let a = Args::parse(rest, "tcFeEn");
                let (s, ..) = target(self, &a)?;
                let s = s.ok_or("no session")?;
                let size = self.sessions.get(&s).and_then(|x| self.windows.get(&x.current)).map_or((80, 24), |w| w.size);
                let (w, p) = self.new_window(s, &a, size)?;
                Ok(self.print(&a, (Some(s), Some(w), Some(p)), "#{session_name}:#{window_index}.#{pane_index}"))
            }
            "split-window" | "splitw" => {
                let a = Args::parse(rest, "tclFeE");
                let (s, w, p) = target(self, &a)?;
                let (wid, at) = (w.ok_or("no window")?, p.ok_or("no pane")?);
                let win = self.windows.get(&wid).ok_or("no window")?;
                let i = win.panes.iter().position(|x| *x == at).unwrap_or(0);
                let old = if a.has('f') { win.widths.iter().copied().max().unwrap_or(0) } else { win.widths[i] };
                let want = a.get('l').and_then(|l| l.trim_end_matches('%').parse::<u16>().ok()).unwrap_or(old / 2).min(old.saturating_sub(2)).max(1);
                let h = win.size.1;
                let pane = self.spawn(wid, &a, (want, h))?;
                let win = self.windows.get_mut(&wid).ok_or("no window")?;
                // -f: at the edge of the whole window, the room taken from the widest pane
                let from = if a.has('f') { (0..win.widths.len()).max_by_key(|j| win.widths[*j]).unwrap_or(i) } else { i };
                win.widths[from] = win.widths[from].saturating_sub(want + 1).max(1);
                let at_i = match (a.has('f'), a.has('b')) {
                    (true, true) => 0,
                    (true, false) => win.panes.len(),
                    (false, true) => i,
                    (false, false) => i + 1,
                };
                win.panes.insert(at_i, pane);
                win.widths.insert(at_i, want);
                if !a.has('d') {
                    win.active = pane;
                }
                self.layout(wid);
                Ok(self.print(&a, (s, Some(wid), Some(pane)), "#{session_name}:#{window_index}.#{pane_index}"))
            }
            "kill-pane" | "killp" => {
                let a = Args::parse(rest, "t");
                let (.., p) = target(self, &a)?;
                self.close_pane(p.ok_or("no pane")?);
                Ok(String::new())
            }
            "kill-window" | "killw" => {
                let a = Args::parse(rest, "t");
                let (_, w, _) = target(self, &a)?;
                self.close_window(w.ok_or("no window")?);
                Ok(String::new())
            }
            "kill-session" => {
                let a = Args::parse(rest, "t");
                let (s, ..) = target(self, &a)?;
                self.kill_session(s.ok_or("no session")?);
                Ok(String::new())
            }
            "kill-server" => {
                for s in self.sessions.keys().copied().collect::<Vec<_>>() {
                    self.kill_session(s);
                }
                Ok(String::new())
            }
            "has-session" | "has" => {
                let a = Args::parse(rest, "t");
                target(self, &a).map(|_| String::new())
            }
            "list-sessions" | "ls" => {
                let a = Args::parse(rest, "F");
                let f = a.get('F').unwrap_or("#{session_name}: #{session_windows} windows");
                Ok(self.sessions.values().map(|s| format::expand(f, &self.ctx((Some(s.id), Some(s.current), self.windows.get(&s.current).map(|w| w.active)), client)) + "\n").collect())
            }
            "list-windows" | "lsw" => {
                let a = Args::parse(rest, "tF");
                let (s, ..) = target(self, &a)?;
                let f = a.get('F').unwrap_or("#{window_id}");
                let s = self.sessions.get(&s.ok_or("no session")?).ok_or("no session")?;
                Ok(s.windows.iter().map(|w| format::expand(f, &self.ctx((Some(s.id), Some(*w), self.windows.get(w).map(|x| x.active)), client)) + "\n").collect())
            }
            "list-panes" | "lsp" => {
                let a = Args::parse(rest, "tF");
                let f = a.get('F').unwrap_or("#{pane_id}");
                let windows: Vec<u32> = if a.has('a') {
                    self.sessions.values().flat_map(|s| s.windows.clone()).collect()
                } else if a.has('s') {
                    let (s, ..) = target(self, &a)?;
                    self.sessions.get(&s.ok_or("no session")?).map(|s| s.windows.clone()).unwrap_or_default()
                } else {
                    let (_, w, _) = target(self, &a)?;
                    w.into_iter().collect()
                };
                let mut out = String::new();
                for w in windows {
                    let Some(win) = self.windows.get(&w) else { continue };
                    for p in &win.panes {
                        out += &format::expand(f, &self.ctx((Some(win.session), Some(w), Some(*p)), client));
                        out.push('\n');
                    }
                }
                Ok(out)
            }
            "list-clients" | "lsc" => {
                let a = Args::parse(rest, "tF");
                let f = a.get('F').unwrap_or("#{client_name}");
                let only = a.get('t').map(|_| target(self, &a)).transpose()?.and_then(|t| t.0);
                Ok(self.clients.iter().filter(|c| c.session.is_some() && !c.closing && (only.is_none() || c.session == only))
                    .map(|c| format::expand(f, &Ctx { s: self, client: Some(c.id), session: c.session, window: None, pane: None }) + "\n").collect())
            }
            "display-message" | "display" => {
                let a = Args::parse(rest, "tcF");
                let t = target(self, &a).unwrap_or((None, None, None));
                let f = a.get('F').map(String::from).or_else(|| a.rest.first().cloned()).unwrap_or_default();
                let me = a.get('c').and_then(|c| c.strip_prefix("client-")).and_then(|n| n.parse().ok()).or(client);
                let text = format::expand(&f, &self.ctx(t, me)).replace("%%", "%");
                Ok(if a.has('p') { text + "\n" } else { String::new() })
            }
            "set-option" | "set" | "set-window-option" | "setw" => {
                let a = Args::parse(rest, "t");
                let (Some(key), value) = (a.rest.first().cloned(), a.rest.get(1).cloned().unwrap_or_default()) else { return Err("no option".into()) };
                let t = target(self, &a);
                let opts: &mut Opts = if a.has('g') || a.has('s') {
                    &mut self.globals
                } else if a.has('p') {
                    let p = t?.2.ok_or("no pane")?;
                    &mut self.panes.get_mut(&p).ok_or("no pane")?.options
                } else if a.has('w') || name.starts_with("setw") || name == "set-window-option" {
                    let w = t?.1.ok_or("no window")?;
                    &mut self.windows.get_mut(&w).ok_or("no window")?.options
                } else {
                    let s = t?.0.ok_or("no session")?;
                    &mut self.sessions.get_mut(&s).ok_or("no session")?.options
                };
                if a.has('u') {
                    opts.remove(&key);
                } else {
                    opts.insert(key, value);
                }
                Ok(String::new())
            }
            "show-options" | "show" | "show-option" | "show-window-options" | "showw" => {
                let a = Args::parse(rest, "t");
                let t = target(self, &a);
                let key = a.rest.first().cloned();
                let opts: Option<&Opts> = if a.has('g') || a.has('s') {
                    Some(&self.globals)
                } else if a.has('p') {
                    t.ok().and_then(|t| t.2).and_then(|p| self.panes.get(&p)).map(|p| &p.options)
                } else if a.has('w') {
                    t.ok().and_then(|t| t.1).and_then(|w| self.windows.get(&w)).map(|w| &w.options)
                } else {
                    t.ok().and_then(|t| t.0).and_then(|s| self.sessions.get(&s)).map(|s| &s.options)
                };
                let Some(opts) = opts else { return Ok(String::new()) };
                Ok(match key {
                    Some(k) => match opts.get(&k) {
                        Some(v) if a.has('v') => format!("{v}\n"),
                        Some(v) => format!("{k} {v}\n"),
                        None => String::new(),
                    },
                    None => opts.iter().map(|(k, v)| format!("{k} {v}\n")).collect(),
                })
            }
            "set-environment" | "setenv" => {
                let a = Args::parse(rest, "t");
                if let Some(k) = a.rest.first() {
                    if a.has('u') { self.env.remove(k); } else { self.env.insert(k.clone(), a.rest.get(1).cloned().unwrap_or_default()); }
                }
                Ok(String::new())
            }
            "set-hook" => {
                let a = Args::parse(rest, "t");
                let (_, w, _) = target(self, &a)?;
                if let (Some(w), Some(hook)) = (w.and_then(|w| self.windows.get_mut(&w)), a.rest.first()) {
                    if a.has('u') { w.hooks.remove(hook); } else { w.hooks.insert(hook.clone(), a.rest.get(1).cloned().unwrap_or_default()); }
                }
                Ok(String::new())
            }
            "send-keys" | "send" => {
                let a = Args::parse(rest, "tN");
                let (.., p) = target(self, &a)?;
                let p = p.ok_or("no pane")?;
                let app = self.panes.get(&p).is_some_and(|x| x.parser.screen().application_cursor());
                let mut bytes = vec![];
                for k in &a.rest {
                    if a.has('H') {
                        bytes.push(u8::from_str_radix(k, 16).map_err(|_| format!("bad hex: {k}"))?);
                    } else if a.has('l') {
                        bytes.extend(k.as_bytes());
                    } else {
                        bytes.extend(keys::bytes(k, app).unwrap_or_else(|| k.as_bytes().to_vec()));
                    }
                }
                self.write_pane(p, &bytes);
                Ok(String::new())
            }
            "capture-pane" | "capturep" => {
                let a = Args::parse(rest, "tSEb");
                let (.., p) = target(self, &a)?;
                let pane = self.panes.get_mut(&p.ok_or("no pane")?).ok_or("no pane")?;
                let (rows, cols) = (pane.height, pane.width);
                let history = pane.parser.screen().scrollback_len() as i64;
                let line = |v: Option<&str>, default: i64| -> i64 {
                    match v {
                        Some("-") => -history,
                        Some(x) => x.parse().unwrap_or(default),
                        None => default,
                    }
                };
                let (from, to) = (line(a.get('S'), 0).max(-history), line(a.get('E'), rows as i64 - 1).min(rows as i64 - 1));
                let mut out = Vec::<u8>::new();
                let mut l = from;
                while l <= to {
                    // a page of history at a time: vt100 shows the screen scrolled back
                    let back = if l < 0 { (-l) as usize } else { 0 };
                    pane.parser.screen_mut().set_scrollback(back);
                    let screen = pane.parser.screen();
                    let row = if l < 0 { 0 } else { l as u16 };
                    let take = if l < 0 { (back as i64).min(to - l + 1).min(rows as i64) } else { (to - l + 1).min(rows as i64 - row as i64) };
                    if a.has('e') {
                        for r in screen.rows_formatted(0, cols).skip(row as usize).take(take as usize) {
                            out.extend_from_slice(AsRef::<[u8]>::as_ref(&r));
                            out.extend(b"\x1b[0m\n");
                        }
                    } else {
                        out.extend(plain_rows(screen, row, take as u16, cols, a.has('N'), a.has('J')));
                    }
                    l += take.max(1);
                }
                pane.parser.screen_mut().set_scrollback(0);
                Ok(String::from_utf8_lossy(&out).into_owned())
            }
            "select-pane" | "selectp" => {
                let a = Args::parse(rest, "t");
                let (_, w, p) = target(self, &a)?;
                let (w, p) = (w.ok_or("no window")?, p.ok_or("no pane")?);
                if let Some(win) = self.windows.get_mut(&w) && win.active != p {
                    win.active = p;
                    self.notify_window(w, &format!("%window-pane-changed @{w} {}\n", hex(p)));
                }
                Ok(String::new())
            }
            "swap-pane" | "swapp" => {
                let a = Args::parse(rest, "st");
                let (_, ws, ps) = self.resolve(a.get('s').unwrap_or(""), client, caller)?;
                let (_, wt, pt) = target(self, &a)?;
                let (ws, ps, wt, pt) = (ws.ok_or("no window")?, ps.ok_or("no pane")?, wt.ok_or("no window")?, pt.ok_or("no pane")?);
                if ps == pt {
                    return Ok(String::new());
                }
                for (w, from, to) in [(ws, ps, pt), (wt, pt, ps)] {
                    if let Some(win) = self.windows.get_mut(&w) {
                        for x in win.panes.iter_mut() {
                            if *x == from { *x = to; }
                        }
                        if win.active == from { win.active = to; }
                    }
                }
                if ws != wt {
                    if let Some(p) = self.panes.get_mut(&ps) { p.window = wt; }
                    if let Some(p) = self.panes.get_mut(&pt) { p.window = ws; }
                }
                self.layout(ws);
                if wt != ws { self.layout(wt); }
                Ok(String::new())
            }
            "resize-pane" | "resizep" => {
                let a = Args::parse(rest, "txy");
                let (_, w, p) = target(self, &a)?;
                let (w, p) = (w.ok_or("no window")?, p.ok_or("no pane")?);
                if let (Some(win), Some(x)) = (self.windows.get_mut(&w), a.get('x').and_then(|x| x.parse::<u16>().ok())) && let Some(i) = win.panes.iter().position(|q| *q == p) {
                    // the neighbor gives or takes it: the next one, or the one before the last
                    if let Some(j) = if i + 1 < win.widths.len() { Some(i + 1) } else { i.checked_sub(1) } {
                        let delta = (x as i32 - win.widths[i] as i32).min(win.widths[j] as i32 - 1);
                        win.widths[i] = (win.widths[i] as i32 + delta).max(1) as u16;
                        win.widths[j] = (win.widths[j] as i32 - delta).max(1) as u16;
                    }
                }
                self.layout(w);
                Ok(String::new())
            }
            "resize-window" | "resizew" => {
                let a = Args::parse(rest, "txy");
                let (_, w, _) = target(self, &a)?;
                let w = w.ok_or("no window")?;
                if let Some(win) = self.windows.get_mut(&w) {
                    win.options.insert("window-size".into(), "manual".into());
                    win.size = (a.get('x').and_then(|x| x.parse().ok()).unwrap_or(win.size.0), a.get('y').and_then(|y| y.parse().ok()).unwrap_or(win.size.1));
                }
                self.layout(w);
                self.hook(w, "window-resized");
                Ok(String::new())
            }
            "refresh-client" | "refresh" => {
                let a = Args::parse(rest, "CBAt");
                let me = a.get('t').and_then(|c| c.strip_prefix("client-")).and_then(|n| n.parse().ok()).or(client);
                if let Some(size) = a.get('C') && let Some((w, h)) = size.split_once('x') && let Some(c) = self.clients.iter_mut().find(|c| Some(c.id) == me) {
                    c.size = Some((w.parse().unwrap_or(80), h.parse().unwrap_or(24)));
                    c.activity = now();
                    self.sizes();
                }
                if let Some(sub) = a.get('B') && let Some(c) = self.clients.iter_mut().find(|c| Some(c.id) == me) {
                    let mut parts = sub.splitn(3, ':');
                    let (n, what, f) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string(), parts.next().map(String::from));
                    c.subscriptions.retain(|s| s.0 != n);
                    if let Some(f) = f {
                        c.subscriptions.push((n, what, f, None));
                    }
                }
                Ok(String::new())
            }
            "detach-client" | "detach" => {
                let a = Args::parse(rest, "ts");
                let me = a.get('t').and_then(|c| c.strip_prefix("client-")).and_then(|n| n.parse().ok()).or(client);
                if let Some(id) = me {
                    self.detach(id);
                }
                Ok(String::new())
            }
            "if-shell" | "if" => {
                let a = Args::parse(rest, "t");
                if !a.has('F') {
                    return Err("if-shell: only -F".into());
                }
                let t = target(self, &a).unwrap_or((None, None, None));
                let cond = format::expand(a.rest.first().map_or("", |s| s.as_str()), &self.ctx(t, client));
                let pick = if !cond.is_empty() && cond != "0" { a.rest.get(1) } else { a.rest.get(2) };
                let mut out = String::new();
                for cmd in cmdline::parse(pick.map_or("", |s| s.as_str())) {
                    out += &self.exec(&cmd, client, caller)?;
                }
                Ok(out)
            }
            "set-buffer" | "setb" => {
                let a = Args::parse(rest, "b");
                self.buffers.insert(a.get('b').unwrap_or("0").into(), a.rest.first().cloned().unwrap_or_default().into_bytes());
                Ok(String::new())
            }
            "load-buffer" | "loadb" => {
                let a = Args::parse(rest, "b");
                let data = std::fs::read(a.rest.first().ok_or("no file")?).map_err(|e| e.to_string())?;
                self.buffers.insert(a.get('b').unwrap_or("0").into(), data);
                Ok(String::new())
            }
            "paste-buffer" | "pasteb" => {
                let a = Args::parse(rest, "bt");
                let name = a.get('b').unwrap_or("0").to_string();
                let (.., p) = target(self, &a)?;
                let p = p.ok_or("no pane")?;
                let data = if a.has('d') { self.buffers.remove(&name) } else { self.buffers.get(&name).cloned() }.unwrap_or_default();
                let bracket = a.has('p') && self.panes.get(&p).is_some_and(|x| x.parser.screen().bracketed_paste());
                let mut bytes = vec![];
                if bracket { bytes.extend(b"\x1b[200~"); }
                bytes.extend(data.iter().map(|b| if *b == b'\n' { b'\r' } else { *b }));
                if bracket { bytes.extend(b"\x1b[201~"); }
                self.write_pane(p, &bytes);
                Ok(String::new())
            }
            "respawn-pane" | "respawnp" => {
                let a = Args::parse(rest, "tceE");
                let (_, w, p) = target(self, &a)?;
                let (w, p) = (w.ok_or("no window")?, p.ok_or("no pane")?);
                let (width, height, command, dir) = self.panes.get(&p).map(|x| (x.width, x.height, x.command.clone(), x.dir.clone())).ok_or("no pane")?;
                // without a command, the one it had, where it had it: as tmux does
                let mut a = a;
                if a.rest.is_empty() {
                    a.rest = command;
                }
                a.values.entry('c').or_insert_with(|| vec![dir.display().to_string()]);
                let new = self.spawn(w, &a, (width, height))?;
                if let Some(win) = self.windows.get_mut(&w) {
                    for x in win.panes.iter_mut() { if *x == p { *x = new; } }
                    if win.active == p { win.active = new; }
                }
                if let Some(old) = self.panes.remove(&p) {
                    unsafe { libc::kill(-old.pid, libc::SIGHUP) };
                    let options = old.options;
                    if let Some(n) = self.panes.get_mut(&new) { n.options = options; }
                }
                self.layout(w);
                Ok(String::new())
            }
            // the classic interface's own: there is no terminal client here, nothing to bind or show
            "bind-key" | "bind" | "unbind-key" | "unbind" | "switch-client" | "switchc" => Ok(String::new()),
            "display-popup" | "popup" | "attach-session" | "attach" => Err("orcux's engine has no terminal clients: use orcux window".into()),
            other => Err(format!("unknown command: {other}")),
        }
    }

    fn subscriptions(&mut self) {
        self.changed = false;
        let mut sends: Vec<(u32, String)> = vec![];
        let ids: Vec<u32> = self.clients.iter().map(|c| c.id).collect();
        for id in ids {
            let Some(c) = self.clients.iter().find(|c| c.id == id) else { continue };
            let session = c.session;
            let subs = c.subscriptions.clone();
            let mut values = vec![];
            for (name, _what, f, last) in &subs {
                let t = session.and_then(|s| self.sessions.get(&s)).map(|s| (Some(s.id), Some(s.current), self.windows.get(&s.current).map(|w| w.active))).unwrap_or((None, None, None));
                let v = format::expand(f, &self.ctx(t, Some(id)));
                if last.as_ref() != Some(&v) {
                    sends.push((id, format!("%subscription-changed {name} $1 - - - : {v}\n")));
                }
                values.push(v);
            }
            if let Some(c) = self.clients.iter_mut().find(|c| c.id == id) {
                for (s, v) in c.subscriptions.iter_mut().zip(values) {
                    s.3 = Some(v);
                }
            }
        }
        for (id, line) in sends {
            if let Some(c) = self.clients.iter_mut().find(|c| c.id == id) {
                c.outbox.extend(line.as_bytes());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapped_rows_are_joined() {
        let nz = |n: u16| std::num::NonZeroU16::new(n).unwrap();
        let mut p = vt100::Parser::new(nz(3), nz(10), 0);
        p.process(b"$ -n; echo three;\r\nthree");
        let s = p.screen();
        assert_eq!(String::from_utf8_lossy(&plain_rows(s, 0, 3, 10, false, true)), "$ -n; echo three;\nthree\n");
        assert_eq!(String::from_utf8_lossy(&plain_rows(s, 0, 3, 10, false, false)), "$ -n; echo\n three;\nthree\n");
    }

    #[test]
    fn arguments() {
        let w = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let a = Args::parse(&w(&["-dP", "-t", "=orcux:", "-F", "#{pane_id}", "-e", "A=1", "-e", "B=2", "echo", "-x"]), "tFe");
        assert!(a.has('d') && a.has('P') && a.get('t') == Some("=orcux:") && a.all('e') == vec!["A=1", "B=2"]);
        assert_eq!(a.rest, vec!["echo", "-x"]);
        let b = Args::parse(&w(&["-S", "-15", "-E", "-1", "-t%3"]), "SEt");
        assert_eq!((b.get('S'), b.get('E'), b.get('t')), (Some("-15"), Some("-1"), Some("%3")));
        assert_eq!(escape(b"a\x1b[1m\\\n"), "a\\033[1m\\134\\012");
    }
}
