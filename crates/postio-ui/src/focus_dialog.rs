//! The open-message dialog's geometry, with no toolkit in it (the message
//! dialog redesign, T205-T208 in `specs/007-postio-focus/tasks.md`).
//!
//! Every size here comes from the window or from the body's treatment and
//! from nothing else, so moving through the list with `j`/`k` never resizes
//! the dialog or moves its column: the only inputs are the window's size and
//! [`Treatment`], and neither changes on a step unless the body's treatment
//! does.

use postio_body::treatment::Treatment;

/// The narrowest the dialog gets.
pub const DIALOG_MIN: i32 = 640;
/// The widest the dialog gets.
pub const DIALOG_MAX: i32 = 820;
/// The least of the list left showing on each side.
pub const SIDE_MIN: f64 = 96.0;
/// The share of the window's width left showing on each side.
pub const SIDE_SHARE: f64 = 0.18;
/// How much of the window's height the dialog leaves: 40px above and below.
pub const HEIGHT_INSET: i32 = 80;
/// Below this width the action row folds Label, Move and Delete into More.
pub const MORE_BELOW: i32 = 760;

/// The column for a body in the app's colours: 32em at the 15px reading
/// size, about 70 characters.
pub const COLUMN_APP_COLOURS: i32 = 480;
/// What the dialog keeps either side of an app-colours column, at least.
pub const COLUMN_APP_COLOURS_INSET: i32 = 96;
/// The column for a body shown on paper, as sent.
pub const COLUMN_PAPER: i32 = 640;
/// What the dialog keeps either side of a paper column, at least.
pub const COLUMN_PAPER_INSET: i32 = 48;

/// Between a control's words and its keycap.
pub const KEYCAP_GAP: i32 = 6;
/// Between a step's chevron and its keycap (T219): a glyph rather than a
/// word, and the pair has to stay small in the header.
pub const STEP_KEYCAP_GAP: i32 = 4;
/// Between label pills, and between a pill's dot and its name.
pub const LABEL_GAP: i32 = 6;
/// A label pill's colour dot, across.
pub const LABEL_DOT: i32 = 8;
/// Between the pieces of the thread marker's line.
pub const THREAD_GAP: i32 = 8;

/// The sender block's column of field names (From, To, Cc).
pub const SENDER_LABEL_COLUMN: i32 = 44;
/// Between the sender block's columns.
pub const SENDER_COLUMN_GAP: i32 = 12;
/// Between the sender block's rows.
pub const SENDER_ROW_GAP: i32 = 2;

/// The dialog's width in a window `window` pixels wide:
/// `clamp(640, W - 2 * max(96, 0.18 * W), 820)`, rounded to the pixel.
pub fn dialog_width(window: i32) -> i32 {
    let window = f64::from(window);
    let side = SIDE_MIN.max(SIDE_SHARE * window);
    ((window - 2.0 * side).round() as i32).clamp(DIALOG_MIN, DIALOG_MAX)
}

/// The dialog's height in a window `window` pixels tall: 40px off each end.
pub fn dialog_height(window: i32) -> i32 {
    (window - HEIGHT_INSET).max(0)
}

/// Whether a dialog `dialog` pixels wide folds Label, Move and Delete into
/// More (`.`).
pub fn folds_into_more(dialog: i32) -> bool {
    dialog < MORE_BELOW
}

/// The content column's width in a dialog `dialog` pixels wide, for a body
/// shown as `treatment`: `min(480, dialog - 96)` in the app's colours,
/// `min(640, dialog - 48)` on paper. Everything inside the message --
/// thread marker, subject, labels, sender block, action card, body and
/// attachments -- shares it.
pub fn column_width(dialog: i32, treatment: Treatment) -> i32 {
    match treatment {
        Treatment::AppColours => COLUMN_APP_COLOURS.min(dialog - COLUMN_APP_COLOURS_INSET),
        Treatment::Paper => COLUMN_PAPER.min(dialog - COLUMN_PAPER_INSET),
    }
    .max(0)
}

/// The narrowest the list gets beside a reading pane (T232): canvas 1b's
/// list width, the classic app's.
pub const LIST_MIN: i32 = 404;
/// The narrowest reading pane: the app colours column and its inset, so the
/// column is never squeezed.
pub const PANE_MIN: i32 = COLUMN_APP_COLOURS + COLUMN_APP_COLOURS_INSET;
/// The widest reading pane: the dialog at its widest, so a message reads at
/// the same measure beside the list as over it.
pub const PANE_MAX: i32 = DIALOG_MAX;
/// The narrowest window with room for the list and a pane beside it.
pub const PANE_WINDOW_MIN: i32 = LIST_MIN + PANE_MIN;

/// Where an open message is drawn: over the list, or beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Placement {
    /// The message dialog, over the list (T205).
    #[default]
    Dialog,
    /// A pane beside the list (T232).
    Pane,
}

/// The reading pane's width in a window `window` pixels wide:
/// `min(820, W - 404)`, or `None` when that would be under 576 -- a window
/// narrower than [`PANE_WINDOW_MIN`] has no room for one.
pub fn pane_width(window: i32) -> Option<i32> {
    let pane = (window - LIST_MIN).min(PANE_MAX);
    (pane >= PANE_MIN).then_some(pane)
}

/// Where a message opens in a window `window` pixels wide, when the person
/// chose `chosen`: a pane only where one fits, the dialog otherwise.
pub fn placement(chosen: Placement, window: i32) -> Placement {
    match chosen {
        Placement::Pane if pane_width(window).is_some() => Placement::Pane,
        _ => Placement::Dialog,
    }
}

