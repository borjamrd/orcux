mod i18n;
mod prs;
mod theme;
mod tui;

use std::{env, fs, path::{Path, PathBuf}, process::Command, thread, time::Duration};

use i18n::tr;
use theme::hex;

pub type Res<T> = Result<T, String>;

pub const UI: &str = "=orcux"; // `=` = exact match; plain "orcux" would prefix-match "orcux-bg"
const UI_NAME: &str = "orcux";
const BG: &str = "orcux-bg"; // hidden session: one window per worktree terminal
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
    /// runs for every worktree on every redraw.
    pub fn branch(&self) -> Option<String> {
        let link = fs::read_to_string(self.path().join(".git")).ok()?;
        let dir = self.path().join(link.strip_prefix("gitdir:")?.trim());
        let head = fs::read_to_string(dir.join("HEAD")).ok()?;
        Some(head.trim().strip_prefix("ref: refs/heads/")?.into())
    }
    pub fn port(&self) -> u32 {
        10000 + self.slot * 100
    }
    fn env(&self) -> Vec<String> {
        let compose: String = self.id().to_lowercase().chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
        vec![
            format!("ORCUX_ID={}", self.id()),
            format!("ORCUX_PORT={}", self.port()),
            format!("ORCUX_SLOT={}", self.slot),
            format!("COMPOSE_PROJECT_NAME={compose}"),
        ]
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
    run("tmux", args, None)
}

pub fn git(dir: &Path, args: &[&str]) -> Res<String> {
    run("git", args, Some(dir))
}

// ---------- state: two plain files, no db ----------

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
    fs::write(state_dir().join("worktrees.tsv"), s).map_err(|e| e.to_string())
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
    let mut r = repos();
    if !r.contains(&main) {
        r.push(main.clone());
        save_repos(&r)?;
    }
    Ok(main)
}

fn save_repos(r: &[PathBuf]) -> Res<()> {
    let s: String = r.iter().map(|p| format!("{}\n", p.display())).collect();
    fs::write(state_dir().join("repos"), s).map_err(|e| e.to_string())
}

/// Takes a repo off the sidebar; nothing is deleted. Its worktrees must have left before.
pub fn remove_repo(repo: &Path) -> Res<()> {
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
    let repo = add_repo(repo)?;
    let mut wts = load();
    if wts.iter().any(|w| w.repo == repo && w.name == name) {
        return Err((tr().exists)(name));
    }
    let slot = free_slot(&wts);
    let wt = Wt { repo: repo.clone(), name: name.into(), slot, at: None, parent: parent.map(|p| p.name.clone()) };
    let path = wt.path();
    let p = path.to_str().ok_or(tr().non_utf8)?;
    let has = |r: &str| git(&repo, &["rev-parse", "--verify", "--quiet", r]).is_ok();
    let exists = has(&format!("refs/heads/{name}")) || has(&format!("refs/remotes/origin/{name}"));
    if let Some(parent) = parent {
        if exists {
            return Err((tr().branch_exists)(name));
        }
        let base = git(&parent.path(), &["rev-parse", "HEAD"])?;
        git(&repo, &["worktree", "add", "--no-track", "-b", name, p, &base])?;
    } else if exists {
        git(&repo, &["worktree", "add", p, name])?;
    } else {
        let base = if has("refs/remotes/origin/HEAD") { "origin/HEAD" } else { "HEAD" };
        git(&repo, &["worktree", "add", "--no-track", "-b", name, p, base])?;
    }
    copy_includes(&repo, &path)?;
    wts.push(wt.clone());
    save(&wts)?;
    let pane = spawn(&wt)?;
    if let Some(c) = cmd {
        type_into(&pane, c)?;
    }
    Ok(wt)
}

