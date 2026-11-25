//! The files of a worktree, for the sidebar: their tree, and what git says changed in each one. Read-only.
use crate::Res;
use std::{collections::{BTreeMap, HashMap, HashSet}, path::Path, process::Command};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Change {
    Modified,
    Added,
    Deleted,
    Renamed,
    /// not known to git yet
    Untracked,
    Conflict,
}

impl Change {
    pub fn glyph(self) -> &'static str {
        match self {
            Change::Modified => "M",
            Change::Added => "A",
            Change::Deleted => "D",
            Change::Renamed => "R",
            Change::Untracked => "U",
            Change::Conflict => "C",
        }
    }

    /// From the two letters of `git status --porcelain`.
    fn of(x: u8, y: u8) -> Change {
        match (x, y) {
            (b'?', _) => Change::Untracked,
            (b'U', _) | (_, b'U') | (b'A', b'A') | (b'D', b'D') => Change::Conflict,
            (b'D', _) | (_, b'D') => Change::Deleted,
            (b'R' | b'C', _) | (_, b'R' | b'C') => Change::Renamed,
            (b'A', _) => Change::Added,
            _ => Change::Modified,
        }
    }
}

/// What changed in a file: how, and lines added and removed (none for a binary or a big new file).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Mark {
    pub change: Change,
    pub lines: Option<(u32, u32)>,
}

#[derive(Default, Clone, PartialEq, Debug)]
pub struct Files {
    /// every file that is not ignored, sorted
    pub paths: Vec<String>,
    pub marks: HashMap<String, Mark>,
}

impl Files {
    /// Changed files, lines added and lines removed.
    pub fn summary(&self) -> (usize, u32, u32) {
        self.marks.values().fold((0, 0, 0), |(n, a, d), m| {
            let (x, y) = m.lines.unwrap_or_default();
            (n + 1, a + x, d + y)
        })
    }
}

