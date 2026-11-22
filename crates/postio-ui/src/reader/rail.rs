//! Which message is marked in the conversation rail, and how the rail is presented.
//! Pure arithmetic over given geometry, because the test display produces no layout (#1307).

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Extent {
    pub top: f64,
    pub height: f64,
}

impl Extent {
    pub fn visible(self, scroll: f64, height: f64) -> f64 {
        let top = self.top.max(scroll);
        let bottom = (self.top + self.height).min(scroll + height);
        (bottom - top).max(0.0)
    }
}

/// Greatest visible area, not fraction: a fully visible two-line reply must not beat an essay
/// filling the screen.
/// Ties go to the earlier message so the mark does not jump ahead; `None` when nothing is visible.
pub fn current(extents: &[Extent], scroll: f64, height: f64) -> Option<usize> {
    extents
        .iter()
        .enumerate()
        .map(|(index, extent)| (index, extent.visible(scroll, height)))
        .filter(|(_, visible)| *visible > 0.0)
        // `fold`, not `max_by`: `max_by` keeps the last maximum and ties go to the first.
        .fold(
            None,
            |best: Option<(usize, f64)>, (index, visible)| match best {
                Some((_, most)) if most >= visible => best,
                _ => Some((index, visible)),
            },
        )
        .map(|(index, _)| index)
}

const LENGTH_THRESHOLD: u32 = 40;

/// Built from the thread model, not from anything drawn, so rows exist before bodies load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub position: usize,
    pub sender: String,
    pub initials: String,
    /// Not drawn by the rail; the row's accessible name says it.
    pub when: String,
    pub length: Option<u32>,
}

