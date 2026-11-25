mod tui;

use std::{env, fs, path::{Path, PathBuf}, process::Command, thread, time::Duration};

pub type Res<T> = Result<T, String>;

pub const UI: &str = "=orcux"; // `=` = exact match; plain "orcux" would prefix-match "orcux-bg"
const UI_NAME: &str = "orcux";
const BG: &str = "orcux-bg"; // hidden session: one window per worktree terminal
pub const SIDEBAR: &str = "__sidebar";

/// Single source for `orcux help` and the `?` dialog: every new command or shortcut gets its line here.
pub const COMMANDS: &[(&str, &[(&str, &str)])] = &[
    ("Sidebar", &[
        ("⏎", "abrir el worktree seleccionado"),
        ("n", "nuevo worktree en el repo seleccionado"),
        ("d", "borrar el worktree seleccionado"),
        ("j / k, ↓ / ↑", "moverse por la lista"),
        ("?", "ver los comandos disponibles"),
        ("q", "salir (los terminales siguen vivos)"),
    ]),
    ("Terminal", &[
        ("orcux", "abrir la interfaz (sidebar + terminal)"),
        ("orcux new <nombre> [--repo P] [--cmd C]", "crear worktree, puertos y terminal"),
        ("orcux rm <wt> [--force]", "borrar el worktree (conserva la rama)"),
        ("orcux ls", "listar los worktrees"),
        ("orcux show <wt>", "mostrar <wt> en la interfaz"),
        ("orcux send <wt> <texto...>", "escribir texto + Enter en su terminal"),
        ("orcux read <wt> [líneas]", "leer el final de su terminal (40 por defecto)"),
        ("orcux env [wt]", "exports ORCUX_* (por defecto, el del cwd)"),
        ("orcux repo add [ruta]", "registrar un repo en el sidebar"),
        ("orcux commands", "abrir esta lista de comandos"),
        ("orcux help", "imprimir esta ayuda"),
    ]),
    ("Notas", &[
        ("<wt>", "repo/nombre, o solo el nombre si es único"),
        ("ORCUX_PORT", "10000 + slot*100: 100 puertos por worktree"),
        ("COMPOSE_PROJECT_NAME", "propio de cada worktree"),
    ]),
];

