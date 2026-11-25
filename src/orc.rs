//! The orc that programs in the commands dialog. Layers of pixels, one character each, put together for every
//! frame so that each part moves at its own pace.
use crate::{pixels, theme};
use ratatui::prelude::*;

pub const W: usize = 36;
pub const H: usize = 32;

/// Where it goes (column, row) and its pixels; a space is see-through.
type Layer = (i32, i32, &'static [&'static str]);

/// Ink of each character, out of 100.
fn ink(c: u8) -> Option<u32> {
    Some(match c {
        b' ' | b'0' => 0,
        b'g' => 30,
        b'G' => 46,
        b'L' => 66,
        b'l' => 82,
        b'h' => 20,
        b'H' => 34,
        b'r' => 90,
        b'w' => 100,
        b'V' => 86,
        b'e' => 100,
        b'u' => 8,
        b'm' => 4,
        b't' => 50,
        b's' => 60,
        b'c' => 92,
        b'C' => 8,
        b'x' => 88,
        b'X' => 48,
        b'a' => 100,
        b'd' => 14,
        b'j' => 20,
        b'J' => 30,
        b'W' => 62,
        b'k' => 8,
        _ => return None,
    })
}

const BODY: Layer = (4, 23, &[
    "          000000000          ",
    "       000dgggggggd000       ",
    "      0ddddgggggggdddd0      ",
    "     0ddddkgggggggddddd0     ",
    "    0dddddWkkkkkkkWddddd0    ",
    "   0JJJdddWkkkkkkkWddJJJJ0   ",
    " 00JJJjJddWkkkkkkkWddJJJJJ0  ",
    "0JJJJjjJjjWkkXkXkkWjjJjJJJJ0 ",
    "0dJJdjjJjjWkXkkkXkWjjJjjJJd0 ",
    "0ddddjjJjjWkkXkXkkWjjJjjdddd0",
    "0ddddjjJjjWkkkkkkkWjjJjjdddd0",
    "0ddddjjJjjWkkkkkkkWjjJjjdddd0",
    "0ddddjjJjjWkkkkkkkWjjJjjdddd0",
    "0ddddjjjjjWkkkkkkkWjjjjjdddd0",
]);
const HEAD: Layer = (0, 0, &[
    "               hh               ",
    "              HHhh              ",
    "             hhhhhh             ",
    "              rrrr              ",
    "              hhhhh             ",
    "           0HHhhhhHhh           ",
    "          0hhhhhhhhhh0          ",
    "         0hLLLLLLLLLLh0         ",
    "        0GGLLLLLLLLLLGG0        ",
    "       0gGGLLLLLLLLLLGGg0      0",
    " 00   0ggGGGLLLLLLLLGGGgg0   00G",
    "0GG00 0gggGGGGGGGGGGggGgg0 00GG0",
    "0GGgG00gg0ggGGGGGGgg0GGgg00GGGG0",
    " 0GGgGgggG000gGGg000GGGgggGggGG0",
    " 0GGgggggGGGGGGGGGGGGGGgggggGG0 ",
    "  0GGggggGGGGGGGGGGGGlGggggGG0  ",
    "   0GGgggGGGGGLLGlGGGlGggGGGG0  ",
    "   aGGGggGGGGGGLLGGGGlGggGGG0   ",
    "   aaGGggLLLGgGGGGgGLLLggGG0    ",
    "     00ggLLLL0g00g0LLLLgg00     ",
    "      0ggGLGGGGGGGGGGLGgg0      ",
    "      0ggGGGGGGGGGGGGGGgg0      ",
    "       0gGGGGGGGGGGGGGGg0       ",
    "        0GGGLLLLLLLLGGG0        ",
    "         0gllllllllllg0         ",
    "          00gggggggg00          ",
    "            00000000            ",
]);
const GLASSES: Layer = (7, 13, &[
    "ccccccc    ccccccc",
    "cCCCCCccccccCCCCCc",
    "cCCCCCc    cCCCCCc",
    "cCCCCCc    cCCCCCc",
    " ccccc      ccccc ",
]);
const FRAME: Layer = (7, 13, &[
    "ccccccc    ccccccc",
    "c     cccccc     c",
    "c     c    c     c",
    "c     c    c     c",
    " ccccc      ccccc ",
]);
const BLINK: Layer = (8, 14, &[
    "GGGGG      GGGGG",
    "GGGGG      GGGGG",
    "00000      00000",
]);
const ROLLED: Layer = (8, 14, &[
    "uueuu      uueuu",
    "eeeee      eeeee",
    "eeeee      eeeee",
]);
const MOUTH: Layer = (10, 20, &[
    "w          w",
    "w0        0w",
    "V0mmmmmmmm0V",
    " 0000000000 ",
]);
const SHOUT: Layer = (10, 20, &[
    "w 00000000 w",
    "w0mmmmmmmm0w",
    "V0mmttttmm0V",
    " 0mttttttm0 ",
    "  00000000  ",
]);

