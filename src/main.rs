mod agent;
mod config;
mod files;
mod control;
mod doctor;
mod i18n;
mod mark;
mod merge;
mod prs;
mod restore;
mod theme;
mod tui;
mod work;

use std::{collections::BTreeMap, env, fs, path::{Path, PathBuf}, process::Command, thread, time::Duration};

use agent::Agent;
use i18n::tr;
use theme::hex;

pub type Res<T> = Result<T, String>;

pub const UI: &str = "=orcux"; // `=` = exact match; plain "orcux" would prefix-match "orcux-bg"
const UI_NAME: &str = "orcux";
const BG: &str = "orcux-bg"; // hidden session: one window per worktree terminal
/// Claude Code loads the plugins in these folders, for that terminal only.
const PLUGINS: &str = "CLAUDE_CODE_PLUGIN_DIRS";
/// OpenCode adds this to its configuration.
const OPENCODE: &str = "OPENCODE_CONFIG_CONTENT";
pub const SIDEBAR: &str = "__sidebar";
pub const RAIL: &str = "__prs"; // right pane: pull requests
/// Opt+w: what macOS types for it on the Spanish and US layouts, and what terminals with Option as Meta send.
// ponytail: other layouts type other characters; make it a setting if someone needs them.
pub const SWITCH_KEYS: [&str; 3] = ["æ", "∑", "M-w"];
const RAIL_MIN: u32 = 140; // narrower windows start with the rail hidden, to leave the terminal 60 columns

fn help() -> String {
    let mut s = String::from("orcux: worktrees + tmux\n");
    for (group, cmds) in tr().commands {
        s += &format!("\n{group}\n");
        for (usage, desc) in *cmds {
            s += &format!("  {usage:<40} {desc}\n");
        }
    }
    s
}

#[derive(Clone, PartialEq)]
pub struct Wt {
    pub repo: PathBuf,
    pub name: String,
    pub slot: u32,
    /// where it is, when it was imported instead of created by orcux
    pub at: Option<PathBuf>,
    /// name of the worktree (of the same repo) it was created from; only a relation, its folder is not inside
    pub parent: Option<String>,
}

impl Wt {
    pub fn repo_name(&self) -> String {
        self.repo.file_name().unwrap_or_default().to_string_lossy().into()
    }
    pub fn id(&self) -> String {
        format!("{}/{}", self.repo_name(), self.name)
    }
    pub fn path(&self) -> PathBuf {
        if let Some(p) = &self.at {
            return p.clone();
        }
        home().join("orcux").join(self.repo_name()).join(&self.name)
    }
    /// The branch it has checked out now (an imported one is not named after it). Read from git's files: this
    /// runs for every worktree on every redraw. Read again after a few seconds.
    pub fn branch(&self) -> Option<String> {
        self.seen().1
    }
    /// Its folder is not there.
    pub fn gone(&self) -> bool {
        self.seen().0
    }
    fn seen(&self) -> (bool, Option<String>) {
        type Seen = (std::time::Instant, (bool, Option<String>));
        static SEEN: std::sync::Mutex<BTreeMap<PathBuf, Seen>> = std::sync::Mutex::new(BTreeMap::new());
        let path = self.path();
        let Ok(mut all) = SEEN.lock() else { return (!path.exists(), self.read_branch()) };
        match all.get(&path) {
            Some((at, what)) if at.elapsed().as_secs() < 4 => what.clone(),
            _ => {
                let what = (!path.exists(), self.read_branch());
                all.insert(path, (std::time::Instant::now(), what.clone()));
                what
            }
        }
    }
    fn read_branch(&self) -> Option<String> {
        let link = fs::read_to_string(self.path().join(".git")).ok()?;
        let dir = self.path().join(link.strip_prefix("gitdir:")?.trim());
        let head = fs::read_to_string(dir.join("HEAD")).ok()?;
        Some(head.trim().strip_prefix("ref: refs/heads/")?.into())
    }
    /// The same worktree, whatever else changed.
    pub fn is(&self, other: &Wt) -> bool {
        self.repo == other.repo && self.name == other.name
    }
    pub fn port(&self) -> u32 {
        10000 + self.slot * 100
    }
    fn env(&self) -> Vec<String> {
        let plain = |w: &Wt| w.id().to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect::<String>();
        let mut compose = plain(self);
        if load().iter().any(|w| !w.is(self) && w.slot < self.slot && plain(w) == compose) {
            compose += &format!("-{}", self.slot); // "a/b-c" and "a-b/c"
        }
        let mut all = vec![
            format!("ORCUX_ID={}", self.id()),
            format!("ORCUX_PORT={}", self.port()),
            format!("ORCUX_SLOT={}", self.slot),
            format!("COMPOSE_PROJECT_NAME={compose}"),
            format!("ORCUX_NAME={}", compose.replace('-', "_")), // for names of databases
        ];
        let known: Vec<(String, String)> = all.iter().filter_map(|e| e.split_once('=')).map(|(k, v)| (k.into(), v.into())).collect();
        all.extend(config::variables(&config::trusted(&self.repo), self.port(), &known).iter().map(|(k, v)| format!("{k}={v}")));
        if let Some(dir) = agent::plugin() {
            let (dir, others) = (dir.display().to_string(), env::var(PLUGINS).unwrap_or_default());
            let others: Vec<&str> = others.split(':').filter(|d| !d.is_empty() && *d != dir).collect();
            all.push(format!("{PLUGINS}={}", others.into_iter().chain([dir.as_str()]).collect::<Vec<_>>().join(":")));
        }
        // one of the user's own stays
        if env::var(OPENCODE).is_err() && let Some(config) = agent::opencode() {
            all.push(format!("{OPENCODE}={config}"));
        }
        all
    }
}

pub fn home() -> PathBuf {
    PathBuf::from(env::var("HOME").expect("HOME"))
}

pub fn state_dir() -> PathBuf {
    let d = home().join(".local/state/orcux");
    fs::create_dir_all(&d).ok();
    d
}

fn run(cmd: &str, args: &[&str], cwd: Option<&Path>) -> Res<String> {
    let mut c = Command::new(cmd);
    c.args(args);
    if let Some(d) = cwd {
        c.current_dir(d);
    }
    let o = c.output().map_err(|e| format!("{cmd}: {e}"))?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).trim_end().into())
    } else {
        Err(format!("{cmd} {}: {}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim()))
    }
}

pub fn tmux(args: &[&str]) -> Res<String> {
    control::command(args).unwrap_or_else(|| {
        // as arguments of a process, a last ";" would separate commands
        let args: Vec<String> = args.iter().map(|a| a.strip_suffix(';').map_or(a.to_string(), |a| format!("{a}\\;"))).collect();
        run("tmux", &args.iter().map(String::as_str).collect::<Vec<_>>(), None)
    })
}

/// The client of the person, with its size: the connections of `control` are clients too.
pub fn viewer() -> Option<(String, u32, u32)> {
    let f = "#{client_control_mode}\t#{client_activity}\t#{client_name}\t#{client_width}\t#{client_height}";
    let all = tmux(&["list-clients", "-t", UI, "-F", f]).ok()?;
    let people = all.lines().filter_map(|l| {
        let [control, at, name, w, h] = l.split('\t').collect::<Vec<_>>()[..] else { return None };
        (control != "1").then(|| Some((at.parse::<u64>().ok()?, name.to_string(), w.parse().ok()?, h.parse().ok()?)))?
    });
    people.max_by_key(|c| c.0).map(|(_, name, w, h)| (name, w, h))
}

pub fn detach() -> Res<()> {
    match viewer() {
        Some((name, ..)) => tmux(&["detach-client", "-t", &name]).map(|_| ()),
        None => Ok(()),
    }
}

pub fn git(dir: &Path, args: &[&str]) -> Res<String> {
    run("git", args, Some(dir))
}

// ---------- state: two plain files, no db ----------

/// Held while state files or the layout of tmux change: several agents may run orcux at once. Never while git works.
/// The same thread can take it again.
pub struct Lock(Option<fs::File>);

thread_local!(static HELD: std::cell::Cell<u32> = const { std::cell::Cell::new(0) });

pub fn lock() -> Lock {
    let first = HELD.get() == 0;
    HELD.set(HELD.get() + 1);
    let take = || {
        let f = fs::File::create(state_dir().join("lock")).ok()?;
        f.lock().ok()?;
        Some(f)
    };
    Lock(if first { take() } else { None })
}

impl Drop for Lock {
    fn drop(&mut self) {
        HELD.set(HELD.get() - 1);
        drop(self.0.take());
    }
}

/// What cannot be read is not the same as empty: it would be saved as such.
fn read_state(name: &str) -> Res<String> {
    match fs::read_to_string(state_dir().join(name)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("{name}: {e}")),
        r => Ok(r.unwrap_or_default()),
    }
}

