//! What goes over the engine's socket. A request: a byte (C for commands, A to attach as a control client), the pane
//! that asks (`TMUX_PANE`, or nothing), and the words, each with its length. A reply to commands: a byte (0 worked, 1
//! not) and the text. A control client then speaks tmux's control mode, in lines.

pub struct Request {
    pub attach: bool,
    pub caller: Option<String>,
    pub argv: Vec<String>,
}

fn put(out: &mut Vec<u8>, s: &[u8]) {
    out.extend((s.len() as u32).to_le_bytes());
    out.extend(s);
}

impl Request {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![if self.attach { b'A' } else { b'C' }];
        put(&mut out, self.caller.as_deref().unwrap_or("").as_bytes());
        out.extend((self.argv.len() as u32).to_le_bytes());
        for a in &self.argv {
            put(&mut out, a.as_bytes());
        }
        out
    }

    /// A whole request at the start of `b`, and how many bytes it took.
    pub fn decode(b: &[u8]) -> Option<(Request, usize)> {
        let mut at = 1;
        let kind = *b.first()?;
        let take = |at: &mut usize| -> Option<Vec<u8>> {
            let n = u32::from_le_bytes(b.get(*at..*at + 4)?.try_into().ok()?) as usize;
            let v = b.get(*at + 4..*at + 4 + n)?.to_vec();
            *at += 4 + n;
            Some(v)
        };
        let caller = String::from_utf8_lossy(&take(&mut at)?).into_owned();
        let n = u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?) as usize;
        at += 4;
        let mut argv = vec![];
        for _ in 0..n {
            argv.push(String::from_utf8_lossy(&take(&mut at)?).into_owned());
        }
        Some((Request { attach: kind == b'A', caller: Some(caller).filter(|c| !c.is_empty()), argv }, at))
    }

    /// The commands of the words: separated by a ";" word, or a word ending in an unescaped one.
    pub fn commands(&self) -> Vec<Vec<String>> {
        let mut cmds = vec![vec![]];
        for a in &self.argv {
            if a == ";" {
                cmds.push(vec![]);
            } else if let Some(w) = a.strip_suffix(';').filter(|w| !w.ends_with('\\')) {
                cmds.last_mut().unwrap_or_else(|| unreachable!()).push(w.to_string());
                cmds.push(vec![]);
            } else {
                cmds.last_mut().unwrap_or_else(|| unreachable!()).push(a.strip_suffix("\\;").map_or(a.clone(), |w| format!("{w};")));
            }
        }
        cmds.retain(|c| !c.is_empty());
        cmds
    }
}

pub fn reply(ok: bool, text: &str) -> Vec<u8> {
    let mut out = vec![u8::from(!ok)];
    put(&mut out, text.as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests() {
        let r = Request { attach: false, caller: Some("%3".into()), argv: vec!["set-option".into(), "-p".into(), "a\nb".into(), "x\\;".into()] };
        let mut bytes = r.encode();
        bytes.extend(b"more");
        let (back, used) = Request::decode(&bytes).unwrap();
        assert_eq!((back.caller.as_deref(), back.argv.len(), used), (Some("%3"), 4, bytes.len() - 4));
        assert_eq!(back.commands(), vec![vec!["set-option", "-p", "a\nb", "x;"]]);
        assert!(Request::decode(&bytes[..5]).is_none());
        let s = Request { attach: false, caller: None, argv: vec!["a".into(), ";".into(), "b".into(), "c;".into(), "d".into()] };
        assert_eq!(s.commands(), vec![vec!["a"], vec!["b", "c"], vec!["d"]]);
    }
}
