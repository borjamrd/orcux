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

/// The mark as an SVG, for the web and the README: each braille dot a round dot, on a grid where dots are one unit
/// apart (a cell is twice as tall as wide, so its 2 by 4 dots are evenly spaced), and the gradient of `color` as one
/// `linearGradient` along the same diagonal. Every dot is a zero-length stroke with round caps: a few bytes each.
pub fn svg() -> String {
    let (w, h) = (W as f32 * 2.0, H as f32 * 4.0);
    // color() goes as 0.8 x/w + 0.2 y/h: the gradient runs along that slope, reaching 1 where it does
    let (gx, gy) = (0.8 / w, 0.2 / h);
    let n = gx * gx + gy * gy;
    let hex = |c: u32| format!("#{c:06x}");
    let mut dots = String::new();
    for (y, row) in MARK.iter().enumerate() {
        for (x, c) in row.chars().enumerate() {
            let bits = (c as u32).saturating_sub(0x2800); // a space has no dots
            // braille dots 1-8 by bit: left column 0,1,2,6 and right column 3,4,5,7, top to bottom
            for (bit, (dx, dy)) in [(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 2), (0, 3), (1, 3)].into_iter().enumerate() {
                if bits >> bit & 1 == 1 {
                    dots += &format!("M{} {}h0", x * 2 + dx, y * 4 + dy);
                }
            }
        }
    }
    format!(
        concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="-0.5 -0.5 {w} {h}">"#,
            r#"<linearGradient id="g" gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="{x2:.1}" y2="{y2:.1}">"#,
            r#"<stop offset="0" stop-color="{deep}"/><stop offset=".5" stop-color="{current}"/><stop offset="1" stop-color="{foam}"/>"#,
            r#"</linearGradient><path d="{dots}" stroke="url(#g)" stroke-width=".8" stroke-linecap="round"/></svg>"#,
            "\n"
        ),
        w = w, h = h, x2 = gx / n, y2 = gy / n, deep = hex(theme::DEEP), current = hex(theme::CURRENT), foam = hex(theme::FOAM), dots = dots
    )
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
    fn svg_has_every_dot() {
        let dots: u32 = MARK.iter().flat_map(|r| r.chars()).map(|c| (c as u32).saturating_sub(0x2800).count_ones()).sum();
        let s = svg();
        assert_eq!(s.matches("h0").count() as u32, dots);
        assert!(s.starts_with("<svg ") && s.ends_with("</svg>\n"));
    }

    #[test]
    fn size_and_gradient() {
        assert!(lines().len() == H as usize && lines().iter().all(|l| l.width() == W as usize));
        assert_eq!(color(0, 0), theme::DEEP);
        let light = |c: u32| (c >> 16 & 255) + (c >> 8 & 255) + (c & 255);
        assert!((1..W as usize).all(|x| light(color(x, 7)) >= light(color(x - 1, 7))));
    }
}
