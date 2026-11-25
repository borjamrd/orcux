//! Bringing the work of a worktree back (`orcux merge`) and seeing in time what will clash (`orcux conflicts`). Nothing is
//! ever pushed.
use crate::*;

/// Runs git with variables of its own.
fn git_env(dir: &Path, vars: &[(&str, &str)], args: &[&str]) -> Res<String> {
    let o = Command::new("git").args(args).envs(vars.iter().copied()).current_dir(dir).output().map_err(|e| e.to_string())?;
    if o.status.success() { Ok(String::from_utf8_lossy(&o.stdout).trim_end().into()) } else { Err(String::from_utf8_lossy(&o.stderr).trim().into()) }
}

/// The files that clash, without touching any checkout.
fn clashes(repo: &Path, a: &str, b: &str) -> Res<Vec<String>> {
    let o = Command::new("git").args(["merge-tree", "--write-tree", "--name-only", "--no-messages", a, b]).current_dir(repo).output().map_err(|e| e.to_string())?;
    let out = String::from_utf8_lossy(&o.stdout).to_string();
    match o.status.code() {
        Some(0) => Ok(vec![]),
        Some(1) => Ok(out.lines().skip(1).filter(|l| !l.is_empty()).map(String::from).collect()),
        _ => Err(String::from_utf8_lossy(&o.stderr).trim().into()),
    }
}

pub struct Plan {
    pub branch: String,
    pub into: String,
    pub commits: u32,
    pub clashes: Vec<String>,
}

