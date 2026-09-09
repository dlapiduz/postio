//! Which message you are reading, and what the rail says about the rest.
//!
//! The conversation rail lists every message in a thread and marks the one on
//! screen. `Design/conversation-rail-brief.md` is blunt about where the
//! difficulty is: *"The rail's entire value is the marked row being correct,
//! and correct is **not** 'the last one you clicked.'"*
//!
//! # Why the rule lives here
//!
//! It is arithmetic over geometry rather than geometry itself, so it belongs
//! in the toolkit-free crate where it can be proven in milliseconds. That
//! matters more than usual: this repository's test display produces **no
//! layout at all** — every `getBoundingClientRect` is zero on CI (#1307) — so
//! a rule asserted against a rendered pane would be untestable, while the same
//! rule asserted as a function over given geometries is trivially testable and
//! is the same answer in both frontends.

/// Where a message sits in the scroll, in whatever unit the frontend counts
/// in — pixels from the top of the document, and a height.
///
/// The frontend supplies these; nothing here asks how they were obtained.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Extent {
    /// Distance from the top of the scrolled content to the top of the message.
    pub top: f64,
    /// How tall the message is.
    pub height: f64,
}

impl Extent {
    /// How much of this message is inside a viewport of `height` starting at
    /// `scroll`.
    ///
    /// Zero when the message is entirely above or below it — never negative,
    /// which is the arithmetic that makes [`current`] safe to fold over.
    pub fn visible(self, scroll: f64, height: f64) -> f64 {
        let top = self.top.max(scroll);
        let bottom = (self.top + self.height).min(scroll + height);
        (bottom - top).max(0.0)
    }
}

/// Which message the reader is looking at.
///
/// **Greatest visible area, not greatest visible fraction.** The brief gives
/// the case that decides it: *"a fully-visible two-line reply beats an 84-line
/// essay filling 90% of the viewport"* if you rank by ratio. The short reply
/// scores 1.0 and the essay scores 0.9, and the answer is obviously wrong —
/// what a person is reading is the thing filling the screen.
///
/// Ties go to the **earlier** message. A tie means two messages fill the
/// viewport equally, which happens while scrolling between them, and taking
/// the later one would make the mark jump ahead of the reader.
///
/// `None` when nothing is visible at all, which is a real state: a thread can
/// be scrolled past its own content while the pane settles.
pub fn current(extents: &[Extent], scroll: f64, height: f64) -> Option<usize> {
    extents
        .iter()
        .enumerate()
        .map(|(index, extent)| (index, extent.visible(scroll, height)))
        .filter(|(_, visible)| *visible > 0.0)
        // `fold` rather than `max_by`, because `max_by` keeps the *last*
        // maximum and the tie has to go to the first.
        .fold(
            None,
            |best: Option<(usize, f64)>, (index, visible)| match best {
                Some((_, most)) if most >= visible => best,
                _ => Some((index, visible)),
            },
        )
        .map(|(index, _)| index)
}

/// How long a message has to be before its length is worth showing.
///
/// From the brief: a length appears *"only on messages long enough to matter
/// (over ~40 lines), so you can see the essay before you scroll into it"*. A
/// count on every row is noise; a count on the long ones is information.
pub const LENGTH_THRESHOLD: u32 = 40;

/// What one rail row says.
///
/// Built from the thread model rather than from anything drawn, so every
/// message has a row the moment the conversation is known — spec FR-040. The
/// rail exists to let you skip a long message; a row that waited for that
/// message's body would make you wait for exactly what you were skipping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// One-based, as a person counts.
    pub position: usize,
    /// Who wrote it.
    pub sender: String,
    /// Its length, when there is one and it is worth saying.
    pub length: Option<u32>,
}

/// The rail's rows for a thread.
///
/// `lengths` is per message and `None` where the body is not local yet, which
/// is a different fact from a short message and is shown as no length rather
/// than as zero.
pub fn rows(senders: &[String], lengths: &[Option<u32>]) -> Vec<Row> {
    senders
        .iter()
        .enumerate()
        .map(|(index, sender)| Row {
            position: index + 1,
            sender: sender.clone(),
            length: lengths
                .get(index)
                .copied()
                .flatten()
                .filter(|lines| *lines >= LENGTH_THRESHOLD),
        })
        .collect()
}

