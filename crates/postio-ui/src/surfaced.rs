//! Where Focus's surfaced rows sit in its list (spec 007 T095, research
//! R3): a digest delivery or a fired reminder is spliced among the
//! conversations at its position -- after the conversations newer than it
//! came due -- and the list pages over both.
//!
//! The store pages conversations; the list pages positions. This is the
//! arithmetic between them, with no toolkit and no store in it: which of a
//! page's positions are surfaced rows, and which run of the store's
//! conversations fills the rest.

/// What fills one position of a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// The `n`th surfaced row, in the order given to [`Spliced::new`].
    Surfaced(usize),
    /// The `n`th conversation of the store's page asked for.
    Stored(usize),
}

/// A page of the list, in the store's terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorePage {
    /// The first conversation the page needs.
    pub offset: u32,
    /// How many conversations it needs.
    pub limit: u32,
    /// What fills each of the page's positions, in order.
    pub slots: Vec<Slot>,
}

/// The surfaced rows' places among the conversations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Spliced {
    /// Each surfaced row's list position, and its index as given, in list
    /// order.
    at: Vec<(u32, usize)>,
}

impl Spliced {
    /// Place rows whose positions -- how many conversations sort above
    /// each -- are `positions`, in any order. Rows at one position keep the
    /// order they were given in.
    pub fn new(positions: &[u32]) -> Self {
        let mut order: Vec<usize> = (0..positions.len()).collect();
        order.sort_by_key(|index| (positions[*index], *index));
        let at = order
            .into_iter()
            .enumerate()
            .map(|(before, index)| (positions[index] + before as u32, index))
            .collect();
        Spliced { at }
    }

    /// How many surfaced rows there are.
    pub fn len(&self) -> u32 {
        self.at.len() as u32
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.at.is_empty()
    }

    /// How long the list is over `conversations` conversations.
    pub fn total(&self, conversations: u32) -> u32 {
        conversations.saturating_add(self.len())
    }

    /// Where the row at `index` of the list stands among the messages alone,
    /// and how many messages the list holds, when the surfaced rows given
    /// by `digests` (indices as given to [`Spliced::new`]) are not messages:
    /// a digest stands for many and opens as a window, so neither the
    /// position line nor the strip's count includes it.
    pub fn message_place(&self, digests: &[usize], index: u32, total: u32) -> (u32, u32) {
        let before = self
            .at
            .iter()
            .filter(|(at, which)| *at < index && digests.contains(which))
            .count() as u32;
        let held = self
            .at
            .iter()
            .filter(|(_, which)| digests.contains(which))
            .count() as u32;
        (index.saturating_sub(before), total.saturating_sub(held))
    }

    /// What fills the `count` positions from `start`, over a store of
    /// `conversations`, and which of its conversations to ask for.
    pub fn page(&self, start: u32, count: u32, conversations: u32) -> StorePage {
        let end = start.saturating_add(count).min(self.total(conversations));
        let before = self.at.iter().filter(|(at, _)| *at < start).count() as u32;
        let offset = start.saturating_sub(before);
        let mut slots = Vec::new();
        let mut stored = 0;
        for position in start..end {
            match self.at.iter().find(|(at, _)| *at == position) {
                Some((_, index)) => slots.push(Slot::Surfaced(*index)),
                None if offset + stored < conversations => {
                    slots.push(Slot::Stored(stored as usize));
                    stored += 1;
                }
                // A surfaced row placed past the last conversation sits
                // after it; nothing else is past the end.
                None => {}
            }
        }
        StorePage {
            offset,
            limit: stored,
            slots,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_surfaced_is_the_store_s_own_pages() {
        let spliced = Spliced::new(&[]);
        assert_eq!(spliced.total(120), 120);
        let page = spliced.page(50, 50, 120);
        assert_eq!((page.offset, page.limit), (50, 50));
        assert_eq!(page.slots.first(), Some(&Slot::Stored(0)));
    }

    #[test]
    fn a_digest_is_not_a_message_in_the_place_line() {
        // Digest (index 0) at the top, a reminder (index 1) after two
        // conversations: 7 rows, 6 messages.
        let spliced = Spliced::new(&[0, 2]);
        assert_eq!(spliced.message_place(&[0], 1, 7), (0, 6), "first message");
        assert_eq!(spliced.message_place(&[0], 3, 7), (2, 6), "the reminder");
        assert_eq!(spliced.message_place(&[0], 6, 7), (5, 6), "the last");
        assert_eq!(spliced.message_place(&[], 3, 7), (3, 7), "no digests");
    }

    #[test]
    fn a_row_at_its_position_pushes_the_conversations_after_it_down() {
        // Positions 0 and 2: one row on top, one after two conversations.
        let spliced = Spliced::new(&[2, 0]);
        assert_eq!(spliced.total(5), 7);
        let page = spliced.page(0, 4, 5);
        assert_eq!(
            page.slots,
            [
                Slot::Surfaced(1),
                Slot::Stored(0),
                Slot::Stored(1),
                Slot::Surfaced(0)
            ]
        );
        assert_eq!((page.offset, page.limit), (0, 2));
        let next = spliced.page(4, 4, 5);
        assert_eq!(
            next.slots,
            [Slot::Stored(0), Slot::Stored(1), Slot::Stored(2)]
        );
        assert_eq!((next.offset, next.limit), (2, 3), "the page after both");
    }

    #[test]
    fn an_unknown_store_total_asks_for_every_position_not_surfaced() {
        // The feed asks before it knows how the store has moved, and places
        // the answer by the total that comes back.
        let spliced = Spliced::new(&[0]);
        let asked = spliced.page(0, 50, u32::MAX);
        assert_eq!((asked.offset, asked.limit), (0, 49));
    }

    #[test]
    fn rows_at_one_position_keep_their_order_and_the_end_holds_one() {
        let spliced = Spliced::new(&[1, 1, 3]);
        assert_eq!(
            spliced.page(0, 10, 3).slots,
            [
                Slot::Stored(0),
                Slot::Surfaced(0),
                Slot::Surfaced(1),
                Slot::Stored(1),
                Slot::Stored(2),
                Slot::Surfaced(2)
            ]
        );
    }
}
