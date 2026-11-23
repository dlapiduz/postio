//! Where things are on screen, for the mouse. A mouse event is resolved against the last
//! frame drawn, so a click lands on what the person saw; the app never sees a coordinate.

use ratatui::layout::Rect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Row(u32),
    RowAction(u32, &'static str),
    Reader(Option<usize>),
    /// The composer's body.
    ComposerBody,
    ComposerField(crate::composer::Field),
    ComposerAction(&'static str),
    BarRow(usize),
    BarSaved(usize),
    ResultRow(u64),
    ResultTab(usize),
    ResultChip(u32),
    BarRun,
    BarCommands,
    PlaceRow(usize),
    PickRow(usize),
    Command(&'static str),
    NoticeOffer,
    MenuRow(usize),
    Surface(crate::surface::Part, usize),
    Overlay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    pub target: Target,
    pub column: u16,
    pub row: u16,
}

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

    pub fn area_of(&self, target: Target) -> Option<Rect> {
        self.regions
            .iter()
            .filter(|(_, at)| *at == target)
            .map(|(area, _)| *area)
            .reduce(|a, b| a.union(b))
    }

    #[cfg(test)]
    pub fn regions(&self) -> &[(Rect, Target)] {
        &self.regions
    }

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
