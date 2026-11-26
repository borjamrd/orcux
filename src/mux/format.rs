//! The part of tmux's formats that orcux uses: `#{name}`, `#{?cond,a,b}`, `#{==:a,b}`, `#{!=:a,b}`, `#{m:pattern,s}`,
//! `#{||:a,b}`, `#{&&:a,b}`, and the loops `#{S:…}` (sessions), `#{W:…}` (windows), `#{P:…}` (panes). `##` is `#`,
//! `#,` a comma inside a conditional.

/// What a format can ask for: a name, and the items of a loop.
pub trait Scope {
    fn get(&self, name: &str) -> Option<String>;
    /// the scopes of a loop: 'S', 'W' or 'P'
    fn each(&self, kind: char) -> Vec<Box<dyn Scope + '_>>;
}

pub fn expand(f: &str, s: &dyn Scope) -> String {
    let mut out = String::new();
    let b: Vec<char> = f.chars().collect();
    let mut i = 0;
    while i < b.len() {
        if b[i] == '#' && i + 1 < b.len() {
            match b[i + 1] {
                '#' => {
                    out.push('#');
                    i += 2;
                    continue;
                }
                ',' => {
                    out.push(',');
                    i += 2;
                    continue;
                }
                '{' => {
                    if let Some(end) = closing(&b, i + 2) {
                        let inner: String = b[i + 2..end].iter().collect();
                        out += &item(&inner, s);
                        i = end + 1;
                        continue;
                    }
                }
                _ => {}
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// Where the brace that opens at `from` (just inside it) closes.
fn closing(b: &[char], from: usize) -> Option<usize> {
    let (mut depth, mut i) = (1, from);
    while i < b.len() {
        match b[i] {
            '#' if b.get(i + 1) == Some(&'{') => {
                depth += 1;
                i += 1;
            }
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The parts of a format separated by commas at its own level (not inside a nested #{…}).
fn split(f: &str) -> Vec<String> {
    let (mut parts, mut cur, mut depth) = (vec![], String::new(), 0);
    let b: Vec<char> = f.chars().collect();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == '#' && b.get(i + 1) == Some(&',') && depth == 0 {
            cur.push_str("#,");
            i += 2;
            continue;
        }
        if c == '#' && b.get(i + 1) == Some(&'{') {
            depth += 1;
            cur.push_str("#{");
            i += 2;
            continue;
        }
        if c == '}' && depth > 0 {
            depth -= 1;
        }
        if c == ',' && depth == 0 {
            parts.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
        i += 1;
    }
    parts.push(cur);
    parts
}

fn truthy(v: &str) -> bool {
    !v.is_empty() && v != "0"
}

fn item(inner: &str, s: &dyn Scope) -> String {
    if let Some(rest) = inner.strip_prefix('?') {
        let p = split(rest);
        let cond = p.first().map(|c| if c.contains("#{") { expand(c, s) } else { s.get(c).unwrap_or_default() }).unwrap_or_default();
        let pick = if truthy(&cond) { p.get(1) } else { p.get(2) };
        return pick.map(|x| expand(x, s)).unwrap_or_default();
    }
    for (op, f) in [("==:", (|a: &str, b: &str| a == b) as fn(&str, &str) -> bool), ("!=:", |a, b| a != b)] {
        if let Some(rest) = inner.strip_prefix(op) {
            let p = split(rest);
            let v = |i: usize| p.get(i).map(|x| expand(x, s)).unwrap_or_default();
            return if f(&v(0), &v(1)) { "1" } else { "0" }.into();
        }
    }
    for (op, any) in [("||:", true), ("&&:", false)] {
        if let Some(rest) = inner.strip_prefix(op) {
            let v: Vec<bool> = split(rest).iter().map(|x| truthy(&expand(x, s))).collect();
            let r = if any { v.iter().any(|x| *x) } else { v.iter().all(|x| *x) };
            return if r { "1" } else { "0" }.into();
        }
    }
    if let Some(rest) = inner.strip_prefix("m:") {
        let p = split(rest);
        let pat = p.first().map(|x| expand(x, s)).unwrap_or_default();
        let text = p.get(1).map(|x| expand(x, s)).unwrap_or_default();
        return if glob(&pat, &text) { "1" } else { "0" }.into();
    }
    for kind in ['S', 'W', 'P'] {
        if let Some(rest) = inner.strip_prefix(&format!("{kind}:")) {
            return s.each(kind).iter().map(|x| expand(rest, x.as_ref())).collect();
        }
    }
    s.get(inner).unwrap_or_default()
}

/// fnmatch with * and ?.
pub fn glob(pat: &str, text: &str) -> bool {
    fn go(p: &[char], t: &[char]) -> bool {
        match (p.first(), t.first()) {
            (None, None) => true,
            (Some('*'), _) => go(&p[1..], t) || (!t.is_empty() && go(p, &t[1..])),
            (Some('?'), Some(_)) => go(&p[1..], &t[1..]),
            (Some(a), Some(b)) if a == b => go(&p[1..], &t[1..]),
            _ => false,
        }
    }
    go(&pat.chars().collect::<Vec<_>>(), &text.chars().collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct T(HashMap<&'static str, &'static str>, Vec<T>);
    impl Scope for T {
        fn get(&self, n: &str) -> Option<String> {
            self.0.get(n).map(|v| v.to_string())
        }
        fn each(&self, _: char) -> Vec<Box<dyn Scope + '_>> {
            self.1.iter().map(|t| Box::new(T(t.0.clone(), vec![])) as Box<dyn Scope>).collect()
        }
    }

    #[test]
    fn formats() {
        let s = T(HashMap::from([("pane_id", "%3"), ("@orcux", "__sidebar"), ("session_name", "orcux"), ("empty", "")]), vec![
            T(HashMap::from([("pane_id", "%1")]), vec![]),
            T(HashMap::from([("pane_id", "%2")]), vec![]),
        ]);
        assert_eq!(expand("#{pane_id}\t#{@orcux}\t#{nothing}.", &s), "%3\t__sidebar\t.");
        assert_eq!(expand("#{?@orcux,yes #{pane_id},no}", &s), "yes %3");
        assert_eq!(expand("#{?empty,yes,no}", &s), "no");
        assert_eq!(expand("#{==:#{session_name},orcux}", &s), "1");
        assert_eq!(expand("#{?#{m:__*,#{@orcux}},side,term}", &s), "side");
        assert_eq!(expand("#{?#{==:#{session_name},orcux},#[a]#,x,b}", &s), "#[a],x");
        assert_eq!(expand("#{P:#{pane_id};}", &s), "%1;%2;");
        assert_eq!(expand("a##b #{||:0,#{pane_id}}", &s), "a#b 1");
        assert!(glob("orcux*", "orcux-bg") && !glob("orcux", "orcux-bg"));
    }
}
