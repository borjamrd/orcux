//! Every text shown to the user. One file per language: a missing text does not compile.
//! To add a language: copy `en.rs`, translate it, and add it to `LANGS`.
//! Not translated: commands, keys, and what git, gh and tmux say.
use crate::*;
use std::sync::OnceLock;

mod en;
mod es;

/// Language code -> its texts. The first one is the fallback.
const LANGS: &[(&str, &Text)] = &[("en", &en::EN), ("es", &es::ES)];

/// Texts that carry data are functions, so that each language decides its plurals and word order.
pub struct Text {
    /// Groups of (usage, description) for `orcux help` and the `?` dialog: every new command or shortcut goes here.
    pub commands: &'static [(&'static str, &'static [(&'static str, &'static str)])],
    // keys in the footers: the verb after each key
    pub k_new: &'static str,
    pub k_add: &'static str,
    pub k_delete: &'static str,
    pub k_commands: &'static str,
    pub k_switch: &'static str,
    pub k_move: &'static str,
    pub k_close: &'static str,
    pub k_goto: &'static str,
    pub k_view: &'static str,
    pub k_agent: &'static str,
    pub k_paste: &'static str,
    pub k_create: &'static str,
    pub k_web: &'static str,
    pub k_hide: &'static str,
    pub k_refresh: &'static str,
    pub k_back: &'static str,
    pub k_yes: &'static str,
    pub k_no: &'static str,
    pub hint_choose: &'static str,
    pub k_diff: &'static str,
    pub k_only_changes: &'static str,
    pub k_all_files: &'static str,
    pub files_reading: &'static str,
    pub files_clean: &'static str,
    pub files_none: &'static str,
    // dialogs
    pub title_commands: &'static str,
    pub title_new: &'static str,
    pub title_rename: &'static str,
    pub title_child: &'static str,
    pub title_add: &'static str,
    pub title_error: &'static str,
    pub title_unregister: &'static str,
    pub title_rm: &'static str,
    pub title_pr: &'static str,
    pub title_close: &'static str,
    pub title_switch: &'static str,
    pub title_inbox: &'static str,
    pub skill_installed: fn(&str) -> String,
    pub skill_failed: fn(&str) -> String,
    pub doc_skill: &'static str,
    pub doc_no_skill: &'static str,
    pub detached: &'static str,
    pub window_inside: &'static str,
    pub ui_now: fn(&str, &str) -> String,
    pub engine_now: fn(&str, &str) -> String,
    pub doc_engine: &'static str,
    pub hint_new: &'static str,
    pub hint_rename: &'static str,
    pub hint_add: &'static str,
    pub hint_pr: &'static str,
    pub hint_force: &'static str,
    pub hint_force_key: &'static str,
    pub hint_any: &'static str,
    pub hint_scroll: &'static str,
    pub hint_close: &'static str,
    pub hint_view: &'static str,
    pub label_repo: &'static str,
    pub label_parent: &'static str,
    pub parent_dirty: fn(&str) -> String,
    pub label_project: &'static str,
    pub will_register: fn(&str) -> String,
    pub will_clone: fn(&str) -> String,
    pub will_create: fn(&str) -> String,
    pub cloning: &'static str,
    pub path_missing: fn(&str) -> String,
    pub bad_name: &'static str,
    pub repo_listed: &'static str,
    pub repo_has_wts: fn(usize) -> String,
    pub label_name: &'static str,
    pub label_branch: &'static str,
    pub label_port: &'static str,
    pub creating: &'static str,
    pub renaming: &'static str,
    /// what else changes with the name of one created by orcux, and of one imported
    pub rename_moves: &'static str,
    pub rename_imported: &'static str,
    pub fetching: &'static str,
    pub deleting: &'static str,
    // before and after the worktree, which goes in bold
    pub ask_rm: (&'static str, &'static str),
    pub ask_unregister: (&'static str, &'static str),
    pub ask_close: (&'static str, &'static str),
    pub kept_rm: &'static str,
    pub kept_unregister: &'static str,
    pub kept_repo: &'static str,
    pub kept_close: &'static str,
    // sidebar, status bar and switcher
    pub prs_yours: fn(usize) -> String,
    pub n_working: fn(usize) -> String,
    pub agent_asks: &'static str,
    pub agent_permission: &'static str,
    pub agent_failed: &'static str,
    pub agent_done: &'static str,
    pub agents_wait: fn(usize) -> String,
    pub agents_short: fn(usize) -> String,
    pub unopened: &'static str,
    pub gone: &'static str,
    pub no_repos: &'static str,
    pub search: &'static str,
    pub visible: &'static str,
    pub no_worktrees: &'static str,
    pub no_match: &'static str,
    // pull requests
    pub your_prs: &'static str,
    pub updating: &'static str,
    pub loading: &'static str,
    pub ago: fn(&str) -> String,
    pub counts: fn(usize, usize) -> String,
    pub group_yours: &'static str,
    pub group_going: &'static str,
    pub group_ready: &'static str,
    pub no_prs: &'static str,
    pub asks_review: fn(&str) -> String,
    pub draft: &'static str,
    pub draft_red: &'static str,
    pub conflicts: &'static str,
    pub ci_red: fn(usize, usize) -> String,
    pub changes: &'static str,
    pub approved: &'static str,
    pub waiting: &'static str,
    pub open: &'static str,
    pub fork: &'static str,
    pub more: fn(usize) -> String,
    pub s_checks: &'static str,
    /// a check that fails but blocks nothing
    pub optional: fn(&str) -> String,
    pub s_threads: &'static str,
    pub s_conflicts: &'static str,
    pub none: &'static str,
    pub unknown: &'static str,
    pub n_unresolved: fn(usize) -> String,
    pub n_missing: fn(usize) -> String,
    pub n_lines: fn(usize) -> String,
    pub v_checks: fn(usize) -> String,
    pub v_only: &'static str,
    pub no_conflict_files: &'static str,
    pub d_threads: fn(usize) -> String,
    pub d_reviews: &'static str,
    pub d_files: &'static str,
    pub d_body: &'static str,
    pub r_approved: fn(&str) -> String,
    pub r_changes: fn(&str) -> String,
    pub r_commented: fn(&str) -> String,
    pub r_pending: fn(&str) -> String,
    pub r_nobody: &'static str,
    pub a_team: &'static str,
    pub ask_title: &'static str,
    pub ask_intro: fn(&str) -> String,
    pub ask_conflicts: fn(&str, &str) -> String,
    pub ask_checks: fn(&str, &str) -> String,
    pub ask_threads: fn(&str) -> String,
    pub ask_outro: &'static str,
    pub ask_review: fn(&str) -> String,
    pub no_agent: &'static str,
    pub no_wt_agent: &'static str,
    pub nothing_to_ask: &'static str,
    pub no_account: &'static str,
    pub no_token: &'static str,
    // command line
    pub not_repo: &'static str,
    pub no_worktree: fn(&str) -> String,
    pub no_worktree_here: &'static str,
    pub ambiguous: &'static str,
    pub exists: fn(&str) -> String,
    /// a worktree and the program that runs in one of its terminals
    pub rename_busy: fn(&str, &str) -> String,
    pub branch_exists: fn(&str) -> String,
    pub non_utf8: &'static str,
    pub not_running: &'static str,
    pub no_tab: fn(&str) -> String,
    pub which_tab: &'static str,
    pub no_visible: &'static str,
    pub no_pane: &'static str,
    pub gone_at: fn(&str) -> String,
    pub not_worktree: &'static str,
    pub not_root: &'static str,
    pub main_checkout: &'static str,
    pub no_folder: &'static str,
    pub registered: &'static str,
    pub id_taken: fn(&str) -> String,
    /// its folder holds another worktree
    pub has_inside: fn(&str) -> String,
    pub imported: fn(usize) -> String,
    pub to_import: fn(usize) -> String,
    pub lang_now: fn(&str, &str) -> String,
    pub lang_bad: fn(&str, &str) -> String,
    pub icons_now: fn(&str, &str) -> String,
    pub rm_dirty: fn(&str) -> String,
    pub rm_locked: fn(&str) -> String,
    pub rm_submodules: fn(&str) -> String,
    pub merge_moved: fn(&str) -> String,
    pub merge_kept: fn(&str, &str) -> String,
    pub config_bad: fn(usize, &str) -> String,
    pub config_none: fn(&str) -> String,
    pub trust_person: &'static str,
    pub trust_ask: &'static str,
    pub trust_no: &'static str,
    pub untrusted: fn(&str) -> String,
    pub teardown_failed: fn(&str, &str) -> String,
    pub port_used: &'static str,
    pub merge_where: &'static str,
    pub merge_detached: &'static str,
    pub merge_self: fn(&str) -> String,
    pub merge_no_branch: fn(&str) -> String,
    pub merge_dirty: fn(&str) -> String,
    pub merge_pr: fn(&str) -> String,
    pub merge_busy: &'static str,
    pub merge_nothing: fn(&str, &str) -> String,
    pub merge_clash: fn(&str, &str) -> String,
    pub merge_others: fn(&str, &str) -> String,
    pub merge_target_dirty: fn(&str, &str) -> String,
    pub merge_failed: &'static str,
    pub merge_plan: fn(&str, &str, usize) -> String,
    pub merge_then: &'static str,
    pub merge_done: fn(&str) -> String,
    pub merge_usage: &'static str,
    pub title_merge: &'static str,
    pub conflicts_none: &'static str,
    pub restored: fn(usize) -> String,
    pub restore_offer: fn(usize) -> String,
    pub restore_usage: &'static str,
    pub unknown_agent: fn(&str) -> String,
    pub agents_usage: &'static str,
    pub run_usage: &'static str,
    pub wait_usage: &'static str,
    pub nobody_waits: &'static str,
    pub doc_no_tmux: &'static str,
    pub doc_old_tmux: &'static str,
    pub doc_no_git: &'static str,
    pub doc_too_old: fn(&str) -> String,
    pub doc_gh: &'static str,
    pub doc_no_gh: &'static str,
    pub doc_locale: &'static str,
    pub doc_option: &'static str,
    pub doc_no_repo: fn(&str) -> String,
    pub doc_no_folder: fn(&str) -> String,
    pub doc_link: fn(&str, &str) -> String,
    pub doc_counts: fn(usize, usize) -> String,
    pub doc_focus: &'static str,
    pub doc_bad: fn(usize) -> String,
}

