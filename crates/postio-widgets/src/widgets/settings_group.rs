//! A settings column: headed sections spaced by the token ramp (`super::space`), so every pane has
//! one rhythm.
//! `section` sits `S6` below what came before and flush at the top; `control` `S2`; `note` `S3`;
//! `block` `S6`; `append` adds no margin.

use adw::prelude::*;

use super::space;

pub struct SettingsGroup {
    column: gtk::Box,
}

impl SettingsGroup {
    pub fn new() -> Self {
        Self::on(&gtk::Box::new(gtk::Orientation::Vertical, 0))
    }

    pub fn on(column: &gtk::Box) -> Self {
        Self {
            column: column.clone(),
        }
    }

    pub fn section(&self, title: &str) -> gtk::Label {
        let heading = super::kicker(title);
        self.push(&heading, space::S6);
        heading
    }

    pub fn control(&self, widget: &impl IsA<gtk::Widget>) -> &Self {
        self.push(widget, space::S2);
        self
    }

    pub fn note(&self, widget: &impl IsA<gtk::Widget>) -> &Self {
        self.push(widget, space::S3);
        self
    }

    pub fn block(&self, widget: &impl IsA<gtk::Widget>) -> &Self {
        self.push(widget, space::S6);
        self
    }

    pub fn append(&self, widget: &impl IsA<gtk::Widget>) -> &Self {
        self.column.append(widget);
        self
    }

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
