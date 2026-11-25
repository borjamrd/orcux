//! A tmux server and a home of its own, deleted at the end: tests never reach those of the person who runs them.
#![allow(dead_code)]
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU32, Ordering},
};

pub fn has_tmux() -> bool {
    Command::new("tmux").arg("-V").output().is_ok_and(|o| o.status.success())
}

pub struct Lab {
    pub dir: PathBuf,
}

impl Lab {
    pub fn new() -> Lab {
        // short: the path of a tmux socket has a limit
        static COUNT: AtomicU32 = AtomicU32::new(0);
        let dir = PathBuf::from(format!("/tmp/orcux-test-{}-{}", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed)));
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

    pub fn cmd(&self, program: &str) -> Command {
        let mut c = Command::new(program);
        let path = format!("{}:{}", self.dir.join("bin").display(), std::env::var("PATH").unwrap_or_default());
        c.env("HOME", self.dir.join("home")).env("TMUX_TMPDIR", self.dir.join("tmux")).env("PATH", path).env("SHELL", "/bin/sh").env("LANG", "en_US.UTF-8");
        for k in ["TMUX", "TMUX_PANE", "ORCUX_ID", "ORCUX_LANG", "LC_ALL", "GH_TOKEN"] {
            c.env_remove(k);
        }
        c.current_dir(self.dir.join("repo"));
        c
    }

    pub fn run(&self, program: &str, args: &[&str]) -> Output {
        let o = self.cmd(program).args(args).output().unwrap();
        assert!(o.status.success(), "{program} {args:?}: {}", String::from_utf8_lossy(&o.stderr));
        o
    }

    pub fn orcux(&self, args: &[&str]) -> Output {
        self.run(env!("CARGO_BIN_EXE_orcux"), args)
    }

    pub fn tmux(&self, args: &[&str]) -> String {
        String::from_utf8_lossy(&self.run(&self.dir.join("bin/tmux").display().to_string(), args).stdout).into()
    }

    pub fn pid(&self, mark: &str) -> u32 {
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
        // the shells write their history on their way out
        for _ in 0..20 {
            if fs::remove_dir_all(&self.dir).is_err() || !self.dir.exists() {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
    }
}

pub fn write_exe(path: &Path, text: &str) {
    use std::os::unix::fs::PermissionsExt;
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}
