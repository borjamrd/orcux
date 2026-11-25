//! The orc that programs in the commands dialog. Layers of pixels, one character each, put together for every
//! frame so that each part moves at its own pace. Two pixels per cell.
use ratatui::prelude::*;

pub const W: usize = 48;
pub const H: usize = 44;

/// Where it goes (column, row) and its pixels; a space is see-through.
type Layer = (i32, i32, &'static [&'static str]);

fn color(c: u8) -> u32 {
    match c {
        b'0' => 0x12240f,
        b'1' => 0x141a26,
        b'2' => 0x1a2231,
        b'3' => 0x202a3b,
        b'n' => 0x0b1020,
        b'y' => 0xf3e6b3,
        b'Y' => 0xcdbf8a,
        b'*' => 0x8fa3c8,
        b'F' => 0x2c3446,
        b'f' => 0x3a4459,
        b'P' => 0xff5fa2,
        b'q' => 0x7a2f55,
        b'g' => 0x2f6a35,
        b'G' => 0x4b9447,
        b'L' => 0x74bf5f,
        b'l' => 0xa5de86,
        b'h' => 0x1b1512,
        b'H' => 0x3a2c22,
        b'r' => 0xc94f4f,
        b'o' => 0x0c0f0d,
        b'w' => 0xf4f1de,
        b'V' => 0xcfc7a6,
        b'e' => 0xf5f5ee,
        b'u' => 0x22301f,
        b'm' => 0x33121a,
        b't' => 0xd4626a,
        b's' => 0x7cc8ff,
        b'c' => 0x1f2a2a,
        b'C' => 0x0f3b33,
        b'x' => 0x46e0a0,
        b'X' => 0x1e8f6a,
        b'a' => 0xe8c15a,
        b'd' => 0x1c6a58,
        b'j' => 0x2f9a7d,
        b'J' => 0x56c9a4,
        b'W' => 0xdfe5df,
        b'k' => 0x14161b,
        b'b' => 0x3b2a20,
        b'B' => 0x523a2b,
        b'D' => 0x2a1d16,
        b'K' => 0x2b2f38,
        b'i' => 0x4a505c,
        b'I' => 0x7fe3ff,
        b'M' => 0xc9d3dc,
        b'N' => 0x8e99a5,
        b'T' => 0x5b3420,
        b'S' => 0x8a93a6,
        b'U' => 0xf6c945,
        b'v' => 0xd79a1e,
        b'O' => 0xee7b2d,
        _ => 0x000000,
    }
}

const ROOM: Layer = (0, 0, &[
    "111111111111111111111111111111111111111111111111",
    "111111111111111111111111111111111111111111111111",
    "11111111111111112222222222222222FFFFFFFFFFFFFF11",
    "11111111111112222222222222222222FnnnnnFnnnnnnF11",
    "11111111111222222222222222222222FfffffFffffffF11",
    "11111111122222222222222222222222Fnn*nnFnyyynnF11",
    "11111112222222222222222222222222FnnnnnFyyyyynF11",
    "11111122222222222222222222222222FfffffFyfyyyfF11",
    "11111222222222222222222222222222Fnnnn*FnyYynnF11",
    "11112222222222222222222222222222FnnnnnFnnnnnnF11",
    "11122222222222222233333333333332FfffffFffffffF11",
    "11222222222222223333333333333333FnnnnnFnnnn*nF21",
    "12222222222222333333333333333333FnnnnnF*nnnnnF22",
    "22222222222233333333333333333333FfffffFffffffF22",
    "22222222222333333333333333333333FFFFFFFFFFFFFF22",
    "222222222233333333333333333333333333333222222222",
    "222222222333333333333333333333333333333322222222",
    "222222222333333333333333333333333333333322222222",
    "222222223333333333333333333333333333333332222222",
    "222222223333333333333333333333333333333332222222",
    "222222233333333333333333333333333333333333222222",
    "222222233333333333333333333333333333333333222222",
    "222222233333333333333333333333333333333333222222",
    "222222233333333333333333333333333333333333222222",
    "222222233333333333333333333333333333333333222222",
    "222222233333333333333333333333333333333333222222",
    "222222233333333333333333333333333333333333222222",
    "222222233333333333333333333333333333333333222222",
    "222222233333333333333333333333333333333333222222",
    "222222223333333333333333333333333333333332222222",
    "222222223333333333333333333333333333333332222222",
    "222222222333333333333333333333333333333322222222",
    "222222222333333333333333333333333333333322222222",
    "222222222233333333333333333333333333333222222222",
    "222222222223333333333333333333333333332222222222",
    "222222222222333333333333333333333333322222222222",
    "122222222222223333333333333333333332222222222222",
    "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB",
    "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB",
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    "DbbbbDbbDbbDbbbbDbbDbbDbbbbDbbDbbDbbbbDbbDbbDbbb",
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
]);

