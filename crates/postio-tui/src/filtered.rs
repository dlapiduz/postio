//! Filtered as drawn: the controller's view and the scroll this terminal
//! keeps over it.

use postio_focus::{FilteredBar, FilteredView};
use postio_model::MessageId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Heading(usize),
    Item(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filtered {
    view: FilteredView,
    top: usize,
}

impl Filtered {
    // What is drawn from the moment Filtered opens until the first reading
    // arrives.
    pub fn opening() -> Filtered {
        use postio_ui::filtered as words;
        Filtered {
            view: FilteredView {
                title: words::TITLE.to_owned(),
                subtitle: words::SUBTITLE.to_owned(),
                note: words::NOTE.to_owned(),
                sweep: words::SWEEP_BUTTON.to_owned(),
                sweep_key: None,
                restore: words::RESTORE.to_owned(),
                restore_key: None,
                tabs: words::TABS
                    .iter()
                    .enumerate()
                    .map(|(at, (_, name))| postio_focus::FilteredTab {
                        name: (*name).to_owned(),
                        count: 0,
                        key: None,
                        on: at == 0,
                    })
                    .collect(),
                rows: Vec::new(),
                focused: None,
                more: false,
                footer: Vec::new(),
                bar: None,
                empty: None,
                expanded: None,
            },
            top: 0,
        }
    }

    pub fn view(&self) -> &FilteredView {
        &self.view
    }

    pub fn top(&self) -> usize {
        self.top
    }

    pub fn cursor(&self) -> Option<usize> {
        self.view.focused.map(|at| at as usize)
    }

    pub fn replace(&mut self, view: FilteredView) {
        self.view = view;
        self.top = self.top.min(self.slots().len().saturating_sub(1));
    }

    pub fn focus(&mut self, at: Option<u32>) {
        self.view.focused = at;
    }

    pub fn select(&mut self, selected: &[MessageId], bar: Option<FilteredBar>) {
        for line in &mut self.view.rows {
            line.selected = selected.contains(&line.message);
        }
        self.view.bar = bar;
    }

    pub fn slots(&self) -> Vec<Slot> {
        let mut slots = Vec::with_capacity(self.view.rows.len() + 8);
        for (index, line) in self.view.rows.iter().enumerate() {
            if line.heading.is_some() {
                slots.push(Slot::Heading(index));
            }
            slots.push(Slot::Item(index));
        }
        slots
    }

    pub fn scroll(&mut self, lines: isize, height: usize) {
        let slots = self.slots().len();
        let last = slots.saturating_sub(height.min(slots));
        self.top = self.top.saturating_add_signed(lines).min(last);
    }

    pub fn reveal(&mut self, height: usize) {
        let height = height.max(1);
        let slots = self.slots();
        let Some(at) = self
            .cursor()
            .and_then(|cursor| slots.iter().position(|slot| *slot == Slot::Item(cursor)))
        else {
            return;
        };
        let needs = if at > 0 && matches!(slots[at - 1], Slot::Heading(_)) {
            at - 1
        } else {
            at
        };
        if needs < self.top {
            self.top = needs;
        } else if at >= self.top + height {
            self.top = at + 1 - height;
        }
    }

    pub fn wants_more(&self, height: usize) -> bool {
        self.view.more && self.top + height >= self.slots().len()
    }

    pub fn asked_more(&mut self) {
        self.view.more = false;
    }
}
