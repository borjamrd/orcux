//! The name in the sidebar, after Fraunces Bold, redrawn by hand for the few pixels that fit. A terminal draws every
//! text in its own font, so it has to be a picture.
use crate::{pixels, theme};
use ratatui::prelude::*;

pub const W: u16 = 34;
pub const H: u16 = 3;
/// Rows that the sidebar needs to show it; below, its border says the name.
pub const FROM: u16 = 40;

/// A half tone (`+`) only where the letters curve: smoother shades look blurred at this size.
const NAME: [&str; 6] = [
    "+####+.##.##.+####.##..##...###.##",
    "##++##..####.##.##..##..##..+##+#.",
    "##..##..##...##.....##..##...+##..",
    "##..##..##...##.....##..##....##+.",
    "##++##..##...##++#..##+###...#+##+",
    "+####+.####..+###+..+##+###.##.###",
];

pub fn lines() -> Vec<Line<'static>> {
    let rows: Vec<Vec<u32>> = NAME.iter().map(|r| r.bytes().map(|c| match c { b'#' => 100, b'+' => 42, _ => 0 }).collect()).collect();
    pixels::lines(&rows, theme::ABYSS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_the_sidebar() {
        assert!(NAME.iter().all(|r| r.len() == W as usize) && NAME.len() == 2 * H as usize);
        assert!(lines().len() == H as usize && lines().iter().all(|l| l.width() == W as usize));
        assert!(W + 1 < 36);
    }
}