/// How the rail is presented at a given window width.
///
/// One component, three presentations over the same data and the same
/// activation behaviour (FR-044). The brief is explicit that these are not
/// three widgets: *"Build the index as one component with two presentations
/// (column and popover) over the same data and the same click behaviour"*, and
/// screen 29 counts the narrowed column as a third.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presentation {
    /// A 150px column. Numbers, senders and lengths.
    Full,
    /// A 118px column. Numbers and initials only — there is not room for a
    /// name, and a truncated name is worse than none.
    Narrow,
    /// No column. The header carries a counter that opens the same index.
    Popover,
}

/// Below this the rail unmounts and the header carries the counter.
pub const UNMOUNT_BELOW: i32 = 1100;

/// Below this the rail narrows, at and above it the rail is full width.
pub const NARROW_BELOW: i32 = 1240;

/// Which presentation a window of `width` gets, if any.
///
/// `None` means no rail and no counter at all — a single-message conversation
/// has nothing to index (FR-045), and `hidden` is the reader's own `⇧R`
/// choice, which belongs to the window rather than to the conversation open in
/// it (FR-047).
///
/// The reading measure never gives up width to fund the rail (FR-043), which
/// is what the bottom step of the ladder is *for*: the rail unmounts rather
/// than the body narrowing.
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

/// What the caller must do after asking the rail to move its mark.
///
/// Returned by every entry point, so a caller that does nothing on
/// [`Effect::Nothing`] repaints only when something actually changed. That is
/// where the brief's *"never animate it"* is enforceable: a widget told to
/// move only when the value differs has nothing to animate between.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// The mark is already where it was asked to go. Do not repaint.
    Nothing,
    /// The mark moved. Repaint the rail; leave the pane where it is.
    Mark,
    /// The mark moved and the pane must be scrolled to it. Hand the token
    /// back to [`Rail::settled`] when the scroll finishes.
    MarkAndScroll(Settle),
}

/// Names one programmatic scroll, so a settle that arrives late cannot end
/// the suppression belonging to a newer one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settle(u64);

/// The rail's state for one window: which message is marked, and whether the
/// reader wants to see the rail at all.
///
/// # One entry point
///
/// Three things move the mark — a rail row being activated, `J`/`K`, and the
/// observer reporting what is on screen — and the brief requires they resolve
/// through one place *"so keyboard nav and scroll-derived marking can never
/// disagree"*. They all end up in [`Rail::mark`], which is private: there is
/// no second way to set the value.
#[derive(Debug, Clone)]
pub struct Rail {
    count: usize,
    marked: Option<usize>,
    /// The scroll currently in flight, if any. While this is set the observer
    /// is not listened to, because what it can see is the scroll passing over
    /// messages on its way somewhere the reader already chose.
    suppressed: Option<u64>,
    /// Names each scroll in turn. Without it, the settle belonging to a
    /// finished scroll would end the suppression of a newer one.
    scrolls: u64,
}

impl Rail {
    /// A rail for a conversation of `count` messages, nothing marked yet.
    pub fn new(count: usize) -> Self {
        Self {
            count,
            marked: None,
            suppressed: None,
            scrolls: 0,
        }
    }

    /// A rail for a conversation of `count` messages with `marked` already
    /// current.
    ///
    /// For a caller that keeps the mark somewhere else and wants this to
    /// answer one question about it — where `J` goes from here.
    pub fn at(count: usize, marked: Option<usize>) -> Self {
        Self {
            marked: marked.filter(|index| *index < count),
            ..Self::new(count)
        }
    }

    /// Which message is marked.
    pub fn marked(&self) -> Option<usize> {
        self.marked
    }

    /// A new conversation is being shown.
    pub fn set_conversation(&mut self, count: usize) {
        self.count = count;
        self.marked = None;
        // A scroll belonging to the conversation that just went away must not
        // go on silencing the observer in the one that replaced it.
        self.suppressed = None;
    }

    /// A rail row was activated, or `J`/`K` moved. The pane follows.
    pub fn activate(&mut self, index: usize) -> Effect {
        self.mark(index, true)
    }

    /// `J`: walk to the next message. Stops at the last one rather than
    /// wrapping — a thread has an oldest and a newest, and jumping from the
    /// newest back to the oldest is not what the key means.
    ///
    /// Named for the message rather than as `next`, because a `next` taking
    /// `&mut self` on a non-iterator reads as one and clippy says so.
    pub fn next_message(&mut self) -> Effect {
        match self.marked {
            Some(index) if index + 1 < self.count => self.activate(index + 1),
            None if self.count > 0 => self.activate(0),
            _ => Effect::Nothing,
        }
    }