fn current() -> &'static (&'static str, &'static Text) {
    static LANG: OnceLock<&(&str, &Text)> = OnceLock::new();
    LANG.get_or_init(|| {
        let var = |k: &str| env::var(k).unwrap_or_default();
        let saved = fs::read_to_string(state_dir().join("lang")).unwrap_or_default();
        pick(&[&var("ORCUX_LANG"), &saved], &[&var("LC_ALL"), &var("LC_MESSAGES"), &var("LANG")])
    })
}

/// The texts of the current language.
pub fn tr() -> &'static Text {
    current().1
}

pub fn code() -> &'static str {
    current().0
}

pub fn codes() -> String {
    LANGS.iter().map(|l| l.0).collect::<Vec<_>>().join(", ")
}

/// "es_ES.UTF-8" -> the entry of "es".
fn find(locale: &str) -> Option<&'static (&'static str, &'static Text)> {
    let lang = locale.trim().split(['_', '-', '.', '@']).next()?.to_lowercase();
    LANGS.iter().find(|l| l.0 == lang)
}

/// The first choice that is a known language; else the system's, which is its first variable that is set.
fn pick(choices: &[&str], system: &[&str]) -> &'static (&'static str, &'static Text) {
    choices.iter().find_map(|c| find(c))
        .or_else(|| find(system.iter().find(|s| !s.is_empty())?))
        .unwrap_or(&LANGS[0])
}

