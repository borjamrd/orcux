//! A lasting connection to tmux (its control mode) for the panes that orcux keeps open: their commands need no
//! process, and tmux tells when something changes instead of being asked over and over. Without it (an old tmux, or
//! while it comes back) every command is a process, as in the rest of orcux.
use crate::*;
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Stdio},
    sync::{mpsc, Mutex},
    time::Duration,
};

pub enum News {
    /// panes, their marks or what they print
    Changed,
    /// a pane took the focus of its window, whichever they are
    Focus,
    Closed,
}

struct Link {
    child: Child,
    stdin: ChildStdin,
    /// whether each command worked, and what it printed
    replies: mpsc::Receiver<(bool, String)>,
    count: u64,
}

impl Drop for Link {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok(); // no zombies
    }
}

static LINK: Mutex<Option<Link>> = Mutex::new(None);

/// What tmux watches, once a second. A hidden terminal has its own window, so its activity is the window's; the
/// visible one shares it with the sidebar and the rail, whose own drawing must not count as news.
fn watched() -> String {
    let visible = "#{?#{m:__*,#{@orcux}},,#{cursor_x} #{cursor_y} #{history_bytes} #{pane_title}}";
    let pane = format!(
        "#{{pane_id}} #{{@orcux}} #{{@orcux_tab}} #{{@orcux_on}} #{{@orcux_state}} #{{pane_active}} #{{pane_current_command}} \
         #{{?#{{==:#{{session_name}},orcux}},{visible},#{{window_activity}}}};"
    );
    format!("#{{S:#{{?#{{m:orcux*,#{{session_name}}}},#{{session_attached}} #{{@orcux_prs}} #{{W:#{{window_width}} #{{P:{pane}}}}},}}}}")
}

pub fn connect(news: impl Fn(News) + Send + 'static) -> bool {
    close();
    let spawned = Command::new("tmux").args(["-u", "-C", "attach-session", "-t", UI, "-f", "ignore-size,no-output"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn();
    let Ok(mut child) = spawned else { return false };
    let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else { return false };
    let (tx, replies) = mpsc::channel();
    std::thread::spawn(move || {
        let mut block: Option<(String, Vec<String>)> = None;
        for line in BufReader::new(stdout).split(b'\n') {
            let Ok(line) = line else { break };
            let line = String::from_utf8_lossy(&line).into_owned();
            if let Some((head, text)) = &mut block {
                let end = line.strip_prefix("%end ").map(|h| (true, h)).or(line.strip_prefix("%error ").map(|h| (false, h)));
                match end.filter(|e| e.1 == head) {
                    Some((ok, _)) => {
                        // 1: asked by this connection
                        if head.ends_with(" 1") && tx.send((ok, text.join("\n"))).is_err() {
                            break;
                        }
                        block = None;
                    }
                    None => text.push(line),
                }
            } else if let Some(head) = line.strip_prefix("%begin ") {
                block = Some((head.into(), vec![]));
            } else if line.starts_with("%subscription-changed ") {
                news(News::Changed);
            } else if line.starts_with("%window-pane-changed ") {
                news(News::Focus);
            }
        }
        drop(tx);
        news(News::Closed);
    });
    if let Ok(mut l) = LINK.lock() {
        *l = Some(Link { child, stdin, replies, count: 0 });
    }
    command(&["refresh-client", "-B", &format!("orcux::{}", watched())]).is_some_and(|r| r.is_ok())
}

/// Also before becoming another version of itself: the process of the connection must not outlive it.
pub fn close() {
    drop(LINK.lock().ok().and_then(|mut l| l.take()));
}

pub fn connected() -> bool {
    LINK.lock().is_ok_and(|l| l.is_some())
}

/// Single quotes keep everything as it is in tmux's parser too.
fn quote(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', "'\"'\"'"))
}

/// Not those that act on the client that asks, that would block the connection, or that start programs: these get
/// the PATH of an unattached client only.
fn suits(args: &[&str]) -> bool {
    !matches!(args.first(), None | Some(&"display-popup" | &"detach-client" | &"switch-client" | &"attach-session" | &"new-session" | &"new-window" | &"split-window" | &"respawn-pane"))
        && !args.iter().any(|a| a.chars().any(|c| c.is_control() && c != '\t'))
}

/// None: without a connection, or not a command for it.
pub fn command(args: &[&str]) -> Option<Res<String>> {
    let mut guard = LINK.lock().ok()?;
    let link = guard.as_mut()?;
    if !suits(args) {
        return None;
    }
    link.count += 1;
    // a command can answer in several blocks (`if-shell`): a second one marks the end
    let end = format!("orcux-end-{}", link.count);
    let line = args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ");
    if link.stdin.write_all(format!("{line}\ndisplay-message -p {end}\n").as_bytes()).is_err() {
        *guard = None;
        return None;
    }
    let mut first = None;
    loop {
        match link.replies.recv_timeout(Duration::from_secs(5)) {
            Ok((true, text)) if text == end => break,
            Ok(reply) => { first.get_or_insert(reply); }
            Err(_) => {
                *guard = None;
                return Some(Err(format!("tmux {}: no answer", args.join(" ")))); // it may have run: not again
            }
        }
    }
    let (ok, text) = first.unwrap_or((true, String::new()));
    Some(if ok { Ok(text.trim_end().into()) } else { Err(format!("tmux {}: {text}", args.join(" "))) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes() {
        assert_eq!(quote("it's #{x} $HOME ~"), "'it'\"'\"'s #{x} $HOME ~'");
        assert!(suits(&["list-panes", "-F", "a\tb"]) && !suits(&["display-popup"]) && !suits(&["set-buffer", "a\nb"]));
    }
}