/// Whole or not at all: others may be reading it.
fn write_state(name: &str, text: &str) -> Res<()> {
    let tmp = state_dir().join(format!("{name}.{}", std::process::id()));
    fs::write(&tmp, text).and_then(|_| fs::rename(&tmp, state_dir().join(name))).map_err(|e| format!("{name}: {e}"))
}

/// Changes the list of worktrees as it is now.
fn change<T>(f: impl FnOnce(&mut Vec<Wt>) -> Res<T>) -> Res<T> {
    let _lock = lock();
    read_state("worktrees.tsv")?;
    let mut wts = load();
    let out = f(&mut wts)?;
    save(&wts)?;
    Ok(out)
}

pub fn load() -> Vec<Wt> {
    fs::read_to_string(state_dir().join("worktrees.tsv")).unwrap_or_default().lines()
        .filter_map(|l| {
            let mut p = l.split('\t');
            let text = |s: Option<&str>| s.filter(|s| !s.is_empty()).map(String::from);
            Some(Wt { repo: p.next()?.into(), name: p.next()?.into(), slot: p.next()?.parse().ok()?, at: text(p.next()).map(PathBuf::from), parent: text(p.next()) })
        })
        .collect()
}

fn save(wts: &[Wt]) -> Res<()> {
    let at = |w: &Wt| w.at.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
    let line = |w: &Wt| format!("{}\t{}\t{}\t{}\t{}\n", w.repo.display(), w.name, w.slot, at(w), w.parent.as_deref().unwrap_or(""));
    let s: String = wts.iter().map(line).collect();
    write_state("worktrees.tsv", &s)
}

/// The worktrees of a repo as a tree, parents before their children. With each one, whether it is the last of its
/// siblings at every level down to its own. A missing parent makes a root.
pub fn tree(wts: &[&Wt]) -> Vec<(Wt, Vec<bool>)> {
    fn walk(wts: &[&Wt], parent: Option<&str>, trail: &[bool], out: &mut Vec<(Wt, Vec<bool>)>) {
        let known = |p: &str| wts.iter().any(|w| w.name == p);
        let here: Vec<&&Wt> = wts.iter().filter(|w| w.parent.as_deref().filter(|p| known(p)) == parent).collect();
        for (i, w) in here.iter().enumerate() {
            let trail: Vec<bool> = trail.iter().copied().chain([i + 1 == here.len()]).collect();
            out.push(((**w).clone(), trail.clone()));
            walk(wts, Some(&w.name), &trail, out);
        }
    }
    let mut out = vec![];
    walk(wts, None, &[], &mut out);
    out
}

pub fn repos() -> Vec<PathBuf> {
    let mut r: Vec<PathBuf> = fs::read_to_string(state_dir().join("repos")).unwrap_or_default()
        .lines().filter(|l| !l.is_empty()).map(PathBuf::from).collect();
    for w in load() {
        if !r.contains(&w.repo) {
            r.push(w.repo);
        }
    }
    r
}