    /// `K`: walk to the previous message, stopping at the first.
    ///
    /// With nothing marked it starts at the **last** message, as `J` starts at
    /// the first — the rule `Conversation::step` already shipped for the
    /// stacked pane. Starting both at the first would make `K` in a
    /// freshly-opened thread walk forwards.
    pub fn previous_message(&mut self) -> Effect {
        match self.marked {
            Some(index) if index > 0 => self.activate(index - 1),
            None if self.count > 0 => self.activate(self.count - 1),
            _ => Effect::Nothing,
        }
    }

    /// The observer reported what is on screen. The pane does not follow —
    /// it is already there.
    pub fn observed(&mut self, index: Option<usize>) -> Effect {
        match index {
            Some(index) => self.mark(index, false),
            None => Effect::Nothing,
        }
    }

    /// A programmatic scroll finished, and the observer may speak again.
    ///
    /// Ignores a token that does not name the scroll in flight, so a `scrollend`
    /// arriving late for a superseded scroll changes nothing. Callers should
    /// also arm a timeout against this: `scrollend` does not always arrive, and
    /// suppression that never ends is a mark frozen for the rest of the
    /// session — worse than the loop it prevents.
    pub fn settled(&mut self, settle: Settle) {
        if self.suppressed == Some(settle.0) {
            self.suppressed = None;
        }
    }