fn git(dir: &Path, args: &[&str]) -> Res<Vec<u8>> {
    let o = Command::new("git").arg("-C").arg(dir).args(args).output().map_err(|e| e.to_string())?;
    if o.status.success() { Ok(o.stdout) } else { Err(String::from_utf8_lossy(&o.stderr).trim().to_string()) }
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

pub fn read(dir: &Path) -> Res<Files> {
    let mut marks = status(&git(dir, &["status", "--porcelain=v1", "-z", "--untracked-files=all"])?);
    // against the last commit: staged and not staged together. A repo without commits has none.
    if let Ok(n) = git(dir, &["diff", "--numstat", "-z", "HEAD"]) {
        numstat(&n, &mut marks);
    }
    for (p, m) in marks.iter_mut().filter(|(_, m)| m.change == Change::Untracked) {
        // all its lines are new; a big file or one that is not text says nothing
        m.lines = std::fs::read(dir.join(p)).ok().filter(|b| b.len() < 1 << 20 && !b.contains(&0)).map(|b| {
            let n = b.iter().filter(|c| **c == b'\n').count() + usize::from(!b.is_empty() && !b.ends_with(b"\n"));
            (n as u32, 0)
        });
    }
    let listed = git(dir, &["ls-files", "-z", "--cached", "--others", "--exclude-standard"])?;
    let mut all: HashSet<String> = listed.split(|c| *c == 0).filter(|s| !s.is_empty()).map(text).collect();
    all.extend(marks.keys().cloned()); // deleted from the index too
    let mut paths: Vec<String> = all.into_iter().collect();
    paths.sort();
    Ok(Files { paths, marks })
}

fn status(b: &[u8]) -> HashMap<String, Mark> {
    let mut out = HashMap::new();
    let mut it = b.split(|c| *c == 0);
    while let Some(e) = it.next() {
        if e.len() < 4 {
            continue;
        }
        if [e[0], e[1]].iter().any(|c| b"RC".contains(c)) {
            it.next(); // the name it had
        }
        out.insert(text(&e[3..]), Mark { change: Change::of(e[0], e[1]), lines: None });
    }
    out
}

fn numstat(b: &[u8], marks: &mut HashMap<String, Mark>) {
    let mut it = b.split(|c| *c == 0);
    while let Some(e) = it.next() {
        let e = text(e);
        let mut f = e.splitn(3, '\t');
        let (Some(added), Some(removed), Some(path)) = (f.next(), f.next(), f.next()) else { continue };
        // a rename: old and new name follow
        let path = if path.is_empty() { it.nth(1).map(text) } else { Some(path.to_string()) };
        if let Some(m) = path.and_then(|p| marks.get_mut(&p)) {
            m.lines = added.parse().ok().zip(removed.parse().ok());
        }
    }
}

/// A row of the tree.
#[derive(Debug, PartialEq)]
pub struct Item {
    pub depth: usize,
    pub name: String,
    pub path: String,
    pub dir: bool,
    pub open: bool,
    pub mark: Option<Mark>,
    /// changed files inside a folder
    pub changed: usize,
}

#[derive(Default)]
struct Node {
    dirs: BTreeMap<String, Node>,
    files: Vec<String>,
    changed: usize,
}

/// The rows to show. A folder is open when it holds changes, unless `flipped` says otherwise; `only` leaves out
/// what did not change. A folder with nothing but one folder inside is one row: `src/main/java`.
pub fn rows(f: &Files, flipped: &HashSet<String>, only: bool) -> Vec<Item> {
    let mut root = Node::default();
    for p in &f.paths {
        let changed = usize::from(f.marks.contains_key(p));
        if only && changed == 0 {
            continue;
        }
        let mut parts: Vec<&str> = p.split('/').collect();
        let file = parts.pop().unwrap_or_default();
        let mut n = &mut root;
        n.changed += changed;
        for d in parts {
            n = n.dirs.entry(d.to_string()).or_default();
            n.changed += changed;
        }
        n.files.push(file.to_string());
    }
    let mut out = vec![];
    walk(&root, "", 0, f, flipped, &mut out);
    out
}

fn walk(n: &Node, base: &str, depth: usize, f: &Files, flipped: &HashSet<String>, out: &mut Vec<Item>) {
    for (name, child) in &n.dirs {
        let (mut name, mut child) = (name.clone(), child);
        while child.files.is_empty() && child.dirs.len() == 1 {
            let (k, v) = child.dirs.iter().next().unwrap_or_else(|| unreachable!());
            (name, child) = (format!("{name}/{k}"), v);
        }
        let path = format!("{base}{name}");
        let open = (child.changed > 0) != flipped.contains(&path);
        out.push(Item { depth, name, path: path.clone(), dir: true, open, mark: None, changed: child.changed });
        if open {
            walk(child, &format!("{path}/"), depth + 1, f, flipped, out);
        }
    }
    for name in &n.files {
        let path = format!("{base}{name}");
        out.push(Item { depth, mark: f.marks.get(&path).copied(), name: name.clone(), path, dir: false, open: false, changed: 0 });
    }
}

/// Shows a file in a pager: its changes, or the whole file if it has none. For a popup.
pub fn show(dir: &Path, path: &str) -> Res<()> {
    const SCRIPT: &str = r#"cd "$1" || exit 1
if ! git ls-files --error-unmatch -- "$2" >/dev/null 2>&1; then git -c color.ui=always diff --no-index -- /dev/null "$2"
elif git diff --quiet HEAD -- "$2" 2>/dev/null; then cat -- "$2"
else git -c color.ui=always diff HEAD -- "$2"; fi | LESS=-R less"#;
    // LESS: an -F of the person would close it at once
    Command::new("sh").args(["-c", SCRIPT, "sh"]).arg(dir).arg(path).status().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(dir: &Path, cmd: &str) {
        assert!(Command::new("sh").args(["-c", cmd]).current_dir(dir).status().unwrap().success(), "{cmd}");
    }

    #[test]
    fn what_changed() {
        let dir = std::env::temp_dir().join(format!("orcux-files-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        sh(&dir, "git init -q && mkdir -p src/deep/er docs && printf 'a\\nb\\n' > src/a.rs && echo x > src/deep/er/x.rs && echo d > docs/d.md && echo gone > old.txt && echo r > r.txt \
            && printf 'target\\n' > .gitignore && git add -A && git -c user.name=t -c user.email=t@t commit -qm i \
            && printf 'a\\nc\\nd\\n' > src/a.rs && rm old.txt && git mv r.txt moved.txt && printf '1\\n2' > src/new.rs && mkdir target && echo t > target/t");
        let f = read(&dir).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        let m = |p: &str| f.marks.get(p).map(|m| (m.change, m.lines));
        assert_eq!(m("src/a.rs"), Some((Change::Modified, Some((2, 1)))));
        assert_eq!(m("old.txt"), Some((Change::Deleted, Some((0, 1)))));
        assert_eq!(m("moved.txt"), Some((Change::Renamed, Some((0, 0)))));
        assert_eq!(m("src/new.rs"), Some((Change::Untracked, Some((2, 0)))));
        assert_eq!(f.summary(), (4, 4, 2));
        assert!(!f.paths.iter().any(|p| p.starts_with("target")) && f.paths.contains(&"old.txt".into()) && !f.paths.contains(&"r.txt".into()));

        let names = |r: &[Item]| r.iter().map(|i| format!("{}{}{}", "  ".repeat(i.depth), i.name, if i.dir && !i.open { "/" } else { "" })).collect::<Vec<_>>();
        let rows = rows(&f, &HashSet::new(), false);
        assert_eq!(names(&rows), ["docs/", "src", "  deep/er/", "  a.rs", "  new.rs", ".gitignore", "moved.txt", "old.txt"]);
        assert_eq!(rows[1].changed, 2);
        let flipped = HashSet::from(["src".to_string(), "docs".to_string()]);
        assert_eq!(names(&super::rows(&f, &flipped, false))[..2], ["docs", "  d.md"]);
        assert_eq!(names(&super::rows(&f, &HashSet::new(), true)), ["src", "  a.rs", "  new.rs", "moved.txt", "old.txt"]);
    }
}