/// Registers the main checkout of the repo containing `p` and returns its path.
fn add_repo(p: &Path) -> Res<PathBuf> {
    let common = git(p, &["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    let main = Path::new(&common).parent().ok_or(tr().not_repo)?.to_path_buf();
    let _lock = lock();
    read_state("repos")?;
    let mut r = repos();
    if !r.contains(&main) {
        r.push(main.clone());
        save_repos(&r)?;
    }
    Ok(main)
}

fn save_repos(r: &[PathBuf]) -> Res<()> {
    let s: String = r.iter().map(|p| format!("{}\n", p.display())).collect();
    write_state("repos", &s)
}

/// Takes a repo off the sidebar; nothing is deleted. Its worktrees must have left before.
pub fn remove_repo(repo: &Path) -> Res<()> {
    let _lock = lock();
    read_state("repos")?;
    match load().iter().filter(|w| w.repo == repo).count() {
        0 => save_repos(&repos().into_iter().filter(|r| r != repo).collect::<Vec<_>>()),
        n => Err((tr().repo_has_wts)(n)),
    }
}

/// What adding a project means for a text: an existing path, something to clone, or a new folder in `base`.
pub enum Project {
    Register(PathBuf),
    Clone(String, PathBuf),
    Create(PathBuf),
}

/// A path as people write it: the home folder is ~.
pub fn tilde(p: &Path) -> String {
    p.strip_prefix(home()).map_or_else(|_| p.display().to_string(), |r| format!("~/{}", r.display()))
}

pub fn expand(path: &str) -> PathBuf {
    match path.strip_prefix('~') {
        Some("") => home(),
        Some(rest) if rest.starts_with('/') => home().join(&rest[1..]),
        _ => PathBuf::from(path),
    }
}

pub fn project(text: &str, base: &Path) -> Res<Project> {
    let text = text.trim();
    if text.contains("://") || text.starts_with("git@") {
        let name = text.trim_end_matches('/').trim_end_matches(".git").rsplit(['/', ':']).next().unwrap_or("");
        return Ok(Project::Clone(text.into(), base.join(name)));
    }
    if text.starts_with(['/', '~', '.']) {
        let p = expand(text);
        let p = if p.is_absolute() { p } else { base.join(p) };
        return if p.exists() { Ok(Project::Register(p)) } else { Err((tr().path_missing)(&tilde(&p))) };
    }
    if text.contains('/') {
        return Err(tr().bad_name.into());
    }
    let p = base.join(text);
    Ok(if p.exists() { Project::Register(p) } else { Project::Create(p) })
}

pub fn add_project(p: &Project) -> Res<PathBuf> {
    let path = match p {
        Project::Register(path) => {
            if repos().contains(&path.canonicalize().unwrap_or_else(|_| path.clone())) {
                return Err(tr().repo_listed.into());
            }
            path
        }
        Project::Clone(url, path) => {
            if path.exists() {
                return Err((tr().exists)(&tilde(path)));
            }
            let to = path.to_str().ok_or(tr().non_utf8)?;
            run("env", &["GIT_TERMINAL_PROMPT=0", "GIT_SSH_COMMAND=ssh -oBatchMode=yes", "git", "clone", "-q", "--", url, to], None)?;
            path
        }
        Project::Create(path) => {
            fs::create_dir(path).map_err(|e| format!("{}: {e}", path.display()))?;
            git(path, &["init", "-q", "-b", "main"])?;
            git(path, &["commit", "-q", "--allow-empty", "-m", "Initial commit"])?; // a worktree needs a commit to start from
            path
        }
    };
    git(path, &["rev-parse", "--git-dir"]).map_err(|_| tr().not_repo)?;
    add_repo(path)
}

pub fn find(q: Option<&str>) -> Res<Wt> {
    let wts = load();
    let hits: Vec<&Wt> = match q {
        Some(q) => wts.iter().filter(|w| w.id() == q || w.name == q).collect(),
        None => {
            let cwd = env::current_dir().map_err(|e| e.to_string())?;
            wts.iter().filter(|w| cwd.starts_with(w.path())).collect()
        }
    };
    match hits.as_slice() {
        [w] => Ok((*w).clone()),
        [] => Err(q.map_or(tr().no_worktree_here.into(), tr().no_worktree)),
        _ => Err(tr().ambiguous.into()),
    }
}

// ---------- worktrees ----------

pub fn free_slot(wts: &[Wt]) -> u32 {
    (1..).find(|s| !wts.iter().any(|w| w.slot == *s)).unwrap()
}

/// Worktree for a PR's branch. Fetches it first: an unknown branch would otherwise start from origin/HEAD.
pub fn create_for_pr(repo: &Path, branch: &str) -> Res<Wt> {
    let spec = format!("+refs/heads/{branch}:refs/remotes/origin/{branch}");
    run("env", &["GIT_TERMINAL_PROMPT=0", "git", "fetch", "origin", &spec], Some(repo))?;
    create(repo, branch, None)
}

pub fn create(repo: &Path, name: &str, cmd: Option<&str>) -> Res<Wt> {
    create_from(repo, name, cmd, None)
}

/// With a parent, the new branch starts at the parent's last commit instead of origin/HEAD.
pub fn create_from(repo: &Path, name: &str, cmd: Option<&str>, parent: Option<&Wt>) -> Res<Wt> {
    create_with(repo, name, parent, |_| Ok(cmd.map(String::from)))
}

/// `cmd` gives what to run in its terminal, once the worktree is prepared.
pub fn create_with(repo: &Path, name: &str, parent: Option<&Wt>, cmd: impl FnOnce(&Wt) -> Res<Option<String>>) -> Res<Wt> {
    let repo = add_repo(repo)?;
    // registered before git works, so that no other process takes its name or its ports
    let wt = change(|wts| {
        let wt = Wt { repo: repo.clone(), name: name.into(), slot: free_slot(wts), at: None, parent: parent.map(|p| p.name.clone()) };
        match wts.iter().find(|w| w.id() == wt.id()) {
            Some(w) if w.repo == repo => return Err((tr().exists)(name)),
            Some(w) => return Err((tr().id_taken)(&w.id())),
            None => wts.push(wt.clone()),
        }
        Ok(wt)
    })?;
    if let Err(e) = check_out(&wt, parent) {
        change(|wts| {
            wts.retain(|w| !w.is(&wt));
            Ok(())
        })?;
        return Err(e);
    }
    let config = config::trusted(&wt.repo);
    if let Err(e) = config::link(&wt, &config) {
        warn(&e); // the worktree is there: better without a link than half made
    }
    let (setup, then) = (config::setup(&wt, &config)?, cmd(&wt)?);
    let pane = spawn(&wt)?;
    if let Some(line) = match (setup, then) {
        (Some(s), Some(c)) => Some(format!("{s} && {c}")),
        (s, c) => s.or(c),
    } {
        // it is at work before its agent can tell
        let mark = format!("working {} {}", prs::now(), if config.setup.is_empty() { "launched" } else { "starting" });
        tmux(&["set-option", "-p", "-t", &pane, "@orcux_state", &mark]).ok();
        type_into(&pane, &line)?;
    }
    Ok(wt)
}

/// Something to tell, unless an interface is being drawn.
pub fn warn(text: &str) {
    if !tui::DRAWING.load(std::sync::atomic::Ordering::Relaxed) {
        eprintln!("orcux: {text}");
    }
}

/// A `.orcux` that nobody approved: say so, because it was not used.
fn tell_untrusted(w: &Wt) {
    if matches!(config::of(&w.repo), Ok(config::Trust::No)) {
        warn(&(tr().untrusted)(&tilde(&w.repo)));
    }
}

fn check_out(wt: &Wt, parent: Option<&Wt>) -> Res<()> {
    let (repo, name, path) = (&wt.repo, wt.name.as_str(), wt.path());
    let p = path.to_str().ok_or(tr().non_utf8)?;
    let has = |r: &str| git(repo, &["rev-parse", "--verify", "--quiet", r]).is_ok();
    let exists = has(&format!("refs/heads/{name}")) || has(&format!("refs/remotes/origin/{name}"));
    if let Some(parent) = parent {
        if exists {
            return Err((tr().branch_exists)(name));
        }
        let base = git(&parent.path(), &["rev-parse", "HEAD"])?;
        git(repo, &["worktree", "add", "--no-track", "-b", name, p, &base])?;
    } else if exists {
        git(repo, &["worktree", "add", p, name])?;
    } else {
        let base = if has("refs/remotes/origin/HEAD") { "origin/HEAD" } else { "HEAD" };
        git(repo, &["worktree", "add", "--no-track", "-b", name, p, base])?;
    }
    copy_includes(repo, &path)
}

/// Copies the local-only files listed in the main checkout's .worktreeinclude.
// ponytail: literal paths only (files or dirs); glob lines are skipped. Add `git ls-files -oi` if globs are needed.
fn copy_includes(repo: &Path, dst: &Path) -> Res<()> {
    let Ok(list) = fs::read_to_string(repo.join(".worktreeinclude")) else { return Ok(()) };
    for l in list.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let rel = l.trim_start_matches('/').trim_end_matches('/');
        let src = repo.join(rel);
        if rel.contains(['*', '?', '[']) || !src.exists() {
            continue;
        }
        let to = dst.join(rel);
        fs::create_dir_all(to.parent().unwrap()).map_err(|e| e.to_string())?;
        run("cp", &["-R", src.to_str().unwrap(), to.to_str().unwrap()], None)?;
    }
    Ok(())
}

/// An existing worktree as orcux would register it, where it is. The slot is set when it is registered.
fn importable(p: &Path, wts: &[Wt]) -> Res<Wt> {
    let p = p.canonicalize().map_err(|e| e.to_string())?;
    let out = git(&p, &["rev-parse", "--path-format=absolute", "--show-toplevel", "--git-dir", "--git-common-dir"])
        .map_err(|_| tr().not_worktree)?;
    let [top, dir, common] = out.lines().collect::<Vec<_>>()[..] else { return Err(tr().not_worktree.into()) };
    if Path::new(top) != p {
        return Err(tr().not_root.into());
    }
    if dir == common {
        return Err(tr().main_checkout.into());
    }
    let repo = Path::new(common).parent().ok_or(tr().no_folder)?.to_path_buf();
    let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
    if wts.iter().any(|w| w.path().canonicalize().is_ok_and(|q| q == p)) {
        return Err(tr().registered.into());
    }
    let w = Wt { repo, name, slot: 0, at: Some(p), parent: None };
    match wts.iter().find(|x| x.id() == w.id()) {
        Some(_) => Err((tr().id_taken)(&w.id())),
        None => Ok(w),
    }
}

/// Registers existing worktrees without moving or changing them; without `yes`, only lists what it would do.
fn import(paths: &[&str], yes: bool) -> Res<()> {
    let mut n = 0;
    let mut list = |wts: &mut Vec<Wt>| {
        for p in paths {
            match importable(Path::new(p), wts) {
                Ok(mut w) => {
                    w.slot = free_slot(wts);
                    println!("+ {:<50} :{:<6} {}", w.id(), w.port(), w.branch().unwrap_or_default());
                    if yes {
                        add_repo(&w.repo)?;
                    }
                    wts.push(w);
                    n += 1;
                }
                Err(e) => println!("- {p}: {e}"),
            }
        }
        Ok(())
    };
    if yes {
        change(list)?;
        println!("{}", (tr().imported)(n));
    } else {
        list(&mut load())?;
        println!("{}", (tr().to_import)(n));
    }
    Ok(())
}

pub fn remove(w: &Wt, force: bool) -> Res<()> {
    remove_then(w, force, &config::trusted(&w.repo), || Ok(()))?;
    close_all(w)
}

/// Why git would refuse to delete it, seen before anything of it is taken down.
fn keeps(w: &Wt) -> Option<String> {
    let path = w.path();
    if git(&path, &["status", "--porcelain"]).is_ok_and(|s| !s.is_empty()) {
        return Some((tr().rm_dirty)(&w.id()));
    }
    let dir = git(&path, &["rev-parse", "--path-format=absolute", "--git-dir"]).ok()?;
    let dir = Path::new(&dir);
    if dir.join("locked").exists() {
        return Some((tr().rm_locked)(&w.id()));
    }
    dir.join("modules").exists().then(|| (tr().rm_submodules)(&w.id()))
}

/// `last` runs when it is sure that it can go, and right before it does; it can still say no.
pub fn remove_then(w: &Wt, force: bool, config: &config::Config, last: impl Fn() -> Res<()>) -> Res<()> {
    let path = w.path();
    if w.at.is_some() || !w.repo.exists() {
        // imported, or of a repo that is not there any more: not ours to delete, not even with --force
        last()?;
    } else if path.exists() {
        let p = path.to_str().ok_or(tr().non_utf8)?;
        // git would delete it with everything inside, also another worktree in an ignored folder
        let real = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.into());
        let others = git(&w.repo, &["worktree", "list", "--porcelain"])?;
        let others = others.lines().filter_map(|l| l.strip_prefix("worktree ")).map(PathBuf::from).chain(load().into_iter().map(|w| w.path()));
        if let Some(inside) = others.map(|o| real(&o)).find(|o| *o != real(&path) && o.starts_with(real(&path))) {
            return Err((tr().has_inside)(&tilde(&inside)));
        }
        // before its teardown, which may delete a database: a worktree that stays keeps what it needs
        if !force && let Some(why) = keeps(w) {
            return Err(why);
        }
        if let Err(e) = config::teardown(w, config) && !force {
            return Err(e);
        }
        last()?;
        let args: &[&str] = if force { &["worktree", "remove", "--force", p] } else { &["worktree", "remove", p] };
        git(&w.repo, args)?; // first: if it fails, its terminal stays
    } else {
        git(&w.repo, &["worktree", "prune"])?;
    }
    change(|wts| {
        let parent = wts.iter().find(|x| x.is(w)).and_then(|x| x.parent.clone());
        wts.retain(|x| !x.is(w));
        for child in wts.iter_mut().filter(|x| x.repo == w.repo && x.parent.as_ref() == Some(&w.name)) {
            child.parent = parent.clone(); // up one level
        }
        Ok(())
    })?;
    Ok(())
}