fn help() -> String {
    let mut s = String::from("orcux: worktrees + tmux\n");
    for (group, cmds) in COMMANDS {
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
}

impl Wt {
    pub fn repo_name(&self) -> String {
        self.repo.file_name().unwrap_or_default().to_string_lossy().into()
    }
    pub fn id(&self) -> String {
        format!("{}/{}", self.repo_name(), self.name)
    }
    pub fn path(&self) -> PathBuf {
        home().join("orcux").join(self.repo_name()).join(&self.name)
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

fn state_dir() -> PathBuf {
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

fn git(dir: &Path, args: &[&str]) -> Res<String> {
    run("git", args, Some(dir))
}

// ---------- state: two plain files, no db ----------

pub fn load() -> Vec<Wt> {
    fs::read_to_string(state_dir().join("worktrees.tsv")).unwrap_or_default().lines()
        .filter_map(|l| {
            let mut p = l.split('\t');
            Some(Wt { repo: p.next()?.into(), name: p.next()?.into(), slot: p.next()?.parse().ok()? })
        })
        .collect()
}

fn save(wts: &[Wt]) -> Res<()> {
    let s: String = wts.iter().map(|w| format!("{}\t{}\t{}\n", w.repo.display(), w.name, w.slot)).collect();
    fs::write(state_dir().join("worktrees.tsv"), s).map_err(|e| e.to_string())
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
    let main = Path::new(&common).parent().ok_or("not a repo")?.to_path_buf();
    let mut r = repos();
    if !r.contains(&main) {
        r.push(main.clone());
        let s: String = r.iter().map(|p| format!("{}\n", p.display())).collect();
        fs::write(state_dir().join("repos"), s).map_err(|e| e.to_string())?;
    }
    Ok(main)
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
        [] => Err(format!("no worktree {}", q.unwrap_or("for cwd"))),
        _ => Err("ambiguous, use repo/name".into()),
    }
}

// ---------- worktrees ----------

pub fn create(repo: &Path, name: &str, cmd: Option<&str>) -> Res<Wt> {
    let repo = add_repo(repo)?;
    let mut wts = load();
    if wts.iter().any(|w| w.repo == repo && w.name == name) {
        return Err(format!("{name} already exists"));
    }
    let slot = (1..).find(|s| !wts.iter().any(|w| w.slot == *s)).unwrap();
    let wt = Wt { repo: repo.clone(), name: name.into(), slot };
    let path = wt.path();
    let p = path.to_str().ok_or("non-utf8 path")?;
    let has = |r: &str| git(&repo, &["rev-parse", "--verify", "--quiet", r]).is_ok();
    if has(&format!("refs/heads/{name}")) || has(&format!("refs/remotes/origin/{name}")) {
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

pub fn remove(w: &Wt, force: bool) -> Res<()> {
    let path = w.path();
    if path.exists() {
        let p = path.to_str().unwrap();
        let args: &[&str] = if force { &["worktree", "remove", "--force", p] } else { &["worktree", "remove", p] };
        git(&w.repo, args)?; // first: a dirty worktree fails here, before we touch its terminal
    } else {
        git(&w.repo, &["worktree", "prune"])?;
    }
    if let Some(p) = pane_of(&w.id()) {
        tmux(&["kill-pane", "-t", &p])?;
    }
    save(&load().into_iter().filter(|x| x != w).collect::<Vec<_>>())?;
    if tmux(&["has-session", "-t", UI]).is_ok() {
        ensure_ui()?; // the killed pane may have been the visible one
    }
    Ok(())
}

// ---------- tmux ----------

/// (pane_id, @wt marker, current command) for every pane on the server.
pub fn panes() -> Vec<(String, String, String)> {
    tmux(&["list-panes", "-a", "-F", "#{pane_id}\t#{@orcux}\t#{pane_current_command}"]).unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let mut p = l.split('\t');
            Some((p.next()?.into(), p.next()?.into(), p.next().unwrap_or("").into()))
        })
        .collect()
}

fn pane_of(id: &str) -> Option<String> {
    panes().into_iter().find(|p| p.1 == id).map(|p| p.0)
}

fn spawn(w: &Wt) -> Res<String> {
    let (id, path) = (w.id(), w.path().to_string_lossy().to_string());
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
    tmux(&["set-option", "-p", "-t", &pane, "@orcux", &id])?;
    Ok(pane)
}

fn ensure_pane(w: &Wt) -> Res<String> {
    match pane_of(&w.id()) {
        Some(p) => Ok(p),
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
    if ui_panes().len() < 2 {
        tmux(&["split-window", "-h", "-d", "-t", &side, "-c", &home])?;
        tmux(&["resize-pane", "-t", &side, "-x", "36"])?;
    }
    Ok(())
}

pub fn show(w: &Wt) -> Res<()> {
    ensure_ui()?;
    let pane = ensure_pane(w)?;
    let (right, mark) = ui_panes().into_iter().find(|p| p.1 != SIDEBAR).ok_or("no terminal pane")?;
    if pane != right {
        tmux(&["swap-pane", "-d", "-s", &pane, "-t", &right])?;
        if mark.is_empty() {
            tmux(&["kill-pane", "-t", &right])?; // placeholder shell, not a worktree
        }
    }
    tmux(&["select-pane", "-t", &pane]).map(|_| ())
}

/// Runs `orcux <args>` in a tmux popup centered over the whole UI (the sidebar alone is too narrow). Blocks until closed.
pub fn popup(title: &str, want: (u32, u32), args: &[&str]) -> Res<()> {
    let exe = env::current_exe().map_err(|e| e.to_string())?;
    // tmux rejects a popup larger than the client instead of clamping it
    let size = tmux(&["display-message", "-p", "#{client_width} #{client_height}"])?;
    let (w, h) = size.split_once(' ').unwrap_or_default();
    let fit = |have: &str, want: u32| have.parse().map_or(want, |n: u32| n.min(want)).to_string();
    let (w, h, title) = (fit(w, want.0), fit(h, want.1), format!(" {title} "));
    let mut a = vec!["display-popup", "-E", "-T", &title, "-w", &w, "-h", &h, exe.to_str().ok_or("non-utf8 path")?];
    a.extend(args); // several args = exec without a shell, so no quoting
    tmux(&a).map(|_| ())
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
        ["commands"] => tui::commands(),
        ["dialog", "new", repo] => tui::dialog_new(Path::new(repo)),
        ["dialog", "rm", id] => tui::dialog_rm(&find(Some(id))?),
        ["dialog", "error", text] => tui::dialog_error(text),
        ["new", name, rest @ ..] => {
            let repo = flag(rest, "--repo").map(PathBuf::from).unwrap_or(env::current_dir().map_err(|e| e.to_string())?);
            let w = create(&repo, name, flag(rest, "--cmd"))?;
            println!("{}  {}  ORCUX_PORT={}", w.id(), w.path().display(), w.port());
            Ok(())
        }
        ["rm", q, rest @ ..] => remove(&find(Some(q))?, rest.contains(&"--force")),
        ["ls"] => {
            let live: Vec<String> = panes().into_iter().map(|p| p.1).collect();
            for w in load() {
                let st = if live.contains(&w.id()) { "up" } else { "-" };
                println!("{:<40} :{:<6} {:<3} {}", w.id(), w.port(), st, w.path().display());
            }
            Ok(())
        }
        ["show", q] => show(&find(Some(q))?),
        ["send", q, text @ ..] if !text.is_empty() => type_into(&ensure_pane(&find(Some(q))?)?, &text.join(" ")),
        ["read", q, rest @ ..] => {
            let n = rest.first().and_then(|n| n.parse::<u32>().ok()).unwrap_or(40);
            let pane = pane_of(&find(Some(q))?.id()).ok_or("terminal not running")?;
            println!("{}", tmux(&["capture-pane", "-p", "-J", "-t", &pane, "-S", &format!("-{n}")])?.trim_end());
            Ok(())
        }
        ["env", rest @ ..] => {
            find(rest.first().copied())?.env().iter().for_each(|e| println!("export {e}"));
            Ok(())
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
