//! What a click or the wheel on one of Focus's surfaces reaches: the part
//! says which surface it belongs to.

use super::{App, Effect};
use crate::surface::Part;

impl App {
    /// A click on a part of a Focus surface.
    pub(super) fn surface_click(&mut self, part: Part, index: usize) -> Vec<Effect> {
        match part {
            Part::FilteredTab | Part::FilteredRow | Part::SweepCancel | Part::SweepConfirm => {
                self.filtered_click(part, index)
            }
            Part::DigestTab
            | Part::DigestRow
            | Part::DigestReference
            | Part::StopCancel
            | Part::StopConfirm => self.digest_click(part, index),
        }
    }

    /// The wheel turned over `lines` on a Focus surface; whether one took it.
    pub(super) fn surface_wheel(&mut self, lines: isize) -> bool {
        if self.surfaces.digest.is_some() {
            self.digest_wheel(lines);
            return true;
        }
        if self.surfaces.filtered.is_some() {
            self.filtered_wheel(lines);
            return true;
        }
        false
    }

    /// `id` as the window runs it, from a surface that leaves the window
    /// where it is.
    pub(super) fn global(&mut self, id: &str) -> Vec<Effect> {
        match id {
            "cheat_sheet" => self.open_keys(),
            _ => self.send(id),
        }
    }
}
