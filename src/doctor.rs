//! `orcux doctor`: what orcux needs, checked and with what to do about it.
use crate::*;

enum Level {
    Good,
    Note,
    Bad,
}

/// "tmux 3.6a" -> (3, 6); "git version 2.50.1 (Apple Git)" -> (2, 50).
fn version(text: &str) -> Option<(u32, u32)> {
    let word = text.split_whitespace().find(|w| w.starts_with(|c: char| c.is_ascii_digit()))?;
    let mut parts = word.split('.').map(|p| p.chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0));
    Some((parts.next()?, parts.next().unwrap_or(0)))
}

fn first_line(cmd: &str, args: &[&str]) -> Option<String> {
    run(cmd, args, None).ok().map(|s| s.lines().next().unwrap_or("").to_string())
}

/// At least `need`; below `good` something is missing.
fn tool(found: Option<String>, need: (u32, u32), good: (u32, u32), missing: &str, older: &str) -> (Level, String) {
    match found.as_deref().map(|f| (f, version(f))) {
        None => (Level::Bad, missing.into()),
        Some((f, Some(v))) if v < need => (Level::Bad, format!("{f}: {}", (tr().doc_too_old)(&format!("{}.{}", need.0, need.1)))),
        Some((f, Some(v))) if v < good => (Level::Note, format!("{f}: {older}")),
        Some((f, _)) => (Level::Good, f.into()),
    }
}

/// Where the `.git` file of a worktree points, if that is not there (its repo moved).
fn broken_link(path: &Path) -> Option<PathBuf> {
    let link = fs::read_to_string(path.join(".git")).ok()?;
    let to = path.join(link.strip_prefix("gitdir:")?.trim());
    (!to.join("HEAD").exists()).then_some(to)
}

pub fn run_all() -> Res<()> {
    let t = tr();
    let var = |k: &str| env::var(k).unwrap_or_default();
    let mut all: Vec<(Level, String)> = vec![
        // with orcux's own engine, tmux is not needed
        if mux::own() { (Level::Good, t.doc_engine.into()) } else { tool(first_line("tmux", &["-V"]), (3, 3), (3, 6), t.doc_no_tmux, t.doc_old_tmux) },
        tool(first_line("git", &["--version"]), (2, 38), (2, 38), t.doc_no_git, ""),
        match run("gh", &["auth", "token"], None) {
            Ok(_) => (Level::Good, t.doc_gh.into()),
            Err(_) => (Level::Note, t.doc_no_gh.into()),
        },
    ];
    let locale = [var("LC_ALL"), var("LC_CTYPE"), var("LANG")].into_iter().find(|s| !s.is_empty()).unwrap_or_default();
    let utf8 = locale.to_uppercase().replace('-', "").contains("UTF8");
    all.push(if utf8 { (Level::Good, locale) } else { (Level::Note, t.doc_locale.into()) });
    if cfg!(target_os = "macos") && (var("TERM_PROGRAM") == "Apple_Terminal" || var("TERM_PROGRAM") == "tmux") {
        all.push((Level::Note, t.doc_option.into()));
    }
    // agents other than Claude Code find orcux only through the shared skill
    all.push(if agent::shared_skill().exists() { (Level::Good, t.doc_skill.into()) } else { (Level::Note, t.doc_no_skill.into()) });
    let state = state_dir();
    all.push(match fs::write(state.join(".doctor"), "").and_then(|_| fs::remove_file(state.join(".doctor"))) {
        Ok(_) => (Level::Good, tilde(&state)),
        Err(e) => (Level::Bad, format!("{}: {e}", tilde(&state))),
    });
    for r in repos().iter().filter(|r| !r.join(".git").exists()) {
        all.push((Level::Note, (t.doc_no_repo)(&tilde(r))));
    }
    let wts = load();
    for w in &wts {
        if !w.path().exists() {
            all.push((Level::Note, (t.doc_no_folder)(&w.id())));
        } else if broken_link(&w.path()).is_some() {
            all.push((Level::Bad, (t.doc_link)(&w.id(), &tilde(&w.repo))));
        }
    }
    all.push((Level::Good, (t.doc_counts)(repos().len(), wts.len())));
    if first_line("tmux", &["show-options", "-gv", "focus-events"]).as_deref() == Some("on") {
        all.push((Level::Note, t.doc_focus.into()));
    }
    let bad = all.iter().filter(|c| matches!(c.0, Level::Bad)).count();
    println!("orcux {} ({}, {})", env!("CARGO_PKG_VERSION"), env::consts::OS, env::consts::ARCH);
    for (level, text) in all {
        println!("  {} {text}", match level { Level::Good => "✓", Level::Note => "!", Level::Bad => "✕" });
    }
    if bad == 0 { Ok(()) } else { Err((t.doc_bad)(bad)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(version("tmux 3.6a"), Some((3, 6)));
        assert_eq!(version("git version 2.50.1 (Apple Git-155)"), Some((2, 50)));
        assert_eq!(version("tmux next-3.7"), None);
        assert!(matches!(tool(Some("tmux 3.2a".into()), (3, 3), (3, 6), "", "").0, Level::Bad));
        assert!(matches!(tool(Some("tmux 3.4".into()), (3, 3), (3, 6), "", "").0, Level::Note));
        assert!(matches!(tool(Some("tmux next-3.8".into()), (3, 3), (3, 6), "", "").0, Level::Good));
        assert!(matches!(tool(None, (3, 3), (3, 6), "", "").0, Level::Bad));
    }
}
