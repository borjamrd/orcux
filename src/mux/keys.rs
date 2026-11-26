//! Keys by tmux's names, as bytes for a terminal: in the mode its program set (application cursor keys).

/// The bytes of one key name, or None if it is not a name (then it is sent as text).
pub fn bytes(name: &str, app_cursor: bool) -> Option<Vec<u8>> {
    let (mut ctrl, mut alt, mut shift, mut rest) = (false, false, false, name);
    loop {
        if let Some(r) = rest.strip_prefix("C-") {
            (ctrl, rest) = (true, r);
        } else if let Some(r) = rest.strip_prefix("M-") {
            (alt, rest) = (true, r);
        } else if let Some(r) = rest.strip_prefix("S-") {
            (shift, rest) = (true, r);
        } else {
            break;
        }
    }
    // modifiers of the xterm sequences: 1 + shift 1, alt 2, ctrl 4
    let m = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);
    let csi = |final_: char| -> Vec<u8> {
        if m > 1 { format!("\x1b[1;{m}{final_}").into_bytes() } else if app_cursor { format!("\x1bO{final_}").into_bytes() } else { format!("\x1b[{final_}").into_bytes() }
    };
    let tilde = |n: u8| -> Vec<u8> { if m > 1 { format!("\x1b[{n};{m}~").into_bytes() } else { format!("\x1b[{n}~").into_bytes() } };
    let mut b: Vec<u8> = match rest {
        "Enter" => b"\r".to_vec(),
        "Tab" => b"\t".to_vec(),
        "BTab" => return Some(b"\x1b[Z".to_vec()),
        "BSpace" => b"\x7f".to_vec(),
        "Escape" => b"\x1b".to_vec(),
        "Space" => b" ".to_vec(),
        "Up" => return Some(csi('A')),
        "Down" => return Some(csi('B')),
        "Right" => return Some(csi('C')),
        "Left" => return Some(csi('D')),
        "Home" => return Some(csi('H')),
        "End" => return Some(csi('F')),
        "IC" => return Some(tilde(2)),
        "DC" => return Some(tilde(3)),
        "PPage" => return Some(tilde(5)),
        "NPage" => return Some(tilde(6)),
        f if f.starts_with('F') && f[1..].parse::<u8>().is_ok() => {
            let n: u8 = f[1..].parse().ok()?;
            return match n {
                1..=4 => Some(if m > 1 { format!("\x1b[1;{m}{}", (b'P' + n - 1) as char).into_bytes() } else { format!("\x1bO{}", (b'P' + n - 1) as char).into_bytes() }),
                5 => Some(tilde(15)),
                6..=10 => Some(tilde(11 + n)),
                11 | 12 => Some(tilde(12 + n)),
                _ => None,
            };
        }
        r if r.chars().count() == 1 && (ctrl || alt) => {
            let c = r.chars().next()?;
            let mut s = c.to_string().into_bytes();
            if ctrl {
                let low = c.to_ascii_lowercase();
                s = vec![match low {
                    'a'..='z' => low as u8 - b'a' + 1,
                    '@' | ' ' | '2' => 0,
                    '[' | '3' => 0x1b,
                    '\\' | '4' => 0x1c,
                    ']' | '5' => 0x1d,
                    '^' | '6' => 0x1e,
                    '_' | '/' | '7' => 0x1f,
                    '?' | '8' => 0x7f,
                    _ => return None,
                }];
            }
            if alt {
                s.insert(0, 0x1b);
            }
            return Some(s);
        }
        _ => return None,
    };
    if ctrl && rest == "Enter" {
        b = b"\r".to_vec();
    }
    if alt {
        b.insert(0, 0x1b);
    }
    Some(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(bytes("Enter", false), Some(b"\r".to_vec()));
        assert_eq!(bytes("C-c", false), Some(vec![3]));
        assert_eq!(bytes("M-Enter", false), Some(b"\x1b\r".to_vec()));
        assert_eq!(bytes("M-w", false), Some(b"\x1bw".to_vec()));
        assert_eq!(bytes("Up", false), Some(b"\x1b[A".to_vec()));
        assert_eq!(bytes("Up", true), Some(b"\x1bOA".to_vec()));
        assert_eq!(bytes("S-Up", true), Some(b"\x1b[1;2A".to_vec()));
        assert_eq!(bytes("BTab", false), Some(b"\x1b[Z".to_vec()));
        assert_eq!(bytes("NPage", false), Some(b"\x1b[6~".to_vec()));
        assert_eq!(bytes("F5", false), Some(b"\x1b[15~".to_vec()));
        assert_eq!(bytes("C-]", false), Some(vec![0x1d]));
        assert_eq!(bytes("hello", false), None);
        assert_eq!(bytes("x", false), None);
    }
}
