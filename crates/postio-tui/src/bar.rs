//! The command bar over the controller's views: the field, which line the
//! arrows are on, and where the keyboard goes back to.

use postio_focus::{BarView, DropdownView};
use tui_input::backend::crossterm::EventHandler;

use crate::app::Focus;

#[derive(Debug, Clone)]
pub enum Shown {
    Lines(BarView),
    Dropdown(DropdownView),
}

#[derive(Debug)]
pub struct Bar {
    from: Focus,
    field: tui_input::Input,
    shown: Shown,
    selected: Option<u64>,
}

impl Bar {
    pub fn open(from: Focus) -> Bar {
        Bar {
            from,
            field: tui_input::Input::default(),
            shown: Shown::Lines(BarView::default()),
            selected: None,
        }
    }

    pub fn from(&self) -> Focus {
        self.from
    }

    pub fn typed(&self) -> &str {
        self.field.value()
    }

    pub fn caret(&self) -> usize {
        self.field.cursor()
    }

    pub fn set_text(&mut self, text: &str) {
        if self.field.value() != text {
            self.field = tui_input::Input::new(text.to_owned());
        }
    }

    pub fn shown(&self) -> &Shown {
        &self.shown
    }

    pub fn type_in(&mut self, key: &crossterm::event::KeyEvent) -> bool {
        let before = self.field.value().to_owned();
        self.field.handle_event(&crossterm::event::Event::Key(*key));
        self.field.value() != before
    }

    pub fn stops(&self) -> Vec<u64> {
        match &self.shown {
            Shown::Lines(view) => view
                .lines
                .iter()
                .filter(|line| line.selectable)
                .map(|line| line.token)
                .collect(),
            Shown::Dropdown(view) => view
                .sections
                .iter()
                .flat_map(|section| {
                    let rows = section.rows.iter().filter(|row| row.selectable);
                    rows.map(|row| row.token)
                        .chain(section.pills.iter().map(|pill| pill.token))
                })
                .collect(),
        }
    }

    pub fn selected(&self) -> Option<u64> {
        self.selected
    }

    pub fn select(&mut self, token: u64) {
        self.selected = Some(token);
    }

    pub fn show_lines(&mut self, view: BarView) {
        let kept = self.selected;
        self.shown = Shown::Lines(view.clone());
        self.selected = self.landing(view.highlight, kept);
    }

    pub fn show_dropdown(&mut self, view: DropdownView) {
        let kept = self.selected;
        let wanted = view.select.or(view.highlight);
        self.shown = Shown::Dropdown(view);
        self.selected = self.landing(wanted, kept);
    }

    fn landing(&self, wanted: Option<u64>, kept: Option<u64>) -> Option<u64> {
        let stops = self.stops();
        [wanted, kept]
            .into_iter()
            .flatten()
            .find(|token| stops.contains(token))
            .or_else(|| stops.first().copied())
    }

    /// Move the arrows; the token now on, when it moved.
    pub fn step(&mut self, by: isize) -> Option<u64> {
        let stops = self.stops();
        let at = self
            .selected
            .and_then(|token| stops.iter().position(|stop| *stop == token));
        let next = match at {
            Some(at) => at
                .checked_add_signed(by)
                .filter(|next| *next < stops.len())?,
            None => 0,
        };
        let token = stops.get(next).copied()?;
        self.selected = Some(token);
        Some(token)
    }
}