    /// The one place the mark is set.
    ///
    /// `scroll` distinguishes the two kinds of caller: someone choosing a
    /// message, whom the pane must follow, and the observer describing where
    /// the pane already is. Only the second is suppressible — swallowing a
    /// keypress because the pane is still catching up would read as a broken
    /// key.
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
        // The brief's own case, and the reason the rule is area rather than
        // ratio: the reply is 100% visible and the essay is 90% visible, and
        // the essay is what fills the screen.
        let reply = extent(0.0, 40.0);
        let essay = extent(40.0, 2000.0);
        // A 1000px viewport showing all of the reply and 960px of the essay.
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
        // Scrolled past the end while the pane settles.
        assert_eq!(current(&[extent(0.0, 50.0)], 500.0, 100.0), None);
    }

    #[test]
    fn a_tie_goes_to_the_earlier_message() {
        // Two halves of the viewport each. Taking the later one would put the
        // mark ahead of the reader every time they scrolled between messages.
        let first = extent(0.0, 50.0);
        let second = extent(50.0, 50.0);
        assert_eq!(current(&[first, second], 0.0, 100.0), Some(0));
    }

    #[test]
    fn a_length_below_the_threshold_is_not_worth_saying() {
        let rows = rows(
            &["Ada".to_owned(), "Grace".to_owned()],
            &[Some(LENGTH_THRESHOLD - 1), Some(LENGTH_THRESHOLD)],
        );
        assert_eq!(rows[0].length, None, "a short message shows no length");
        assert_eq!(rows[1].length, Some(LENGTH_THRESHOLD));
    }

    #[test]
    fn a_body_that_has_not_arrived_has_no_length_rather_than_zero() {
        // Different facts: "nothing to count yet" and "counted, and it is
        // short" both show no number, but the row exists either way — which is
        // the whole of FR-040.
        let rows = rows(&["Ada".to_owned()], &[None]);
        assert_eq!(rows[0].length, None);
        assert_eq!(rows[0].position, 1, "the row exists without a body");
    }

    #[test]
    fn rows_are_numbered_as_a_person_counts() {
        let rows = rows(&["Ada".to_owned(), "Grace".to_owned()], &[None, None]);
        assert_eq!(
            rows.iter().map(|row| row.position).collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    /// A helper that activates and hands back the token, because every
    /// suppression test needs both.
    fn activate(rail: &mut Rail, index: usize) -> Settle {
        match rail.activate(index) {
            Effect::MarkAndScroll(settle) => settle,
            other => panic!("activating a row must scroll the pane, got {other:?}"),
        }
    }

    #[test]
    fn activation_and_the_observer_agree_on_the_mark() {
        // The whole point of one entry point: once the scroll activation asked
        // for has finished, the observer reporting that same message is not
        // news. If these were two paths setting the value independently, the
        // agreement would be a coincidence rather than a property.
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
        // The sync loop the brief names: the programmatic scroll passes over
        // message 0 on its way to 3, the observer sees it, and without
        // suppression the mark lands back where the reader was not going.
        let mut rail = Rail::new(6);
        activate(&mut rail, 3);
        assert_eq!(rail.observed(Some(0)), Effect::Nothing);
        assert_eq!(rail.marked(), Some(3), "the mark stays where it was sent");
    }

    #[test]
    fn a_stale_settle_does_not_unsuppress_a_newer_scroll() {
        // Two presses of J in quick succession. The first scroll's settle
        // arrives while the second is still in flight; clearing suppression on
        // it would reopen exactly the window this closes.
        let mut rail = Rail::new(6);
        let first = activate(&mut rail, 1);
        activate(&mut rail, 4);
        rail.settled(first);
        assert_eq!(rail.observed(Some(0)), Effect::Nothing);
        assert_eq!(rail.marked(), Some(4));
    }

    #[test]
    fn settling_lets_the_observer_speak_again() {
        // The other half: suppression that never ends is a mark frozen for the
        // rest of the session, which is worse than the loop it prevents.
        let mut rail = Rail::new(6);
        let settle = activate(&mut rail, 3);
        rail.settled(settle);
        assert_eq!(rail.observed(Some(0)), Effect::Mark);
        assert_eq!(rail.marked(), Some(0));
    }

    #[test]
    fn an_activation_is_never_suppressed() {
        // Suppression is aimed at the observer alone. Holding J must keep
        // moving; a reader whose keypresses were swallowed while the pane
        // caught up would think the key was broken.
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
        // `current` returns None while the pane is scrolled past its content.
        // Blanking the rail for that moment would be a flicker, not a fact.
        let mut rail = Rail::new(6);
        rail.observed(Some(2));
        assert_eq!(rail.observed(None), Effect::Nothing);
        assert_eq!(rail.marked(), Some(2));
    }

    #[test]
    fn walking_the_thread_stops_at_the_ends() {
        // No wrapping: a thread has a first and a last message, and jumping
        // from the newest back to the oldest is not what J means.
        let mut rail = Rail::new(2);
        activate(&mut rail, 1);
        assert_eq!(rail.next_message(), Effect::Nothing);
        assert_eq!(rail.marked(), Some(1));
        activate(&mut rail, 0);
        assert_eq!(rail.previous_message(), Effect::Nothing);
        assert_eq!(rail.marked(), Some(0));
    }

    #[test]
    fn a_new_conversation_forgets_the_old_mark() {
        // Message 3 of the thread you just left is not message 3 of this one.
        let mut rail = Rail::new(6);
        rail.observed(Some(3));
        rail.set_conversation(4);
        assert_eq!(rail.marked(), None);
    }

    #[test]
    fn a_new_conversation_forgets_a_scroll_in_flight() {
        // Selecting another conversation while a scroll is still travelling
        // would otherwise leave the observer muted in the new one, and the
        // mark would sit on nothing until the reader happened to press a key.
        let mut rail = Rail::new(6);
        activate(&mut rail, 3);
        rail.set_conversation(4);
        assert_eq!(rail.observed(Some(1)), Effect::Mark);
        assert_eq!(rail.marked(), Some(1));
    }

    #[test]
    fn walking_from_nothing_starts_at_the_end_you_came_from() {
        // The rule `Conversation::step` already shipped: with nothing marked,
        // `J` starts at the first message and `K` at the last. Landing on the
        // first for both would make `K` in a fresh thread walk forwards.
        let mut rail = Rail::new(6);
        assert_eq!(rail.next_message(), Effect::MarkAndScroll(Settle(1)));
        assert_eq!(rail.marked(), Some(0), "J starts at the beginning");

        let mut rail = Rail::new(6);
        rail.previous_message();
        assert_eq!(rail.marked(), Some(5), "K starts at the end");
    }

    #[test]
    fn the_ladder_has_three_steps_and_a_floor() {
        // Screen 29's table, read straight across.
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
        // FR-045. Not even the counter: a `1/1` that opens a list of one is
        // an index of nothing, and it would appear on most of the mail a
        // person actually reads.
        for width in [900, 1200, 1600] {
            assert_eq!(presentation(width, 1, false), None, "at {width}px");
        }
    }

    #[test]
    fn hiding_the_rail_holds_at_every_width() {
        // FR-047: `⇧R` is a decision about this window, and widening the
        // window is not a request to undo it.
        for width in [900, 1200, 1600] {
            assert_eq!(presentation(width, 6, true), None, "at {width}px");
        }
    }
}
