//! The open-message dialog's geometry, with no toolkit in it (the message
//! dialog redesign, T205-T208 in `specs/007-postio-focus/tasks.md`).
//!
//! Every size here comes from the window or from the body's treatment and
//! from nothing else, so moving through the list with `j`/`k` never resizes
//! the dialog or moves its column: the only inputs are the window's size and
//! [`Treatment`], and neither changes on a step unless the body's treatment
//! does.
//!
//! And which verbs its action row offers a message on its way or stopped
//! ([`send_verbs`], T239), since that is a rule too and not a widget's.

use postio_body::treatment::Treatment;
use postio_core::CommandId;

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
/// list width.
pub const LIST_MIN: i32 = 404;
/// The narrowest reading pane: the app colours column and its inset, so the
/// column is never squeezed.
pub const PANE_MIN: i32 = COLUMN_APP_COLOURS + COLUMN_APP_COLOURS_INSET;
/// The widest reading pane: the dialog at its widest, so a message reads at
/// the same measure beside the list as over it.
pub const PANE_MAX: i32 = DIALOG_MAX;

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
/// narrower than [`LIST_MIN`] and [`PANE_MIN`] together has no room for one.
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

/// Whether `Return` on a row in `state` opens it as the open message, rather
/// than in the composer (T239; screens.md, "Sending states").
///
/// Only a draft still being written opens to be written. One on its way, or
/// one that stopped, opens to be read: Focus has no preview, so `Return` is
/// the only way to look, and opening it in the composer would change it --
/// editing a waiting send takes it off the queue, and editing an
/// unconfirmed one clears what lets Postio find it in Sent (ADR 0021).
pub fn opens_to_read(state: Option<postio_model::DraftState>) -> bool {
    state != Some(postio_model::DraftState::Editing)
}

/// The open message's verbs for a draft whose send is in `state`, in the
/// order its action row draws them, or `None` for mail that is not on its
/// way anywhere -- received mail, and a send the server took -- which keeps
/// the received toolbar.
///
/// `OpenMessage` is Edit: for a draft, opening is writing. `Sending` offers
/// nothing: cancelling is refused once the submission has started, and
/// retrying would risk a second copy (ADR 0021), so offering either would be
/// offering a refusal. Mark as sent is drawn for an unconfirmed send rather
/// than left to the palette, and Edit, since Focus's open message has no
/// composer behind it.
pub fn send_verbs(state: Option<postio_model::DraftState>) -> Option<&'static [CommandId]> {
    use CommandId::{CancelSend, MarkSent, OpenMessage, RetrySend};
    use postio_model::DraftState;
    Some(match state? {
        DraftState::Sent => return None,
        DraftState::Editing => &[OpenMessage],
        DraftState::Queued => &[CancelSend, OpenMessage],
        DraftState::Sending => &[],
        DraftState::Failed => &[RetrySend, OpenMessage],
        DraftState::Unconfirmed => &[RetrySend, MarkSent, OpenMessage],
    })
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
    /// The action card (or the sender block, with no card) to the reader's
    /// notice: the notice is one more block about the message, spaced as
    /// the card is under the sender block.
    pub const CARD_TO_NOTICE: i32 = 12;
    /// The last block above the body -- the notice, the action card, or the
    /// sender block -- to the body, or to its render-mode line.
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

/// One button of a Focus action row: the command it runs, the words on it,
/// and the slug a surface builds its CSS class or accessible id from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verb {
    /// The command it runs, whose key its cap shows.
    pub command: CommandId,
    /// The words on it.
    pub label: &'static str,
    /// What names it: `reply-all`.
    pub slug: &'static str,
}

const fn verb(command: CommandId, label: &'static str, slug: &'static str) -> Verb {
    Verb {
        command,
        label,
        slug,
    }
}

/// The open message's toolbar, in the order screen 04 draws it. Task and
/// Note join in milestone 3 (spec C9).
pub const OPEN_TOOLBAR: &[Verb] = &[
    verb(CommandId::Reply, "Reply", "reply"),
    verb(CommandId::ReplyAll, "Reply all", "reply-all"),
    verb(CommandId::Forward, "Forward", "forward"),
    verb(CommandId::Archive, "Archive", "archive"),
    verb(CommandId::Snooze, "Snooze", "snooze"),
    verb(CommandId::RemindIfNoReply, "Remind", "remind"),
    verb(CommandId::AddLabel, "Label", "label"),
    verb(CommandId::Move, "Move", "move"),
    verb(CommandId::Delete, "Delete", "delete"),
    verb(CommandId::MoreActions, "More", "more"),
];