struct Frame([[u8; W]; H]);

impl Frame {
    fn dot(&mut self, x: i32, y: i32, c: u8) {
        if (0..W as i32).contains(&x) && (0..H as i32).contains(&y) {
            self.0[y as usize][x as usize] = c;
        }
    }

    fn put(&mut self, layer: Layer, dx: i32, dy: i32) {
        for (j, row) in layer.2.iter().enumerate() {
            for (i, c) in row.bytes().enumerate().filter(|c| c.1 != b' ') {
                self.dot(layer.0 + dx + i as i32, layer.1 + dy + j as i32, c);
            }
        }
    }
}

/// The frame at tick `t`. Every so often he loses it for a while.
fn frame(t: usize) -> Frame {
    let mut f = Frame([[b' '; W]; H]);
    let at = |list: &[i32], i: usize| list[i % list.len()];
    let wild = t % 96 >= 64;
    f.put(BODY, 0, 0);
    let dx = 2 + if wild { at(&[0, 1, 0, -1], t) } else { at(&[0, 0, 1, 1, 1, 0, 0, -1, -1, -1], t / 2) };
    let dy = 1 + if wild { t % 2 } else { t / 4 % 2 } as i32;
    f.put(HEAD, dx, dy);
    if wild {
        f.put(ROLLED, dx, dy);
        f.put(SHOUT, dx, dy);
        f.put(FRAME, dx, dy);
        for (i, (x, y)) in [(5, 9), (26, 8), (4, 12), (27, 12)].into_iter().enumerate() {
            let k = ((t + i * 3) % 6) as i32;
            if k < 4 {
                f.dot(dx + x + if x < 16 { -k / 2 } else { k / 2 }, dy + y + k, b's'); // sweat
            }
        }
    } else if t % 37 < 2 {
        f.put(MOUTH, dx, dy);
        f.put(BLINK, dx, dy);
        f.put(FRAME, dx, dy);
    } else {
        f.put(MOUTH, dx, dy);
        f.put(GLASSES, dx, dy);
        for lens in [8, 19] {
            for y in 0..3 {
                let line = y + t / 2; // code that scrolls by
                let (indent, len) = (line % 2, 1 + (line * 7 + lens) % 5);
                for x in indent..(indent + len - 1).min(5) {
                    let c = if !(line * 3 + x).is_multiple_of(4) { b'x' } else { b'X' };
                    f.dot(dx + (lens + x) as i32, dy + 14 + y as i32, c);
                }
            }
        }
    }
    f
}

pub fn lines(t: usize) -> Vec<Line<'static>> {
    let rows: Vec<Vec<u32>> = frame(t).0.iter().map(|r| r.iter().map(|c| ink(*c).unwrap_or(0)).collect()).collect();
    pixels::lines(&rows, theme::HULL)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pixel_has_ink() {
        for t in 0..300 {
            assert!(frame(t).0.iter().flatten().all(|c| ink(*c).is_some()), "tick {t}");
            assert!(lines(t).iter().all(|l| l.width() == W) && lines(t).len() == H / 2);
        }
    }
}