/// Same contract as Orca: copy the local-only files listed in the main checkout's .worktreeinclude.
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
    let mut wts = load();
    let mut n = 0;
    for p in paths {
        match importable(Path::new(p), &wts) {
            Ok(mut w) => {
                w.slot = free_slot(&wts);
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
    if yes {
        save(&wts)?;
        println!("{}", (tr().imported)(n));
    } else {
        println!("{}", (tr().to_import)(n));
    }
    Ok(())
}

pub fn remove(w: &Wt, force: bool) -> Res<()> {
    let path = w.path();
    if w.at.is_some() {
        // imported: not ours to delete, not even with --force
    } else if path.exists() {
        let p = path.to_str().unwrap();
        let args: &[&str] = if force { &["worktree", "remove", "--force", p] } else { &["worktree", "remove", p] };
        git(&w.repo, args)?; // first: a dirty worktree fails here, before we touch its terminal
    } else {
        git(&w.repo, &["worktree", "prune"])?;
    }
    for p in tabs(&w.id()) {
        tmux(&["kill-pane", "-t", &p.pane])?;
    }
    let mut left: Vec<Wt> = load().into_iter().filter(|x| x != w).collect();
    for child in left.iter_mut().filter(|x| x.repo == w.repo && x.parent.as_ref() == Some(&w.name)) {
        child.parent = w.parent.clone(); // up one level
    }
    save(&left)?;
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
}

pub fn panes() -> Vec<Pane> {
    let f = "#{pane_id}\t#{@orcux}\t#{@orcux_tab}\t#{@orcux_on}\t#{@orcux_busy}\t#{pane_current_command}";
    tmux(&["list-panes", "-a", "-F", f]).unwrap_or_default().lines()
        .map(|l| {
            let mut p = l.split('\t').map(String::from);
            let mut next = || p.next().unwrap_or_default();
            Pane { pane: next(), wt: next(), tab: next().parse().unwrap_or(1), on: next() == "1", busy: next() == "1", cmd: next() }
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
    let (id, path) = (w.id(), w.path().to_string_lossy().to_string());
    let taken: Vec<u32> = tabs(&id).iter().map(|p| p.tab).collect();
    let tab = (1..).find(|n| !taken.contains(n)).unwrap().to_string();
    let mut a: Vec<String> = if tmux(&["has-session", "-t", &format!("={BG}")]).is_ok() {
        vec!["new-window".into(), "-d".into(), "-t".into(), format!("={BG}:")]
    } else {
        vec!["new-session".into(), "-d".into(), "-s".into(), BG.into()]
    };
    a.extend(["-n".into(), id.clone(), "-c".into(), path, "-P".into(), "-F".into(), "#{pane_id}".into()]);
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
    tmux(&["send-keys", "-t", pane, "-l", text])?;
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
    let home = home().to_string_lossy().to_string();
    if tmux(&["has-session", "-t", UI]).is_err() {
        tmux(&["new-session", "-d", "-s", UI_NAME, "-c", &home])?;
        tmux(&["set-option", "-t", &format!("{UI}:"), "mouse", "on"])?;
    }
    let side = match ui_panes().into_iter().find(|p| p.1 == SIDEBAR) {
        Some(p) => p.0,
        None => {
            let exe = env::current_exe().map_err(|e| e.to_string())?;
            let cmd = format!("'{}' sidebar", exe.display());
            let p = tmux(&["split-window", "-hb", "-l", "36", "-t", &format!("{UI}:"), "-P", "-F", "#{pane_id}", &cmd])?;
            tmux(&["set-option", "-p", "-t", &p, "@orcux", SIDEBAR])?;
            p
        }
    };
    let missing = terminal_pane().is_none();
    if missing {
        tmux(&["split-window", "-h", "-d", "-t", &side, "-c", &home])?;
    }
    pin(missing)
}

fn ui_pane(mark: &str) -> Option<String> {
    ui_panes().into_iter().find(|p| p.1 == mark).map(|p| p.0)
}

/// (pane, marker) of the pane that shows a worktree's terminal, or the placeholder shell.
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
    match env::current_exe() {
        Ok(exe) => Command::new(exe).args(env::args().skip(1)).exec().to_string(),
        Err(e) => e.to_string(),
    }
}

/// Colors and status bar of the orcux session only; the user's other sessions keep theirs.
pub fn style_ui() -> Res<()> {
    let t = format!("{UI}:");
    let (abyss, hull, line, foam, muted) = (hex(theme::ABYSS), hex(theme::HULL), hex(theme::LINE), hex(theme::FOAM), hex(theme::MUTED));
    let status = format!("bg={abyss},fg={foam}");
    for (k, v) in [("status", "on"), ("status-style", status.as_str()), ("status-left-length", "120"), ("status-right-length", "80")] {
        tmux(&["set-option", "-t", &t, k, v])?;
    }
    let (gap, card) = (format!("bg={abyss},fg={abyss}"), format!("bg={hull}"));
    let rail = format!("#[bg={hull}#,fg={foam}#,bold]  {}  #[default]", tr().your_prs);
    let name = format!("#[fg={muted}]orcux");
    let titles = format!("#{{?#{{==:#{{@orcux}},{SIDEBAR}}},{name},#{{?#{{==:#{{@orcux}},{RAIL}}},{rail},#{{@orcux_bar}}}}}}");
    for (k, v) in [
        ("window-status-format", ""),
        ("window-status-current-format", ""),
        ("pane-border-lines", "spaces"), // gaps instead of lines
        ("pane-border-style", gap.as_str()),
        ("pane-active-border-style", gap.as_str()),
        ("window-style", card.as_str()),
        ("pane-border-status", "top"),
        ("pane-border-format", titles.as_str()),
    ] {
        tmux(&["set-option", "-w", "-t", &t, k, v])?;
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
    ] {
        for k in keys {
            tmux(&["bind-key", "-n", k, "if-shell", "-F", "#{==:#{session_name},orcux}", &cmd, &format!("send-keys {k}")])?;
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
    ensure_ui()?;
    let pane = pane_at(w, tab)?;
    let (right, mark) = terminal_pane().ok_or(tr().no_pane)?;
    if pane != right {
        tmux(&["swap-pane", "-d", "-s", &pane, "-t", &right])?;
        if mark.is_empty() {
            tmux(&["kill-pane", "-t", &right])?; // placeholder shell, not a worktree
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
    let size = tmux(&["display-message", "-p", "#{client_width} #{client_height}"])?;
    let (w, h) = size.split_once(' ').unwrap_or_default();
    let fit = |have: &str, want: u32| have.parse().map_or(want, |n: u32| n.min(want)).to_string();
    let (w, h, title) = (fit(w, want.0), fit(h, want.1), format!("#[fg={}] {title} ", hex(theme::FOAM)));
    let (style, edge) = (format!("bg={},fg={}", hex(theme::HULL), hex(theme::FOAM)), format!("bg={},fg={}", hex(theme::HULL), hex(theme::LINE)));
    let mut a = vec!["display-popup", "-E", "-b", "rounded", "-s", &style, "-S", &edge, "-T", &title, "-w", &w, "-h", &h, exe.to_str().ok_or(tr().non_utf8)?];
    a.extend(args); // several args = exec without a shell, so no quoting
    tmux(&a).map(|_| ())
}

fn recent() -> Vec<String> {
    fs::read_to_string(state_dir().join("recent")).unwrap_or_default().lines().map(String::from).collect()
}

fn touch_recent(id: &str) {
    let mut r = recent();
    r.retain(|x| x != id);
    r.insert(0, id.into());
    r.truncate(50);
    fs::write(state_dir().join("recent"), r.join("\n")).ok();
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
    let one = |p: &Pane| {
        let cmd = tui::program(&p.cmd).replace('#', "##");
        let (hull, foam, muted) = (hex(theme::HULL), hex(theme::FOAM), hex(theme::MUTED));
        let dot = if p.busy { format!("#[fg={}]● ", hex(theme::CURRENT)) } else { String::new() };
        if p.pane == pane {
            // same fill as the terminal below it: they read as one card
            format!("#[bg={hull},fg={muted}]  {} {dot}#[fg={foam},bold]{cmd}  #[default]", p.tab)
        } else {
            format!("#[fg={muted}]  {} {dot}#[fg={muted}]{cmd}  #[default]", p.tab)
        }
    };
    tabs(&id).iter().map(one).collect::<String>() + &format!("#[fg={}] +", hex(theme::LINE))
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
    match a {
        [] => attach(),
        ["sidebar"] => tui::run(),
        ["rail"] => tui::rail(),
        ["prs"] => prs::print(),
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
        ["dialog", "close"] => tui::dialog_close(),
        ["dialog", "error", text] => tui::dialog_error(text),
        ["new", name, rest @ ..] => {
            let parent = flag(rest, "--parent").map(|q| find(Some(q))).transpose()?;
            let repo = match &parent {
                Some(p) => p.repo.clone(),
                None => flag(rest, "--repo").map(PathBuf::from).unwrap_or(env::current_dir().map_err(|e| e.to_string())?),
            };
            let w = create_from(&repo, name, flag(rest, "--cmd"), parent.as_ref())?;
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
        ["ls"] => {
            let live: Vec<String> = panes().into_iter().map(|p| p.wt).collect();
            for w in load() {
                let st = if live.contains(&w.id()) { "up" } else { "-" };
                println!("{:<40} :{:<6} {:<3} {}", w.id(), w.port(), st, w.path().display());
            }
            Ok(())
        }
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
            find(rest.first().copied())?.env().iter().for_each(|e| println!("export {e}"));
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
    fn trees() {
        let w = |name: &str, parent: Option<&str>| Wt { repo: "/r".into(), name: name.into(), slot: 1, at: None, parent: parent.map(String::from) };
        let all = [w("a", None), w("b", None), w("a2", Some("a")), w("orphan", Some("gone")), w("a1", Some("a")), w("a2x", Some("a2"))];
        let got: Vec<(String, Vec<bool>)> = tree(&all.iter().collect::<Vec<_>>()).into_iter().map(|(w, t)| (w.name, t)).collect();
        let want = [("a", vec![false]), ("a2", vec![false, false]), ("a2x", vec![false, false, true]), ("a1", vec![false, true]), ("b", vec![false]), ("orphan", vec![true])];
        assert_eq!(got, want.map(|(n, t)| (n.to_string(), t)));
    }
}