fn default_branch(repo: &Path) -> Res<String> {
    if let Ok(b) = git(repo, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"]) {
        return Ok(b.trim_start_matches("origin/").into());
    }
    let has = |b: &&str| git(repo, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{b}")]).is_ok();
    ["main", "master"].iter().find(|b| has(b)).map(|b| b.to_string()).ok_or_else(|| tr().merge_where.into())
}

pub fn plan(w: &Wt, into: Option<&str>, force: bool) -> Res<Plan> {
    let branch = git(&w.path(), &["symbolic-ref", "--short", "HEAD"]).map_err(|_| tr().merge_detached)?;
    let parent = load().into_iter().find(|p| p.repo == w.repo && Some(&p.name) == w.parent.as_ref()).and_then(|p| p.branch());
    let into = match (into, parent) {
        (Some(b), _) => b.to_string(),
        (None, Some(b)) => b,
        (None, None) => default_branch(&w.repo)?,
    };
    if into == branch {
        return Err((tr().merge_self)(&branch));
    }
    git(&w.repo, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{into}")]).map_err(|_| (tr().merge_no_branch)(&into))?;
    if !git(&w.path(), &["status", "--porcelain"])?.is_empty() {
        return Err((tr().merge_dirty)(&w.id())); // they would be lost with the worktree, also by force
    }
    if !force {
        if let Some(p) = prs::cached().prs.iter().find(|p| p.is_of(w)) {
            return Err((tr().merge_pr)(&p.number));
        }
        if panes().iter().any(|p| p.wt == w.id() && p.agent.is_some_and(|a| a.0 == Agent::Working || a.0.waits())) {
            return Err(tr().merge_busy.into());
        }
    }
    let commits = git(&w.repo, &["rev-list", "--count", &format!("refs/heads/{into}..refs/heads/{branch}")])?.parse().unwrap_or(0);
    if commits == 0 {
        return Err((tr().merge_nothing)(&branch, &into));
    }
    let clashes = clashes(&w.repo, &format!("refs/heads/{into}"), &format!("refs/heads/{branch}"))?;
    Ok(Plan { branch, into, commits, clashes })
}

/// The branch that a checkout has now.
fn on(dir: &Path) -> Option<String> {
    git(dir, &["symbolic-ref", "--short", "HEAD"]).ok()
}

/// Where a branch is checked out.
fn checkout_of(repo: &Path, branch: &str) -> Res<Option<PathBuf>> {
    let list = git(repo, &["worktree", "list", "--porcelain"])?;
    Ok(list.split("\n\n").find(|b| b.lines().any(|l| l == format!("branch refs/heads/{branch}")))
        .and_then(|b| b.lines().next()?.strip_prefix("worktree ").map(PathBuf::from)))
}

/// Merges, and only then takes the worktree, its terminals and its branch away. The plan may be old: what matters is
/// looked at again, and exactly the commit that was merged is what may be deleted.
pub fn run_plan(w: &Wt, p: &Plan) -> Res<()> {
    let config = config::trusted(&w.repo); // as approved now: the merge may bring another .orcux
    let (from, to) = (format!("refs/heads/{}", p.branch), format!("refs/heads/{}", p.into));
    let tip = git(&w.repo, &["rev-parse", "--verify", &from])?;
    let stays = || -> Res<()> {
        let same = on(&w.path()).as_ref() == Some(&p.branch) && git(&w.path(), &["rev-parse", "HEAD"])? == tip && git(&w.path(), &["status", "--porcelain"])?.is_empty();
        if same { Ok(()) } else { Err((tr().merge_moved)(&w.id())) }
    };
    stays()?;
    let clash = clashes(&w.repo, &to, &tip)?;
    if !clash.is_empty() {
        return Err((tr().merge_clash)(&p.into, &clash.join(", ")));
    }
    match checkout_of(&w.repo, &p.into)? {
        Some(dir) => {
            if !git(&dir, &["status", "--porcelain", "--untracked-files=no"])?.is_empty() {
                return Err((tr().merge_target_dirty)(&p.into, &tilde(&dir)));
            }
            if on(&dir).as_ref() != Some(&p.into) {
                return Err((tr().merge_moved)(&p.into));
            }
            if let Err(e) = git(&dir, &["merge", "-m", &format!("Merge branch '{}' into {}", p.branch, p.into), &tip]) {
                git(&dir, &["merge", "--abort"]).ok();
                return Err(e);
            }
        }
        // checked out nowhere: no folder needs to change
        None => {
            let old = git(&w.repo, &["rev-parse", &to])?;
            let new = if git(&w.repo, &["merge-base", "--is-ancestor", &old, &tip]).is_ok() {
                tip.clone()
            } else {
                let tree = git(&w.repo, &["merge-tree", "--write-tree", "--no-messages", &old, &tip])?;
                git(&w.repo, &["commit-tree", &tree, "-p", &old, "-p", &tip, "-m", &format!("Merge branch '{}' into {}", p.branch, p.into)])?
            };
            git(&w.repo, &["update-ref", "-m", "orcux merge", &to, &new, &old])?; // unless it moved meanwhile
        }
    }
    git(&w.repo, &["merge-base", "--is-ancestor", &tip, &to]).map_err(|_| tr().merge_failed)?;
    remove_then(w, false, &config, stays).map_err(|e| (tr().merge_kept)(&p.into, &e))?;
    if w.at.is_none() {
        git(&w.repo, &["update-ref", "-d", &from, &tip]).ok(); // only if nothing was added to it
        git(&w.repo, &["config", "--remove-section", &format!("branch.{}", p.branch)]).ok();
    }
    close_all(w) // this process may be in one of its terminals
}

pub fn cmd(args: &[&str]) -> Res<()> {
    // nothing is guessed: it ends up deleting
    let (mut q, mut into, mut check, mut force) = (None, None, false, false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match *a {
            "--check" => check = true,
            "--force" => force = true,
            "--into" => into = Some(*it.next().filter(|b| !b.starts_with('-')).ok_or(tr().merge_usage)?),
            a if !a.starts_with('-') && q.is_none() => q = Some(a),
            _ => return Err(tr().merge_usage.into()),
        }
    }
    let w = find(Some(q.ok_or(tr().merge_usage)?))?;
    let p = plan(&w, into, force)?;
    println!("{}", (tr().merge_plan)(&p.branch, &p.into, p.commits as usize));
    for (other, files) in with_others(&w)? {
        println!("{}", (tr().merge_others)(&other, &files.join(", ")));
    }
    if check {
        return if p.clashes.is_empty() { Ok(()) } else { Err((tr().merge_clash)(&p.into, &p.clashes.join(", "))) };
    }
    println!("{}", tr().merge_then);
    run_plan(&w, &p)?;
    println!("{}", (tr().merge_done)(&p.into));
    Ok(())
}

/// A commit with what a worktree holds now, committed or not, without touching its index: its agent may be using it.
fn snapshot(w: &Wt) -> Res<String> {
    let dir = w.path();
    let head = git(&dir, &["rev-parse", "HEAD"])?;
    if git_env(&dir, &[("GIT_OPTIONAL_LOCKS", "0")], &["status", "--porcelain"])?.is_empty() {
        return Ok(head);
    }
    let index = state_dir().join(format!("index.{}.{}", std::process::id(), w.slot));
    let real = git(&dir, &["rev-parse", "--path-format=absolute", "--git-path", "index"])?;
    fs::copy(real, &index).ok();
    let path = index.to_string_lossy().to_string();
    let vars = [("GIT_INDEX_FILE", path.as_str()), ("GIT_AUTHOR_NAME", "orcux"), ("GIT_AUTHOR_EMAIL", "orcux"), ("GIT_COMMITTER_NAME", "orcux"), ("GIT_COMMITTER_EMAIL", "orcux")];
    let made = git_env(&dir, &vars, &["add", "-A"])
        .and_then(|_| git_env(&dir, &vars, &["write-tree"]))
        .and_then(|tree| git_env(&dir, &vars, &["commit-tree", &tree, "-p", &head, "-m", "orcux: not a commit of yours"]));
    fs::remove_file(&index).ok();
    made
}

type Clash = (String, Vec<String>);

/// What clashes between one worktree and the others of its repo.
pub fn with_others(w: &Wt) -> Res<Vec<Clash>> {
    let mine = snapshot(w)?;
    let others = load().into_iter().filter(|o| o.repo == w.repo && !o.is(w) && o.path().exists());
    Ok(others.filter_map(|o| {
        let files = clashes(&w.repo, &mine, &snapshot(&o).ok()?).ok()?;
        (!files.is_empty()).then(|| (o.id(), files))
    }).collect())
}

/// `orcux conflicts [wt]`: ends with 1 if something clashes.
pub fn conflicts(q: Option<&str>) -> Res<i32> {
    let only = q.map(|q| find(Some(q))).transpose()?;
    let wts: Vec<Wt> = load().into_iter().filter(|w| w.path().exists() && only.as_ref().is_none_or(|o| o.repo == w.repo)).collect();
    let shots: Vec<Option<String>> = wts.iter().map(|w| snapshot(w).ok()).collect();
    let mut found = 0;
    // with the main branch: what will clash anyway
    for (w, shot) in wts.iter().zip(&shots).filter(|(w, _)| only.as_ref().is_none_or(|o| o.is(w))) {
        if let (Some(x), Ok(main)) = (shot, default_branch(&w.repo)) && let Ok(files) = clashes(&w.repo, &format!("refs/heads/{main}"), x) && !files.is_empty() {
            found += 1;
            println!("{} ↔ {main}: {}", w.id(), files.join(", "));
        }
    }
    for i in 0..wts.len() {
        for j in i + 1..wts.len() {
            let (a, b) = (&wts[i], &wts[j]);
            if a.repo != b.repo || only.as_ref().is_some_and(|o| !o.is(a) && !o.is(b)) {
                continue;
            }
            if let (Some(x), Some(y)) = (&shots[i], &shots[j]) && let Ok(files) = clashes(&a.repo, x, y) && !files.is_empty() {
                found += 1;
                println!("{} ↔ {}: {}", a.id(), b.id(), files.join(", "));
            }
        }
    }
    if found == 0 {
        println!("{}", tr().conflicts_none);
    }
    Ok((found > 0).into())
}
