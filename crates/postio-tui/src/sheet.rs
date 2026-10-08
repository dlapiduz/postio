//! The key map's state and layout (terminal.md, "Key map"): `?` opens
//! `postio_ui::keymap_sheet::key_map` for the terminal over everything but
//! the top bar, as columns 38 wide, as many as fit, scrolling when they do
//! not.
//!
//! Every key shown is the keymap's, filtered to those this terminal can
//! send; the groups, their order and the words are the shared ones.

use crossterm::event::{KeyCode, KeyEvent};
use postio_core::{Frontend, Keymap};
use postio_ui::keymap::{KeyContext, Outcome};
use postio_ui::keymap_sheet::{self, COLUMNS};

use crate::app::Focus;
use crate::input::Keys;

/// How wide a column is.
pub const COLUMN: u16 = 38;
/// The gap between columns.
pub const GAP: u16 = 2;

/// What a key asks of the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Draw again.
    Stay,
    /// Put the key map away.
    Close,
}

/// A line of a column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    /// A group's heading.
    Heading(&'static str),
    /// A command and the keys that run it here.
    Row {
        /// What the registry calls it.
        title: &'static str,
        /// Its keys, as the keymap spells them.
        keys: String,
    },
    /// A gap between groups.
    Blank,
}

/// The key map, open.
#[derive(Debug)]
pub struct Sheet {
    from: Focus,
    scroll: usize,
}

impl Sheet {
    /// A key map open over `from`.
    pub fn open(from: Focus) -> Sheet {
        Sheet { from, scroll: 0 }
    }

    /// Where the keyboard was when it opened.
    pub fn from(&self) -> Focus {
        self.from
    }

    /// How many lines down it is scrolled.
    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// Scroll by `lines`, no further than `max`.
    pub fn scroll_by(&mut self, lines: isize, max: usize) {
        self.scroll = self.scroll.saturating_add_signed(lines).min(max);
    }

    /// A key: `?` and `Esc` close it, the arrows scroll, and nothing else is
    /// heard.
    pub fn key(&mut self, key: &KeyEvent, keys: &mut Keys, page: usize, max: usize) -> Step {
        if let Outcome::Command(id) = keys.press(key, KeyContext::List, false) {
            if id == "cheat_sheet" || id == "back" {
                return Step::Close;
            }
        } else if key.code == KeyCode::Esc {
            return Step::Close;
        }
        match key.code {
            KeyCode::Down => self.scroll_by(1, max),
            KeyCode::Up => self.scroll_by(-1, max),
            KeyCode::PageDown => self.scroll_by(page as isize, max),
            KeyCode::PageUp => self.scroll_by(-(page as isize), max),
            KeyCode::Home => self.scroll = 0,
            KeyCode::End => self.scroll = max,
            _ => {}
        }
        Step::Stay
    }
}

/// How many columns fit in `width` cells, at most [`COLUMNS`].
pub fn columns_in(width: u16) -> usize {
    usize::from((width + GAP) / (COLUMN + GAP)).clamp(1, COLUMNS)
}

/// The keys of a row this terminal can send, as the key map spells them.
fn deliverable(keys: &[String], enhanced: bool) -> String {
    keys.iter()
        .filter(|binding| {
            enhanced
                || binding
                    .parse::<postio_ui::keymap::Binding>()
                    .is_ok_and(|parsed| {
                        parsed
                            .chords()
                            .iter()
                            .all(postio_ui::terminal::legacy_deliverable)
                    })
        })
        .cloned()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The key map's columns for a sheet `width` cells wide.
pub fn columns(keymap: &Keymap, enhanced: bool, width: u16) -> Vec<Vec<Line>> {
    let map = keymap_sheet::key_map(keymap, Frontend::Terminal);
    let sizes: Vec<usize> = map.iter().map(|(_, rows)| rows.len()).collect();
    keymap_sheet::pack_columns(&sizes, columns_in(width))
        .into_iter()
        .map(|packed| {
            let mut lines = Vec::new();
            for index in packed {
                let (group, rows) = &map[index];
                if !lines.is_empty() {
                    lines.push(Line::Blank);
                }
                lines.push(Line::Heading(group.title()));
                lines.extend(rows.iter().map(|row| Line::Row {
                    title: row.title,
                    keys: deliverable(&row.keys, enhanced),
                }));
            }
            lines
        })
        .collect()
}

/// How many lines the longest column has.
pub fn height(columns: &[Vec<Line>]) -> usize {
    columns.iter().map(Vec::len).max().unwrap_or(0)
}
