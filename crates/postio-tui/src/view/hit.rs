//! Where things are on screen, for the mouse (US5).
//!
//! Every frame records each region it draws and what that region stands for;
//! a mouse event is resolved against the last frame drawn, so a click lands on
//! what the person saw, not on what the state has since become. The app never
//! sees a coordinate, only what was clicked.

use ratatui::layout::Rect;

/// What a region of the screen stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// A row of the list, by position in the list.
    Row(u32),
    /// One of a marked row's drawn answers, by the row's position and the
    /// command it runs: what its key does, for that row.
    RowAction(u32, &'static str),
    /// A line of what is being read, by index into its lines; `None` for the
    /// reader's header.
    Reader(Option<usize>),
    /// The composer's body.
    ComposerBody,
    /// Another field of the composer.
    ComposerField(crate::composer::Field),
    /// One of the composer's buttons, by the command it runs.
    ComposerAction(&'static str),
    /// A line of the command bar, by its place in its list.
    BarRow(usize),
    /// One of the bar's saved searches.
    BarSaved(usize),
    /// One of the bar's chips.
    BarChip(usize),
    /// A place in the folders popover, by its place in the list.
    PlaceRow(usize),
    /// A control of the window that runs a command, by its id: the top bar's
    /// and the strip's.
    Command(&'static str),
    /// A row of the menu over the open message, by its place in the menu.
    MenuRow(usize),
    /// Something drawn over everything else: clicks there land on nothing
    /// underneath.
    Overlay,
}

/// A resolved click: what was under it, and where inside that region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    /// What was under the pointer.
    pub target: Target,
    /// Columns in from the region's left edge.
    pub column: u16,
    /// Rows down from the region's top.
    pub row: u16,
}

/// One frame's regions, in the order they were drawn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hits {
    regions: Vec<(Rect, Target)>,
}

impl Hits {
    /// `area` stands for `target`. A region drawn later covers one drawn
    /// before it, as it does on screen.
    pub fn add(&mut self, area: Rect, target: Target) {
        if area.width > 0 && area.height > 0 {
            self.regions.push((area, target));
        }
    }

    /// What is at column `x`, row `y`.
    pub fn at(&self, x: u16, y: u16) -> Option<Hit> {
        self.regions
            .iter()
            .rev()
            .find(|(area, _)| {
                x >= area.x && x < area.x + area.width && y >= area.y && y < area.y + area.height
            })
            .map(|(area, target)| Hit {
                target: *target,
                column: x - area.x,
                row: y - area.y,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_drawn_last_is_what_is_hit() {
        let mut hits = Hits::default();
        hits.add(Rect::new(0, 0, 10, 10), Target::Row(0));
        hits.add(Rect::new(2, 2, 3, 3), Target::Overlay);
        assert_eq!(hits.at(3, 3).unwrap().target, Target::Overlay);
        assert_eq!(
            hits.at(8, 1),
            Some(Hit {
                target: Target::Row(0),
                column: 8,
                row: 1
            })
        );
        assert_eq!(hits.at(10, 0), None);
    }
}