/// `lengths` is `None` where the body is not local yet (no length shown, not zero).
/// `initials` is passed in because `sender` may be a bare address that would abbreviate wrong.
pub fn rows(
    senders: &[String],
    initials: &[String],
    whens: &[String],
    lengths: &[Option<u32>],
) -> Vec<Row> {
    senders
        .iter()
        .enumerate()
        .map(|(index, sender)| Row {
            position: index + 1,
            sender: sender.clone(),
            initials: initials.get(index).cloned().unwrap_or_default(),
            when: whens.get(index).cloned().unwrap_or_default(),
            length: lengths
                .get(index)
                .copied()
                .flatten()
                .filter(|lines| *lines >= LENGTH_THRESHOLD),
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presentation {
    Full,
    Narrow,
    Popover,
}

const UNMOUNT_BELOW: i32 = 1100;

const NARROW_BELOW: i32 = 1240;

/// `None` for a single-message conversation, or when the reader hid the rail with `⇧I` (a window
/// choice).
/// The body never narrows to fund the rail.
pub fn presentation(width: i32, messages: usize, hidden: bool) -> Option<Presentation> {
    if hidden || messages < 2 {
        return None;
    }
    if width < UNMOUNT_BELOW {
        return Some(Presentation::Popover);
    }
    if width < NARROW_BELOW {
        return Some(Presentation::Narrow);
    }
    Some(Presentation::Full)
}

/// The column kept for the rail whatever the thread holds, so the body does not change width
/// between threads.
/// `None` for the popover step and a hidden rail.
pub fn column(width: i32, hidden: bool) -> Option<Presentation> {
    match presentation(width, 2, hidden) {
        Some(Presentation::Popover) | None => None,
        step => step,
    }
}

/// Returned so a caller repaints only when something changed; there is nothing to animate between.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Nothing,
    Mark,
    /// Hand the token back to `Rail::settled` when the scroll finishes.
    MarkAndScroll(Settle),
}

/// Names one programmatic scroll, so a settle that arrives late cannot end
/// the suppression belonging to a newer one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settle(u64);

/// Every move of the mark goes through the private `Rail::mark`, so keyboard and scroll-derived
/// marking cannot disagree.
#[derive(Debug, Clone)]
pub struct Rail {
    count: usize,
    marked: Option<usize>,
    /// The scroll currently in flight, if any. While this is set the observer
    /// is not listened to, because what it can see is the scroll passing over
    /// messages on its way somewhere the reader already chose.
    suppressed: Option<u64>,
    scrolls: u64,
}

impl Rail {
    pub fn new(count: usize) -> Self {
        Self {
            count,
            marked: None,
            suppressed: None,
            scrolls: 0,
        }
    }

    pub fn at(count: usize, marked: Option<usize>) -> Self {
        Self {
            marked: marked.filter(|index| *index < count),
            ..Self::new(count)
        }
    }

    pub fn marked(&self) -> Option<usize> {
        self.marked
    }

    pub fn activate(&mut self, index: usize) -> Effect {
        self.mark(index, true)
    }

    pub fn observed(&mut self, index: Option<usize>) -> Effect {
        match index {
            Some(index) => self.mark(index, false),
            None => Effect::Nothing,
        }
    }

    /// A stale token changes nothing. Callers should arm a timeout: `scrollend` does not always
    /// arrive, and suppression that never ends freezes the mark.
    pub fn settled(&mut self, settle: Settle) {
        if self.suppressed == Some(settle.0) {
            self.suppressed = None;
        }
    }

    /// Only the observer is suppressible: swallowing a keypress while the pane catches up would
    /// read as a broken key.
    fn mark(&mut self, index: usize, scroll: bool) -> Effect {
        if index >= self.count {
            return Effect::Nothing;
        }
        if !scroll && self.suppressed.is_some() {
            return Effect::Nothing;
        }
        if self.marked == Some(index) {
            return Effect::Nothing;
        }
        self.marked = Some(index);
        if !scroll {
            return Effect::Mark;
        }
        self.scrolls += 1;
        self.suppressed = Some(self.scrolls);
        Effect::MarkAndScroll(Settle(self.scrolls))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extent(top: f64, height: f64) -> Extent {
        Extent { top, height }
    }

    #[test]
    fn the_essay_wins_over_the_fully_visible_reply() {
        let reply = extent(0.0, 40.0);
        let essay = extent(40.0, 2000.0);
        assert_eq!(current(&[reply, essay], 0.0, 1000.0), Some(1));
    }

    #[test]
    fn a_message_entirely_off_screen_counts_for_nothing() {
        let above = extent(0.0, 100.0);
        let showing = extent(100.0, 100.0);
        assert_eq!(current(&[above, showing], 100.0, 100.0), Some(1));
    }

    #[test]
    fn nothing_visible_is_a_real_answer() {
        assert_eq!(current(&[extent(0.0, 50.0)], 500.0, 100.0), None);
    }

    #[test]
    fn a_tie_goes_to_the_earlier_message() {
        let first = extent(0.0, 50.0);
        let second = extent(50.0, 50.0);
        assert_eq!(current(&[first, second], 0.0, 100.0), Some(0));
    }

    #[test]
    fn a_length_below_the_threshold_is_not_worth_saying() {
        let rows = rows(
            &["Ada".to_owned(), "Grace".to_owned()],
            &["AD".to_owned(), "GR".to_owned()],
            &["1 Sep".to_owned(), "2 Sep".to_owned()],
            &[Some(LENGTH_THRESHOLD - 1), Some(LENGTH_THRESHOLD)],
        );
        assert_eq!(rows[0].length, None, "a short message shows no length");
        assert_eq!(rows[1].length, Some(LENGTH_THRESHOLD));
    }

    #[test]
    fn a_body_that_has_not_arrived_has_no_length_rather_than_zero() {
        let rows = rows(
            &["Ada".to_owned()],
            &["AD".to_owned()],
            &["1 Sep".to_owned()],
            &[None],
        );
        assert_eq!(rows[0].length, None);
        assert_eq!(rows[0].position, 1, "the row exists without a body");
    }

    #[test]
    fn rows_are_numbered_as_a_person_counts() {
        let rows = rows(
            &["Ada".to_owned(), "Grace".to_owned()],
            &["AD".to_owned(), "GR".to_owned()],
            &["1 Sep".to_owned(), "2 Sep".to_owned()],
            &[None, None],
        );
        assert_eq!(
            rows.iter().map(|row| row.position).collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    fn activate(rail: &mut Rail, index: usize) -> Settle {
        match rail.activate(index) {
            Effect::MarkAndScroll(settle) => settle,
            other => panic!("activating a row must scroll the pane, got {other:?}"),
        }
    }

    #[test]
    fn activation_and_the_observer_agree_on_the_mark() {
        let mut rail = Rail::new(6);
        let settle = activate(&mut rail, 2);
        rail.settled(settle);
        assert_eq!(rail.marked(), Some(2));
        assert_eq!(
            rail.observed(Some(2)),
            Effect::Nothing,
            "the observer confirming the activation is not a change"
        );
    }

    #[test]
    fn a_report_from_a_scroll_in_flight_does_not_move_the_mark() {
        // The programmatic scroll passes over message 0 on the way to 3; unsuppressed, the mark
        // lands back there.
        let mut rail = Rail::new(6);
        activate(&mut rail, 3);
        assert_eq!(rail.observed(Some(0)), Effect::Nothing);
        assert_eq!(rail.marked(), Some(3), "the mark stays where it was sent");
    }

    #[test]
    fn a_stale_settle_does_not_unsuppress_a_newer_scroll() {
        // The first scroll's settle must not clear the second's suppression.
        let mut rail = Rail::new(6);
        let first = activate(&mut rail, 1);
        activate(&mut rail, 4);
        rail.settled(first);
        assert_eq!(rail.observed(Some(0)), Effect::Nothing);
        assert_eq!(rail.marked(), Some(4));
    }

    #[test]
    fn settling_lets_the_observer_speak_again() {
        let mut rail = Rail::new(6);
        let settle = activate(&mut rail, 3);
        rail.settled(settle);
        assert_eq!(rail.observed(Some(0)), Effect::Mark);
        assert_eq!(rail.marked(), Some(0));
    }

    #[test]
    fn an_activation_is_never_suppressed() {
        let mut rail = Rail::new(6);
        activate(&mut rail, 1);
        activate(&mut rail, 2);
        assert_eq!(rail.marked(), Some(2));
    }

    #[test]
    fn re_reporting_the_marked_message_is_not_a_change() {
        let mut rail = Rail::new(6);
        assert_eq!(rail.observed(Some(1)), Effect::Mark);
        assert_eq!(
            rail.observed(Some(1)),
            Effect::Nothing,
            "a settled scroll reports repeatedly; only the first is news"
        );
    }

    #[test]
    fn nothing_visible_leaves_the_mark_alone() {
        let mut rail = Rail::new(6);
        rail.observed(Some(2));
        assert_eq!(rail.observed(None), Effect::Nothing);
        assert_eq!(rail.marked(), Some(2));
    }

    #[test]
    fn the_ladder_has_three_steps_and_a_floor() {
        assert_eq!(presentation(1400, 6, false), Some(Presentation::Full));
        assert_eq!(
            presentation(NARROW_BELOW, 6, false),
            Some(Presentation::Full)
        );
        assert_eq!(
            presentation(NARROW_BELOW - 1, 6, false),
            Some(Presentation::Narrow)
        );
        assert_eq!(
            presentation(UNMOUNT_BELOW, 6, false),
            Some(Presentation::Narrow)
        );
        assert_eq!(
            presentation(UNMOUNT_BELOW - 1, 6, false),
            Some(Presentation::Popover)
        );
    }

    #[test]
    fn a_single_message_has_no_rail_at_any_width() {
        for width in [900, 1200, 1600] {
            assert_eq!(presentation(width, 1, false), None, "at {width}px");
        }
    }

    #[test]
    fn the_column_kept_does_not_depend_on_the_thread() {
        assert_eq!(column(1400, false), Some(Presentation::Full));
        assert_eq!(column(NARROW_BELOW - 1, false), Some(Presentation::Narrow));
        assert_eq!(
            column(UNMOUNT_BELOW - 1, false),
            None,
            "the popover step keeps no column: the index is in the header"
        );
        for width in [900, 1150, 1400, 1900] {
            assert_eq!(column(width, true), None, "hidden at {width}px");
            assert_eq!(presentation(width, 1, false), None);
            assert_eq!(
                column(width, false),
                presentation(width, 6, false).filter(|step| *step != Presentation::Popover),
                "at {width}px"
            );
        }
    }

    #[test]
    fn hiding_the_rail_holds_at_every_width() {
        for width in [900, 1200, 1600] {
            assert_eq!(presentation(width, 6, true), None, "at {width}px");
        }
    }
}

#[cfg(test)]
mod at_tests {
    use super::Rail;

    #[test]
    fn a_mark_inside_the_conversation_is_kept_and_one_past_it_is_not() {
        assert_eq!(Rail::at(4, Some(2)).marked(), Some(2));
        assert_eq!(Rail::at(4, Some(4)).marked(), None);
        assert_eq!(Rail::at(4, None).marked(), None);
    }
}
