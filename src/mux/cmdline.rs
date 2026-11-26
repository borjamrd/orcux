//! Command lines as orcux writes them for tmux: words in single or double quotes or bare, joined (`'it'"'"'s'`), and
//! commands separated by `;` (a word that is only `;`, or ends in an unquoted one).

/// The commands of one line, each as its words.
pub fn parse(line: &str) -> Vec<Vec<String>> {
    let (mut cmds, mut cmd, mut word, mut quoted) = (vec![], vec![], String::new(), false);
    let mut chars = line.chars().peekable();
    let end_word = |cmd: &mut Vec<String>, word: &mut String, quoted: &mut bool| {
        if !word.is_empty() || *quoted {
            cmd.push(std::mem::take(word));
        }
        *quoted = false;
    };
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                quoted = true;
                for c in chars.by_ref() {
                    if c == '\'' {
                        break;
                    }
                    word.push(c);
                }
            }
            '"' => {
                quoted = true;
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => word.push(chars.next().unwrap_or('\\')),
                        c => word.push(c),
                    }
                }
            }
            '\\' => word.push(chars.next().unwrap_or('\\')),
            ' ' | '\t' => end_word(&mut cmd, &mut word, &mut quoted),
            ';' if !quoted && word.is_empty() => {
                end_word(&mut cmd, &mut word, &mut quoted);
                if !cmd.is_empty() {
                    cmds.push(std::mem::take(&mut cmd));
                }
            }
            c => word.push(c),
        }
    }
    end_word(&mut cmd, &mut word, &mut quoted);
    if !cmd.is_empty() {
        cmds.push(cmd);
    }
    cmds
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_and_commands() {
        let w = |l: &str| parse(l);
        assert_eq!(w("send-keys -t '%1' -l 'it'\"'\"'s'"), vec![vec!["send-keys", "-t", "%1", "-l", "it's"]]);
        assert_eq!(w("'a' ; 'b' 'c d'"), vec![vec!["a"], vec!["b", "c d"]]);
        assert_eq!(w("display-message -p 'x;y'"), vec![vec!["display-message", "-p", "x;y"]]);
        assert_eq!(w("set-option -t x '' v"), vec![vec!["set-option", "-t", "x", "", "v"]]);
        assert_eq!(w("a \"b \\\" c\""), vec![vec!["a", "b \" c"]]);
        assert!(w("   ").is_empty());
    }
}
