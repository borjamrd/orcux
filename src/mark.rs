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

/// The mark waving like a flag, at `t` seconds: its shape stays, and a light runs across it diagonally, as over cloth.
pub fn waving(t: f32) -> Vec<Line<'static>> {
    use std::f32::consts::TAU;
    MARK.iter().enumerate().map(|(y, row)| {
        row.chars().enumerate().map(|(x, c)| {
            let wave = (TAU * (x as f32 + y as f32 * 2.2) / 26.0 - t * 2.4).sin();
            // the crests take it towards the foam, the troughs towards the deep
            let color = if wave > 0.0 { mix(color(x, y), theme::FOAM, (wave * 55.0) as u32) } else { mix(color(x, y), theme::DEEP, (-wave * 35.0) as u32) };
            Span::styled(c.to_string(), Color::from_u32(color))
        }).collect::<Vec<_>>().into()
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
        let text = |f: &[Line]| f.iter().map(|l| l.to_string()).collect::<Vec<_>>();
        for t in [0.0, 0.7, 12.3] {
            assert_eq!(text(&waving(t)), MARK); // the same shape
        }
        assert_ne!(waving(0.0), waving(0.5)); // other colors
    }

    #[test]
    fn size_and_gradient() {
        assert!(lines().len() == H as usize && lines().iter().all(|l| l.width() == W as usize));
        assert_eq!(color(0, 0), theme::DEEP);
        let light = |c: u32| (c >> 16 & 255) + (c >> 8 & 255) + (c & 255);
        assert!((1..W as usize).all(|x| light(color(x, 7)) >= light(color(x - 1, 7))));
    }
}
