//! Open pull requests of the registered repos, read-only, through the `gh` CLI.
//! The sidebar fetches them into a cache file; the rail and the sidebar draw from it.
use crate::{i18n::tr, *};
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

/// GitHub's own summary counts every run, also one that was cancelled and run again, and checks that block nothing.
/// Out of `contexts.nodes`: the last run of each check, and whether it is "ok", "pending", "fail" or "optional" (it
/// fails, but the branch requires others). Without required checks, all of them matter.
const CHECKS_JQ: &str = r#"
def outcome:
  if .__typename == "CheckRun" then
    if .status != "COMPLETED" then "pending"
    elif (.conclusion as $c | ["FAILURE", "TIMED_OUT", "STARTUP_FAILURE", "ACTION_REQUIRED", "CANCELLED"] | index($c)) != null then "fail"
    else "ok" end
  elif .state == "SUCCESS" then "ok" elif .state == "PENDING" or .state == "EXPECTED" then "pending" else "fail" end;
def checks:
  (. // []) | group_by([(.name // .context), (.checkSuite.workflowRun.workflow.name // ""), (.checkSuite.workflowRun.event // "")])
  | map(max_by(.databaseId // 0)) | any(.[]; .isRequired) as $some
  | map(. as $n | outcome as $o | {name: ($n.name // $n.context),
      is: (if $o == "fail" and $some and ($n.isRequired | not) then "optional" else $o end)});
"#;

/// What CHECKS_JQ needs of each check; `pr` is a number or a variable.
fn check_fields(pr: &str) -> String {
    format!("__typename ... on CheckRun{{databaseId name status conclusion isRequired(pullRequestNumber:{pr}) \
        checkSuite{{workflowRun{{event workflow{{name}}}}}}}} ... on StatusContext{{context state isRequired(pullRequestNumber:{pr})}}")
}

const RED_JQ: &str = r#"
.data | to_entries[] | .key as $k | .value.pullRequest.commits.nodes[0].commit.statusCheckRollup.contexts
  | select(. != null and .totalCount <= 100) | (.nodes | checks) as $n
  | def count($s): [$n[] | select(.is == $s)] | length;
  [$k, (if count("fail") > 0 then "FAILURE" elif count("pending") > 0 then "PENDING" else "SUCCESS" end),
   count("ok"), count("fail"), ($n | length)] | @tsv"#;

/// Looks closer at the PRs whose CI GitHub calls red. If it cannot, they stay red.
fn recheck(lines: Vec<String>, token: Option<&str>) -> Vec<String> {
    let mut query = String::new();
    for (i, l) in lines.iter().enumerate() {
        let f: Vec<&str> = l.split('\t').collect();
        let [_, repo, number, ..] = f[..] else { continue };
        let plain = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
        if let Some((owner, name)) = repo.split_once('/')
            && matches!(f.get(11), Some(&"FAILURE" | &"ERROR")) && plain(owner) && plain(name) && number.parse::<u32>().is_ok()
        {
            let fields = check_fields(number);
            query += &format!("p{i}:repository(owner:\"{owner}\",name:\"{name}\"){{pullRequest(number:{number}){{commits(last:1){{nodes{{\
                commit{{statusCheckRollup{{contexts(first:100){{totalCount nodes{{{fields}}}}}}}}}}}}}}}}} ");
        }
    }
    if query.is_empty() {
        return lines;
    }
    match gh(&["api", "graphql", "-f", &format!("query=query{{{query}}}"), "--jq", &format!("{CHECKS_JQ}{RED_JQ}")], token) {
        Ok(out) => with_ci(lines, &out),
        Err(_) => lines,
    }
}

/// Puts what RED_JQ printed in the CI fields of the lines it is about.
fn with_ci(mut lines: Vec<String>, out: &str) -> Vec<String> {
    for l in out.lines() {
        let new: Vec<&str> = l.split('\t').collect();
        let [key, ci, ok, failed, total] = new[..] else { continue };
        let Some(line) = key.strip_prefix('p').and_then(|i| i.parse::<usize>().ok()).and_then(|i| lines.get_mut(i)) else { continue };
        let mut f: Vec<&str> = line.split('\t').collect();
        if f.len() > 15 {
            (f[11], f[13], f[14], f[15]) = (ci, ok, failed, total);
            *line = f.join("\t");
        }
    }
    lines
}

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
    /// the gh account that can read it
    pub login: String,
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
    /// an agent that waits for the user, that failed and that is done: never of a PR, so that both can be told
    /// apart on one row
    Asking,
    Broken,
    Finished,
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
        let [kind, repo, number, title, url, draft, decision, mergeable, branch, fork, author, ci, age, ok, failed, total, login, path] = f[..] else {
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
            login: login.into(),
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
            (Group::Yours, State::Waiting, (tr().asks_review)(&self.author))
        } else if self.draft {
            (Group::Going, State::Idle, if red { tr().draft_red.into() } else { tr().draft.into() })
        } else if self.mergeable == "CONFLICTING" {
            (Group::Yours, State::Failed, tr().conflicts.into())
        } else if red {
            (Group::Yours, State::Failed, (tr().ci_red)(self.failed as usize, self.total as usize))
        } else if self.decision == "CHANGES_REQUESTED" {
            (Group::Yours, State::Waiting, tr().changes.into())
        } else if pending {
            (Group::Going, State::Working, ci)
        } else if self.decision == "APPROVED" {
            (Group::Ready, State::Done, if self.total > 0 { format!("{}, {ci}", tr().approved) } else { tr().approved.into() })
        } else if self.decision == "REVIEW_REQUIRED" {
            (Group::Going, State::Idle, tr().waiting.into())
        } else {
            (Group::Going, State::Idle, tr().open.into())
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

/// None: the account is only known through the environment.
fn token(login: &str) -> Option<String> {
    gh(&["auth", "token", "--user", login], Some("")).ok()
}

/// (account, error)
type Failed = Vec<(String, String)>;

/// What could be read, plus the accounts that failed with their error.
fn fetch() -> Res<(Vec<String>, Failed)> {
    let repos: Vec<(PathBuf, String)> = repos().into_iter()
        .filter_map(|p| Some((p.clone(), slug(&git(&p, &["remote", "get-url", "origin"]).ok()?)?)))
        .collect();
    if repos.is_empty() {
        return Ok((vec![], vec![]));
    }
    let scope: String = repos.iter().map(|r| format!(" repo:{}", r.1)).collect();
    // Repos may belong to different accounts (work, personal): ask as each one and merge. Whatever state gh reports
    // for them: it comes from a network check that can fail once.
    let status = gh(&["auth", "status", "--json", "hosts", "--jq", r#".hosts["github.com"][] | [.login, .tokenSource] | @tsv"#], None)?;
    let sources: Vec<(&str, &str)> = status.lines().map(|l| l.split_once('\t').unwrap_or((l, ""))).collect();
    let mut logins: Vec<&str> = sources.iter().map(|s| s.0).collect();
    logins.sort();
    logins.dedup();
    if logins.is_empty() {
        return Err(tr().no_account.into());
    }
    let (mut lines, mut failed) = (vec![], vec![]);
    for login in logins {
        let stored = sources.iter().any(|s| s.0 == login && !s.1.ends_with("_TOKEN"));
        let token = token(login);
        if stored && token.is_none() {
            // without its token, gh would answer as another account: no PRs, and no error
            failed.push((login.to_string(), tr().no_token.to_string()));
            continue;
        }
        let q = |who: &str| format!("is:pr is:open archived:false {who}:@me{scope}");
        let (m, r) = (format!("m={}", q("author")), format!("r={}", q("user-review-requested")));
        match gh(&["api", "graphql", "-f", &format!("query={QUERY}"), "-f", &m, "-f", &r, "--jq", JQ], token.as_deref()) {
            Ok(out) => {
                let out = recheck(out.lines().map(String::from).collect(), token.as_deref());
                for l in &out {
                    let f: Vec<&str> = l.split('\t').collect();
                    let path = f.get(1).and_then(|s| repos.iter().find(|r| r.1.eq_ignore_ascii_case(s)));
                    if let Some((p, _)) = path {
                        lines.push(format!("{l}\t{login}\t{}", p.display()));
                    }
                }
            }
            Err(e) => failed.push((login.to_string(), e)),
        }
    }
    Ok((lines, failed))
}

/// The lines just read, plus the previous ones of the accounts that failed: a PR does not vanish because GitHub
/// did not answer once. A PR seen by two accounts counts once, as mine ("mine" sorts before "review").
fn merge(new: Vec<String>, old: &[&str], failed: &[(String, String)]) -> Vec<String> {
    let kept = old.iter().filter(|l| l.rsplit('\t').nth(1).is_some_and(|login| failed.iter().any(|f| f.0 == login)));
    let mut lines: Vec<String> = new.into_iter().chain(kept.map(|l| l.to_string())).collect();
    lines.sort();
    let mut seen = vec![];
    lines.retain(|l| {
        let url = l.split('\t').nth(4).unwrap_or("").to_string();
        let new = !seen.contains(&url);
        seen.push(url);
        new
    });
    lines
}

const DETAIL: &str = "query($o:String!,$r:String!,$n:Int!){repository(owner:$o,name:$r){pullRequest(number:$n){\
  additions deletions changedFiles baseRefName headRefName body \
  files(first:100){nodes{path additions deletions}} \
  latestReviews(first:30){nodes{author{login} state}} \
  reviewRequests(first:30){nodes{requestedReviewer{__typename ... on User{login}}}} \
  reviewThreads(first:100){nodes{isResolved path line comments(first:1){nodes{author{login} body}}}} \
  commits(last:1){nodes{commit{statusCheckRollup{contexts(first:100){nodes{CHECK_FIELDS}}}}}}}}}";

/// One tagged TSV line per thing. Comments lose their line breaks; the description keeps them.
const DETAIL_JQ: &str = r#"
def flat: (. // "") | gsub("[[:space:]]+"; " ") | .[0:400];
.data.repository.pullRequest as $p | (
  ["M", $p.additions, $p.deletions, $p.changedFiles, $p.baseRefName, $p.headRefName],
  ($p.commits.nodes[0].commit.statusCheckRollup.contexts.nodes | checks[] | select(.is != "ok") | ["C", .is, (.name | flat)]),
  ($p.latestReviews.nodes[] | ["R", .state, (.author.login // "?")]),
  ($p.reviewRequests.nodes[] | ["R", "PENDING", (.requestedReviewer.login // "")]),
  ($p.reviewThreads.nodes[] | select(.isResolved | not)
    | ["T", .path, (.line // 0), (.comments.nodes[0].author.login // "?"), (.comments.nodes[0].body | flat)]),
  ($p.files.nodes[] | ["F", .path, .additions, .deletions]),
  (($p.body // "") | gsub("\r"; "") | gsub("(?s)<!--.*?-->"; "") | .[0:4000] | split("\n")[] | ["B", .])
) | @tsv"#;

#[derive(Default)]
pub struct Thread {
    pub path: String,
    pub line: u32,
    pub author: String,
    pub text: String,
}

/// What the dialog of a PR shows besides what the list already knows.
#[derive(Default)]
pub struct Detail {
    pub added: u32,
    pub deleted: u32,
    pub changed: usize,
    pub base: String,
    pub head: String,
    pub failing: Vec<String>,
    /// they fail, but block nothing
    pub optional: Vec<String>,
    pub pending: Vec<String>,
    /// (state, login; empty for a team)
    pub reviews: Vec<(String, String)>,
    pub threads: Vec<Thread>,
    pub files: Vec<(String, u32, u32)>,
    pub body: Vec<String>,
    /// files that conflict with the base
    pub conflicts: Vec<String>,
}

impl Detail {
    fn parse(tsv: &str) -> Detail {
        // written by other people: no control characters may reach the terminal or an agent's prompt
        let clean = |s: &str| s.chars().filter(|c| !c.is_control()).collect::<String>();
        let mut d = Detail::default();
        for l in tsv.lines() {
            let f: Vec<String> = l.split('\t').map(clean).collect();
            let n = |i: usize| f.get(i).and_then(|s| s.parse().ok()).unwrap_or(0u32);
            let s = |i: usize| f.get(i).cloned().unwrap_or_default();
            match s(0).as_str() {
                "M" => (d.added, d.deleted, d.changed, d.base, d.head) = (n(1), n(2), n(3) as usize, s(4), s(5)),
                "C" if s(1) == "fail" => d.failing.push(s(2)),
                "C" if s(1) == "optional" => d.optional.push(s(2)),
                "C" => d.pending.push(s(2)),
                "R" => d.reviews.push((s(1), s(2))),
                "T" => d.threads.push(Thread { path: s(1), line: n(2), author: s(3), text: s(4) }),
                "F" => d.files.push((s(1), n(2), n(3))),
                "B" => d.body.push(s(1)),
                _ => {}
            }
        }
        d
    }
}

pub fn detail(p: &Pr) -> Res<Detail> {
    let (owner, name) = p.repo.split_once('/').ok_or_else(|| p.repo.clone())?;
    let args = [
        "api", "graphql", "-f", &format!("query={}", DETAIL.replace("CHECK_FIELDS", &check_fields("$n"))), "-f", &format!("o={owner}"), "-f", &format!("r={name}"),
        "-F", &format!("n={}", p.number), "--jq", &format!("{CHECKS_JQ}{DETAIL_JQ}"),
    ];
    let mut d = Detail::parse(&gh(&args, token(&p.login).as_deref())?);
    if p.mergeable == "CONFLICTING" && !p.fork {
        d.conflicts = conflicts(&p.path, &d.base, &d.head);
    }
    Ok(d)
}

/// GitHub does not say which files conflict: fetches both branches and merges them in memory. Empty if it cannot.
fn conflicts(repo: &Path, base: &str, head: &str) -> Vec<String> {
    let spec = |b: &str| format!("+refs/heads/{b}:refs/remotes/origin/{b}");
    let mut fetch = Command::new("git");
    fetch.args(["fetch", "-q", "origin", &spec(base), &spec(head)]).current_dir(repo);
    fetch.env("GIT_TERMINAL_PROMPT", "0").env("GIT_SSH_COMMAND", "ssh -oBatchMode=yes");
    if !fetch.output().is_ok_and(|o| o.status.success()) {
        return vec![];
    }
    let (a, b) = (format!("refs/remotes/origin/{base}"), format!("refs/remotes/origin/{head}"));
    let args = ["merge-tree", "--write-tree", "--name-only", "--no-messages", &a, &b];
    let Ok(o) = Command::new("git").args(args).current_dir(repo).output() else { return vec![] };
    // first line: the tree; exit code 1 means conflicts
    String::from_utf8_lossy(&o.stdout).lines().skip(1).filter(|l| !l.is_empty()).map(String::from).collect()
}

/// The number goes up whenever the format of its lines changes: during an update, an old process and a new one
/// must not overwrite each other's.
pub fn cache_file() -> PathBuf {
    state_dir().join("prs.2.tsv")
}

/// Fetches and stores. A failure keeps the previous PRs and records the error.
pub fn refresh() -> Res<()> {
    let old = fs::read_to_string(cache_file()).unwrap_or_default();
    let (head, rest) = old.split_once('\n').unwrap_or(("0\t", ""));
    let before = head.split('\t').next().unwrap_or("0").to_string();
    let old_lines: Vec<&str> = rest.lines().collect();
    let (at, error, lines) = match fetch() {
        Ok((new, failed)) => {
            let at = if failed.is_empty() { now().to_string() } else { before };
            let error = failed.first().map(|f| format!("{}: {}", f.0, f.1)).unwrap_or_default();
            (at, error, merge(new, &old_lines, &failed))
        }
        Err(e) => (before, e, old_lines.iter().map(|l| l.to_string()).collect()),
    };
    let text = format!("{at}\t{}\n{}", error.replace(['\n', '\t'], " "), lines.iter().map(|l| format!("{l}\n")).collect::<String>());
    let tmp = state_dir().join(format!("prs.tsv.{}", std::process::id()));
    fs::write(&tmp, text).and_then(|_| fs::rename(&tmp, cache_file())).map_err(|e| e.to_string())
}

/// The cached PRs, those that need the user first, and by repo within each group.
pub fn cached() -> Cache {
    let text = fs::read_to_string(cache_file()).unwrap_or_default();
    let (head, rest) = text.split_once('\n').unwrap_or(("0\t", ""));
    let (at, error) = head.split_once('\t').unwrap_or((head, ""));
    let mut prs: Vec<Pr> = rest.lines().filter_map(Pr::parse).collect();
    prs.sort_by_key(|p| { let s = p.status(); (s.0 as u8, p.repo_name().to_lowercase(), s.1 as u8, p.age) });
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
    fn a_failing_account_keeps_its_prs() {
        let l = |kind: &str, url: &str, login: &str| format!("{kind}\tr\t1\tt\t{url}\tx\t{login}\t/p");
        let (a, b, c) = (l("mine", "u1", "work"), l("review", "u2", "work"), l("mine", "u3", "home"));
        let old = [a.as_str(), b.as_str(), c.as_str()];
        let failed = [("work".to_string(), "boom".to_string())];
        assert_eq!(merge(vec![], &old, &failed), [a.clone(), b.clone()]); // home answered: it has none now
        assert_eq!(merge(vec![], &old, &[]), Vec::<String>::new());
        assert_eq!(merge(vec![l("review", "u1", "home")], &old, &failed), [a, b]);
    }

    #[test]
    fn red_ci_is_rechecked() {
        let l = |n: u32| format!("mine\tA/b\t{n}\tt\tu\tfalse\t\tMERGEABLE\tbr\tfalse\tme\tFAILURE\t9\t25\t2\t27");
        let got = with_ci(vec![l(1), l(2), l(3)], "p1\tSUCCESS\t22\t0\t23\np7\tSUCCESS\t1\t0\t1\nbad\np2\tFAILURE\t21\t1\t23");
        let status = |l: &str| Pr::parse(&format!("{l}\tme\t/r")).unwrap().status().2;
        assert_eq!(got[0], l(1)); // not answered: as it was
        assert_eq!(status(&got[1]), tr().open);
        assert_eq!(status(&got[2]), (tr().ci_red)(1, 23));
    }

    #[test]
    fn details() {
        let d = Detail::parse("M\t1\t2\t3\tmain\tfeat\nC\tfail\te2e\nC\tpending\tlint\nC\toptional\tqa\nR\tPENDING\t\nT\ta.rs\t7\tana\thi \u{1b}[31m\nF\ta.rs\t1\t2\nB\tx\nB\t\nB\ty\nZ\t?");
        assert_eq!((d.added, d.changed, d.base.as_str(), d.head.as_str()), (1, 3, "main", "feat"));
        assert_eq!(d.optional, ["qa"]);
        assert_eq!((d.failing, d.pending, d.reviews), (vec!["e2e".into()], vec!["lint".into()], vec![("PENDING".into(), String::new())]));
        assert_eq!((d.threads[0].line, d.threads[0].text.as_str()), (7, "hi [31m"));
        assert_eq!((d.files.len(), d.body), (1, vec!["x".to_string(), "".into(), "y".into()]));
    }

    #[test]
    fn empty_fields_survive() {
        let p = Pr::parse("mine\tA/b\t7\tT\tu\tfalse\t\tUNKNOWN\tbr\tfalse\tme\t\t90\t0\t0\t0\tme\t/r").unwrap();
        assert_eq!((p.number.as_str(), p.ci.as_str(), p.age), ("7", "", 90));
    }
}
