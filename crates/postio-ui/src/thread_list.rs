//! An expanded thread as a splice over the windowed rows.
//!
//! The list is paged, and the controller never holds its rows. Putting a
//! thread's children into the paged query would shift every page's offsets
//! and put a thread read into the hot list query, so the children are only
//! a splice: after the anchor row, `children` display positions that are not
//! rows. The cursor, `j`/`k`, scope and the footer all ask the splice, so
//! one set of tests covers both frontends (spec 013, R7).

/// What a display position holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// The list's own row at this position.
    Row(u32),
    /// The `index`th child (from zero, oldest first) of the anchor `row`.
    Child {
        /// The anchor row's position among the rows.
        row: u32,
        /// Which child it is.
        index: u32,
    },
}

/// One expanded thread: its children sit right after the anchor row.
///
/// At most one per list, so a list holds an `Option<Splice>`; `None` is the
/// identity, and the free functions [`slot`], [`display`] and [`len`] say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Splice {
    /// The expanded row's position among the rows.
    pub anchor: u32,
    /// How many children follow it.
    pub children: u32,
}

impl Splice {
    /// What `display` holds. A position past the end is the row it would be
    /// after the splice, so an anchor beyond the landed rows is harmless.
    pub fn slot(self, display: u32) -> Slot {
        let first = self.anchor.saturating_add(1);
        if display < first {
            Slot::Row(display)
        } else if display - first < self.children {
            Slot::Child {
                row: self.anchor,
                index: display - first,
            }
        } else {
            Slot::Row(display - self.children)
        }
    }

    /// Where `slot` is drawn.
    pub fn display(self, slot: Slot) -> u32 {
        match slot {
            Slot::Row(row) if row <= self.anchor => row,
            Slot::Row(row) => row + self.children,
            Slot::Child { index, .. } => {
                self.anchor + 1 + index.min(self.children.saturating_sub(1))
            }
        }
    }

    /// How many positions `rows` rows draw: the children, once the anchor is
    /// among them.
    pub fn len(self, rows: u32) -> u32 {
        if self.anchor < rows {
            rows + self.children
        } else {
            rows
        }
    }
}

/// What `display` holds in a list with `splice`, or none.
pub fn slot(splice: Option<Splice>, display: u32) -> Slot {
    splice.map_or(Slot::Row(display), |splice| splice.slot(display))
}

/// Where `slot` is drawn in a list with `splice`, or none.
pub fn display(splice: Option<Splice>, slot: Slot) -> u32 {
    match (splice, slot) {
        (Some(splice), slot) => splice.display(slot),
        (None, Slot::Row(row)) => row,
        (None, Slot::Child { row, index }) => row + 1 + index,
    }
}

/// How many positions `rows` rows draw in a list with `splice`, or none.
pub fn len(splice: Option<Splice>, rows: u32) -> u32 {
    splice.map_or(rows, |splice| splice.len(rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(anchor: u32, children: u32) -> Splice {
        Splice { anchor, children }
    }

    /// Every row and child round-trips through its display position, and
    /// the positions are exactly `0..len` in order.
    fn round_trips(splice: Splice, rows: u32) {
        let mut expected = Vec::new();
        for row in 0..rows {
            expected.push(Slot::Row(row));
            if row == splice.anchor {
                expected.extend((0..splice.children).map(|index| Slot::Child { row, index }));
            }
        }
        assert_eq!(splice.len(rows) as usize, expected.len());
        for (position, slot) in expected.into_iter().enumerate() {
            assert_eq!(splice.slot(position as u32), slot, "slot at {position}");
            assert_eq!(splice.display(slot), position as u32, "display of {slot:?}");
        }
    }

    #[test]
    fn an_anchor_at_the_top_pushes_every_other_row_down() {
        round_trips(at(0, 3), 5);
        assert_eq!(at(0, 3).slot(1), Slot::Child { row: 0, index: 0 });
        assert_eq!(at(0, 3).slot(4), Slot::Row(1));
    }

    #[test]
    fn an_anchor_in_the_middle_leaves_the_rows_above_it_alone() {
        round_trips(at(2, 4), 6);
        assert_eq!(at(2, 4).slot(2), Slot::Row(2));
        assert_eq!(at(2, 4).slot(7), Slot::Row(3));
    }

    #[test]
    fn an_anchor_at_the_end_ends_in_its_children() {
        round_trips(at(4, 2), 5);
        assert_eq!(at(4, 2).slot(6), Slot::Child { row: 4, index: 1 });
    }

    #[test]
    fn no_children_is_no_splice() {
        round_trips(at(1, 0), 4);
        assert_eq!(at(1, 0).len(4), 4);
    }

    #[test]
    fn an_anchor_beyond_the_landed_rows_adds_nothing_yet() {
        let splice = at(9, 3);
        assert_eq!(splice.len(5), 5);
        for position in 0..5 {
            assert_eq!(splice.slot(position), Slot::Row(position));
            assert_eq!(splice.display(Slot::Row(position)), position);
        }
        // Once the anchor lands, the children appear after it.
        assert_eq!(splice.len(10), 13);
    }

    #[test]
    fn no_splice_is_the_identity() {
        assert_eq!(slot(None, 3), Slot::Row(3));
        assert_eq!(display(None, Slot::Row(3)), 3);
        assert_eq!(len(None, 8), 8);
        assert_eq!(len(Some(at(1, 2)), 8), 10);
        assert_eq!(slot(Some(at(1, 2)), 2), Slot::Child { row: 1, index: 0 });
        assert_eq!(display(Some(at(1, 2)), Slot::Row(5)), 7);
    }
}
