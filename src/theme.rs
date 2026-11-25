//! Design system "orcux". Each status color has one meaning and always goes with its glyph and a word.
//! Surfaces are soft fills and gaps, not lines: the only outlines are the rounded dialogs.

use crate::*;
use std::sync::OnceLock;

pub const ABYSS: u32 = 0x0f1114; // ground: sidebar, gaps, status bar
pub const HULL: u32 = 0x181b20; // cards: terminal, PR rail, dialogs
pub const WAKE: u32 = 0x2a2f36; // selection, the pending half of a progress bar
pub const LINE: u32 = 0x4a525c; // tree branches, dialog outlines
pub const FOAM: u32 = 0xe8eaed; // text
pub const MUTED: u32 = 0x9aa1a9; // secondary text
pub const CURRENT: u32 = 0x8ab8f5; // working
pub const SIGNAL: u32 = 0xe0b060; // waiting for the user
pub const CORAL: u32 = 0xf08a78; // failed
pub const KELP: u32 = 0xa3d39c; // done
pub const DEEP: u32 = 0x24528c; // start of the logo's gradient: no meaning
/// Repo badges: only identity, no meaning.
pub const BADGES: [u32; 4] = [0x7f9cc4, 0x84ad8d, 0xa59bd0, 0x78b3b5];

/// Left and right ends of a badge: rounded if the user said that their terminal has a Nerd Font, else plain
/// fills. Half blocks do not work: many fonts draw them shorter than the cell.
pub fn caps() -> Option<(&'static str, &'static str)> {
    static NERD: OnceLock<bool> = OnceLock::new();
    NERD.get_or_init(|| icons() == "nerd").then_some(("\u{e0b6}", "\u{e0b4}"))
}

pub const ICONS: [&str; 2] = ["plain", "nerd"];

pub fn icons() -> &'static str {
    let saved = fs::read_to_string(state_dir().join("icons")).unwrap_or_default();
    ICONS.into_iter().find(|i| *i == saved.trim()).unwrap_or(ICONS[0])
}

pub fn set_icons(which: &str) -> Res<()> {
    fs::write(state_dir().join("icons"), which).map_err(|e| e.to_string())
}

/// For tmux styles.
pub fn hex(c: u32) -> String {
    format!("#{c:06x}")
}
