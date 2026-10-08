//! A settings column: headed sections, spaced by the token ramp.
//!
//! Every settings pane stacked the same things -- a kicker, the control it
//! names, a line saying what the choice costs, the next kicker -- and spaced
//! them with whatever literal the pane was written with: 18, 20 and 22 above
//! a kicker, 8 below one, 10 or 14 above a note. That is how three panes of
//! one window came to have three rhythms.
//!
//! This is the rhythm, once, on [`super::space`]:
//!
//! * a [`section`](SettingsGroup::section) heading sits `S6` below whatever
//!   came before it, and flush at the top of the column;
//! * a [`control`](SettingsGroup::control) sits `S2` under its heading;
//! * a [`note`](SettingsGroup::note) -- a stat line, a sentence -- sits
//!   `S3` under what it describes;
//! * a [`block`](SettingsGroup::block) that stands apart without a heading
//!   of its own sits `S6` down, like a section;
//! * [`append`](SettingsGroup::append) adds with no margin, for content
//!   whose spacing is the list's own.

use adw::prelude::*;

use super::space;

/// A column of settings sections.
pub struct SettingsGroup {
    column: gtk::Box,
}

impl SettingsGroup {
    /// A new, empty column.
    pub fn new() -> Self {
        Self::on(&gtk::Box::new(gtk::Orientation::Vertical, 0))
    }

    /// Lay out into an existing vertical box -- a pane the window already
    /// holds a handle to.
    pub fn on(column: &gtk::Box) -> Self {
        Self {
            column: column.clone(),
        }
    }

    /// A section heading: a kicker, `S6` below whatever came before.
    pub fn section(&self, title: &str) -> gtk::Label {
        let heading = super::kicker(title);
        self.push(&heading, space::S6);
        heading
    }

    /// The control a heading names, `S2` under it.
    pub fn control(&self, widget: &impl IsA<gtk::Widget>) -> &Self {
        self.push(widget, space::S2);
        self
    }

    /// A line describing what is above it, `S3` down.
    pub fn note(&self, widget: &impl IsA<gtk::Widget>) -> &Self {
        self.push(widget, space::S3);
        self
    }

    /// Something that stands apart without a heading, `S6` down.
    pub fn block(&self, widget: &impl IsA<gtk::Widget>) -> &Self {
        self.push(widget, space::S6);
        self
    }

    /// Something whose spacing is its own, with no margin added.
    pub fn append(&self, widget: &impl IsA<gtk::Widget>) -> &Self {
        self.column.append(widget);
        self
    }

    /// The column, to place in a pane.
    pub fn widget(&self) -> &gtk::Box {
        &self.column
    }

    fn push(&self, widget: &impl IsA<gtk::Widget>, gap: i32) {
        // Flush at the top: the pane's own inset is the space above its
        // first line, and a margin on top of it doubles it.
        let first = self.column.first_child().is_none();
        widget.set_margin_top(if first { 0 } else { gap });
        self.column.append(widget);
    }
}

impl Default for SettingsGroup {
    fn default() -> Self {
        Self::new()
    }
}