/// The vertical rhythm, in pixels (an 8px grid around a 24px body line).
/// A block that is absent takes the gap above it with it; the next gap
/// stays as listed.
pub mod rhythm {
    /// The action row to the first block: the content's top padding.
    pub const TOP: i32 = 28;
    /// The thread marker to the subject.
    pub const MARKER_TO_SUBJECT: i32 = 12;
    /// The subject to the labels.
    pub const SUBJECT_TO_LABELS: i32 = 10;
    /// The labels to the sender block.
    pub const LABELS_TO_SENDER: i32 = 16;
    /// Inside the sender block, above and below its rows.
    pub const SENDER_PADDING: i32 = 12;
    /// One row of the sender block.
    pub const SENDER_ROW: i32 = 22;
    /// The sender block to the action card.
    pub const SENDER_TO_CARD: i32 = 12;
    /// The action card (or the sender block, with no card) to the body.
    pub const CARD_TO_BODY: i32 = 24;
    /// The render-mode line to the body.
    pub const MODE_LINE_TO_BODY: i32 = 12;
    /// The body to the attachments' hairline.
    pub const BODY_TO_ATTACHMENTS: i32 = 24;
    /// The attachments' hairline to the chips.
    pub const ATTACHMENTS_RULE_TO_CHIPS: i32 = 16;
    /// Between two attachment chips.
    pub const CHIP_GAP: i32 = 8;
    /// Under the last block.
    pub const BOTTOM: i32 = 32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dialog_is_as_wide_as_the_window_allows_and_no_wider() {
        // 1024 - 2 * 184.32 = 655.36, rounded; 1280 - 2 * 230.4 = 819.2.
        assert_eq!(dialog_width(1024), 655);
        assert_eq!(dialog_width(1280), 819);
        assert_eq!(dialog_width(1440), 820);
        assert_eq!(dialog_width(1920), 820);
        // Narrow windows keep the floor, and the list's share runs out first.
        assert_eq!(dialog_width(800), 640);
        assert_eq!(dialog_width(500), 640);
    }

    #[test]
    fn at_least_184px_of_the_list_shows_each_side_at_1024() {
        let side = f64::from(1024 - dialog_width(1024)) / 2.0;
        assert!(side >= 184.0, "{side}px a side");
    }

    #[test]
    fn the_dialog_leaves_40px_above_and_below() {
        assert_eq!(dialog_height(900), 820);
        assert_eq!(dialog_height(768), 688);
        assert_eq!(dialog_height(40), 0);
    }

    #[test]
    fn label_move_and_delete_fold_into_more_below_760() {
        assert!(folds_into_more(dialog_width(1024)));
        assert!(!folds_into_more(dialog_width(1280)));
        assert!(!folds_into_more(760));
        assert!(folds_into_more(759));
    }

    #[test]
    fn an_app_colours_column_is_480_or_the_dialog_less_96() {
        assert_eq!(column_width(820, Treatment::AppColours), 480);
        assert_eq!(column_width(dialog_width(1024), Treatment::AppColours), 480);
        assert_eq!(column_width(560, Treatment::AppColours), 464);
    }

    #[test]
    fn the_pane_is_the_dialog_at_its_widest_and_the_list_keeps_404() {
        assert_eq!(pane_width(1024), Some(620));
        assert_eq!(pane_width(1280), Some(820));
        assert_eq!(pane_width(1440), Some(820));
        assert_eq!(pane_width(1920), Some(820));
        // The list takes the rest, never less than its floor.
        for window in [980, 1024, 1180, 1280, 1440, 1920] {
            let pane = pane_width(window).expect("room for a pane");
            assert!(window - pane >= LIST_MIN, "{window}: list {}", window - pane);
            assert!((PANE_MIN..=PANE_MAX).contains(&pane), "{window}: pane {pane}");
        }
    }

    #[test]
    fn a_window_under_980_has_no_pane() {
        assert_eq!(PANE_WINDOW_MIN, 980);
        assert_eq!(pane_width(980), Some(576));
        assert_eq!(pane_width(979), None);
        assert_eq!(pane_width(800), None);
    }

    #[test]
    fn a_pane_holds_the_app_colours_column_whole() {
        for window in [980, 1024, 1280] {
            let pane = pane_width(window).expect("a pane");
            assert_eq!(column_width(pane, Treatment::AppColours), 480, "{window}");
        }
        assert_eq!(column_width(620, Treatment::Paper), 572);
        assert_eq!(column_width(820, Treatment::Paper), 640);
    }

    #[test]
    fn a_pane_opens_where_one_fits_and_the_dialog_everywhere_else() {
        assert_eq!(placement(Placement::Pane, 1280), Placement::Pane);
        assert_eq!(placement(Placement::Pane, 980), Placement::Pane);
        assert_eq!(placement(Placement::Pane, 979), Placement::Dialog);
        assert_eq!(placement(Placement::Dialog, 1920), Placement::Dialog);
        assert_eq!(Placement::default(), Placement::Dialog);
    }

    #[test]
    fn a_pane_under_760_folds_its_action_row_as_the_dialog_does() {
        assert!(folds_into_more(pane_width(1024).expect("a pane")));
        assert!(!folds_into_more(pane_width(1280).expect("a pane")));
    }

    #[test]
    fn a_paper_column_is_640_or_the_dialog_less_48() {
        assert_eq!(column_width(820, Treatment::Paper), 640);
        assert_eq!(column_width(dialog_width(1024), Treatment::Paper), 607);
        assert_eq!(column_width(640, Treatment::Paper), 592);
    }
}
