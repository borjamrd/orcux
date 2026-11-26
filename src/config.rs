//! `.orcux` in the main checkout of a repo: what its worktrees need. It can run commands, so nothing in it counts until
//! the person has read it (`orcux trust`), and again whenever it changes.
use crate::*;
use std::{
    hash::{DefaultHasher, Hash, Hasher},
    io::IsTerminal,
    net::TcpListener,
};

#[derive(Default, Debug, PartialEq)]
pub struct Config {
    /// commands for a new worktree, in its terminal
    pub setup: Vec<String>,
    /// commands before it is deleted
    pub teardown: Vec<String>,
    /// files or folders of the main checkout, as links
    pub link: Vec<String>,
    /// name of a variable, and which of the ports of the worktree it is
    pub ports: Vec<(String, u32)>,
    pub env: Vec<(String, String)>,
    /// Compose files of the repo, whose ports each worktree publishes on its own (`compose.rs`)
    pub compose: Vec<String>,
}

pub enum Trust {
    NoFile,
    Yes(Config),
    No,
}

fn is_name(s: &str) -> bool {
    s.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !s.starts_with("ORCUX")
}

pub fn parse(text: &str) -> Res<Config> {
    let mut c = Config::default();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let bad = || (tr().config_bad)(i + 1, line);
        let (key, value) = line.split_once('=').ok_or_else(bad)?;
        let (key, value) = (key.trim(), value.trim().to_string());
        let mut words = key.split_whitespace();
        match (words.next(), words.next(), words.next()) {
            (Some("setup"), None, _) => c.setup.push(value),
            (Some("teardown"), None, _) => c.teardown.push(value),
            (Some("link"), None, _) => {
                let inside = !value.is_empty() && Path::new(&value).components().all(|p| matches!(p, std::path::Component::Normal(_)));
                c.link.push(if inside { value } else { return Err(bad()) });
            }
            (Some("port"), Some(name), None) if is_name(name) => {
                let n = value.parse().ok().filter(|n| *n < 100).ok_or_else(bad)?;
                c.ports.push((name.into(), n));
            }
            (Some("env"), Some(name), None) if is_name(name) => c.env.push((name.into(), value)),
            (Some("compose"), None, _) => {
                let inside = !value.is_empty() && Path::new(&value).components().all(|p| matches!(p, std::path::Component::Normal(_)));
                c.compose.push(if inside { value } else { return Err(bad()) });
            }
            _ => return Err(bad()),
        }
    }
    Ok(c)
}

fn text_of(repo: &Path) -> Option<String> {
    fs::read_to_string(repo.join(".orcux")).ok()
}

/// What was approved, kept whole: a hash could be matched on purpose.
fn approved(repo: &Path) -> PathBuf {
    let mut h = DefaultHasher::new();
    repo.hash(&mut h);
    state_dir().join("trusted").join(format!("{:016x}", h.finish()))
}

fn record(repo: &Path, text: &str) -> String {
    format!("{}\n{text}", repo.display())
}

pub fn of(repo: &Path) -> Res<Trust> {
    let Some(text) = text_of(repo) else { return Ok(Trust::NoFile) };
    match fs::read_to_string(approved(repo)) {
        Ok(t) if t == record(repo, &text) => Ok(Trust::Yes(parse(&text)?)),
        _ => Ok(Trust::No),
    }
}

/// Only what was approved.
pub fn trusted(repo: &Path) -> Config {
    match of(repo) {
        Ok(Trust::Yes(c)) => c,
        _ => Config::default(),
    }
}

/// Characters that could show a line other than it is.
fn hides(c: char) -> bool {
    c.is_control() || matches!(c, '\u{ad}' | '\u{61c}' | '\u{180e}' | '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}' | '\u{fff9}'..='\u{fffb}' | '\u{e0000}'..='\u{e007f}')
}

/// Asked at a terminal, so that it is not approved in passing. An agent that types into other terminals could still do
/// it: it can run any command anyway.
pub fn trust(repo: &Path) -> Res<()> {
    let main = git(repo, &["rev-parse", "--path-format=absolute", "--git-common-dir"]).map_err(|_| tr().not_repo)?;
    let repo = Path::new(&main).parent().ok_or(tr().not_repo)?.to_path_buf();
    let text = text_of(&repo).ok_or_else(|| (tr().config_none)(&tilde(&repo)))?;
    parse(&text)?;
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err(tr().trust_person.into());
    }
    println!("{}\n", tilde(&repo.join(".orcux")));
    // no control characters: they could hide lines from the person
    text.lines().for_each(|l| println!("  {}", l.chars().map(|c| if hides(c) { '�' } else { c }).collect::<String>()));
    println!("\n{}", tr().trust_ask);
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer).map_err(|e| e.to_string())?;
    if !["y", "s", "yes", "si", "sí"].contains(&answer.trim().to_lowercase().as_str()) {
        return Err(tr().trust_no.into());
    }
    let file = approved(&repo);
    fs::create_dir_all(file.parent().unwrap()).and_then(|_| fs::write(file, record(&repo, &text))).map_err(|e| e.to_string())
}