/// The terminals of a worktree that is gone. Its own goes last: with it, this process ends.
pub fn close_all(w: &Wt) -> Res<()> {
    let me = env::var("TMUX_PANE").unwrap_or_default();
    let mut all = tabs(&w.id());
    all.sort_by_key(|p| p.pane == me);
    for p in all {
        tmux(&["kill-pane", "-t", &p.pane]).ok(); // it may be gone already
    }
    if tmux(&["has-session", "-t", UI]).is_ok() {
        ensure_ui()?; // the killed pane may have been the visible one
    }
    Ok(())
}

// ---------- tmux ----------

/// A pane that orcux marked. A worktree has one per tab; `tab` is its number, which never changes.
pub struct Pane {
    pub pane: String,
    pub wt: String,
    pub tab: u32,
    /// the tab to show when its worktree is shown
    pub on: bool,
    pub busy: bool,
    pub cmd: String,
    /// what its agent said, and when
    pub agent: Option<(Agent, u64)>,
    /// its mark as it is, and the title of its terminal
    pub mark: String,
    pub title: String,
    /// the conversation of its agent, for `restore`
    pub session: String,
    /// no program but the shell
    pub shell: bool,
    /// when its window last printed something: its own only while it is hidden
    pub activity: u64,
}

impl Pane {
    pub fn look(&self) -> prs::State {
        match self.agent.map(|a| a.0) {
            Some(a) if a.waits() => prs::State::Asking,
            Some(Agent::Failed) => prs::State::Broken,
            _ if self.busy => prs::State::Working,
            Some(Agent::Done) => prs::State::Finished,
            _ => prs::State::Idle,
        }
    }
}

/// The tab of a worktree that matters most: one that waits for the user, then failed, working, done and idle.
pub fn foremost<'a>(tabs: impl Iterator<Item = &'a Pane>) -> Option<&'a Pane> {
    let rank = |s: prs::State| match s {
        prs::State::Asking | prs::State::Waiting => 0,
        prs::State::Broken | prs::State::Failed => 1,
        prs::State::Working => 2,
        prs::State::Finished | prs::State::Done => 3,
        prs::State::Idle => 4,
    };
    tabs.min_by_key(|p| (rank(p.look()), !p.on))
}

pub fn panes() -> Vec<Pane> {
    let f = "#{pane_id}\t#{@orcux}\t#{@orcux_tab}\t#{@orcux_on}\t#{@orcux_busy}\t#{pane_current_command}\t#{@orcux_state}\t#{window_activity}\t#{?@orcux_session,#{@orcux_session}|#{@orcux_model}|#{@orcux_mode},}\t#{pane_title}";
    tmux(&["list-panes", "-a", "-F", f]).unwrap_or_default().lines()
        .map(|l| {
            let mut p = l.split('\t').map(String::from);
            let mut next = || p.next().unwrap_or_default();
            let (pane, wt, tab, on, busy, cmd) = (next(), next(), next().parse().unwrap_or(1), next() == "1", next() == "1", next());
            // back in the shell: the agent is gone, whatever it said last
            let shell = ["zsh", "bash", "fish", "sh", "dash", "ksh", "nu"].contains(&cmd.trim_start_matches('-'));
            let (mark, activity, session, title) = (next(), next().parse().unwrap_or(0), next(), next());
            // a new one is at work while its shell still prepares it
            let (begins, over) = agent::begins(&mark);
            let agent = if over || (shell && !begins) { None } else { Agent::parse(&mark) };
            Pane { pane, wt, tab, on, busy, cmd, agent, mark, title, session, shell, activity }
        })
        .filter(|p| !p.wt.is_empty())
        .collect()
}

pub fn tabs(id: &str) -> Vec<Pane> {
    let mut t: Vec<Pane> = panes().into_iter().filter(|p| p.wt == id).collect();
    t.sort_by_key(|p| p.tab);
    t
}

/// The worktree's active tab.
fn pane_of(id: &str) -> Option<String> {
    let t = tabs(id);
    t.iter().find(|p| p.on).or(t.first()).map(|p| p.pane.clone())
}

/// "wt" or "wt:2" (git refs cannot contain ':').
fn target(q: &str) -> Res<(Wt, Option<u32>)> {
    match q.rsplit_once(':').and_then(|(w, n)| Some((w, n.parse().ok()?))) {
        Some((w, n)) => Ok((find(Some(w))?, Some(n))),
        None => Ok((find(Some(q))?, None)),
    }
}

/// The pane of that tab, or of the active one, which is opened if there is none.
fn pane_at(w: &Wt, tab: Option<u32>) -> Res<String> {
    let Some(n) = tab else { return ensure_pane(w) };
    let t = tabs(&w.id()).into_iter().find(|p| p.tab == n);
    t.map(|p| p.pane).ok_or_else(|| (tr().no_tab)(&format!("{}:{n}", w.id())))
}

