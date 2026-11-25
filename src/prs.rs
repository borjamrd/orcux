//! Open pull requests of the registered repos, read-only, through the `gh` CLI.
//! The sidebar fetches them into a cache file; the rail and the sidebar draw from it.
use crate::*;
use std::time::{SystemTime, UNIX_EPOCH};

const QUERY: &str = "query($m:String!,$r:String!){\
  mine:search(query:$m,type:ISSUE,first:40){nodes{...P}} review:search(query:$r,type:ISSUE,first:40){nodes{...P}}}\
  fragment P on PullRequest{number title url isDraft reviewDecision mergeable headRefName isCrossRepository updatedAt \
  author{login} repository{nameWithOwner} commits(last:1){nodes{commit{statusCheckRollup{state contexts(first:1){\
  totalCount checkRunCountsByState{state count} statusContextCountsByState{state count}}}}}}}";

/// One TSV line per PR, so that no JSON crate is needed. Numbers go last: run() trims trailing empty fields.
const JQ: &str = r#"
def line($kind): (.commits.nodes[0].commit.statusCheckRollup // {}) as $c
  | [($c.contexts.checkRunCountsByState // [])[], ($c.contexts.statusContextCountsByState // [])[]] as $n
  | def count($s): [$n[] | select(.state as $x | $s | index($x)) | .count] | add // 0;
  [$kind, .repository.nameWithOwner, .number, .title, .url, .isDraft, .reviewDecision, .mergeable, .headRefName,
   .isCrossRepository, .author.login, $c.state,
   (now - (.updatedAt | fromdateiso8601) | floor),
   count(["SUCCESS", "NEUTRAL", "SKIPPED"]),
   count(["FAILURE", "ERROR", "TIMED_OUT", "STARTUP_FAILURE", "ACTION_REQUIRED", "CANCELLED"]),
   ($c.contexts.totalCount // 0)] | @tsv;
.data | (.mine.nodes[] | line("mine")), (.review.nodes[] | line("review"))"#;

#[derive(Clone, Default)]
pub struct Pr {
    pub review: bool,
    pub repo: String,
    pub number: String,
    pub title: String,
    pub url: String,
    pub draft: bool,
    pub decision: String,
    pub mergeable: String,
    pub branch: String,
    pub fork: bool,
    pub author: String,
    pub ci: String,
    /// seconds since its last update, when fetched
    pub age: u64,
    pub ok: u32,
    pub failed: u32,
    pub total: u32,
    /// local main checkout
    pub path: PathBuf,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Group {
    Yours,
    Going,
    Ready,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum State {
    Failed,
    Waiting,
    Working,
    Idle,
    Done,
}

#[derive(Default)]
pub struct Cache {
    /// unix time of the last good fetch; 0 = never
    pub at: u64,
    pub error: String,
    pub prs: Vec<Pr>,
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

pub fn ago(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs} s"),
        60..3600 => format!("{} min", secs / 60),
        3600..86400 => format!("{} h", secs / 3600),
        _ => format!("{} d", secs / 86400),
    }
}

impl Pr {
    fn parse(l: &str) -> Option<Pr> {
        let f: Vec<&str> = l.split('\t').collect();
        let [kind, repo, number, title, url, draft, decision, mergeable, branch, fork, author, ci, age, ok, failed, total, path] = f[..] else {
            return None;
        };
        Some(Pr {
            review: kind == "review",
            repo: repo.into(),
            number: number.into(),
            title: title.into(),
            url: url.into(),
            draft: draft == "true",
            decision: decision.into(),
            mergeable: mergeable.into(),
            branch: branch.into(),
            fork: fork == "true",
            author: author.into(),
            ci: ci.into(),
            age: age.parse().ok()?,
            ok: ok.parse().ok()?,
            failed: failed.parse().ok()?,
            total: total.parse().ok()?,
            path: path.into(),
        })
    }

    pub fn repo_name(&self) -> &str {
        self.repo.rsplit('/').next().unwrap_or(&self.repo)
    }

    /// Whether `w` is the worktree of this PR: same repo, with its branch checked out.
    pub fn is_of(&self, w: &Wt) -> bool {
        w.repo == self.path && w.branch().as_ref() == Some(&self.branch)
    }

    pub fn worktree<'a>(&self, wts: &'a [Wt]) -> Option<&'a Wt> {
        wts.iter().find(|w| self.is_of(w))
    }

    /// Who has to act, how it is going, and the same in words.
    pub fn status(&self) -> (Group, State, String) {
        let red = matches!(self.ci.as_str(), "FAILURE" | "ERROR");
        let pending = matches!(self.ci.as_str(), "PENDING" | "EXPECTED");
        let ci = format!("CI {}/{}", self.ok, self.total);
        if self.review {
            (Group::Yours, State::Waiting, format!("te pide revisión {}", self.author))
        } else if self.draft {
            (Group::Going, State::Idle, if red { "borrador, CI falla".into() } else { "borrador".into() })
        } else if self.mergeable == "CONFLICTING" {
            (Group::Yours, State::Failed, "conflictos con la base".into())
        } else if red {
            (Group::Yours, State::Failed, format!("CI falla: {} de {}", self.failed, self.total))
        } else if self.decision == "CHANGES_REQUESTED" {
            (Group::Yours, State::Waiting, "cambios pedidos".into())
        } else if pending {
            (Group::Going, State::Working, ci)
        } else if self.decision == "APPROVED" {
            (Group::Ready, State::Done, if self.total > 0 { format!("aprobada, {ci}") } else { "aprobada".into() })
        } else if self.decision == "REVIEW_REQUIRED" {
            (Group::Going, State::Idle, "espera revisión".into())
        } else {
            (Group::Going, State::Idle, "abierta".into())
        }
    }
}

/// "owner/name" out of any GitHub remote URL (https, ssh or an ssh host alias).
fn slug(url: &str) -> Option<String> {
    let mut p = url.trim_end_matches('/').trim_end_matches(".git").rsplit(['/', ':']);
    let (name, owner) = (p.next()?, p.next()?);
    url.contains("github").then(|| format!("{owner}/{name}"))
}

/// Runs gh as one given account, without switching the active one.
fn gh(args: &[&str], token: Option<&str>) -> Res<String> {
    let mut c = Command::new("gh");
    c.args(args).stdin(std::process::Stdio::null());
    if let Some(t) = token {
        c.env_remove("GITHUB_TOKEN").env_remove("GH_TOKEN");
        if !t.is_empty() {
            c.env("GH_TOKEN", t);
        }
    }
    let o = c.output().map_err(|e| format!("gh: {e}"))?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).trim_end().into())
    } else {
        Err(format!("gh: {}", String::from_utf8_lossy(&o.stderr).trim()))
    }
}

fn fetch() -> Res<Vec<String>> {
    let repos: Vec<(PathBuf, String)> = repos().into_iter()
        .filter_map(|p| Some((p.clone(), slug(&git(&p, &["remote", "get-url", "origin"]).ok()?)?)))
        .collect();
    if repos.is_empty() {
        return Ok(vec![]);
    }
    let scope: String = repos.iter().map(|r| format!(" repo:{}", r.1)).collect();
    // Repos may belong to different accounts (work, personal): ask as each logged-in one and merge.
    let logins = gh(&["auth", "status", "--json", "hosts", "--jq", r#".hosts["github.com"][] | select(.state == "success") | .login"#], None)?;
    let mut logins: Vec<&str> = logins.lines().collect();
    logins.sort();
    logins.dedup();
    let (mut lines, mut good, mut err) = (vec![], false, "gh: ninguna cuenta con sesión".to_string());
    for l in logins {
        let token = gh(&["auth", "token", "--user", l], Some("")).ok(); // None: only known through the environment
        let q = |who: &str| format!("is:pr is:open archived:false {who}:@me{scope}");
        let (m, r) = (format!("m={}", q("author")), format!("r={}", q("user-review-requested")));
        match gh(&["api", "graphql", "-f", &format!("query={QUERY}"), "-f", &m, "-f", &r, "--jq", JQ], token.as_deref()) {
            Ok(out) => {
                good = true;
                for l in out.lines() {
                    let f: Vec<&str> = l.split('\t').collect();
                    let path = f.get(1).and_then(|s| repos.iter().find(|r| r.1.eq_ignore_ascii_case(s)));
                    if let Some((p, _)) = path {
                        lines.push(format!("{l}\t{}", p.display()));
                    }
                }
            }
            Err(e) => err = e,
        }
    }
    lines.sort(); // "mine" before "review": a PR seen by two accounts counts once, as mine
    let mut seen = vec![];
    lines.retain(|l| {
        let url = l.split('\t').nth(4).unwrap_or("").to_string();
        let new = !seen.contains(&url);
        seen.push(url);
        new
    });
    if good { Ok(lines) } else { Err(err) }
}

fn cache_file() -> PathBuf {
    state_dir().join("prs.tsv")
}

/// Fetches and stores. A failure keeps the previous PRs and records the error.
pub fn refresh() -> Res<()> {
    let old = fs::read_to_string(cache_file()).unwrap_or_default();
    let text = match fetch() {
        Ok(lines) => format!("{}\t\n{}", now(), lines.iter().map(|l| format!("{l}\n")).collect::<String>()),
        Err(e) => {
            let (head, rest) = old.split_once('\n').unwrap_or(("0\t", ""));
            let at = head.split('\t').next().unwrap_or("0");
            format!("{at}\t{}\n{rest}", e.replace(['\n', '\t'], " "))
        }
    };
    let tmp = state_dir().join(format!("prs.tsv.{}", std::process::id()));
    fs::write(&tmp, text).and_then(|_| fs::rename(&tmp, cache_file())).map_err(|e| e.to_string())
}

/// The cached PRs, those that need the user first.
pub fn cached() -> Cache {
    let text = fs::read_to_string(cache_file()).unwrap_or_default();
    let (head, rest) = text.split_once('\n').unwrap_or(("0\t", ""));
    let (at, error) = head.split_once('\t').unwrap_or((head, ""));
    let mut prs: Vec<Pr> = rest.lines().filter_map(Pr::parse).collect();
    prs.sort_by_key(|p| { let s = p.status(); (s.0 as u8, s.1 as u8, p.age) });
    Cache { at: at.parse().unwrap_or(0), error: error.into(), prs }
}

pub fn print() -> Res<()> {
    refresh()?;
    let c = cached();
    for p in &c.prs {
        println!("{:<34} {:<32} {}", format!("{}#{}", p.repo_name(), p.number), p.status().2, p.title);
    }
    if c.error.is_empty() { Ok(()) } else { Err(c.error) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        for u in ["git@github.com:Acme/web.git", "https://github.com/Acme/web", "https://github.com/Acme/web.git/", "git@github-work:Acme/web.git"] {
            assert_eq!(slug(u).as_deref(), Some("Acme/web"), "{u}");
        }
        assert_eq!(slug("git@gitlab.com:Acme/web.git"), None);
    }

    #[test]
    fn who_acts() {
        let pr = |f: fn(&mut Pr)| { let mut p = Pr::default(); f(&mut p); let s = p.status(); (s.0, s.1) };
        assert_eq!(pr(|p| p.review = true), (Group::Yours, State::Waiting));
        assert_eq!(pr(|p| p.ci = "FAILURE".into()), (Group::Yours, State::Failed));
        assert_eq!(pr(|p| { p.ci = "FAILURE".into(); p.draft = true }), (Group::Going, State::Idle));
        assert_eq!(pr(|p| { p.decision = "APPROVED".into(); p.mergeable = "CONFLICTING".into() }), (Group::Yours, State::Failed));
        assert_eq!(pr(|p| { p.decision = "APPROVED".into(); p.ci = "PENDING".into() }), (Group::Going, State::Working));
        assert_eq!(pr(|p| { p.decision = "APPROVED".into(); p.ci = "SUCCESS".into() }), (Group::Ready, State::Done));
        assert_eq!(pr(|p| p.decision = "REVIEW_REQUIRED".into()), (Group::Going, State::Idle));
    }

    #[test]
    fn empty_fields_survive() {
        let p = Pr::parse("mine\tA/b\t7\tT\tu\tfalse\t\tUNKNOWN\tbr\tfalse\tme\t\t90\t0\t0\t0\t/r").unwrap();
        assert_eq!((p.number.as_str(), p.ci.as_str(), p.age), ("7", "", 90));
    }
}
