//! Where Focus's surfaced rows (digest deliveries, fired reminders) sit among conversations, and
//! the arithmetic between the store's pages and the list's positions.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Surfaced(usize),
    Stored(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorePage {
    pub offset: u32,
    pub limit: u32,
    pub slots: Vec<Slot>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Spliced {
    at: Vec<(u32, usize)>,
}

impl Spliced {
    /// Rows at one position keep the order they were given in.
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

    pub fn len(&self) -> u32 {
        self.at.len() as u32
    }

    pub fn is_empty(&self) -> bool {
        self.at.is_empty()
    }

    pub fn total(&self, conversations: u32) -> u32 {
        conversations.saturating_add(self.len())
    }

    /// A digest stands for many and opens as a window, so neither the position line nor the strip's
    /// count includes it.
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
        let spliced = Spliced::new(&[0, 2]);
        assert_eq!(spliced.message_place(&[0], 1, 7), (0, 6), "first message");
        assert_eq!(spliced.message_place(&[0], 3, 7), (2, 6), "the reminder");
        assert_eq!(spliced.message_place(&[0], 6, 7), (5, 6), "the last");
        assert_eq!(spliced.message_place(&[], 3, 7), (3, 7), "no digests");
    }

    #[test]
    fn a_row_at_its_position_pushes_the_conversations_after_it_down() {
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