const NEON: Layer = (3, 6, &[
    "P    ",
    " P   ",
    "  P  ",
    " P   ",
    "P PPP",
]);
const BODY: Layer = (10, 23, &[
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
const HAND_L: Layer = (0, 0, &[
    "  0000 ",
    " 0LLGG0",
    "0LGGGGg",
    "0GGGGg0",
    " 0g0g0 ",
]);
const HAND_R: Layer = (0, 0, &[
    " 0000  ",
    "0GGLL0 ",
    "gGGGGL0",
    "0gGGGG0",
    " 0g0g0 ",
]);
const SLEEVE_L: Layer = (8, 30, &[
    "   000    ",
    " 00jjj0   ",
    "0ddJJjj0  ",
    "0dJJJjjj0 ",
    "0JJJjjjjj0",
    "0dJJjjjjj0",
    "0ddjjjjjj0",
    "0ddjjjjjj0",
    " 00000000 ",
]);
const SLEEVE_R: Layer = (30, 30, &[
    "    000   ",
    "   0jjj00 ",
    "  0jjJJdd0",
    " 0jjjJJJd0",
    "0jjjjjJJJ0",
    "0jjjjjJJd0",
    "0jjjjjjdd0",
    "0jjjjjjdd0",
    " 00000000 ",
]);
const MUG: Layer = (0, 0, &[
    "MMMMMM ",
    "TTTTTT ",
    "MMMMNMM",
    "MMMMN M",
    "MMMMNMM",
    "MMMNNN ",
    " NNNN  ",
]);
const DUCK: Layer = (0, 0, &[
    "  UUU  ",
    "  UoUOO",
    "  UUUO ",
    "UUUUU  ",
    "UUvUUU ",
    "UvvvUU ",
    " UUUU  ",
]);
const KEYBOARD: Layer = (0, 0, &[
    "KKKKKKKKKKKKKKKKKKKKKKKKKK",
    "KiKiiKiKiiKiKiiKiKiiKiKiiK",
    "KKKKKKKKKKKKKKKKKKKKKKKKKK",
]);
const STEAM: [Layer; 3] = [
    (0, 0, &[" S ", "S  ", " S ", "  S"]),
    (0, 0, &["S  ", " S ", "  S", " S "]),
    (0, 0, &["  S", " S ", "S  ", " S "]),
];

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
    let mut f = Frame([[b'1'; W]; H]);
    let at = |list: &[i32], i: usize| list[i % list.len()];
    let wild = t % 96 >= 64;
    f.put(ROOM, 0, 0);
    f.put(NEON, 0, 0);
    if t % 29 < 2 || t % 41 == 7 {
        f.0.iter_mut().flatten().filter(|c| **c == b'P').for_each(|c| *c = b'q'); // it flickers
    }
    f.put(BODY, 0, 0);
    let dx = 8 + if wild { at(&[0, 1, 0, -1], t) } else { at(&[0, 0, 1, 1, 1, 0, 0, -1, -1, -1], t / 2) };
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
                    let c = if (line * 3 + x) % 4 > 0 { b'x' } else { b'X' };
                    f.dot(dx + (lens + x) as i32, dy + 14 + y as i32, c);
                }
            }
        }
    }
    f.put(KEYBOARD, 11, 38);
    let key = t * 7 % 24;
    f.dot(12 + key as i32, 39, b'I');
    f.dot(12 + ((key * 5 + 9) % 24) as i32, 39, b'I');
    let up = |down: bool| if down { 0 } else { -2 };
    let (left, right) = if wild { (t % 3 != 0, t % 3 != 1) } else { (t % 2 == 1, t % 2 == 0) };
    f.put(SLEEVE_L, 0, 0);
    f.put(SLEEVE_R, 0, 0);
    f.put(HAND_L, 13, 36 + up(left));
    f.put(HAND_R, 28, 36 + up(right));
    f.put(MUG, 40, 33);
    f.put(STEAM[t / 3 % 3], 41, 28);
    f.put(DUCK, 1, if wild && t % 4 == 0 { 31 } else { 32 });
    f
}

/// Lower half blocks: the upper ones leave a stripe in fonts that draw them shorter than the cell.
pub fn lines(t: usize) -> Vec<Line<'static>> {
    let f = frame(t);
    f.0.chunks(2).map(|rows| {
        let mut spans: Vec<Span> = vec![];
        let mut last = None;
        for (top, low) in rows[0].iter().zip(&rows[1]) {
            if last == Some((top, low)) {
                spans.last_mut().unwrap().content.to_mut().push('▄');
            } else {
                let st = Style::new().bg(Color::from_u32(color(*top))).fg(Color::from_u32(color(*low)));
                spans.push(Span::styled("▄", st));
                last = Some((top, low));
            }
        }
        Line::from(spans)
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pixel_has_a_color() {
        for t in 0..300 {
            assert!(frame(t).0.iter().flatten().all(|c| color(*c) != 0), "tick {t}");
            assert!(lines(t).iter().all(|l| l.width() == W) && lines(t).len() == H / 2);
        }
    }
}