fn spawn(w: &Wt) -> Res<String> {
    let _lock = lock();
    let (id, path) = (w.id(), w.path().to_string_lossy().to_string());
    let taken: Vec<u32> = tabs(&id).iter().map(|p| p.tab).collect();
    let tab = (1..).find(|n| !taken.contains(n)).unwrap().to_string();
    let mut a: Vec<String> = if tmux(&["has-session", "-t", &format!("={BG}")]).is_ok() {
        vec!["new-window".into(), "-d".into(), "-t".into(), format!("={BG}:")]
    } else {
        vec!["new-session".into(), "-d".into(), "-s".into(), BG.into()]
    };
    // tmux expands formats in these two: "fix-#Solo" would start somewhere else
    let plain = |s: &str| s.replace('#', "##");
    a.extend(["-n".into(), plain(&id), "-c".into(), plain(&path), "-P".into(), "-F".into(), "#{pane_id}".into()]);
    for e in w.env() {
        a.extend(["-e".into(), e]);
    }
    let pane = tmux(&a.iter().map(String::as_str).collect::<Vec<_>>())?;
    tmux(&["set-option", "-p", "-t", &pane, "@orcux_tab", &tab])?;
    tmux(&["set-option", "-p", "-t", &pane, "@orcux", &id])?;
    Ok(pane)
}

fn ensure_pane(w: &Wt) -> Res<String> {
    match pane_of(&w.id()) {
        Some(p) => Ok(p),
        None if !w.path().exists() => Err((tr().gone_at)(&w.path().to_string_lossy())),
        None => spawn(w),
    }
}

fn type_into(pane: &str, text: &str) -> Res<()> {
    tmux(&["send-keys", "-t", pane, "-l", "--", text])?;
    thread::sleep(Duration::from_millis(150)); // TUIs like claude treat text+Enter in one burst as a paste
    tmux(&["send-keys", "-t", pane, "Enter"]).map(|_| ())
}

fn ui_panes() -> Vec<(String, String)> {
    tmux(&["list-panes", "-t", &format!("{UI}:"), "-F", "#{pane_id}\t#{@orcux}"]).unwrap_or_default()
        .lines()
        .map(|l| { let (a, b) = l.split_once('\t').unwrap_or((l, "")); (a.into(), b.into()) }) // run() trims a trailing empty @wt
        .collect()
}

/// Session "orcux": left pane = sidebar, right pane = whichever worktree terminal is shown.
pub fn ensure_ui() -> Res<()> {
    let _lock = lock();
    let home = home().to_string_lossy().to_string();
    let exe = env::current_exe().map_err(|e| e.to_string())?;
    let welcome = format!("'{}' welcome", exe.display());
    if tmux(&["has-session", "-t", UI]).is_err() {
        tmux(&["new-session", "-d", "-s", UI_NAME, "-c", &home, &welcome])?;
        tmux(&["set-option", "-t", &format!("{UI}:"), "mouse", "on"])?;
    }
    let side = match ui_panes().into_iter().find(|p| p.1 == SIDEBAR) {
        Some(p) => p.0,
        None => {
            let cmd = format!("'{}' sidebar", exe.display());
            let p = tmux(&["split-window", "-hb", "-l", "36", "-t", &format!("{UI}:"), "-P", "-F", "#{pane_id}", &cmd])?;
            tmux(&["set-option", "-p", "-t", &p, "@orcux", SIDEBAR])?;
            p
        }
    };
    let missing = terminal_pane().is_none();
    if missing {
        tmux(&["split-window", "-h", "-d", "-t", &side, "-c", &home, &welcome])?;
    }
    pin(missing)
}

pub fn ui_pane(mark: &str) -> Option<String> {
    ui_panes().into_iter().find(|p| p.1 == mark).map(|p| p.0)
}

/// (pane, marker) of the pane that shows a worktree's terminal, or the placeholder (`orcux welcome`, no marker).
pub fn terminal_pane() -> Option<(String, String)> {
    ui_panes().into_iter().find(|p| p.1 != SIDEBAR && p.1 != RAIL)
}

/// tmux scales panes proportionally when the window changes size (e.g. on attach): keep both side panes fixed.
/// A hook, not their own resize events, so that dragging a border by hand still works. `now` also resizes at once.
fn pin(now: bool) -> Res<()> {
    let Some(side) = ui_pane(SIDEBAR) else { return Ok(()) };
    let mut cmds = vec![format!("resize-pane -t {side} -x 36")];
    cmds.extend(ui_pane(RAIL).map(|r| format!("resize-pane -t {r} -x 44")));
    tmux(&["set-hook", "-w", "-t", &side, "window-resized", &cmds.join(" ; ")])?;
    if now {
        for c in &cmds {
            tmux(&c.split(' ').collect::<Vec<_>>())?;
        }
    }
    Ok(())
}

/// Opens or closes the rail: what the user chose with `p`, else whether it fits. Only the sidebar calls this
/// on its own (one owner, so it is never opened twice).
pub fn sync_rail() -> Res<()> {
    let Some(side) = ui_pane(SIDEBAR) else { return Ok(()) };
    let info = tmux(&["display-message", "-p", "-t", &side, "#{window_width} #{@orcux_prs}"])?;
    let (width, choice) = info.split_once(' ').unwrap_or((&info, ""));
    let want = match choice {
        "on" => true,
        "off" => false,
        _ => width.parse().is_ok_and(|w: u32| w >= RAIL_MIN),
    };
    match (want, ui_pane(RAIL)) {
        (true, None) => {
            let exe = env::current_exe().map_err(|e| e.to_string())?;
            let cmd = format!("'{}' rail", exe.display());
            let p = tmux(&["split-window", "-hf", "-d", "-l", "44", "-t", &side, "-P", "-F", "#{pane_id}", &cmd])?;
            tmux(&["set-option", "-p", "-t", &p, "@orcux", RAIL])?;
            pin(true)
        }
        (false, Some(p)) => {
            tmux(&["set-hook", "-w", "-u", "-t", &side, "window-resized"])?;
            tmux(&["kill-pane", "-t", &p])?;
            pin(true)
        }
        _ => Ok(()),
    }
}

/// Restarts the sidebar and the rail, if they are open. Terminals are not touched.
fn restart_panes() -> Res<()> {
    for p in [SIDEBAR, RAIL].into_iter().filter_map(ui_pane) {
        tmux(&["respawn-pane", "-k", "-t", &p])?;
    }
    Ok(())
}

pub fn toggle_rail() -> Res<()> {
    let choice = if ui_pane(RAIL).is_some() { "off" } else { "on" };
    tmux(&["set-option", "-t", &format!("{UI}:"), "@orcux_prs", choice])?;
    if ui_pane(RAIL) == env::var("TMUX_PANE").ok() {
        return Ok(()); // it would close itself halfway: the sidebar is told of the option
    }
    sync_rail()
}

/// Moves the focus to the pane with that marker, or to the terminal when it is not open.
pub fn focus(mark: &str) -> Res<()> {
    let pane = ui_pane(mark).or_else(|| terminal_pane().map(|p| p.0)).ok_or(tr().no_pane)?;
    tmux(&["select-pane", "-t", &pane]).map(|_| ())
}

/// Watches this process's own binary, so that the sidebar and the rail do not go on running an old version.
pub struct Binary(Option<std::time::SystemTime>);

