//! Pixel art out of lower half blocks, two pixels per cell: the upper ones leave a stripe in fonts that draw them
//! shorter than the cell. Everything is drawn in shades of the text over its ground.
use crate::theme;
use ratatui::prelude::*;

/// `ink` hundredths of text over the ground.
pub fn shade(ground: u32, ink: u32) -> u32 {
    [16, 8, 0].into_iter().fold(0, |c, s| {
        let (a, b) = (ground >> s & 255, theme::FOAM >> s & 255);
        c << 8 | (a * (100 - ink) + b * ink) / 100
    })
}

/// Each row holds the ink of its pixels.
pub fn lines(rows: &[Vec<u32>], ground: u32) -> Vec<Line<'static>> {
    rows.chunks(2).map(|pair| {
        let mut spans: Vec<Span> = vec![];
        let mut last = None;
        for (top, low) in pair[0].iter().zip(&pair[1]) {
            if last == Some((top, low)) {
                let t = spans.last_mut().unwrap().content.to_mut();
                t.push(if top == low { ' ' } else { '▄' });
            } else {
                let st = Style::new().bg(Color::from_u32(shade(ground, *top))).fg(Color::from_u32(shade(ground, *low)));
                spans.push(Span::styled(if top == low { " " } else { "▄" }, st));
                last = Some((top, low));
            }
        }
        Line::from(spans)
    }).collect()
}
