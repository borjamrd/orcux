//! orcux's own engine (`ORCUX_ENGINE=own`): a server that keeps the terminals alive instead of tmux, and understands
//! the part of tmux's language that orcux speaks, so the rest of orcux does not change. `orcux tmux …` is its command
//! line, as tmux's is `tmux …`; the panes it opens carry ORCUX_MUX, the path of its socket.
pub mod cmdline;
pub mod format;
pub mod keys;
pub mod proto;
pub mod server;

use crate::Res;
use std::{
    env,
    io::{Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// Whether orcux runs on its own engine: asked for, or inside one of its panes.
pub fn own() -> bool {
    env::var("ORCUX_MUX").is_ok_and(|s| !s.is_empty()) || env::var("ORCUX_ENGINE").is_ok_and(|e| e == "own")
}

/// Inside a pane of tmux or of the engine.
pub fn inside() -> bool {
    env::var("TMUX").is_ok() || env::var("ORCUX_MUX").is_ok_and(|s| !s.is_empty())
}

/// The socket: the one of the pane this runs in, else a folder of this user's, as tmux's.
pub fn socket(name: Option<&str>) -> PathBuf {
    if name.is_none() && let Ok(s) = env::var("ORCUX_MUX") && !s.is_empty() {
        return PathBuf::from(s);
    }
    let base = env::var("TMUX_TMPDIR").ok().filter(|d| !d.is_empty()).unwrap_or_else(|| "/tmp".into());
    let dir = PathBuf::from(base).join(format!("orcux-{}", unsafe { libc::getuid() }));
    if !dir.exists() {
        std::fs::create_dir_all(&dir).ok();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).ok();
    }
    dir.join(name.unwrap_or("default"))
}

/// The program that speaks to the engine as tmux would be spoken to: `orcux tmux`, or tmux itself.
pub fn tmux_command() -> Command {
    if own() && let Ok(exe) = env::current_exe() {
        let mut c = Command::new(exe);
        c.arg("tmux");
        return c;
    }
    Command::new("tmux")
}

fn caller() -> Option<String> {
    env::var("TMUX_PANE").ok().filter(|p| !p.is_empty())
}

fn connect(path: &PathBuf, start: bool) -> Res<UnixStream> {
    if let Ok(s) = UnixStream::connect(path) {
        return Ok(s);
    }
    if !start {
        return Err(format!("no server running on {}", path.display()));
    }
    // started apart from whoever asked: its own session, no terminal
    let exe = env::current_exe().map_err(|e| e.to_string())?;
    let mut c = Command::new(exe);
    c.args(["mux-server"]).arg(path).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    unsafe {
        use std::os::unix::process::CommandExt;
        c.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = c.spawn().map_err(|e| e.to_string())?;
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(3) {
        if let Ok(s) = UnixStream::connect(path) {
            // the daemon forks away from this child, which ends at once
            std::thread::spawn(move || child.wait());
            return Ok(s);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Err(format!("the engine did not start at {}", path.display()))
}

/// One request to the engine: what it printed, or its error.
fn ask(path: &PathBuf, argv: &[String]) -> (bool, String) {
    let start = argv.first().is_some_and(|c| c == "new-session" || c == "new");
    let mut s = match connect(path, start) {
        Ok(s) => s,
        Err(e) => return (false, e),
    };
    let req = proto::Request { attach: false, caller: caller(), argv: argv.to_vec() };
    if s.write_all(&req.encode()).is_err() {
        return (false, "the engine closed the connection".into());
    }
    let mut buf = vec![];
    s.read_to_end(&mut buf).ok();
    let ok = buf.first() == Some(&0);
    let text = buf.get(5..).map(|t| String::from_utf8_lossy(t).into_owned()).unwrap_or_default();
    (ok, text)
}

/// As `tmux()` does with tmux: the output, trimmed at the end, or the error.
pub fn command(args: &[&str]) -> Res<String> {
    let argv: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    match ask(&socket(None), &argv) {
        (true, t) => Ok(t.trim_end().into()),
        (false, e) => Err(format!("tmux {}: {}", args.join(" "), e.trim())),
    }
}

/// `orcux tmux [-u] [-L name] [-S path] [-C] command …`: the engine's command line. Returns the exit code.
pub fn cli(args: &[&str]) -> i32 {
    let (mut name, mut path, mut control, mut i) = (None, None, false, 0);
    while i < args.len() && args[i].starts_with('-') {
        match args[i] {
            "-u" | "-2" => {}
            "-C" | "-CC" => control = true,
            "-L" => {
                i += 1;
                name = args.get(i).copied();
            }
            "-S" => {
                i += 1;
                path = args.get(i).map(PathBuf::from);
            }
            "-f" => i += 1,
            _ => break,
        }
        i += 1;
    }
    let path = path.unwrap_or_else(|| socket(name));
    let argv: Vec<String> = args[i..].iter().map(|a| a.to_string()).collect();
    if control && argv.first().is_some_and(|c| c == "attach-session" || c == "attach") {
        return relay(&path, &argv[1..]);
    }
    let (ok, text) = ask(&path, &argv);
    if ok {
        print!("{text}");
        0
    } else {
        if !text.is_empty() {
            eprintln!("{}", text.trim_end());
        }
        1
    }
}

/// A control client: lines from stdin to the engine, its lines to stdout, until either ends.
fn relay(path: &PathBuf, attach: &[String]) -> i32 {
    let Ok(mut s) = connect(path, false) else {
        eprintln!("no server running on {}", path.display());
        return 1;
    };
    let req = proto::Request { attach: true, caller: caller(), argv: attach.to_vec() };
    if s.write_all(&req.encode()).is_err() {
        return 1;
    }
    let Ok(mut up) = s.try_clone() else { return 1 };
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let mut buf = [0u8; 8192];
        while let Ok(n) = stdin.read(&mut buf) {
            if n == 0 || up.write_all(&buf[..n]).is_err() {
                break;
            }
        }
        up.shutdown(std::net::Shutdown::Write).ok();
    });
    let mut out = std::io::stdout();
    let mut buf = [0u8; 65536];
    while let Ok(n) = s.read(&mut buf) {
        if n == 0 || out.write_all(&buf[..n]).and_then(|_| out.flush()).is_err() {
            break;
        }
    }
    0
}

/// `orcux mux-server <socket>`: the engine itself, as a daemon. Only one per socket: a live one is left alone.
pub fn serve(path: &str) -> Res<()> {
    let path = PathBuf::from(path);
    if UnixStream::connect(&path).is_ok() {
        return Ok(());
    }
    std::fs::remove_file(&path).ok();
    let listener = UnixListener::bind(&path).map_err(|e| e.to_string())?;
    unsafe {
        // away from the terminal that started it, with room for many terminals
        if libc::fork() > 0 {
            libc::_exit(0);
        }
        libc::setsid();
        let mut lim = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) == 0 {
            lim.rlim_cur = lim.rlim_max.min(8192);
            libc::setrlimit(libc::RLIMIT_NOFILE, &lim);
        }
    }
    env::set_current_dir("/").ok();
    server::Server::new(&path).run(listener);
    Ok(())
}
