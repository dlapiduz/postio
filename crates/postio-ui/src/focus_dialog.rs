//! The open-message dialog's geometry, with no toolkit. Sizes come only from the window and the
//! body's `Treatment`, so `j`/`k` never resizes the dialog or moves its column.
//! Also which verbs its action row offers a message on its way or stopped (`send_verbs`).

use postio_body::treatment::Treatment;
use postio_config::paths::Platform;
use postio_core::CommandId;

pub const DIALOG_MIN: i32 = 640;
pub const DIALOG_MAX: i32 = 820;
pub const SIDE_MIN: f64 = 96.0;
pub const SIDE_SHARE: f64 = 0.18;
pub const HEIGHT_INSET: i32 = 80;
pub const MORE_BELOW: i32 = 760;

/// What the dialog keeps either side of the message, so it shares the left edge of the
/// header buttons, action row verbs and thread bar title.
pub const GUTTER: i32 = 20;

pub const KEYCAP_GAP: i32 = 6;
/// A glyph rather than a word, so the pair stays small in the header.
pub const STEP_KEYCAP_GAP: i32 = 4;
pub const LABEL_GAP: i32 = 6;
pub const LABEL_DOT: i32 = 8;
pub const THREAD_GAP: i32 = 8;

pub const SENDER_LABEL_COLUMN: i32 = 44;
pub const SENDER_COLUMN_GAP: i32 = 12;
pub const SENDER_ROW_GAP: i32 = 2;

/// The dialog's width in a window `window` pixels wide:
/// `clamp(640, W - 2 * max(96, 0.18 * W), 820)`, rounded to the pixel.
pub fn dialog_width(window: i32) -> i32 {
    Geometry::LINUX.dialog_width(window)
}

pub fn dialog_height(window: i32) -> i32 {
    (window - HEIGHT_INSET).max(0)
}

pub fn folds_into_more(dialog: i32) -> bool {
    Geometry::LINUX.folds_into_more(dialog)
}

/// The dialog less a `GUTTER` each side, in app colours and on paper alike; everything in the
/// message shares it.
pub fn column_width(dialog: i32, treatment: Treatment) -> i32 {
    Geometry::LINUX.column_width(dialog, treatment)
}

pub const DIGEST_LINUX: (i32, i32) = (980, 820);

/// The free functions above are Linux's numbers. The Mac's message window is narrower and
/// its column wider, being a window beside an undimmed list; its digest is the message
/// window's size so opening a reference never resizes it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    dialog_min: i32,
    dialog_max: i32,
    more_below: i32,
    app_column: i32,
    app_inset: i32,
    paper_column: i32,
    paper_inset: i32,
    digest: Option<(i32, i32)>,
}

impl Geometry {
    pub const LINUX: Geometry = Geometry {
        dialog_min: DIALOG_MIN,
        dialog_max: DIALOG_MAX,
        more_below: MORE_BELOW,
        app_column: DIALOG_MAX,
        app_inset: 2 * GUTTER,
        paper_column: DIALOG_MAX,
        paper_inset: 2 * GUTTER,
        digest: Some(DIGEST_LINUX),
    };

    /// The Mac's: a window of its own; the column is the window less 80, up to a measure no screen
    /// makes unreadable.
    pub const MAC: Geometry = Geometry {
        dialog_min: 640,
        dialog_max: 880,
        more_below: 700,
        app_column: 960,
        app_inset: 80,
        paper_column: 1040,
        paper_inset: 80,
        digest: None,
    };

    pub const fn for_platform(platform: Platform) -> Geometry {
        match platform {
            Platform::Freedesktop => Geometry::LINUX,
            Platform::Apple => Geometry::MAC,
        }
    }

    pub fn dialog_width(&self, window: i32) -> i32 {
        let window = f64::from(window);
        let side = SIDE_MIN.max(SIDE_SHARE * window);
        ((window - 2.0 * side).round() as i32).clamp(self.dialog_min, self.dialog_max)
    }

    pub fn dialog_height(&self, window: i32) -> i32 {
        dialog_height(window)
    }

    pub fn folds_into_more(&self, dialog: i32) -> bool {
        dialog < self.more_below
    }

    pub fn column_width(&self, dialog: i32, treatment: Treatment) -> i32 {
        match treatment {
            Treatment::AppColours => self.app_column.min(dialog - self.app_inset),
            Treatment::Paper => self.paper_column.min(dialog - self.paper_inset),
        }
        .max(0)
    }

    pub fn digest_size(&self, width: i32, height: i32) -> (i32, i32) {
        self.digest
            .unwrap_or_else(|| (self.dialog_width(width), self.dialog_height(height)))
    }
}

pub const LIST_MIN: i32 = 404;
pub const PANE_MIN: i32 = 576;
pub const PANE_MAX: i32 = DIALOG_MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Placement {
    #[default]
    Dialog,
    Pane,
}

/// `None` when that would be under 576: a narrower window has no room for a pane.
pub fn pane_width(window: i32) -> Option<i32> {
    let pane = (window - LIST_MIN).min(PANE_MAX);
    (pane >= PANE_MIN).then_some(pane)
}

