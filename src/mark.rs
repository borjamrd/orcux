//! The logo: the "o" of Fraunces Bold with an "x" as its subscript, in braille dots, with a gradient. In the sidebar's
//! border, only the "o", of four by four dots.
use crate::theme;

/// `t` hundredths of the way from one color to the other.
pub fn mix(from: u32, to: u32, t: u32) -> u32 {
    [16, 8, 0].into_iter().fold(0, |c, s| (c << 8) | (((from >> s & 255) * (100 - t) + (to >> s & 255) * t) / 100))
}
use ratatui::prelude::*;

pub const W: u16 = 43;
pub const H: u16 = 18;

const MARK: [&str; H as usize] = [
    "         ⣀⣤⣴⣶⣾⣿⣿⣿⣿⣿⣿⣷⣶⣶⣤⣄⡀                 ",
    "     ⢀⣤⣶⣿⣿⣿⣿⡿⠟⠛⠉⠛⠻⣿⣿⣿⣿⣿⣿⣿⣿⣷⣦⣀              ",
    "   ⢀⣴⣿⣿⣿⣿⣿⣿⡟⠁     ⠈⢻⣿⣿⣿⣿⣿⣿⣿⣿⣿⣷⣄            ",
    "  ⣰⣿⣿⣿⣿⣿⣿⣿⣿⠁        ⢹⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣦           ",
    " ⣼⣿⣿⣿⣿⣿⣿⣿⣿⣿          ⢻⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣧          ",
    "⢰⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿          ⠈⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡆         ",
    "⣾⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿           ⢻⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣷         ",
    "⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡇          ⠸⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿         ",
    "⢿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣧           ⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡿         ",
    "⠸⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡄          ⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⠇         ",
    " ⢻⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣷          ⣿⣿⣿⣿⣿⣿⣿⣿⣿⡟          ",
    "  ⠻⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣧        ⢀⣿⣿⣿⣿⣿⣿⣿⣿⠟           ",
    "   ⠙⢿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣧⡀      ⣼⣿⣿⣿⣿⣿⣿⠟⠁            ",
    "     ⠈⠻⢿⣿⣿⣿⣿⣿⣿⣿⣿⣦⣤⣀⣠⣴⣾⣿⣿⣿⣿⠿⠛⠁  ⠙⢿⣿⣿⣿⣟⠃ ⠛⣿⠟⠃",
    "        ⠈⠙⠛⠿⠿⢿⣿⣿⣿⣿⣿⣿⠿⠿⠟⠛⠉        ⠹⣿⣿⣿⣦⣀⡼⠃  ",
    "                                  ⢘⣿⣿⣿⣿⣄   ",
    "                                 ⣠⡞⠁⠻⣿⣿⣿⣦⡀ ",
    "                               ⣠⣾⣿⣤ ⢠⣽⣿⣿⣿⣷⣤",
];

/// From deep to light, left to right and a little downwards.
fn color(x: usize, y: usize) -> u32 {
    let (w, h) = (W as usize, H as usize);
    let t = (x * 4 * h + y * w) * 100 / (4 * w * h + w * h);
    match t {
        0..50 => mix(theme::DEEP, theme::CURRENT, t as u32 * 2),
        _ => mix(theme::CURRENT, theme::FOAM, (t as u32 - 50) * 2),
    }
}

/// The dots of the mark: (columns, rows) of dots, two by four per cell.
fn dots() -> Vec<Vec<bool>> {
    let (w, h) = (W as usize * 2, H as usize * 4);
    let mut d = vec![vec![false; w]; h];
    for (cy, row) in MARK.iter().enumerate() {
        for (cx, c) in row.chars().enumerate() {
            let bits = (c as u32).wrapping_sub(0x2800);
            if bits > 0xff {
                continue;
            }
            for (bit, (dx, dy)) in [(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 2), (0, 3), (1, 3)].into_iter().enumerate() {
                if bits & (1 << bit) != 0 {
                    d[cy * 4 + dy][cx * 2 + dx] = true;
                }
            }
        }
    }
    d
}

/// The mark waving like a flag, at `t` seconds: a wave runs across it, diagonally, moving its dots up and down and
/// lighting them as it passes. One row taller than the still one.
pub fn waving(t: f32) -> Vec<Line<'static>> {
    use std::f32::consts::TAU;
    let d = dots();
    let h = d.len();
    let phase = |x: usize, y: usize| TAU * ((x as f32) * 0.9 + (y as f32) * 0.55) / 34.0 - t * 2.6;
    let lift = |x: usize, y: usize| 1.6 * phase(x, y).sin();
    let at = |x: usize, y: i32| y >= 0 && (y as usize) < h && d[y as usize][x];
    (0..H as usize + 1).map(|cy| {
        let spans: Vec<Span> = (0..W as usize).map(|cx| {
            let mut bits = 0u32;
            for (bit, (dx, dy)) in [(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 2), (0, 3), (1, 3)].into_iter().enumerate() {
                let (x, y) = (cx * 2 + dx, cy * 4 + dy);
                // where this dot comes from, before the wave moved it
                if at(x, y as i32 - 2 - lift(x, y).round() as i32) {
                    bits |= 1 << bit;
                }
            }
            let glow = ((phase(cx * 2, cy * 4).sin() + 1.0) * 14.0) as u32;
            let base = color(cx, cy.min(H as usize - 1));
            let c = char::from_u32(0x2800 + bits).unwrap_or(' ');
            Span::styled(if bits == 0 { " ".into() } else { c.to_string() }, Color::from_u32(mix(base, theme::FOAM, glow)))
        }).collect();
        Line::from(spans)
    }).collect()
}

pub fn lines() -> Vec<Line<'static>> {
    MARK.iter().enumerate().map(|(y, row)| {
        row.chars().enumerate().map(|(x, c)| Span::styled(c.to_string(), Color::from_u32(color(x, y)))).collect::<Vec<_>>().into()
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waves() {
        let still: usize = dots().iter().flatten().filter(|d| **d).count();
        for t in [0.0, 0.7, 1.9, 12.3] {
            let f = waving(t);
            assert!(f.len() == H as usize + 1 && f.iter().all(|l| l.width() == W as usize));
            let shown: u32 = f.iter().flat_map(|l| l.spans.iter()).filter_map(|s| s.content.chars().next()).map(|c| (c as u32).saturating_sub(0x2800).count_ones()).sum();
            assert!((shown as f32 - still as f32).abs() < still as f32 * 0.15, "{t}: {shown} of {still}"); // moved, not lost
        }
        assert_ne!(waving(0.0), waving(0.5));
    }

    #[test]
    fn size_and_gradient() {
        assert!(lines().len() == H as usize && lines().iter().all(|l| l.width() == W as usize));
        assert_eq!(color(0, 0), theme::DEEP);
        let light = |c: u32| (c >> 16 & 255) + (c >> 8 & 255) + (c & 255);
        assert!((1..W as usize).all(|x| light(color(x, 7)) >= light(color(x - 1, 7))));
    }
}