/// ${NAME}, for the names known so far; others stay as they are.
fn expand_vars(value: &str, known: &[(String, String)]) -> String {
    known.iter().fold(value.to_string(), |v, (k, x)| v.replace(&format!("${{{k}}}"), x))
}

/// The variables that the file adds to those of orcux.
pub fn variables(c: &Config, port: u32, known: &[(String, String)]) -> Vec<(String, String)> {
    let mut all = known.to_vec();
    let mine = all.len();
    all.extend(c.ports.iter().map(|(k, n)| (k.clone(), (port + n).to_string())));
    for (k, v) in &c.env {
        let v = expand_vars(v, &all);
        all.push((k.clone(), v));
    }
    all.split_off(mine)
}

/// Links to the main checkout. Git must not see them: an agent would commit them, and the worktree would look changed.
pub fn link(w: &Wt, c: &Config) -> Res<()> {
    for l in &c.link {
        let (from, to) = (w.repo.join(l), w.path().join(l));
        if !from.exists() || to.symlink_metadata().is_ok() {
            continue;
        }
        fs::create_dir_all(to.parent().unwrap()).and_then(|_| std::os::unix::fs::symlink(&from, &to)).map_err(|e| format!("{l}: {e}"))?;
        if git(&w.path(), &["check-ignore", "-q", "--", l]).is_err() {
            let common = git(&w.repo, &["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
            let file = Path::new(&common).join("info/exclude");
            let old = fs::read_to_string(&file).unwrap_or_default();
            let gap = if old.is_empty() || old.ends_with('\n') { "" } else { "\n" };
            fs::create_dir_all(file.parent().unwrap()).and_then(|_| fs::write(&file, format!("{old}{gap}/{l}\n"))).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// What to type in the terminal of a new worktree to prepare it.
pub fn setup(w: &Wt, c: &Config) -> Res<Option<String>> {
    if c.setup.is_empty() {
        return Ok(None);
    }
    let file = state_dir().join("setup").join(format!("{}.sh", w.slot));
    let exe = env::current_exe().map_err(|e| e.to_string())?;
    let tell = |what: &str| format!("{} hook {what} exact", sh(&exe.display().to_string()));
    let steps: String = c.setup.iter().map(|s| format!("printf '%s\\n' {}\n{s}\n", sh(&format!("+ {s}")))).collect();
    fs::create_dir_all(file.parent().unwrap()).and_then(|_| fs::write(&file, format!("trap 'c=$?; [ $c = 0 ] || {}; exit $c' EXIT\nset -e\n{steps}{}\n", tell("end").replace('\'', "'\\''"), tell("launched")))).map_err(|e| e.to_string())?;
    Ok(Some(format!("sh {}", sh(&file.display().to_string()))))
}

/// As one word of a shell.
pub fn sh(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub fn teardown(w: &Wt, c: &Config) -> Res<()> {
    for step in &c.teardown {
        let vars = w.env().into_iter().filter_map(|e| e.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())));
        let o = Command::new("sh").args(["-c", step]).current_dir(w.path()).envs(vars).stdin(std::process::Stdio::null()).output().map_err(|e| format!("{step}: {e}"))?;
        if !o.status.success() {
            let why = String::from_utf8_lossy(&o.stderr).trim().lines().last().unwrap_or("").to_string();
            return Err((tr().teardown_failed)(step, &why));
        }
    }
    Ok(())
}

/// The ports that have a name, and whether something listens there.
pub fn ports(only: Option<&Wt>) {
    for w in load().iter().filter(|w| only.is_none_or(|o| o.is(w))) {
        let mut named: Vec<(String, u32)> = vec![("ORCUX_PORT".into(), w.port())];
        named.extend(trusted(&w.repo).ports.into_iter().map(|(k, n)| (k, w.port() + n)));
        for (k, p) in named {
            let used = TcpListener::bind(("127.0.0.1", p as u16)).is_err();
            println!("{:<40} {k:<24} {p:<6} {}", w.id(), if used { tr().port_used } else { "" });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads() {
        let c = parse("# x\nsetup = npm ci\nport DB_PORT = 3\nenv URL = pg://h:${DB_PORT}/${ORCUX_NAME}?${NOT}\nlink=node_modules\nteardown = docker compose down -v\n").unwrap();
        assert_eq!((c.setup.as_slice(), c.link.as_slice(), c.ports.as_slice()), (&["npm ci".to_string()][..], &["node_modules".to_string()][..], &[("DB_PORT".to_string(), 3)][..]));
        let known = [("ORCUX_NAME".to_string(), "fix_login".to_string())];
        assert_eq!(variables(&c, 10200, &known), [("DB_PORT".into(), "10203".into()), ("URL".into(), "pg://h:10203/fix_login?${NOT}".into())]);
        for bad in ["setup npm", "port X = 100", "port X = a", "link = ../x", "link = /etc", "env 1X = a", "env ORCUX_PORT = 1", "what = x", "env = x"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        assert_eq!(sh("it's"), "'it'\\''s'");
    }
}