/// The action row of a message on its way or stopped (T239), in the order
/// it draws them: which of these show is [`send_verbs`]'s answer for the
/// message's state. Edit is `OpenMessage`: for a draft, opening is writing.
pub const SEND_TOOLBAR: &[Verb] = &[
    verb(CommandId::CancelSend, "Cancel send", "cancel-send"),
    verb(CommandId::RetrySend, "Retry send", "retry-send"),
    verb(CommandId::MarkSent, "Mark as sent", "mark-sent"),
    verb(CommandId::OpenMessage, "Edit", "edit"),
];

/// What a narrow dialog folds into More (T206), in the action row's order.
pub const FOLDED: [CommandId; 3] = [CommandId::AddLabel, CommandId::Move, CommandId::Delete];

/// The bulk bar's verbs while anything is selected, in the order screen 01
/// draws them. Task joins them once Obsidian exists (milestone 3, spec C9).
pub const BULK: &[Verb] = &[
    verb(CommandId::Archive, "Archive", "archive"),
    verb(CommandId::Snooze, "Snooze", "snooze"),
    verb(CommandId::ToggleRead, "Mark read", "read"),
    verb(CommandId::DigestRule, "Digest these\u{2026}", "digest"),
    verb(CommandId::AddLabel, "Label", "label"),
    verb(CommandId::Move, "Move", "move"),
    verb(CommandId::Delete, "Delete", "delete"),
];

/// The open message's position line: "Message 3 of 60", then, for a
/// conversation of `messages`, where the dialog is in it -- "thread of 6"
/// at the latest, "2 of 6 in the thread" stepped back -- and, for a draft
/// on its way or stopped, which.
pub fn position_line(
    index: usize,
    total: usize,
    messages: u32,
    thread_at: usize,
    latest: bool,
    send_state: Option<postio_model::DraftState>,
) -> String {
    line(
        "Message", index, total, messages, thread_at, latest, send_state,
    )
}

/// [`position_line`] for a message opened from search: "Result 2 of 8", then
/// where it is in its own conversation.
pub fn result_line(
    index: usize,
    total: usize,
    messages: u32,
    thread_at: usize,
    latest: bool,
) -> String {
    line("Result", index, total, messages, thread_at, latest, None)
}

/// `line` with the account the message is in, when more than one account is
/// enabled: two copies of one conversation in two accounts read alike
/// otherwise, and an Archive or a Reply from here would act in an account
/// nobody can see.
pub fn with_account(line: &str, account: Option<&str>) -> String {
    match account {
        Some(account) => format!("{line} \u{b7} {account}"),
        None => line.to_owned(),
    }
}

fn line(
    noun: &str,
    index: usize,
    total: usize,
    messages: u32,
    thread_at: usize,
    latest: bool,
    send_state: Option<postio_model::DraftState>,
) -> String {
    let mut said = format!("{noun} {} of {}", index + 1, total);
    if messages > 1 {
        if latest {
            said.push_str(&format!(" \u{b7} thread of {messages}"));
        } else {
            said.push_str(&format!(
                " \u{b7} {} of {messages} in the thread",
                thread_at + 1
            ));
        }
    }
    if let Some(state) = send_state.filter(|state| *state != postio_model::DraftState::Sent) {
        said.push_str(&format!(" \u{b7} {}", crate::row::send_state_word(state)));
    }
    said
}

/// The thread chip's sentence, "Latest of 6 in this thread", for a
/// conversation of more than one message.
pub fn thread_chip(messages: u32) -> Option<String> {
    (messages > 1).then(|| format!("Latest of {messages} in this thread"))
}

/// The thread chip for the message at `at` of `messages`: the latest says
/// so, an earlier one says which it is.
pub fn thread_chip_at(messages: u32, at: usize, latest: bool) -> Option<String> {
    if latest {
        return thread_chip(messages);
    }
    (messages > 1).then(|| format!("{} of {messages} in this thread", at + 1))
}

/// What follows the key on the thread chip, for stepping to a newer one.
pub const LATER_MESSAGE: &str = "later message";

/// What follows the key on the thread chip.
pub const EARLIER_MESSAGE: &str = "earlier message";

