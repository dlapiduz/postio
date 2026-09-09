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
}
