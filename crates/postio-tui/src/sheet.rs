//! The key map's state and layout: columns 38 wide, as many as fit, scrolling when they do not.
//! Every key shown is the keymap's, filtered to those this terminal can send.

use crossterm::event::{KeyCode, KeyEvent};
use postio_core::{Frontend, Keymap};
use postio_ui::keymap::{KeyContext, Outcome};
use postio_ui::keymap_sheet::{self, COLUMNS};

use crate::app::Focus;
use crate::input::Keys;

pub const COLUMN: u16 = 38;
pub const GAP: u16 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Stay,
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    Heading(&'static str),
    Row { title: &'static str, keys: String },
    Blank,
}

#[derive(Debug)]
pub struct Sheet {
    from: Focus,
    scroll: usize,
}

impl Sheet {
    pub fn open(from: Focus) -> Sheet {
        Sheet { from, scroll: 0 }
    }

    pub fn from(&self) -> Focus {
        self.from
    }

    pub fn scroll(&self) -> usize {
        self.scroll
    }

    pub fn scroll_by(&mut self, lines: isize, max: usize) {
        self.scroll = self.scroll.saturating_add_signed(lines).min(max);
    }

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

pub fn columns_in(width: u16) -> usize {
    usize::from((width + GAP) / (COLUMN + GAP)).clamp(1, COLUMNS)
}

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

pub fn height(columns: &[Vec<Line>]) -> usize {
    columns.iter().map(Vec::len).max().unwrap_or(0)
}