/// Where stepping `by` messages from `at` lands in a conversation of `len`
/// messages: `None` past either end, or before the conversation is read.
pub fn step_thread(at: usize, len: usize, by: isize) -> Option<usize> {
    at.checked_add_signed(by).filter(|next| *next < len)
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
            assert!(
                window - pane >= LIST_MIN,
                "{window}: list {}",
                window - pane
            );
            assert!(
                (PANE_MIN..=PANE_MAX).contains(&pane),
                "{window}: pane {pane}"
            );
        }
    }

    #[test]
    fn a_window_under_980_has_no_pane() {
        assert_eq!(LIST_MIN + PANE_MIN, 980);
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

    #[test]
    fn a_draft_on_its_way_or_stopped_opens_to_be_read_and_one_being_written_does_not() {
        use postio_model::DraftState;
        // Received mail, and a send the server took, read as mail.
        assert!(opens_to_read(None));
        assert!(opens_to_read(Some(DraftState::Sent)));
        // Looking at these must not change them: editing a waiting send
        // takes it off the queue (ADR 0021).
        for state in [
            DraftState::Queued,
            DraftState::Sending,
            DraftState::Failed,
            DraftState::Unconfirmed,
        ] {
            assert!(
                opens_to_read(Some(state)),
                "{state:?} opens in the composer"
            );
        }
        assert!(!opens_to_read(Some(DraftState::Editing)));
    }

    #[test]
    fn each_send_state_offers_the_verbs_that_settle_it() {
        use postio_core::CommandId::{CancelSend, MarkSent, OpenMessage, RetrySend};
        use postio_model::DraftState;
        assert_eq!(send_verbs(None), None, "received mail has its own verbs");
        assert_eq!(send_verbs(Some(DraftState::Sent)), None);
        assert_eq!(
            send_verbs(Some(DraftState::Queued)),
            Some(&[CancelSend, OpenMessage][..])
        );
        // Cancelling is refused once the submission starts, and retrying
        // would risk a second copy: nothing is offered rather than a refusal.
        assert_eq!(send_verbs(Some(DraftState::Sending)), Some(&[][..]));
        assert_eq!(
            send_verbs(Some(DraftState::Failed)),
            Some(&[RetrySend, OpenMessage][..])
        );
        assert_eq!(
            send_verbs(Some(DraftState::Unconfirmed)),
            Some(&[RetrySend, MarkSent, OpenMessage][..])
        );
        assert_eq!(
            send_verbs(Some(DraftState::Editing)),
            Some(&[OpenMessage][..])
        );
    }

    #[test]
    fn the_position_line_says_where_in_the_list_and_the_thread() {
        assert_eq!(position_line(2, 60, 1, 0, true, None), "Message 3 of 60");
        assert_eq!(
            position_line(2, 60, 6, 5, true, None),
            "Message 3 of 60 \u{b7} thread of 6"
        );
        assert_eq!(
            position_line(2, 60, 6, 1, false, None),
            "Message 3 of 60 \u{b7} 2 of 6 in the thread"
        );
        assert!(
            position_line(0, 1, 1, 0, true, Some(postio_model::DraftState::Queued))
                .starts_with("Message 1 of 1 \u{b7} ")
        );
        assert_eq!(
            position_line(0, 1, 1, 0, true, Some(postio_model::DraftState::Sent)),
            "Message 1 of 1"
        );
    }

    #[test]
    fn a_thread_chip_is_for_threads_and_stepping_stops_at_the_ends() {
        assert_eq!(thread_chip(1), None);
        assert_eq!(
            thread_chip(6).as_deref(),
            Some("Latest of 6 in this thread")
        );
        // Stepped back, the chip names the message shown, not the latest.
        assert_eq!(
            thread_chip_at(7, 5, false).as_deref(),
            Some("6 of 7 in this thread")
        );
        assert_eq!(
            thread_chip_at(7, 6, true).as_deref(),
            Some("Latest of 7 in this thread")
        );
        assert_eq!(thread_chip_at(1, 0, true), None);
        assert_eq!(step_thread(2, 4, -1), Some(1));
        assert_eq!(step_thread(0, 4, -1), None);
        assert_eq!(step_thread(3, 4, 1), None);
        assert_eq!(step_thread(0, 0, 1), None);
    }

    #[test]
    fn a_hit_says_which_result_it_is_and_where_in_its_thread() {
        assert_eq!(result_line(1, 8, 1, 0, true), "Result 2 of 8");
        assert_eq!(
            result_line(1, 8, 3, 0, true),
            "Result 2 of 8 \u{b7} thread of 3"
        );
        assert_eq!(
            result_line(0, 8, 3, 1, false),
            "Result 1 of 8 \u{b7} 2 of 3 in the thread"
        );
    }

    #[test]
    fn the_place_line_names_the_account_only_when_there_is_one_to_name() {
        assert_eq!(
            with_account("Result 2 of 8", Some("home@example.net")),
            "Result 2 of 8 \u{b7} home@example.net"
        );
        assert_eq!(with_account("Result 2 of 8", None), "Result 2 of 8");
    }

    #[test]
    fn what_a_narrow_dialog_folds_is_in_the_toolbar() {
        for command in FOLDED {
            assert!(OPEN_TOOLBAR.iter().any(|verb| verb.command == command));
        }
    }
}