/// Saves the language, for every later orcux process, and returns its texts.
pub fn set(lang: &str) -> Res<&'static Text> {
    let (code, text) = LANGS.iter().find(|l| l.0 == lang).ok_or_else(|| (tr().lang_bad)(lang, &codes()))?;
    fs::write(state_dir().join("lang"), code).map_err(|e| e.to_string())?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::text::Span;

    #[test]
    fn which_language() {
        let p = |c: &[&str], s: &[&str]| pick(c, s).0;
        assert_eq!(p(&["", ""], &["", "", "es_ES.UTF-8"]), "es");
        assert_eq!(p(&["en", "es"], &["es_ES.UTF-8"]), "en");
        assert_eq!(p(&["xx", "es\n"], &["en_US.UTF-8"]), "es");
        assert_eq!(p(&["", ""], &["C", "", "es_ES.UTF-8"]), "en"); // LC_ALL wins over LANG
        assert_eq!(p(&["", ""], &["", "", "fr_FR.UTF-8"]), "en");
        assert_eq!(p(&["", ""], &["", "", ""]), "en");
    }

    /// Same commands in every language, and everything fits: sidebar 36 columns, rail 44, dialogs 60, commands 94.
    #[test]
    fn every_language_fits() {
        let width = |s: &str| Span::raw(s).width();
        let keys = |k: &[(&str, &str)]| k.iter().map(|(a, b)| width(a) + width(b) + 3).sum::<usize>();
        for (lang, t) in LANGS {
            let shape = |t: &Text| t.commands.iter().map(|g| g.1.len()).collect::<Vec<_>>();
            assert_eq!(shape(t), shape(LANGS[0].1), "{lang}: commands");
            for (usage, desc) in t.commands.iter().flat_map(|g| g.1) {
                assert!(width(usage) <= 40 && width(desc) <= 53, "{lang}: {usage}");
            }
            for k in [
                vec![("n", t.k_new), ("A", t.k_add), ("d", t.k_delete)],
                vec![("Opt+o", t.k_commands), ("Opt+w", t.k_switch)],
                vec![("⏎", t.k_diff), ("c", t.k_only_changes)],
                vec![("esc", t.k_back), ("Opt+o", t.k_commands)],
            ] {
                assert!(keys(&k) <= 36, "{lang}: sidebar {k:?}");
            }
            for k in [
                vec![("⏎", t.k_view), ("o", t.k_web)],
                vec![("⏎", t.k_view), ("w", t.k_create), ("o", t.k_web)],
                vec![("p", t.k_hide), ("r", t.k_refresh), ("tab", t.k_back)],
            ] {
                assert!(keys(&k) <= 44, "{lang}: rail {k:?}");
            }
            assert!(width(t.k_yes) + width(t.k_no) + width(t.hint_choose) + width(t.hint_force_key) + 10 <= 60, "{lang}: choice");
            for s in [t.hint_new, t.hint_rename, t.renaming, t.rename_moves, t.rename_imported, t.hint_pr, t.hint_force, t.hint_any, t.kept_rm, t.kept_unregister, t.kept_close, t.kept_repo, t.hint_add, t.fetching] {
                assert!(width(s) <= 60, "{lang}: {s}");
            }
            for s in [t.label_repo, t.label_parent, t.label_name, t.label_branch, t.label_port] {
                assert!(width(s) <= 6, "{lang}: {s}");
            }
            assert!(width(&(t.counts)(99, 99)) <= 42, "{lang}: counts");
        }
    }
}