impl Binary {
    pub fn now() -> Binary {
        Binary(Self::modified())
    }
    fn modified() -> Option<std::time::SystemTime> {
        fs::metadata(env::current_exe().ok()?).ok()?.modified().ok()
    }
    /// Replaced, and at least 2 s ago: the build is over.
    pub fn replaced(&self) -> bool {
        let m = Self::modified();
        m.is_some() && m != self.0 && m.unwrap().elapsed().is_ok_and(|d| d.as_secs() >= 2)
    }
}

/// Becomes the new version of itself. Only returns if it could not.
pub fn restart_self() -> String {
    use std::os::unix::process::CommandExt;
    control::close();
    match env::current_exe() {
        Ok(exe) => Command::new(exe).args(env::args().skip(1)).exec().to_string(),
        Err(e) => e.to_string(),
    }
}

/// Colors and status bar of the orcux session only; the user's other sessions keep theirs.
pub fn style_ui() -> Res<()> {
    let t = format!("{UI}:");
    // the ground is the terminal's own: it shows around the grid too, so another color would frame it
    let (abyss, hull, line, foam) = ("default".to_string(), hex(theme::HULL), hex(theme::LINE), hex(theme::FOAM));
    let status = format!("bg={abyss},fg={foam}");
    for (k, v) in [("status", "on"), ("mouse", "on"), ("status-style", status.as_str()), ("status-left-length", "120"), ("status-right-length", "80")] {
        tmux(&["set-option", "-t", &t, k, v]).ok();
    }
    let gap = format!("bg={abyss},fg={abyss}");
    let rail = format!("#[bg={hull}#,fg={foam}#,bold]  {}  #[default]", tr().your_prs);
    // the logo, an "o" of four by four dots, and the name. No commas: it goes inside a tmux conditional
    let blue = hex(theme::CURRENT);
    let name = format!("#[fg={blue}]⢎#[fg={foam}]⡱ #[bold]orcux");
    let titles = format!("#{{?#{{==:#{{@orcux}},{SIDEBAR}}},{name},#{{?#{{==:#{{@orcux}},{RAIL}}},{rail},#{{@orcux_bar}}}}}}");
    for (k, v) in [
        ("window-status-format", ""),
        ("window-status-current-format", ""),
        // gaps, not lines: tmux draws two cells of line before every title. The sidebar and the rail draw the lines
        // beside the terminal themselves
        ("pane-border-lines", "spaces"),
        ("pane-border-style", gap.as_str()),
        ("pane-active-border-style", gap.as_str()),
        ("window-style", "bg=default"),
        ("pane-border-status", "top"),
        ("pane-border-format", titles.as_str()),
    ] {
        tmux(&["set-option", "-w", "-t", &t, k, v]).ok(); // an older tmux lacks some
    }
    // Key tables are shared by every session: outside orcux the key goes on to the program, as if not bound.
    let exe = env::current_exe().map_err(|e| e.to_string())?;
    let open = format!(
        "display-popup -E -b rounded -T '#[fg={foam}] Worktrees ' -w 60% -h 50% -s 'bg={hull},fg={foam}' -S 'bg={hull},fg={line}' \"'{}' switch\"",
        exe.display()
    );
    let close = format!(
        "display-popup -E -b rounded -T '#[fg={foam}] {} ' -w 60% -h 10 -s 'bg={hull},fg={foam}' -S 'bg={hull},fg={line}' \"'{}' dialog close\"",
        tr().title_close, exe.display()
    );
    // run-shell would print any output or failure over the pane
    let tab = |what: &str| format!("run-shell -b \"'{}' tab {what} >/dev/null 2>&1 || true\"", exe.display());
    for (keys, cmd) in [
        (&SWITCH_KEYS[..], open),
        (&["†", "M-t"], tab("new")),
        (&["å", "M-a"], tab("prev")),
        (&["∂", "M-d"], tab("next")),
        (&["œ", "M-q"], close),
        (&["®", "M-r"], format!("run-shell -b \"'{}' next >/dev/null 2>&1 || true\"", exe.display())),
    ] {
        for k in keys {
            tmux(&["bind-key", "-n", k, "if-shell", "-F", "#{==:#{session_name},orcux}", &cmd, &format!("send-keys {k}")]).ok();
        }
    }
    Ok(())
}

pub fn set_status(left: &str, right: &str) {
    let t = format!("{UI}:");
    tmux(&["set-option", "-t", &t, "status-left", left]).ok();
    tmux(&["set-option", "-t", &t, "status-right", right]).ok();
}

pub fn open_url(url: &str) -> Res<()> {
    run(if cfg!(target_os = "macos") { "open" } else { "xdg-open" }, &[url], None).map(|_| ())
}

pub fn show(w: &Wt) -> Res<()> {
    show_tab(w, None)
}

pub fn show_tab(w: &Wt, tab: Option<u32>) -> Res<()> {
    let _lock = lock();
    ensure_ui()?;
    let pane = pane_at(w, tab)?;
    let (right, mark) = terminal_pane().ok_or(tr().no_pane)?;
    if pane != right {
        tmux(&["swap-pane", "-d", "-s", &pane, "-t", &right])?;
        if mark.is_empty() {
            tmux(&["kill-pane", "-t", &right])?; // the placeholder, not a worktree
        }
    }
    touch_recent(&w.id());
    for p in tabs(&w.id()) {
        if p.on != (p.pane == pane) {
            tmux(&["set-option", "-p", "-t", &p.pane, "@orcux_on", if p.on { "0" } else { "1" }])?;
        }
    }
    set_tab_bar(&tab_bar());
    tmux(&["select-pane", "-t", &pane]).map(|_| ())
}

/// Runs `orcux <args>` in a tmux popup centered over the whole UI (the sidebar alone is too narrow). Blocks until closed.
pub fn popup(title: &str, want: (u32, u32), args: &[&str]) -> Res<()> {
    let exe = env::current_exe().map_err(|e| e.to_string())?;
    // tmux rejects a popup larger than the client instead of clamping it
    let (client, w, h) = viewer().ok_or(tr().no_pane)?;
    let (w, h, title) = (w.min(want.0).to_string(), h.min(want.1).to_string(), format!("#[fg={}] {title} ", hex(theme::FOAM)));
    let (style, edge) = (format!("bg={},fg={}", hex(theme::HULL), hex(theme::FOAM)), format!("bg={},fg={}", hex(theme::HULL), hex(theme::LINE)));
    let mut a = vec!["display-popup", "-c", &client, "-E", "-b", "rounded", "-s", &style, "-S", &edge, "-T", &title, "-w", &w, "-h", &h, exe.to_str().ok_or(tr().non_utf8)?];
    a.extend(args); // several args = exec without a shell, so no quoting
    tmux(&a).map(|_| ())
}

/// Folders and repos that the user folded in the sidebar.
pub fn folded() -> Vec<PathBuf> {
    fs::read_to_string(state_dir().join("folded")).unwrap_or_default().lines().map(PathBuf::from).collect()
}

pub fn toggle_fold(p: &Path) {
    let _lock = lock();
    let mut f = folded();
    match f.iter().position(|x| x == p) {
        Some(i) => { f.remove(i); }
        None => f.push(p.into()),
    }
    let s: String = f.iter().map(|p| format!("{}\n", p.display())).collect();
    write_state("folded", &s).ok();
}

