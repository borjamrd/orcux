//! The logo: the "o" of Fraunces Bold and a smaller "x" on the same baseline, in braille dots, with a gradient. In
//! the sidebar's border they are four by four and three by three dots.
use crate::{pixels::mix, theme};
use ratatui::prelude::*;

pub const W: u16 = 53;
pub const H: u16 = 15;

const MARK: [&str; H as usize] = [
    "         ⣀⣤⣴⣶⣾⣿⣿⣿⣿⣿⣿⣷⣶⣶⣤⣄⡀                           ",
    "     ⢀⣤⣶⣿⣿⣿⣿⡿⠟⠛⠉⠛⠻⣿⣿⣿⣿⣿⣿⣿⣿⣷⣦⣀                        ",
    "   ⢀⣴⣿⣿⣿⣿⣿⣿⡟⠁     ⠈⢻⣿⣿⣿⣿⣿⣿⣿⣿⣿⣷⣄                      ",
    "  ⣰⣿⣿⣿⣿⣿⣿⣿⣿⠁        ⢹⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣦                     ",
    " ⣼⣿⣿⣿⣿⣿⣿⣿⣿⣿          ⢻⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣧                    ",
    "⢰⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿          ⠈⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡆                   ",
    "⣾⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿           ⢻⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣷                   ",
    "⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡇          ⠸⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿                   ",
    "⢿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣧           ⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡿  ⠺⢿⣿⣿⣿⣿⣿⣿⠟  ⠛⢿⣿⣿⠿⠃",
    "⠸⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡄          ⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⠇    ⠙⣿⣿⣿⣿⣿⣦   ⣼⡟⠁  ",
    " ⢻⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣷          ⣿⣿⣿⣿⣿⣿⣿⣿⣿⡟      ⠈⢿⣿⣿⣿⣿⣷⣤⣾⠋    ",
    "  ⠻⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣧        ⢀⣿⣿⣿⣿⣿⣿⣿⣿⠟         ⢹⣿⣿⣿⣿⣿⣧     ",
    "   ⠙⢿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣧⡀      ⣼⣿⣿⣿⣿⣿⣿⠟⠁         ⣰⡿⠉⢿⣿⣿⣿⣿⣷⡄   ",
    "     ⠈⠻⢿⣿⣿⣿⣿⣿⣿⣿⣿⣦⣤⣀⣠⣴⣾⣿⣿⣿⣿⠿⠛⠁         ⢠⣾⠏   ⠻⣿⣿⣿⣿⣿⣦⡀ ",
    "        ⠈⠙⠛⠿⠿⢿⣿⣿⣿⣿⣿⣿⠿⠿⠟⠛⠉           ⣤⣾⣿⣿⣶⡄ ⢠⣤⣿⣿⣿⣿⣿⣿⣷⣦",
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

pub fn lines() -> Vec<Line<'static>> {
    MARK.iter().enumerate().map(|(y, row)| {
        row.chars().enumerate().map(|(x, c)| Span::styled(c.to_string(), Color::from_u32(color(x, y)))).collect::<Vec<_>>().into()
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_and_gradient() {
        assert!(lines().len() == H as usize && lines().iter().all(|l| l.width() == W as usize));
        assert_eq!(color(0, 0), theme::DEEP);
        let light = |c: u32| (c >> 16 & 255) + (c >> 8 & 255) + (c & 255);
        assert!((1..W as usize).all(|x| light(color(x, 7)) >= light(color(x - 1, 7))));
    }
}
