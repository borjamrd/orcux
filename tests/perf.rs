//! What orcux costs with 50 terminals open, against a tmux of its own: never the one of the person who runs it.
//! Slow, so it is asked for: `cargo test --release --test perf -- --ignored --nocapture`.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    thread::sleep,
    time::{Duration, Instant},
};

const TERMINALS: usize = 50;
const WATCH: u64 = 20;
/// Share of one core for each pane that stays open.
const CPU: f64 = 0.005;

struct Lab {
    dir: PathBuf,
}

impl Lab {
    fn new() -> Lab {
        // short: the path of a tmux socket has a limit
        let dir = PathBuf::from(format!("/tmp/orcux-perf-{}", std::process::id()));
        for d in ["home", "bin", "tmux", "repo"] {
            fs::create_dir_all(dir.join(d)).unwrap();
        }
        for f in [".zshrc", ".bashrc", ".profile"] {
            fs::write(dir.join("home").join(f), "").unwrap();
        }
        let real = String::from_utf8(Command::new("sh").args(["-c", "command -v tmux"]).output().unwrap().stdout).unwrap();
        assert!(!real.trim().is_empty(), "tmux is needed");
        // counts every tmux process, and refuses to reach any other server
        let wrapper = format!(
            "#!/bin/sh\ncase \"$TMUX\" in ''|*{0}/*) ;; *) exit 97;; esac\n[ \"$TMUX_TMPDIR\" = {0}/tmux ] || exit 97\necho \"$*\" >> {0}/log\nexec {1} \"$@\"\n",
            dir.display(), real.trim()
        );
        write_exe(&dir.join("bin/tmux"), &wrapper);
        write_exe(&dir.join("bin/gh"), "#!/bin/sh\nexit 1\n");
        Lab { dir }
    }

    fn cmd(&self, program: &str) -> Command {
        let mut c = Command::new(program);
        let path = format!("{}:{}", self.dir.join("bin").display(), std::env::var("PATH").unwrap_or_default());
        c.env("HOME", self.dir.join("home")).env("TMUX_TMPDIR", self.dir.join("tmux")).env("PATH", path).env("SHELL", "/bin/sh").env("LANG", "en_US.UTF-8");
        for k in ["TMUX", "TMUX_PANE", "ORCUX_ID", "ORCUX_LANG", "LC_ALL", "GH_TOKEN"] {
            c.env_remove(k);
        }
        c.current_dir(self.dir.join("repo"));
        c
    }

    fn run(&self, program: &str, args: &[&str]) -> Output {
        let o = self.cmd(program).args(args).output().unwrap();
        assert!(o.status.success(), "{program} {args:?}: {}", String::from_utf8_lossy(&o.stderr));
        o
    }

    fn orcux(&self, args: &[&str]) -> Output {
        self.run(env!("CARGO_BIN_EXE_orcux"), args)
    }

    fn tmux(&self, args: &[&str]) -> String {
        String::from_utf8_lossy(&self.run(&self.dir.join("bin/tmux").display().to_string(), args).stdout).into()
    }

    fn pid(&self, mark: &str) -> u32 {
        let all = self.tmux(&["list-panes", "-a", "-F", "#{pane_pid} #{@orcux}"]);
        all.lines().find_map(|l| l.strip_suffix(&format!(" {mark}"))).unwrap_or_else(|| panic!("no {mark}:\n{all}")).parse().unwrap()
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        for socket in fs::read_dir(self.dir.join("tmux")).into_iter().flatten().flatten().flat_map(|d| fs::read_dir(d.path()).into_iter().flatten().flatten()) {
            // by its path: no environment can send this anywhere else
            Command::new("tmux").arg("-S").arg(socket.path()).arg("kill-server").env_remove("TMUX").output().ok();
        }
        fs::remove_dir_all(&self.dir).ok();
    }
}

fn write_exe(path: &Path, text: &str) {
    use std::os::unix::fs::PermissionsExt;
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// Seconds of CPU used so far.
fn cpu(pid: u32) -> f64 {
    if let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) {
        let f: Vec<f64> = stat.rsplit(')').next().unwrap().split_whitespace().map(|x| x.parse().unwrap_or(0.0)).collect();
        return (f[11] + f[12]) / 100.0;
    }
    let out = Command::new("ps").env("LC_ALL", "C").args(["-o", "time=", "-p", &pid.to_string()]).output().unwrap().stdout;
    String::from_utf8_lossy(&out).trim().split(':').fold(0.0, |all, x| all * 60.0 + x.parse::<f64>().unwrap())
}

#[test]
#[ignore]
fn fifty_terminals_cost_nothing_at_rest() {
    let lab = Lab::new();
    lab.run("git", &["init", "-q", "."]);
    lab.run("git", &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "i"]);
    for i in 0..10 {
        lab.orcux(&["new", &format!("w{i}")]);
        for _ in 1..TERMINALS / 10 {
            lab.orcux(&["tab", "new", &format!("w{i}")]);
        }
    }

    let started = Instant::now();
    let app = format!("env -u TMUX '{}'", env!("CARGO_BIN_EXE_orcux"));
    lab.tmux(&["-L", "outer", "-f", "/dev/null", "new-session", "-d", "-s", "o", "-x", "160", "-y", "42", &app]);
    while !lab.tmux(&["-L", "outer", "capture-pane", "-p", "-t", "o"]).contains("w9") {
        assert!(started.elapsed().as_secs() < 10, "it did not start");
        sleep(Duration::from_millis(20));
    }
    let start = started.elapsed();
    let panes = lab.tmux(&["list-panes", "-a"]).lines().count();
    assert!(panes >= TERMINALS, "{panes} panes");

    sleep(Duration::from_secs(8));
    let pids = [lab.pid("__sidebar"), lab.pid("__prs")];
    let before = pids.map(cpu);
    fs::remove_file(lab.dir.join("log")).ok();
    sleep(Duration::from_secs(WATCH));
    let calls = fs::read_to_string(lab.dir.join("log")).unwrap_or_default();
    let used: Vec<f64> = pids.iter().zip(before).map(|(p, b)| (cpu(*p) - b) / WATCH as f64).collect();

    let mut ls: Vec<Duration> = (0..9).map(|_| { let t = Instant::now(); lab.orcux(&["ls"]); t.elapsed() }).collect();
    ls.sort();
    println!("{panes} panes, start {start:?}, `orcux ls` {:?}, CPU sidebar {:.2} % rail {:.2} %, {} tmux processes in {WATCH} s", ls[4], used[0] * 100.0, used[1] * 100.0, calls.lines().count());
    assert!(calls.is_empty(), "tmux processes at rest:\n{calls}");
    assert!(used.iter().all(|u| *u <= CPU), "CPU at rest: {used:?}");
    assert!(ls[4] < Duration::from_millis(50), "`orcux ls` took {:?}", ls[4]);
    assert!(start < Duration::from_secs(1), "it took {start:?} to start");
}