/// Repos by the folder they are in, both in order of appearance, with a name for each folder: its last part, or the
/// last two if another one ends the same.
pub fn by_folder(repos: &[PathBuf]) -> Vec<(PathBuf, String, Vec<PathBuf>)> {
    let mut groups: Vec<(PathBuf, Vec<PathBuf>)> = vec![];
    for r in repos {
        let dir = r.parent().unwrap_or(r).to_path_buf();
        match groups.iter_mut().find(|g| g.0 == dir) {
            Some(g) => g.1.push(r.clone()),
            None => groups.push((dir, vec![r.clone()])),
        }
    }
    let last = |p: &Path, n: usize| {
        let parts: Vec<_> = p.iter().collect();
        parts[parts.len().saturating_sub(n)..].iter().map(|s| s.to_string_lossy()).collect::<Vec<_>>().join("/")
    };
    let names: Vec<String> = groups.iter().map(|g| last(&g.0, 1)).collect();
    groups.iter().zip(&names).map(|((dir, repos), name)| {
        let name = if *dir == home() { "~".into() } else if names.iter().filter(|n| *n == name).count() > 1 { last(dir, 2) } else { name.clone() };
        (dir.clone(), name, repos.clone())
    }).collect()
}

fn recent() -> Vec<String> {
    fs::read_to_string(state_dir().join("recent")).unwrap_or_default().lines().map(String::from).collect()
}

fn touch_recent(id: &str) {
    let _lock = lock();
    let mut r = recent();
    r.retain(|x| x != id);
    r.insert(0, id.into());
    r.truncate(50);
    write_state("recent", &r.join("\n")).ok();
}

/// Every worktree: the visible one first, then by last use.
pub fn by_recent(visible: &str) -> Vec<Wt> {
    let r = recent();
    let mut wts = load();
    wts.sort_by_key(|w| {
        let id = w.id();
        (id != visible, r.iter().position(|x| *x == id).unwrap_or(usize::MAX))
    });
    wts
}

/// Marks a worktree's pane as working or not, for other processes (only the sidebar watches the screens).
pub fn set_busy(pane: &str, busy: bool) {
    tmux(&["set-option", "-p", "-t", pane, "@orcux_busy", if busy { "1" } else { "0" }]).ok();
}

/// The tab of a worktree where an agent runs: the active one if it does.
pub fn agent_tab(w: &Wt) -> Option<Pane> {
    let mut t: Vec<Pane> = tabs(&w.id()).into_iter().filter(|p| ["claude", "codex"].contains(&tui::program(&p.cmd))).collect();
    t.sort_by_key(|p| !p.on);
    t.into_iter().next()
}

/// Leaves text in an agent's prompt, without Enter. As a paste, not as keys: if a menu is open (a permission
/// request, say), keys could answer it.
pub fn paste_into(pane: &str, text: &str) -> Res<()> {
    tmux(&["set-buffer", "-b", "orcux", "--", text])?;
    tmux(&["paste-buffer", "-p", "-d", "-b", "orcux", "-t", pane]).map(|_| ())
}

/// The worktree on screen, and which of its tabs.
pub fn visible() -> Res<(Wt, u32)> {
    let (pane, id) = terminal_pane().ok_or(tr().no_pane)?;
    let w = load().into_iter().find(|w| w.id() == id).ok_or(tr().no_visible)?;
    let t = tabs(&id).into_iter().find(|p| p.pane == pane).ok_or(tr().no_visible)?;
    Ok((w, t.tab))
}

/// Opens a tab without showing it.
fn new_tab(w: &Wt, cmd: Option<&str>) -> Res<u32> {
    if !w.path().exists() {
        return Err((tr().gone_at)(&w.path().to_string_lossy()));
    }
    let pane = spawn(w)?;
    if let Some(c) = cmd {
        type_into(&pane, c)?;
    }
    Ok(tabs(&w.id()).into_iter().find(|p| p.pane == pane).map_or(1, |p| p.tab))
}

pub fn open_tab(w: &Wt) -> Res<()> {
    let n = new_tab(w, None)?;
    show_tab(w, Some(n))
}

/// Shows the next tab of the visible worktree, or the previous one.
pub fn step_tab(forward: bool) -> Res<()> {
    let (w, n) = visible()?;
    let t = tabs(&w.id());
    let i = t.iter().position(|p| p.tab == n).unwrap_or(0);
    let to = (i + if forward { 1 } else { t.len() - 1 }) % t.len();
    if to == i { Ok(()) } else { show_tab(&w, Some(t[to].tab)) }
}

pub fn close_tab(w: &Wt, n: u32) -> Res<()> {
    let _lock = lock();
    let pane = pane_at(w, Some(n))?;
    let on_screen = terminal_pane().is_some_and(|p| p.0 == pane);
    let t = tabs(&w.id());
    let i = t.iter().position(|p| p.tab == n).unwrap_or(0);
    let neighbor = t.get(i + 1).or(i.checked_sub(1).and_then(|j| t.get(j)));
    if on_screen && let Some(p) = neighbor {
        show_tab(w, Some(p.tab))?; // first, so that the layout never lacks a terminal
    }
    tmux(&["kill-pane", "-t", &pane])?;
    if on_screen && neighbor.is_none() {
        ensure_ui()?;
    }
    set_tab_bar(&tab_bar());
    Ok(())
}

/// The tabs of the visible worktree, as a tmux format for the border above the terminal.
pub fn tab_bar() -> String {
    let Some((pane, id)) = terminal_pane().filter(|p| !p.1.is_empty()) else { return String::new() };
    let (hull, muted, wake, current) = (hex(theme::HULL), hex(theme::MUTED), hex(theme::WAKE), hex(theme::CURRENT));
    let one = |p: &Pane| {
        let cmd = tui::program(&p.cmd).replace('#', "##");
        let (glyph, color) = tui::look_hex(p.look());
        let dot = if p.look() == prs::State::Idle { String::new() } else { format!("#[fg={}]{glyph} ", hex(color)) };
        // one style per #[…]: a comma inside a tmux conditional breaks the whole format
        if p.pane == pane {
            // filled with the accent, on the frame of the terminal
            let dot = if dot.is_empty() { dot } else { format!("#[fg={hull}]{glyph} ") };
            format!("#[bg={current}]#[fg={hull}]#[bold] {} {dot}{cmd} #[default] ", p.tab)
        } else {
            format!("#[bg={wake}]#[fg={muted}] {} {dot}#[fg={muted}]{cmd} #[default] ", p.tab)
        }
    };
    format!(" {}#[fg={muted}]+ ", tabs(&id).iter().map(one).collect::<String>())
}

pub fn set_tab_bar(bar: &str) {
    tmux(&["set-option", "-t", &format!("{UI}:"), "@orcux_bar", bar]).ok();
}

fn attach() -> Res<()> {
    ensure_ui()?;
    if env::var("TMUX").is_ok() {
        tmux(&["switch-client", "-t", UI]).map(|_| ())
    } else {
        Command::new("tmux").args(["attach", "-t", UI]).status().map(|_| ()).map_err(|e| e.to_string())
    }
}

// ---------- cli ----------

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if let Err(e) = cli(&args.iter().map(String::as_str).collect::<Vec<_>>()) {
        eprintln!("orcux: {e}");
        std::process::exit(1);
    }
}

fn flag<'a>(a: &[&'a str], f: &str) -> Option<&'a str> {
    a.iter().position(|x| *x == f).and_then(|i| a.get(i + 1).copied())
}