pub fn placement(chosen: Placement, window: i32) -> Placement {
    match chosen {
        Placement::Pane if pane_width(window).is_some() => Placement::Pane,
        _ => Placement::Dialog,
    }
}

/// Only a draft still being written opens to be written. One on its way or stopped opens
/// to be read: editing a waiting send takes it off the queue, and editing an unconfirmed
/// one clears what lets Postio find it in Sent (ADR 0021).
pub fn opens_to_read(state: Option<postio_model::DraftState>) -> bool {
    state != Some(postio_model::DraftState::Editing)
}

/// `None` for mail not on its way anywhere (received, or a send the server took), which
/// keeps the received toolbar. `Sending` offers nothing: cancelling is refused once
/// submission starts and retrying risks a second copy (ADR 0021).
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

/// A draft written in another client has no correspondent to answer and cannot be edited
/// here, so Reply, Reply all and Forward are not offered (#1212).
pub fn received_offers(command: CommandId, foreign_draft: bool) -> bool {
    !(foreign_draft
        && matches!(
            command,
            CommandId::Reply | CommandId::ReplyAll | CommandId::Forward
        ))
}

/// The vertical rhythm in pixels (8px grid around a 24px body line). An absent block takes the gap
/// above it with it.
pub mod rhythm {
    pub const TOP: i32 = 28;
    pub const MARKER_TO_SUBJECT: i32 = 12;
    pub const SUBJECT_TO_LABELS: i32 = 10;
    pub const LABELS_TO_SENDER: i32 = 16;
    pub const SENDER_PADDING: i32 = 12;
    pub const SENDER_ROW: i32 = 22;
    pub const SENDER_TO_CARD: i32 = 12;
    pub const CARD_TO_NOTICE: i32 = 12;
    pub const CARD_TO_BODY: i32 = 24;
    pub const MODE_LINE_TO_BODY: i32 = 12;
    pub const BODY_TO_ATTACHMENTS: i32 = 24;
    pub const ATTACHMENTS_RULE_TO_CHIPS: i32 = 16;
    pub const CHIP_GAP: i32 = 8;
    pub const BOTTOM: i32 = 32;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verb {
    pub command: CommandId,
    pub label: &'static str,
    pub slug: &'static str,
}

const fn verb(command: CommandId, label: &'static str, slug: &'static str) -> Verb {
    Verb {
        command,
        label,
        slug,
    }
}

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

pub const SEND_TOOLBAR: &[Verb] = &[
    verb(CommandId::CancelSend, "Cancel send", "cancel-send"),
    verb(CommandId::RetrySend, "Retry send", "retry-send"),
    verb(CommandId::MarkSent, "Mark as sent", "mark-sent"),
    verb(CommandId::OpenMessage, "Edit", "edit"),
];

pub const FOLDED: [CommandId; 3] = [CommandId::AddLabel, CommandId::Move, CommandId::Delete];

pub const BULK: &[Verb] = &[
    verb(CommandId::Archive, "Archive", "archive"),
    verb(CommandId::Snooze, "Snooze", "snooze"),
    verb(CommandId::ToggleRead, "Mark read", "read"),
    verb(CommandId::DigestRule, "Digest these\u{2026}", "digest"),
    verb(CommandId::AddLabel, "Label", "label"),
    verb(CommandId::Move, "Move", "move"),
    verb(CommandId::Delete, "Delete", "delete"),
];

/// The inbox's bulk verbs less "Digest these…", which is about senders, so a selection looks the
/// same wherever made (#2082).
pub const RESULTS_BULK: &[Verb] = &[
    verb(CommandId::Archive, "Archive", "archive"),
    verb(CommandId::Snooze, "Snooze", "snooze"),
    verb(CommandId::ToggleRead, "Mark read", "read"),
    verb(CommandId::AddLabel, "Label", "label"),
    verb(CommandId::Move, "Move", "move"),
    verb(CommandId::Delete, "Delete", "delete"),
];

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

pub fn result_line(
    index: usize,
    total: usize,
    messages: u32,
    thread_at: usize,
    latest: bool,
) -> String {
    line("Result", index, total, messages, thread_at, latest, None)
}

/// With more than one account enabled, names the account: an Archive or Reply would otherwise act
/// in an account nobody can see.
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

pub const ADD_LABEL: &str = "+ Label";

pub const DISMISS: &str = "Dismiss";

pub const FIELD_FROM: &str = "From";
pub const FIELD_TO: &str = "To";
pub const FIELD_CC: &str = "Cc";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dialog_is_as_wide_as_the_window_allows_and_no_wider() {
        assert_eq!(dialog_width(1024), 655);
        assert_eq!(dialog_width(1280), 819);
        assert_eq!(dialog_width(1440), 820);
        assert_eq!(dialog_width(1920), 820);
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
    fn a_column_is_the_dialog_less_its_gutters() {
        for treatment in [Treatment::AppColours, Treatment::Paper] {
            assert_eq!(column_width(820, treatment), 780);
            assert_eq!(column_width(dialog_width(1024), treatment), 615);
            assert_eq!(column_width(640, treatment), 600);
        }
        assert_eq!(column_width(30, Treatment::AppColours), 0);
    }

    #[test]
    fn the_pane_is_the_dialog_at_its_widest_and_the_list_keeps_404() {
        assert_eq!(pane_width(1024), Some(620));
        assert_eq!(pane_width(1280), Some(820));
        assert_eq!(pane_width(1440), Some(820));
        assert_eq!(pane_width(1920), Some(820));
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
    fn a_pane_reads_its_own_width_less_its_gutters() {
        for window in [980, 1024, 1280] {
            let pane = pane_width(window).expect("a pane");
            for treatment in [Treatment::AppColours, Treatment::Paper] {
                assert_eq!(column_width(pane, treatment), pane - 40, "{window}");
            }
        }
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
    fn a_draft_on_its_way_or_stopped_opens_to_be_read_and_one_being_written_does_not() {
        use postio_model::DraftState;
        assert!(opens_to_read(None));
        assert!(opens_to_read(Some(DraftState::Sent)));
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
    fn a_draft_from_another_client_is_offered_no_reply_or_forward() {
        for verb in OPEN_TOOLBAR {
            let answers = matches!(
                verb.command,
                CommandId::Reply | CommandId::ReplyAll | CommandId::Forward
            );
            assert_eq!(
                received_offers(verb.command, true),
                !answers,
                "{:?} on a foreign draft",
                verb.command
            );
            assert!(
                received_offers(verb.command, false),
                "{:?} on received mail",
                verb.command
            );
        }
        assert!(received_offers(CommandId::Delete, true));
        assert!(received_offers(CommandId::Archive, true));
    }

    #[test]
    fn what_a_narrow_dialog_folds_is_in_the_toolbar() {
        for command in FOLDED {
            assert!(OPEN_TOOLBAR.iter().any(|verb| verb.command == command));
        }
    }

    #[test]
    fn linux_geometry_is_the_free_functions() {
        let linux = Geometry::for_platform(Platform::Freedesktop);
        for window in [500, 800, 1024, 1280, 1440, 1920] {
            assert_eq!(linux.dialog_width(window), dialog_width(window));
            assert_eq!(linux.dialog_height(window), dialog_height(window));
            let dialog = linux.dialog_width(window);
            assert_eq!(linux.folds_into_more(dialog), folds_into_more(dialog));
            for treatment in [Treatment::AppColours, Treatment::Paper] {
                assert_eq!(
                    linux.column_width(dialog, treatment),
                    column_width(dialog, treatment)
                );
            }
        }
    }

    #[test]
    fn the_mac_window_is_at_most_880() {
        let mac = Geometry::for_platform(Platform::Apple);
        assert_eq!(mac.dialog_width(1024), 655);
        assert_eq!(mac.dialog_width(1280), 819);
        assert_eq!(mac.dialog_width(1440), 880);
        assert_eq!(mac.dialog_width(1920), 880);
        assert_eq!(mac.dialog_width(800), 640);
        assert_eq!(mac.dialog_height(900), 820);
    }

    #[test]
    fn the_mac_columns_are_the_window_less_80() {
        let mac = Geometry::for_platform(Platform::Apple);
        assert_eq!(mac.column_width(880, Treatment::AppColours), 800);
        assert_eq!(mac.column_width(880, Treatment::Paper), 800);
        assert_eq!(mac.column_width(655, Treatment::AppColours), 575);
        assert_eq!(mac.column_width(655, Treatment::Paper), 575);
        assert_eq!(mac.column_width(600, Treatment::AppColours), 520);
        assert_eq!(mac.column_width(1000, Treatment::AppColours), 920);
        assert_eq!(mac.column_width(1400, Treatment::AppColours), 960);
        assert_eq!(mac.column_width(1400, Treatment::Paper), 1040);
    }

    #[test]
    fn a_wider_mac_window_reads_wider_from_its_default() {
        let mac = Geometry::for_platform(Platform::Apple);
        for main in [1280, 1440, 1920] {
            let window = mac.dialog_width(main);
            for treatment in [Treatment::AppColours, Treatment::Paper] {
                assert!(
                    mac.column_width(window + 80, treatment) > mac.column_width(window, treatment),
                    "{main}: {treatment:?} grows past the default {window}"
                );
            }
        }
    }

    #[test]
    fn the_mac_folds_more_below_700() {
        let mac = Geometry::for_platform(Platform::Apple);
        assert!(mac.folds_into_more(699));
        assert!(!mac.folds_into_more(700));
        assert!(!mac.folds_into_more(720));
    }

    #[test]
    fn the_digest_window_is_the_message_window_on_the_mac_and_fixed_on_linux() {
        let mac = Geometry::for_platform(Platform::Apple);
        assert_eq!(mac.digest_size(1440, 900), (880, 820));
        assert_eq!(mac.digest_size(1024, 768), (655, 688));
        let linux = Geometry::for_platform(Platform::Freedesktop);
        assert_eq!(linux.digest_size(1440, 900), (980, 820));
        assert_eq!(linux.digest_size(1024, 768), (980, 820));
    }
}