fn cli(a: &[&str]) -> Res<()> {
    // what a person waits for: a dozen questions to tmux, through one connection
    if matches!(a, ["show", ..] | ["tab", ..] | ["switch"] | ["next"] | ["dialog", ..]) {
        control::borrow();
    }
    match a {
        [] => attach(),
        ["sidebar"] => tui::run(),
        ["hook", event, rest @ ..] => {
            agent::hook(event, rest);
            Ok(())
        }
        ["run", rest @ ..] => work::run_task(rest),
        ["agents", rest @ ..] => work::agents_cmd(rest),
        ["wait", rest @ ..] => std::process::exit(work::wait(rest)?),
        ["next"] => work::next(),
        ["trust", rest @ ..] => config::trust(&rest.first().map(PathBuf::from).unwrap_or(env::current_dir().map_err(|e| e.to_string())?)),
        ["ports"] => {
            config::ports(None);
            Ok(())
        }
        ["ports", q] => {
            config::ports(Some(&find(Some(q))?));
            Ok(())
        }
        ["merge", rest @ ..] => merge::cmd(rest),
        ["conflicts", rest @ ..] => std::process::exit(merge::conflicts(rest.first().copied())?),
        ["restore", rest @ ..] => restore::cmd(rest),
        ["dialog", "merge", id] => tui::dialog_merge(&find(Some(id))?),
        ["skill"] => {
            print!("{}", agent::SKILL);
            Ok(())
        }
        ["notify"] => {
            println!("{} ({})", work::notify_mode(), work::NOTIFY.join(", "));
            Ok(())
        }
        ["notify", mode] if work::NOTIFY.contains(mode) => work::set_notify(mode),
        ["rail"] => tui::rail(),
        ["welcome"] => tui::welcome(),
        ["prs"] => prs::print(),
        ["doctor"] => doctor::run_all(),
        ["--version" | "-V" | "version"] => {
            println!("orcux {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        ["switch"] => tui::switch(),
        ["commands"] => tui::commands(),
        ["dialog", "new", repo] => tui::dialog_new(Path::new(repo), None),
        ["dialog", "child", id] => {
            let w = find(Some(id))?;
            tui::dialog_new(&w.repo, Some(&w))
        }
        ["dialog", "add", base] => tui::dialog_add(Path::new(base)),
        ["dialog", "rmrepo", repo] => tui::dialog_rm_repo(Path::new(repo)),
        ["dialog", "rm", id] => tui::dialog_rm(&find(Some(id))?),
        ["dialog", "pr", repo, branch, label] => tui::dialog_pr(Path::new(repo), branch, label),
        ["dialog", "view", url] => tui::dialog_view(url),
        ["dialog", "diff", dir, path] => files::show(Path::new(dir), path),
        ["dialog", "close"] => tui::dialog_close(),
        ["dialog", "error", text] => tui::dialog_error(text),
        ["new", name, rest @ ..] => {
            let parent = flag(rest, "--parent").map(|q| find(Some(q))).transpose()?;
            let repo = match &parent {
                Some(p) => p.repo.clone(),
                None => flag(rest, "--repo").map(PathBuf::from).unwrap_or(env::current_dir().map_err(|e| e.to_string())?),
            };
            let w = create_from(&repo, name, flag(rest, "--cmd"), parent.as_ref())?;
            tell_untrusted(&w);
            println!("{}  {}  ORCUX_PORT={}", w.id(), w.path().display(), w.port());
            Ok(())
        }
        ["rm", q, rest @ ..] => remove(&find(Some(q))?, rest.contains(&"--force")),
        ["import", rest @ ..] if rest.iter().any(|p| *p != "--yes") => {
            let paths: Vec<&str> = rest.iter().copied().filter(|p| *p != "--yes").collect();
            import(&paths, rest.contains(&"--yes"))
        }
        ["icons"] => {
            println!("{}", (tr().icons_now)(theme::icons(), &theme::ICONS.join(", ")));
            Ok(())
        }
        ["icons", which] if theme::ICONS.contains(which) => {
            theme::set_icons(which)?;
            restart_panes()
        }
        ["lang"] => {
            println!("{}", (tr().lang_now)(i18n::code(), &i18n::codes()));
            Ok(())
        }
        ["lang", code] => {
            let text = i18n::set(code)?;
            println!("{}", (text.lang_now)(code, &i18n::codes()));
            restart_panes()
        }
        ["ls"] => work::ls(false),
        ["ls", "--json"] => work::ls(true),
        ["show", q] => {
            let (w, n) = target(q)?;
            show_tab(&w, n)
        }
        ["tab", "new"] => open_tab(&visible()?.0),
        ["tab", "new", q, rest @ ..] => {
            let w = find(Some(q))?;
            println!("{}:{}", w.id(), new_tab(&w, flag(rest, "--cmd"))?);
            Ok(())
        }
        ["tab", "ls", q] => {
            for p in tabs(&find(Some(q))?.id()) {
                println!("{}:{:<3} {} {}", p.wt, p.tab, if p.on { "*" } else { " " }, tui::program(&p.cmd));
            }
            Ok(())
        }
        ["tab", "close", q] => {
            let (w, n) = target(q)?;
            close_tab(&w, n.ok_or(tr().which_tab)?)
        }
        ["tab", "next"] => step_tab(true),
        ["tab", "prev"] => step_tab(false),
        ["send", q, text @ ..] if !text.is_empty() => {
            let (w, n) = target(q)?;
            type_into(&pane_at(&w, n)?, &text.join(" "))
        }
        ["read", q, rest @ ..] => {
            let n = rest.first().and_then(|n| n.parse::<u32>().ok()).unwrap_or(40);
            let (w, tab) = target(q)?;
            let pane = match tab {
                Some(_) => pane_at(&w, tab)?,
                None => pane_of(&w.id()).ok_or(tr().not_running)?,
            };
            println!("{}", tmux(&["capture-pane", "-p", "-J", "-t", &pane, "-S", &format!("-{n}")])?.trim_end());
            Ok(())
        }
        ["env", rest @ ..] => {
            // quoted: one of them is JSON
            for e in find(rest.first().copied())?.env() {
                let (k, v) = e.split_once('=').unwrap_or((&e, ""));
                let plain = v.chars().all(|c| c.is_ascii_alphanumeric() || "/._:-@+".contains(c));
                println!("export {k}={}", if plain { v.into() } else { format!("'{}'", v.replace('\'', "'\\''")) });
            }
            Ok(())
        }
        ["repo", "rm", rest @ ..] => {
            let p = rest.first().map(PathBuf::from).unwrap_or(env::current_dir().map_err(|e| e.to_string())?);
            let p = p.canonicalize().unwrap_or(p); // it may be gone from disk
            if !repos().contains(&p) {
                return Err((tr().path_missing)(&p.to_string_lossy()));
            }
            remove_repo(&p)
        }
        ["repo", "add", rest @ ..] => {
            let p = rest.first().map(PathBuf::from).unwrap_or(env::current_dir().map_err(|e| e.to_string())?);
            println!("{}", add_repo(&p)?.display());
            Ok(())
        }
        _ => {
            print!("{}", help());
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders() {
        let p = |l: &[&str]| l.iter().map(PathBuf::from).collect::<Vec<_>>();
        let got = by_folder(&p(&["/w/acme/a", "/w/own/b", "/w/acme/c", "/z/own/d"]));
        let names: Vec<(&str, usize)> = got.iter().map(|g| (g.1.as_str(), g.2.len())).collect();
        assert_eq!(names, [("acme", 2), ("w/own", 1), ("z/own", 1)]);
    }

    #[test]
    fn trees() {
        let w = |name: &str, parent: Option<&str>| Wt { repo: "/r".into(), name: name.into(), slot: 1, at: None, parent: parent.map(String::from) };
        let all = [w("a", None), w("b", None), w("a2", Some("a")), w("orphan", Some("gone")), w("a1", Some("a")), w("a2x", Some("a2"))];
        let got: Vec<(String, Vec<bool>)> = tree(&all.iter().collect::<Vec<_>>()).into_iter().map(|(w, t)| (w.name, t)).collect();
        let want = [("a", vec![false]), ("a2", vec![false, false]), ("a2x", vec![false, false, true]), ("a1", vec![false, true]), ("b", vec![false]), ("orphan", vec![true])];
        assert_eq!(got, want.map(|(n, t)| (n.to_string(), t)));
    }
}
