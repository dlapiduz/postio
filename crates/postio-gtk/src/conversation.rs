//! The conversation pane: every message of a thread, stacked (ADR 0015 Q4).
//!
//! The reading pane used to show one message. Opening a conversation now
//! fills it with all of them, oldest first, read ones collapsed to a one-line
//! header and the rest expanded onto the reader Postio already has.
//!
//! # There is one surface, and this is it
//!
//! A drill-in column used to list the same messages in the pane the message
//! list occupies, cast as a table of contents into this one (#1003). It is
//! gone: the list is only ever the list, and a conversation lives here and
//! nowhere else. Nothing has to be kept in step with anything.
//!
//! # Why the policy is pure and lives at the top of this file
//!
//! Where focus opens and how much expands are the two decisions with real
//! consequences — one for whether the pane lands where you stopped reading,
//! the other for whether a thirty-message conversation instantiates thirty
//! `WebKitWebView`s. Both are worth testing without a display, so both are
//! functions over rows rather than behaviour buried in a widget.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use postio_model::ids::MessageId;
use postio_ui::reader::rail::{Effect, NARROW_BELOW, Presentation, Rail, presentation, rows};

use crate::list::Row;

/// How many messages open expanded at most.
///
/// Every expanded message is a `WebKitWebView`, and "expand everything
/// unread" over a conversation nobody has read is one per message — which
/// holds neither the interaction budget nor the memory. Three is what a
/// person reads before they scroll, and scrolling expands more.
pub const EAGER_EXPANSION_CAP: usize = 3;

/// How many message bodies keep a live `WebKitWebView` at once.
///
/// `EAGER_EXPANSION_CAP` bounds how many open *when a conversation opens*.
/// Nothing bounded how many accumulate as it is **scrolled**: `expand` builds
/// a reader the first time each message opens and `collapse` deliberately
/// keeps it, so reading down a thirty-message thread ended with thirty web
/// processes, held until the thread changed. At roughly 50 MB each that is
/// well over a gigabyte for one conversation.
///
/// So the bodies are windowed, the way the message list is windowed over
/// paged SQLite: the ones near the focus stay live, and the furthest is
/// released when a new one opens. An entry whose reader was released stays
/// *expanded* -- scrolling back rebuilds it, which is the same cost the first
/// open paid and is why the window is generous enough that ordinary reading
/// never reaches the edge.
///
/// Six: the three a conversation opens with, the one focus warms ahead, and
/// two of slack so moving up and down a few messages never rebuilds.
pub const LIVE_BODY_CAP: usize = 6;

/// How a conversation orders its messages.
///
/// Was `crate::thread::Order`, when the drill-in column offered `o` to
/// reverse it (#1003). The column is gone and so is the key: a conversation
/// stacks oldest first, the way it was had. The type stays because the
/// ordering itself is still a decision, and one worth being able to state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Order {
    /// Oldest first — how a conversation was actually had, and how the pane
    /// stacks it.
    #[default]
    Oldest,
    /// Newest first, matching the message list.
    Newest,
}

/// The rows a conversation shows, given what is in it and how it is ordered.
///
/// Pure, and tested without a display: the ordering is the part worth being
/// sure about, and it has nothing to do with GTK.
pub fn arrange(rows: &[Row], order: Order, unread_only: bool) -> Vec<Row> {
    let mut rows: Vec<Row> = rows
        .iter()
        .filter(|row| !unread_only || !row.seen)
        .cloned()
        .collect();
    // By id after the timestamp, so two messages that claim the same second —
    // a sender and their own auto-reply, commonly — do not swap places
    // between one redraw and the next.
    rows.sort_by_key(|row| (row.received_at, row.id));
    if order == Order::Newest {
        rows.reverse();
    }
    rows
}

/// How many distinct people are in a conversation.
///
/// By address, folded: one correspondent who has changed their display name
/// mid-thread is still one person, and the header's count is a count of
/// correspondents rather than of `From` headers.
pub fn people(rows: &[Row]) -> usize {
    let mut seen: Vec<String> = rows
        .iter()
        .filter_map(|row| row.from.as_ref())
        .map(|from| from.address.to_lowercase())
        .collect();
    seen.sort();
    seen.dedup();
    seen.len()
}

/// Which message the pane opens on.
///
/// **The most recent, always** — spec FR-015, *"whether or not earlier
/// messages are unread"*, and the maintainer's own words when the spec was
/// clarified: the focus is on the last message when the thread opens.
///
/// # What this supersedes, and the argument it overrides
///
/// This used to be *the first unread*, from ADR 0015, and the reasoning was
/// good: a conversation you open is one you are part way through, and landing
/// at the end means scrolling back past everything you have already read. The
/// case it was strongest on is a wholly unread thread, where the old rule
/// opened at the beginning because reading from the end backwards is not how
/// anyone reads.
///
/// FR-015 overrides it deliberately. Two things changed underneath it. The
/// pane now shows every message's body rather than collapsing the read ones
/// (FR-013), so "landing at the end" no longer means scrolling past collapsed
/// headers to find anything — the thread is one document and the newest is
/// where a reply is aimed. And the rail (#1374) makes the position of the
/// mark a thing you can see and move, so opening somewhere and walking back
/// is a gesture rather than a hunt.
///
/// The overridden argument is recorded rather than deleted because the
/// wholly-unread case is where it still bites, and whoever revisits this
/// should be arguing with a rule rather than rediscovering one. ADR 0015 is
/// amended to match.
///
/// `None` only for an empty conversation, which the pane does not draw.
///
/// `messages` is oldest first, which is the order the pane stacks them in.
pub fn opening_focus(messages: &[Row]) -> Option<usize> {
    messages.len().checked_sub(1)
}

/// How long a one-document pane gathers body arrivals before it redraws.
///
/// Every redraw is a full document teardown and reload, and the bodies of a
/// thread arrive one per turn of the main loop, so rendering on arrival costs
/// one load per message (#1316). This is the window they coalesce in.
const REDRAW_COALESCE: std::time::Duration = std::time::Duration::from_millis(30);

/// How long a redraw will wait for the bodies that have not arrived.
///
/// The pane draws as soon as every message it is showing has a body, and this
/// is the longest it will hold out for the ones that have not — a thread with
/// a body that is not on this machine has to draw, showing the rest, rather
/// than waiting for something that is not coming.
const REDRAW_DEADLINE: std::time::Duration = std::time::Duration::from_millis(400);

/// Which messages the **one-document** pane draws open.
///
/// Separate from [`expanded_on_open`], which bounds the stacked pane, and it
/// has to be: there every open message is a `WebKitWebView`, and the cap is
/// what stops a thirty-message thread from opening thirty processes. Here the
/// whole thread is one view (ADR 0032), so a collapsed message saves no
/// process and almost no memory — #1348 measured one document flat at
/// ~101 MiB whatever the message count.
///
/// **All of them**, which is the whole of FR-013: no message is reduced to a
/// summary row and there is nothing to expand in order to read the
/// conversation. It read `!seen || focused || newest`, which opened a
/// conversation you had already read as one body and five one-line headers —
/// and since the pane opens on the newest (FR-015) that was the common case,
/// hiding exactly what the reader came back for.
///
/// A function over the thread rather than a bare `true`, because the bound
/// that *does* survive lands here: FR-051 says a conversation of a hundred
/// messages or more must not prepare every body at once. That is about
/// preparing bodies, not about hiding the ones already prepared, and it is not
/// implemented yet — when it is, this is where it goes.
///
/// `seen` is per message, in thread order.
pub fn expanded_in_document(seen: &[bool]) -> Vec<bool> {
    vec![true; seen.len()]
}

/// Which messages are expanded when the conversation opens.
///
/// Read messages are collapsed: they are one line, and collapsing them is
/// what makes a long conversation readable at all. The focused message and
/// the unread ones nearest it expand, because that is the part being read —
/// up to `cap`, after which the rest stay one keystroke away rather than
/// costing a web view each.
///
/// The focused message always expands, even when it has been read: focus
/// means "this is the one you are looking at", and looking at a one-line
/// header is not reading.
///
/// # Backwards, since FR-015
///
/// This used to walk *forwards* from the focus, which was right while the
/// pane opened on the first unread: the focus was the start of the run being
/// read and everything after it was the rest of that run. FR-015 moved the
/// opening focus to the newest message (#1385), and forwards from the last
/// message is nothing at all — a six-message thread opened with one body and
/// five collapsed headers.
///
/// So it walks back from the focus instead. The intent is unchanged: the
/// message you landed on, and the ones a reader would want with it. Under the
/// old rule those were ahead of you; under the new one they are behind.
pub fn expanded_on_open(messages: &[Row], focus: usize, cap: usize) -> Vec<bool> {
    if messages.is_empty() {
        return Vec::new();
    }
    let mut expanded = vec![false; messages.len()];
    let mut spent = 0;
    for index in (0..=focus.min(messages.len() - 1)).rev() {
        if spent >= cap {
            break;
        }
        if index == focus || !messages[index].seen {
            expanded[index] = true;
            spent += 1;
        }
    }
    expanded
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use postio_model::ids::MessageId;

    /// A message in the conversation, read or not.
    fn message(id: i64, seen: bool) -> Row {
        Row {
            id: MessageId::new(id),
            thread: None,
            from: None,
            subject: None,
            preview: None,
            received_at: Utc.timestamp_opt(1_770_000_000 + id, 0).single().unwrap(),
            seen,
            flagged: false,
            answered: false,
            draft: false,
            has_attachments: false,
            thread_count: 1,
            participants: Vec::new(),
        }
    }

    // -- where the pane opens ---------------------------------------------

    #[test]
    fn a_conversation_opens_on_its_most_recent_message() {
        // FR-015. Two read, then two unread: the old rule landed on index 2
        // and this one lands at the end regardless.
        let messages = [
            message(1, true),
            message(2, true),
            message(3, false),
            message(4, false),
        ];
        assert_eq!(opening_focus(&messages), Some(3));
    }

    #[test]
    fn a_conversation_read_all_the_way_through_opens_on_its_newest() {
        let messages = [message(1, true), message(2, true), message(3, true)];
        assert_eq!(opening_focus(&messages), Some(2));
    }

    #[test]
    fn a_wholly_unread_conversation_still_opens_at_the_end() {
        // The case the superseded rule was strongest on: it opened at the
        // beginning, because reading a thread from the end backwards is not
        // how anyone reads. FR-015 overrides that on purpose -- see
        // `opening_focus` for the argument, which is recorded rather than
        // deleted.
        let messages = [message(1, false), message(2, false), message(3, false)];
        assert_eq!(opening_focus(&messages), Some(2));
    }

    #[test]
    fn read_state_does_not_move_the_opening_focus_at_all() {
        // The old rule read every message's `seen` flag and could land
        // anywhere. This one reads none of them, so a message marked unread
        // out of order cannot move where the pane opens.
        let unread_early = [message(1, true), message(2, false), message(3, true)];
        let all_read = [message(1, true), message(2, true), message(3, true)];
        assert_eq!(opening_focus(&unread_early), opening_focus(&all_read));
    }

    #[test]
    fn an_empty_conversation_has_nowhere_to_open() {
        assert_eq!(opening_focus(&[]), None);
    }

    #[test]
    fn an_empty_conversation_has_nowhere_to_focus() {
        assert_eq!(opening_focus(&[]), None);
    }

    // -- what the one-document pane opens ---------------------------------

    #[test]
    fn a_conversation_you_have_read_still_shows_every_body() {
        // FR-013: every message's body is visible, no message is reduced to a
        // summary row, and there is nothing to expand in order to read the
        // conversation. The designer's brief says it twice.
        //
        // The common case, not an edge one: the pane opens on the newest
        // message (FR-015), so a thread you have already read is exactly what
        // you come back to.
        let read = [true, true, true, true, true, true];
        assert_eq!(
            expanded_in_document(&read),
            vec![true; 6],
            "a read conversation must not open as one body and five headers"
        );
    }

    #[test]
    fn an_unread_conversation_shows_every_body_too() {
        // The rule reads nothing about a message to decide: FR-013 is not
        // "unread ones open", it is "all of them".
        let mixed = [true, false, true, false];
        assert_eq!(expanded_in_document(&mixed), vec![true; 4]);
    }

    // -- what opens expanded ----------------------------------------------

    #[test]
    fn everything_after_the_focus_stays_collapsed() {
        // Read messages are one line. That is what makes a long conversation
        // readable rather than a wall.
        //
        // *After*, not before: the walk reversed with FR-015 (#1385). The
        // focus is now the newest message rather than the start of the unread
        // run, so the messages worth opening with it are the ones behind it.
        let messages = [
            message(1, true),
            message(2, true),
            message(3, false),
            message(4, false),
        ];
        let expanded = expanded_on_open(&messages, 2, EAGER_EXPANSION_CAP);
        assert_eq!(
            expanded,
            vec![false, false, true, false],
            "the focus opens, the read ones behind it stay shut, and the \
             unread one *after* it is not part of what was landed on"
        );
    }

    #[test]
    fn a_long_unread_conversation_does_not_expand_all_of_it() {
        // The cost question. Thirty unread messages is thirty web views, and
        // the cap is what stops the pane from opening one per message.
        let messages: Vec<Row> = (0..30).map(|id| message(id, false)).collect();
        // At the newest, which is where the pane now opens (FR-015).
        let expanded = expanded_on_open(&messages, 29, EAGER_EXPANSION_CAP);

        assert_eq!(
            expanded.iter().filter(|open| **open).count(),
            EAGER_EXPANSION_CAP,
            "opening a conversation must not cost a web view per message"
        );
        assert!(
            expanded[30 - EAGER_EXPANSION_CAP..]
                .iter()
                .all(|open| *open),
            "the ones that do expand are the focus and the ones behind it"
        );
    }

    #[test]
    fn the_focused_message_expands_even_when_it_has_been_read() {
        // Focus means "this is the one you are looking at", and looking at a
        // one-line header is not reading. This is the fully-read case: focus
        // lands on the newest and it has to open.
        let messages = [message(1, true), message(2, true), message(3, true)];
        let expanded = expanded_on_open(&messages, 2, EAGER_EXPANSION_CAP);
        assert_eq!(expanded, vec![false, false, true]);
    }

    #[test]
    fn a_read_message_behind_the_focus_stays_collapsed() {
        // Only the focus is expanded unconditionally; behind it, unread is
        // what earns a web view. A read message in the middle of the run does
        // not stop the walk -- it is skipped, and the unread one past it
        // still opens.
        let messages = [message(1, false), message(2, true), message(3, false)];
        let expanded = expanded_on_open(&messages, 2, EAGER_EXPANSION_CAP);
        assert_eq!(expanded, vec![true, false, true]);
    }

    #[test]
    fn a_cap_of_one_opens_only_what_is_focused() {
        // The fallback shape ADR 0015 names if the stack proves too
        // expensive: one reader, the rest collapsed.
        let messages: Vec<Row> = (0..5).map(|id| message(id, false)).collect();
        let expanded = expanded_on_open(&messages, 1, 1);
        assert_eq!(expanded, vec![false, true, false, false, false]);
    }

    #[test]
    fn an_empty_conversation_expands_nothing() {
        assert!(expanded_on_open(&[], 0, EAGER_EXPANSION_CAP).is_empty());
    }
}

/// What the pane asks for when a message needs a body: the reader for that
/// message.
///
/// A callback rather than a constructor argument, because building a
/// [`crate::reader::Reader`] needs a blob source and an allow-list path that
/// only the window has — and because it is what makes the cost testable. A
/// test hands back a bare reader and counts the calls; the application hands
/// back the hardened one.
///
/// Hands back the [`Reader`](crate::reader::Reader) itself, not just its
/// widget: the pane keeps it, so an arrival for an already-expanded entry
/// can be re-drawn into the reader already on screen ([`reader_for`],
/// #739) instead of tearing the whole entry down to rebuild one.
///
/// [`reader_for`]: ConversationView::reader_for
pub type ReaderFactory = Box<dyn Fn(MessageId) -> Option<crate::reader::Reader>>;

/// The three verbs a single message in a stack offers.
///
/// Reply is the primary: it is what the pane is for. Archive and delete are
/// deliberately absent — every verb but these three is the conversation's
/// (ADR 0015 Q4), and a delete button on every message in a stack is how
/// people delete the wrong one.
pub const MESSAGE_ACTIONS: [crate::widgets::Action; 3] = [
    crate::widgets::Action::new(
        postio_core::CommandId::Reply,
        "Reply",
        "conversation-action-reply",
    )
    .primary(),
    crate::widgets::Action::new(
        postio_core::CommandId::ReplyAll,
        "Reply all",
        "conversation-action-reply-all",
    ),
    crate::widgets::Action::new(
        postio_core::CommandId::Forward,
        "Forward",
        "conversation-action-forward",
    ),
];

/// Who to ask to run a `CommandId` one of this pane's bars carries.
/// A whole [`Command`](postio_core::Command), not a
/// [`CommandId`](postio_core::CommandId): which message a verb aims at is
/// the substance for a pane that holds several.
///
/// `Archive thread` needs no target — the pane is the thread — but
/// `Continue editing` does: a draft inside a longer thread is not the row
/// the list cursor is on, so an untargeted open would resume whatever the
/// list is pointing at (#1212).
type CommandHandler = Box<dyn Fn(postio_core::Command)>;

/// What a message offers when it is the only one there is.
///
/// #1173: a one-message thread drew both bars, so `Reply` appeared twice
/// with `e` printed on each. The question it raised — *is a single message a
/// degenerate conversation with a footer, or its own view with its own bar?*
/// — is answered by `Design/screens/19-threaded-view.png`, which draws
/// one bar at the foot of a single message reading `Reply · Reply all ·
/// Forward · Archive`, and by `17-conversation-view.png`, which draws the
/// footer's conversation verbs only on a thread that has a conversation in
/// it.
///
/// So: its own view, and `Archive` joins the three. That answers the
/// objection the issue raised against simply hiding the footer — archive is
/// the verb people reach for most, and hiding the footer alone would have
/// taken it away. `ArchiveThread` rather than `Archive` because a
/// one-message thread *is* the message: the two verbs mean the same act
/// here, and using the thread's own command keeps the button on the same
/// path `A` already takes.
pub const LONE_MESSAGE_ACTIONS: [crate::widgets::Action; 4] = [
    crate::widgets::Action::new(
        postio_core::CommandId::Reply,
        "Reply",
        "conversation-action-reply",
    )
    .primary(),
    crate::widgets::Action::new(
        postio_core::CommandId::ReplyAll,
        "Reply all",
        "conversation-action-reply-all",
    ),
    crate::widgets::Action::new(
        postio_core::CommandId::Forward,
        "Forward",
        "conversation-action-forward",
    ),
    crate::widgets::Action::new(
        postio_core::CommandId::ArchiveThread,
        "Archive",
        "conversation-action-archive",
    ),
];

/// What a draft offers: the one verb that is true of it.
///
/// #1212. A draft is a message you wrote and never sent, so reply, reply-all
/// and forward are the correspondent's verbs and it has no correspondent yet
/// — a `Reply` here would quote your own unsent text back at you. The verb
/// that is right was already reachable and unannounced: activating the row
/// resumes the composer on the draft, cancelling a queued send first.
///
/// `CommandId::OpenMessage` is that command, so the button and `Return`
/// cannot come to mean different things and nothing new enters the registry.
pub const DRAFT_ACTIONS: [crate::widgets::Action; 1] = [crate::widgets::Action::new(
    postio_core::CommandId::OpenMessage,
    "Continue editing",
    "conversation-action-continue",
)
.primary()];

/// A draft that is the whole thread — the Drafts folder, which is where
/// nearly every draft is seen.
///
/// Archive joins it for the same reason it joins [`LONE_MESSAGE_ACTIONS`]:
/// the footer stands down at n=1, and archive would otherwise have no control
/// in the pane at all.
pub const LONE_DRAFT_ACTIONS: [crate::widgets::Action; 2] = [
    crate::widgets::Action::new(
        postio_core::CommandId::OpenMessage,
        "Continue editing",
        "conversation-action-continue",
    )
    .primary(),
    crate::widgets::Action::new(
        postio_core::CommandId::ArchiveThread,
        "Archive",
        "conversation-action-archive",
    ),
];

/// The thread's own verbs, drawn in the footer.
///
/// Reply, reply-all and forward are per *message* and live inside each entry
/// ([`MESSAGE_ACTIONS`]); everything else is the thread's (ADR 0015 Q4). So
/// one verb, and a verb is drawn on the thing it acts on.
///
/// **There is no `Reply to conversation` here** (#1173). It ran
/// `CommandId::Reply` aimed at the *focused* message — the same command and
/// the same key as the bar drawn on that message — under a label naming a
/// scope Postio does not have: you reply to a message, never to a thread. A
/// pinned control drawn a pane away from the message it will answer is the
/// mistake ADR 0015 Q4 gives as its own reason for making the reply verbs
/// per-message, *"answering the wrong message of a conversation is a real and
/// common mistake"*, and it is worse than the bar it duplicated because it
/// does not show you which message you are about to answer. At n=1 that read
/// as two identical buttons stacked on each other; at n>1 it is the same
/// defect with a pane's height between them, which is why it survived three
/// issues.
///
/// Nothing becomes mouse-only or keyboard-only by its going. The pane scrolls
/// the focused message into view and expands it (ADR 0015 Q4 §Focus), so the
/// bar drawn on it is on screen wherever focus is, and `e`, the palette and
/// the message's own menu all still reach reply.
///
/// Not [`primary`](crate::widgets::Action::primary): the pane's primary verb
/// is Reply, and it is drawn on the message. A filled archive button as the
/// most prominent control in the reading pane inverts that.
///
/// `Design/screens/17-conversation-view.png` still draws the footer with a
/// `Reply to conversation`. The canvas is authority on spacing, colour and
/// proportion; this is a behaviour call, and the screen as drawn puts two
/// controls on one command and one key.
pub const CONVERSATION_ACTIONS: [crate::widgets::Action; 1] = [crate::widgets::Action::new(
    postio_core::CommandId::ArchiveThread,
    "Archive thread",
    "conversation-footer-archive",
)];

/// The verbs the **one-document** pane offers, in canvas order (#1349).
///
/// A second set rather than an extension of [`CONVERSATION_ACTIONS`], because
/// the two panes need different ones and the difference is not cosmetic. The
/// stacked pane draws a bar per message, so a conversation bar carrying
/// `Reply` would put it on screen twice with `e` on each (#1173) -- which is
/// why that set holds one verb. The one-document pane has no per-message bar
/// at all: its message chrome is HTML inside the document, so this is the only
/// bar there is, and a single verb left a pane you could not reply to with the
/// mouse.
///
/// Scoped per spec FR-008: reply, reply all and forward act on the latest
/// message; archive acts on the whole conversation, which is why the fourth is
/// `ArchiveThread` and not `Archive`. Labels and commands for the first three
/// come from [`postio_ui::reader::header::ReaderAction`], so the two frontends
/// name them identically.
pub const DOCUMENT_ACTIONS: [crate::widgets::Action; 4] = [
    crate::widgets::Action::new(
        postio_ui::reader::header::ReaderAction::Reply.command(),
        postio_ui::reader::header::ReaderAction::Reply.title(),
        "conversation-document-reply",
    )
    .primary(),
    // Icon-only from here, per canvas screen 30. Four labelled buttons
    // crowded the subject down to "Tuesday w..." at the narrow breakpoint --
    // the header's most important line, truncated by its own controls.
    //
    // Platform icon names rather than drawn assets: `PRODUCT.md` §19 asks for
    // an app that reads as native, and these three ship with it. Archive is
    // the exception and always was -- `postio-archive-symbolic` is vendored in
    // `data/icons/` because the platform has none worth using.
    crate::widgets::Action::new(
        postio_ui::reader::header::ReaderAction::ReplyAll.command(),
        postio_ui::reader::header::ReaderAction::ReplyAll.title(),
        "conversation-document-reply-all",
    )
    .icon("mail-reply-all-symbolic"),
    crate::widgets::Action::new(
        postio_ui::reader::header::ReaderAction::Forward.command(),
        postio_ui::reader::header::ReaderAction::Forward.title(),
        "conversation-document-forward",
    )
    .icon("mail-forward-symbolic"),
    crate::widgets::Action::new(
        postio_core::CommandId::ArchiveThread,
        "Archive thread",
        "conversation-document-archive",
    )
    .icon("postio-archive-symbolic"),
];

/// The pane's own header: what conversation this is, and how much of it.
///
/// Subject at the largest size in the pane — this is the one place the
/// conversation is named, and before the drill-in column went there were two
/// places and they could disagree. Under it one metadata line, ellipsised
/// rather than wrapped, and the way to open everything at once.
pub struct Header {
    root: gtk::Box,
    subject: gtk::Label,
    meta: gtk::Label,
    expand_all: std::rc::Rc<crate::widgets::KeycapButton>,
    /// The conversation's verbs, at row one's trailing edge (canvas screen 30).
    actions: std::rc::Rc<crate::widgets::ActionBar>,
    /// Up to three participant chips, at row two's leading edge.
    avatars: gtk::Box,
    /// Row two's trailing note: `latest · all 6`.
    ///
    /// Required rather than decorative. The bar's verbs are scoped two
    /// different ways — reply to the latest message, archive to the whole
    /// conversation — and the brief is explicit that this "is not obvious, so
    /// the scoping note in row 2 is required".
    scoping: gtk::Label,
    /// `3/6 ⌄` at row two's trailing edge, below the ladder's floor.
    ///
    /// A `MenuButton` rather than a button and a popover wired together: it
    /// brings the open-on-click, close-on-`Esc` and close-on-click-outside
    /// behaviour the brief asks for, and an accessible role that says the
    /// control opens something.
    counter: gtk::MenuButton,
    /// What the counter opens. Holds the rail itself while the window is too
    /// narrow to draw a column.
    index: gtk::Popover,
    /// The metadata line in both its lengths: with the participants, and
    /// without them.
    ///
    /// Two strings rather than a recomposition, because the second is only
    /// ever the first minus one part and rebuilding it would mean keeping the
    /// senders and the dates around to rebuild it *from*.
    meta_text: std::cell::RefCell<(String, String)>,
    /// Whether this header belongs to a pane drawing the thread as one
    /// document, where nothing is collapsed and so nothing can be expanded.
    one_document: std::cell::Cell<bool>,
    /// Whether there is a scoping note to show when there is room.
    has_scoping: std::cell::Cell<bool>,
    /// Whether there are participant chips to show when there is room.
    has_participants: std::cell::Cell<bool>,
    /// Whether the metadata line is currently the short one.
    ///
    /// Remembered rather than applied once, because `set_conversation` writes
    /// the label too: with only a setter, opening a conversation put the long
    /// line back and the ladder never ran again to correct it. Every test
    /// passed -- they set the width *after* opening, which the application
    /// does in the other order.
    compact: std::cell::Cell<bool>,
}

impl Header {
    /// Build the header, empty.
    pub fn new() -> Self {
        // Two rows, each with its own trailing element, rather than one row
        // of [titles | button]: canvas screen 30 puts the action cluster
        // beside the *subject* and the scoping note beside the *meta line*,
        // which a single trailing column spanning both rows cannot express.
        let root = gtk::Box::new(gtk::Orientation::Vertical, 2);
        root.add_css_class("conversation-header");
        root.set_accessible_role(gtk::AccessibleRole::Group);

        let first = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let second = gtk::Box::new(gtk::Orientation::Horizontal, 8);

        let subject = gtk::Label::new(None);
        subject.set_xalign(0.0);
        subject.set_wrap(false);
        subject.set_ellipsize(gtk::pango::EllipsizeMode::End);
        subject.add_css_class("conversation-subject");
        subject.set_hexpand(true);
        first.append(&subject);

        // Canvas screens 28 and 30: overlapping initials before the names.
        // Ahead of the meta line rather than beside it, because the chips
        // identify the same people the line then names.
        let avatars = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        avatars.add_css_class("conversation-participants");
        avatars.set_visible(false);
        second.append(&avatars);

        let meta = gtk::Label::new(None);
        meta.set_xalign(0.0);
        // One line, ellipsised. The participants are the unbounded part —
        // a twelve-person thread must not grow the header.
        meta.set_wrap(false);
        meta.set_ellipsize(gtk::pango::EllipsizeMode::End);
        meta.add_css_class("conversation-meta");
        meta.set_hexpand(true);
        second.append(&meta);

        let expand_all = std::rc::Rc::new(crate::widgets::KeycapButton::new(
            Some(postio_core::CommandId::ExpandAll),
            "Expand all",
            "conversation-expand-all",
            false,
        ));
        crate::widgets::KeycapButton::arm(&expand_all);
        first.append(&expand_all.widget());

        let actions =
            crate::widgets::ActionBar::new(&DOCUMENT_ACTIONS, "conversation-header-actions");
        actions.set_visible(false);
        first.append(&actions.widget());

        let scoping = gtk::Label::new(None);
        scoping.set_wrap(false);
        scoping.add_css_class("conversation-scoping");
        scoping.set_visible(false);
        second.append(&scoping);

        let index = gtk::Popover::new();
        index.add_css_class("conversation-index");
        let counter = gtk::MenuButton::new();
        counter.add_css_class("conversation-counter");
        counter.set_popover(Some(&index));
        counter.set_visible(false);
        second.append(&counter);

        root.append(&first);
        root.append(&second);

        Header {
            root,
            subject,
            meta,
            expand_all,
            actions,
            scoping,
            avatars,
            counter,
            index,
            meta_text: std::cell::RefCell::new((String::new(), String::new())),
            compact: std::cell::Cell::new(false),
            one_document: std::cell::Cell::new(false),
            has_scoping: std::cell::Cell::new(false),
            has_participants: std::cell::Cell::new(false),
        }
    }

    /// The conversation's verbs. Wired and shown by the pane that owns them.
    /// Show or hide the `3/6` counter, and say what it counts.
    ///
    /// `None` hides it: at wider widths the rail itself says the position, and
    /// a conversation with no rail has no position to state (FR-045).
    pub fn set_counter(&self, position: Option<(usize, usize)>) {
        match position {
            Some((at, total)) => {
                self.counter.set_label(&format!("{at}/{total}"));
                self.counter
                    .set_tooltip_text(Some(&format!("Message {at} of {total} — open the index")));
                self.counter
                    .update_property(&[gtk::accessible::Property::Label(&format!(
                        "Message {at} of {total}, open the index"
                    ))]);
                self.counter.set_visible(true);
            }
            None => self.counter.set_visible(false),
        }
    }

    /// Drop the participants from the metadata line, or put them back.
    ///
    /// Their names are the unbounded part of the line and the first thing to
    /// go when the header is short of room. The avatar chips stay: three
    /// initials say who is here in a width a name cannot.
    pub fn set_compact(&self, compact: bool) {
        self.compact.set(compact);
        self.draw_meta();
        // Screen 29's narrow header carries the count, the dates and the
        // counter, and nothing else. The names went first and the *dates*
        // then ellipsised to a single character -- there is only so much room
        // and four things were asking for it. The avatars and the scoping
        // note stand down together, which is the drawing.
        self.avatars
            .set_visible(!compact && self.has_participants.get());
        self.scoping.set_visible(!compact && self.has_scoping.get());
    }

    /// Put whichever metadata line is current on screen.
    fn draw_meta(&self) {
        let text = self.meta_text.borrow();
        self.meta
            .set_label(if self.compact.get() { &text.1 } else { &text.0 });
    }

    /// Say whether the pane draws the thread as one document.
    pub fn set_one_document(&self, one_document: bool) {
        self.one_document.set(one_document);
    }

    /// Whether the scoping note is drawn. Test-facing.
    pub fn scoping_visible(&self) -> bool {
        self.scoping.is_visible()
    }

    /// Whether the participant chips are drawn. Test-facing.
    pub fn participants_visible(&self) -> bool {
        self.avatars.is_visible()
    }

    /// The counter, so a test can read back what a person would see.
    pub fn counter(&self) -> &gtk::MenuButton {
        &self.counter
    }

    /// The popover the counter opens.
    pub fn index(&self) -> &gtk::Popover {
        &self.index
    }

    pub fn actions(&self) -> std::rc::Rc<crate::widgets::ActionBar> {
        std::rc::Rc::clone(&self.actions)
    }

    /// The faces on row two, at most three of them.
    ///
    /// The same limit as `conversation::participants` uses for the names
    /// beside them, so the chips and the line agree about who is shown rather
    /// than one of them eliding a person the other kept.
    ///
    /// Distinct by address: a person who wrote five times is one face. The
    /// chips are decoration for the line that follows and are hidden from
    /// assistive technology, which reads the names instead — two letters
    /// announced as "T V" is noise where "Tessa Vaughn" is already there.
    fn set_participants(&self, senders: &[postio_model::address::EmailAddress]) {
        while let Some(child) = self.avatars.first_child() {
            self.avatars.remove(&child);
        }

        let mut seen: Vec<&str> = Vec::new();
        for sender in senders {
            if seen.len() >= postio_ui::conversation::NAMES_SHOWN {
                break;
            }
            if seen.contains(&sender.address.as_str()) {
                continue;
            }
            seen.push(&sender.address);

            let chip = gtk::Label::new(Some(&postio_ui::row::initials(Some(sender))));
            chip.add_css_class("conversation-participant");
            chip.set_accessible_role(gtk::AccessibleRole::Presentation);
            self.avatars.append(&chip);
        }
        self.has_participants.set(!seen.is_empty());
        self.avatars
            .set_visible(!seen.is_empty() && !self.compact.get());
    }

    /// Say what each verb will act on, in words and in the scoping note.
    ///
    /// Spec FR-008a. The bar's verbs are scoped two ways — reply, reply all
    /// and forward to the latest message, archive to the whole conversation —
    /// and a user reading the third message of six who presses Reply gets a
    /// reply to the sixth. The only thing that makes that safe is the
    /// interface saying so before they press it.
    ///
    /// The wording comes from `postio_ui`, so the macOS bar says the same
    /// thing rather than inventing its own phrasing.
    fn describe_actions(&self, messages: usize) {
        use postio_ui::reader::header::ReaderAction;

        for (verb, action) in ReaderAction::ALL.iter().zip(DOCUMENT_ACTIONS.iter()) {
            let Some(button) = self.actions.button(action.command) else {
                continue;
            };
            let described = verb.describe(messages);
            button.widget().set_tooltip_text(Some(&described));
            button
                .widget()
                .update_property(&[gtk::accessible::Property::Label(&described)]);
        }

        // `latest · all 6` — the terse form the brief asks for, and only where
        // there is a distinction to draw. One message is not a conversation
        // and saying so would imply others exist.
        self.has_scoping.set(messages > 1);
        if messages > 1 {
            self.scoping.set_visible(!self.compact.get());
            self.scoping.set_label(&format!("latest · all {messages}"));
        } else {
            self.scoping.set_visible(false);
        }
    }

    /// The widget to pin above the stack.
    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    /// Name the conversation on screen.
    ///
    /// `rows` is the whole conversation, oldest first — the header counts and
    /// spans it rather than being told, so it cannot disagree with the stack
    /// below it about how many messages there are.
    pub fn set_conversation(&self, rows: &[Row], now: chrono::DateTime<chrono::Local>) {
        if rows.is_empty() {
            self.root.set_visible(false);
            return;
        }
        self.root.set_visible(true);
        // Nothing to expand in a thread of one: it opens expanded, so the
        // button would be offered with nothing left to do (#1173). The same
        // n=1 surface as the footer standing down.
        //
        // Nothing to expand in the one-document pane either, at any length --
        // FR-013 is that there is nothing for the user to expand in order to
        // read the conversation, and since #1389 every body is drawn open. A
        // control that would do nothing is worse than no control: it says
        // there is something you have not seen.
        //
        // Not tied to the *narrow* header as well, though screen 29 does not
        // draw it there: that is a second decision about a crowded row, and
        // tying it here hid the control in the stacked pane at every width
        // the test window happened to be.
        self.expand_all
            .widget()
            .set_visible(rows.len() > 1 && !self.one_document.get());
        self.describe_actions(rows.len());
        self.subject.set_label(
            rows.iter()
                .find_map(|row| row.subject.as_deref())
                .filter(|subject| !subject.trim().is_empty())
                .unwrap_or("(no subject)"),
        );

        let senders: Vec<postio_model::address::EmailAddress> =
            rows.iter().filter_map(|row| row.from.clone()).collect();
        let first = rows
            .iter()
            .map(|row| row.received_at)
            .min()
            .unwrap_or_else(chrono::Utc::now);
        let last = rows
            .iter()
            .map(|row| row.received_at)
            .max()
            .unwrap_or_else(chrono::Utc::now);

        let count = postio_ui::conversation::message_count(rows.len());
        let span = postio_ui::conversation::date_span(
            first.with_timezone(&chrono::Local),
            last.with_timezone(&chrono::Local),
            now,
        );
        let join = |parts: &[String]| {
            parts
                .iter()
                .filter(|part| !part.is_empty())
                .cloned()
                .collect::<Vec<_>>()
                .join(" · ")
        };
        let meta = join(&[
            count.clone(),
            postio_ui::conversation::participants(&senders),
            span.clone(),
        ]);
        // Screen 29's narrow header is `6 messages · 22–25 Aug` and nothing
        // else: below the ladder's floor the counter takes the trailing edge,
        // and with the names still there the line ellipsised to a single
        // letter -- which says less than leaving it out.
        let compact = join(&[count, span]);
        self.meta_text.replace((meta.clone(), compact));
        self.set_participants(&senders);
        self.draw_meta();
        // The line ellipsises, so the whole of it has to reach a screen
        // reader some other way.
        self.root
            .update_property(&[gtk::accessible::Property::Description(&meta)]);
    }

    /// What the metadata line currently says. Test-facing.
    pub fn meta(&self) -> String {
        self.meta.label().to_string()
    }

    /// What the subject line currently says. Test-facing.
    pub fn subject(&self) -> String {
        self.subject.label().to_string()
    }

    /// Whether `Expand all` is on offer. Test-facing.
    pub fn offers_expand_all(&self) -> bool {
        self.expand_all.widget().is_visible()
    }

    /// Re-cap `Expand all` from the live keymap.
    pub fn set_keymap(&self, keymap: &postio_core::Keymap) {
        self.expand_all
            .set_key(keymap.binding(postio_core::CommandId::ExpandAll));
    }

    /// Called when `Expand all` is pressed.
    pub fn connect_expand_all(&self, handler: impl Fn() + 'static) {
        self.expand_all.connect_clicked(handler);
    }

    /// Press `Expand all` without a pointer, for a test.
    pub fn press_expand_all(&self) {
        self.expand_all.press();
    }
}

impl Default for Header {
    fn default() -> Self {
        Self::new()
    }
}

type MessageHandler = Box<dyn Fn(MessageId)>;
type ReplyHandler = Box<dyn Fn(MessageId, bool)>;

mod imp {
    use super::*;
    use std::cell::{Cell, RefCell};

    pub struct ConversationView {
        /// The whole pane: header, scroller, footer, stacked.
        pub(super) root: gtk::Box,
        /// The conversation's own name and shape, pinned above the stack so
        /// it survives scrolling. Nothing else on screen says what you are
        /// reading once the drill-in column's header went (#1004).
        pub(super) header: super::Header,
        /// `Archive thread`, pinned below — where the thread's own verbs
        /// live, as against the per-message ones inside each entry (#1006).
        pub(super) footer: std::rc::Rc<crate::widgets::ActionBar>,
        /// Who to ask to run a `CommandId` one of this pane's bars carries.
        ///
        /// The footer had none, so its buttons did nothing when pressed —
        /// `ActionBar` runs its handler list and the list was empty. Found
        /// while giving the lone message's own `Archive` a path (#1173).
        pub(super) on_command: RefCell<Vec<super::CommandHandler>>,
        /// The account's own addresses, folded, so a row can say whether the
        /// message is one the user sent (#1241). Nothing else in this crate
        /// knows them; `postio-app` hands them over from the same account
        /// row the composer's identity picker is built from.
        pub(super) own_addresses: RefCell<Vec<String>>,
        /// The scroller the stack lives in. A conversation is longer than the
        /// pane, and jumping to a message means scrolling this.
        pub(super) scroller: gtk::ScrolledWindow,
        /// The row of the pane the body scrolls in: the scroller, and beside
        /// it the rail.
        ///
        /// The rail is a sibling of the scroller rather than of the whole
        /// pane, because the header spans the full width above both and the
        /// rail must not scroll with the body it indexes.
        pub(super) body: gtk::Box,
        /// The conversation rail (#1374), or nothing drawn when the ladder
        /// says this window is too narrow for one.
        pub(super) rail: crate::reader::rail::RailColumn,
        /// Whether `⇧R` has put the rail away. Per window, not per thread
        /// (FR-047), which is why it lives on the pane and not beside the
        /// messages.
        pub(super) rail_hidden: Cell<bool>,
        /// The window width the ladder was last told about.
        ///
        /// Remembered, because `set_window_width` applying the step and not
        /// keeping it meant every later `open` re-derived the width from the
        /// window itself and threw the answer away. Breakpoints fire on
        /// crossing a line, so between crossings this is the only record of
        /// which side we are on.
        pub(super) rail_width: Cell<Option<i32>>,
        /// The stack itself, one [`Entry`] per message, oldest first.
        pub(super) stack: gtk::Box,
        pub(super) entries: RefCell<Vec<Entry>>,
        /// The current message — the one both this pane and the drill-in
        /// column are showing, and the one a per-message verb aims at.
        pub(super) focused: Cell<Option<MessageId>>,
        pub(super) factory: RefCell<Option<ReaderFactory>>,
        /// A reader built and started before anything asks for one.
        ///
        /// A `WebView` spawns its web process on the first *load*, not when
        /// it is built, so a reader made at the moment a message expands
        /// makes the person wait for a process to start, relocate and paint
        /// -- and it composites black until it has. That is the flicker
        /// moving between messages (#1216).
        ///
        /// So expansion takes this one, already warm, and a replacement is
        /// warmed on the idle after. One spare, not a pool: the cost is a
        /// resident web process, and one is enough to cover the gap between
        /// two keystrokes.
        ///
        /// Held **with the message it was built for**. The factory binds a
        /// reader to a message -- `fill_reader` starts the read for it -- so
        /// a spare is not interchangeable, and the pane warms the one that is
        /// about to be wanted: the next message down the stack, which is the
        /// gesture this is for. Expanding anything else falls back to
        /// building one, exactly as before.
        pub(super) spare: RefCell<Option<(MessageId, crate::reader::Reader)>>,
        pub(super) on_reply: RefCell<Vec<ReplyHandler>>,
        pub(super) on_forward: RefCell<Vec<MessageHandler>>,
        pub(super) on_focus: RefCell<Vec<MessageHandler>>,
        pub(super) on_dwell: RefCell<Vec<MessageHandler>>,
        /// The dwell timer in flight. Cancelled rather than replaced when
        /// focus moves: a `glib` timeout that merely loses its handle still
        /// fires, and one that fires late marks a message read that was
        /// passed over rather than looked at.
        pub(super) dwell: RefCell<Option<glib::SourceId>>,
        pub(super) dwell_delay: Cell<std::time::Duration>,
        /// The dividers currently standing in for folded runs, in stack
        /// order. Rebuilt whenever anything folds or unfolds, because
        /// collapsing a message between two runs joins them (#1005).
        /// Which entries a divider stands in for is not kept: `refold`
        /// recomputes every run from the collapsed flags each time, so a
        /// stored range would be a second source of truth that could only
        /// ever disagree with the first.
        pub(super) dividers: RefCell<Vec<gtk::Box>>,
        /// Whether this pane renders the thread as one document in one
        /// `WebView` (ADR 0032, #1316) rather than as a stack of readers.
        ///
        /// The stacked pane builds a `Reader` per expanded message, and
        /// WebKitGTK runs a process per *view*, so a thirty-message thread
        /// ends with thirty of them. Switched by `POSTIO_ONE_DOCUMENT` so
        /// both shapes can be compared in one binary, on the same mail.
        pub(super) one_document: Cell<bool>,
        /// The single reader, in one-document mode. Built once and kept: not
        /// rebuilding it per message is the whole point.
        pub(super) document_reader: RefCell<Option<crate::reader::Reader>>,
        /// The thread currently open, in the order it is drawn.
        pub(super) thread_rows: RefCell<Vec<Row>>,
        /// The bodies that have arrived so far, by message.
        pub(super) thread_bodies:
            RefCell<std::collections::HashMap<MessageId, postio_model::MessageBody>>,
        /// Whether a redraw is already queued for the next idle turn.
        ///
        /// Bodies arrive one at a time and every one of them changes the
        /// document, so without this a ten-message thread would hand WebKit
        /// ten documents on the way to the one it wants.
        pub(super) redraw_queued: Cell<bool>,
        /// When the pane stops waiting for bodies that have not arrived and
        /// draws what it has. See [`REDRAW_DEADLINE`].
        pub(super) redraw_deadline: Cell<Option<std::time::Instant>>,
        /// How many documents this pane has actually handed over. See
        /// [`super::ConversationView::thread_renders`].
        pub(super) thread_renders: Cell<u32>,
        /// Which thread the pane is holding, so reopening the same one keeps
        /// what it has instead of refetching and re-deciding it.
        pub(super) thread_id: Cell<Option<postio_model::ids::ThreadId>>,
        /// Whether each message is drawn open.
        ///
        /// Decided once per message and then kept, because expansion is the
        /// reader's state and not a function of the model. Recomputing it on
        /// every redraw meant a message folded shut under the person reading
        /// it the moment resting on it marked it read (#1316).
        pub(super) expanded_in_document: RefCell<std::collections::HashMap<MessageId, bool>>,
        /// Told when a thread opens in one-document mode, so whoever owns the
        /// store fetches every body rather than waiting for an expansion that
        /// never comes.
        #[allow(clippy::type_complexity)]
        pub(super) on_thread_opened: RefCell<Vec<Box<dyn Fn(Vec<Row>)>>>,
    }

    /// One message in the stack: its header, and the body when it has one.
    pub struct Entry {
        pub message: MessageId,
        pub row: Row,
        /// The collapsed line, which is always drawn. Expanding adds a body
        /// beneath it rather than replacing it, so the header stays as the
        /// thing you click and the thing focus is drawn on.
        pub header: crate::thread_row::ThreadRowView,
        /// Where the reader goes. Empty until this message is expanded, which
        /// is what keeps a thirty-message conversation from costing thirty
        /// web views.
        pub body: gtk::Box,
        /// The reader built for this entry, once it is expanded — kept
        /// alongside `body` so an arrival for this message can be re-drawn
        /// into the same `WebView` rather than rebuilding it (`reader_for`,
        /// #739). `None` until `expand` fills it, and never cleared by
        /// `collapse`: the widget itself stays parked in `body`, hidden, for
        /// the same reason.
        pub reader: RefCell<Option<crate::reader::Reader>>,
        pub actions: std::rc::Rc<crate::widgets::ActionBar>,
        pub expanded: Cell<bool>,
        /// Whether a folded run this message was in has been shown.
        ///
        /// Distinct from `expanded`: `Show` on a divider puts the individual
        /// *collapsed* rows back, it does not open them (#1005). Without this
        /// the next `refold` would immediately fold the run again, because
        /// the messages are still collapsed and still consecutive.
        pub shown: Cell<bool>,
        /// The box holding header, actions and body — what the stack owns.
        pub container: gtk::Box,
    }

    impl Entry {
        /// The widget the stack holds for this message.
        pub fn container(&self) -> gtk::Box {
            self.container.clone()
        }
    }

    impl Default for ConversationView {
        fn default() -> Self {
            ConversationView {
                root: gtk::Box::new(gtk::Orientation::Vertical, 0),
                header: super::Header::new(),
                footer: crate::widgets::ActionBar::new(
                    &super::CONVERSATION_ACTIONS,
                    "conversation-footer",
                ),
                scroller: gtk::ScrolledWindow::default(),
                body: gtk::Box::new(gtk::Orientation::Horizontal, 0),
                rail: crate::reader::rail::RailColumn::new(),
                rail_hidden: Cell::new(false),
                rail_width: Cell::new(None),
                spare: RefCell::new(None),
                stack: gtk::Box::new(gtk::Orientation::Vertical, 0),
                entries: RefCell::new(Vec::new()),
                focused: Cell::new(None),
                factory: RefCell::new(None),
                on_reply: RefCell::new(Vec::new()),
                on_forward: RefCell::new(Vec::new()),
                on_focus: RefCell::new(Vec::new()),
                on_dwell: RefCell::new(Vec::new()),
                dwell: RefCell::new(None),
                dwell_delay: Cell::new(crate::list_view::DWELL_TO_READ),
                dividers: RefCell::new(Vec::new()),
                on_command: RefCell::new(Vec::new()),
                own_addresses: RefCell::new(Vec::new()),
                one_document: Cell::new(false),
                document_reader: RefCell::new(None),
                thread_rows: RefCell::new(Vec::new()),
                thread_bodies: RefCell::new(std::collections::HashMap::new()),
                redraw_queued: Cell::new(false),
                redraw_deadline: Cell::new(None),
                thread_renders: Cell::new(0),
                thread_id: Cell::new(None),
                expanded_in_document: RefCell::new(std::collections::HashMap::new()),
                on_thread_opened: RefCell::new(Vec::new()),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ConversationView {
        const NAME: &'static str = "PostioConversationView";
        type Type = super::ConversationView;
        type ParentType = gtk::Widget;

        fn class_init(class: &mut Self::Class) {
            class.set_layout_manager_type::<gtk::BinLayout>();
            class.set_css_name("conversation");
        }
    }

    impl ObjectImpl for ConversationView {
        fn constructed(&self) {
            self.parent_constructed();
            let view = self.obj();
            self.scroller.set_child(Some(&self.stack));
            self.scroller.set_hexpand(true);
            self.scroller.set_vexpand(true);
            // A scroll area is a tab stop, and an unnamed one announces
            // nothing — see docs/engineering-notes.md.
            self.scroller
                .update_property(&[gtk::accessible::Property::Label("Conversation")]);

            // Header and footer are *outside* the scroller, deliberately: a
            // header that scrolled away would leave a long conversation with
            // nothing on screen saying what it is, and a footer that scrolled
            // away would put the conversation's own verbs somewhere you have
            // to go looking for.
            self.root.append(&self.header.widget());
            self.body.append(&self.scroller);
            self.body.append(self.rail.widget());
            self.body.set_vexpand(true);
            self.root.append(&self.body);
            self.root.append(&self.footer.widget());
            // Nothing until a conversation says how many messages there are:
            // the ladder's floor is a single message, and a rail drawn before
            // the thread is known would flash on for every one of them.
            self.rail.widget().set_visible(false);
            self.rail.connect_hide({
                let view = view.clone();
                move || view.toggle_rail()
            });
            self.rail.connect_activated({
                let view = view.clone();
                move |index| {
                    view.focus_at(index);
                }
            });
            self.footer.set_visible(false);
            self.footer.connect_command({
                let view = view.clone();
                move |command| view.emit_command(command)
            });
            // The one-document pane's verbs live in the header (canvas screen
            // 30), so the bar is the header's and the pane only wires it.
            self.header.actions().connect_command({
                let view = view.clone();
                move |command| {
                    // FR-008: the conversation bar's reply, reply-all and
                    // forward act on the **most recent message**, and archive
                    // on the whole conversation. Passing the command straight
                    // through let the application resolve it against whatever
                    // was focused, so the bar answered the focused message --
                    // while the header beside it said `latest · all 6`, which
                    // FR-008a requires precisely because the scoping is not
                    // self-evident. The interface was telling the truth and
                    // the button was not (#1394).
                    let kind = match command.id() {
                        postio_core::CommandId::Reply => Some(ReplyKind::Reply),
                        postio_core::CommandId::ReplyAll => Some(ReplyKind::ReplyAll),
                        postio_core::CommandId::Forward => Some(ReplyKind::Forward),
                        _ => None,
                    };
                    match kind.zip(view.latest_message()) {
                        Some((kind, latest)) => view.emit_action(latest, kind),
                        // Archive and the rest are conversation-level already,
                        // and the application scopes them to the thread.
                        None => view.emit_command(command),
                    }
                }
            });
            self.root.set_parent(&*view);
        }

        fn dispose(&self) {
            self.root.unparent();
        }
    }

    impl WidgetImpl for ConversationView {}
}

glib::wrapper! {
    /// Every message of one conversation, stacked.
    pub struct ConversationView(ObjectSubclass<imp::ConversationView>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for ConversationView {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl ConversationView {
    /// An empty pane.
    pub fn new() -> Self {
        let pane: Self = Self::default();
        // The header's button and `O` are the same verb; wiring the button to
        // the pane rather than emitting a command keeps them one path.
        pane.imp().header.connect_expand_all({
            let pane = pane.clone();
            move || pane.expand_all()
        });
        pane
    }

    /// This pane as a widget, for mounting into the reading pane.
    pub fn widget(&self) -> gtk::Widget {
        self.clone().upcast()
    }

    /// How the pane builds the body of an expanded message.
    ///
    /// Set once, by whoever owns the reader's blob source. Until it is set the
    /// pane draws headers and no bodies, which is what a pane with nothing
    /// wired to it should look like rather than a crash.
    ///
    /// `None` is the answer for a factory whose own owner is gone -- the
    /// composition root's factory closes over a weak window (#1072) rather
    /// than a strong one, and an upgrade that fails means there is nothing
    /// left to build a reader for.
    pub fn set_reader_factory(
        &self,
        factory: impl Fn(MessageId) -> Option<crate::reader::Reader> + 'static,
    ) {
        *self.imp().factory.borrow_mut() = Some(Box::new(factory));
    }

    /// Put a conversation in the pane, oldest first.
    ///
    /// Focus lands on the most recent message — see [`opening_focus`] — and
    /// [`expanded_on_open`] decides how much opens with it.
    /// Render this thread as one document in one `WebView` (ADR 0032, #1316).
    ///
    /// Off by default. `postio-app` turns it on from `POSTIO_ONE_DOCUMENT`, so
    /// the stacked pane and this one can be compared in the same binary on the
    /// same mail. Set before the first [`open`](Self::open).
    pub fn set_one_document(&self, one_document: bool) {
        self.imp().one_document.set(one_document);
        self.imp().header.set_one_document(one_document);
    }

    /// Whether this pane is in one-document mode.
    pub fn is_one_document(&self) -> bool {
        self.imp().one_document.get()
    }

    /// Called when a thread opens in one-document mode, with every row in it.
    ///
    /// The stacked pane fetches a body when a message is expanded; one
    /// document has no expansions to hang that on, so whoever owns the store
    /// is told the whole thread at once and fills it through
    /// [`set_thread_body`](Self::set_thread_body).
    pub fn connect_thread_opened(&self, handler: impl Fn(Vec<Row>) + 'static) {
        self.imp()
            .on_thread_opened
            .borrow_mut()
            .push(Box::new(handler));
    }

    /// A body arrived for one message of the open thread.
    ///
    /// Redrawn on the next idle turn rather than here: bodies arrive one at a
    /// time and each changes the document, so drawing on arrival would hand
    /// WebKit one document per message on the way to the one it wants.
    pub fn set_thread_body(&self, message: MessageId, body: postio_model::MessageBody) {
        let imp = self.imp();
        if !imp.one_document.get() {
            return;
        }
        imp.thread_bodies.borrow_mut().insert(message, body);
        self.queue_document_redraw();
    }

    /// Whether every message the pane is showing now has a body.
    fn thread_is_whole(&self) -> bool {
        let imp = self.imp();
        let rows = imp.thread_rows.borrow();
        let bodies = imp.thread_bodies.borrow();
        !rows.is_empty() && rows.iter().all(|row| bodies.contains_key(&row.id))
    }

    fn queue_document_redraw(&self) {
        let imp = self.imp();
        // Everything is here: draw it now rather than waiting out a timer for
        // arrivals that cannot come.
        if self.thread_is_whole() {
            imp.redraw_deadline.set(None);
            imp.redraw_queued.set(false);
            self.redraw_document();
            return;
        }
        if imp.redraw_queued.replace(true) {
            return;
        }
        if imp.redraw_deadline.get().is_none() {
            imp.redraw_deadline
                .set(Some(std::time::Instant::now() + REDRAW_DEADLINE));
        }
        // A short delay, not an idle turn. Every render is a full document
        // teardown and reload -- JavaScript is off, so there is no
        // incremental path -- and the bodies of a thread arrive one per main
        // loop turn, so an idle callback coalesced nothing: a four-message
        // thread cost four loads and a thirty-message one would cost thirty.
        //
        // Long enough to gather a burst of arrivals, short enough not to be
        // felt: what a person waits for is the first paint, and the store
        // reads this is coalescing are already slower than this.
        glib::timeout_add_local_once(REDRAW_COALESCE, {
            let pane = self.clone();
            move || {
                let imp = pane.imp();
                imp.redraw_queued.set(false);
                let overdue = imp
                    .redraw_deadline
                    .get()
                    .is_none_or(|deadline| std::time::Instant::now() >= deadline);
                if pane.thread_is_whole() || overdue {
                    imp.redraw_deadline.set(None);
                    pane.redraw_document();
                } else {
                    // Still filling, and there is time left: wait for the rest
                    // rather than spending a whole document on a partial one.
                    pane.queue_document_redraw();
                }
            }
        });
    }

    /// Compose every message that has a body into one document and hand it
    /// over. A message still waiting for its body is drawn collapsed with its
    /// preview, which is what it would show in the stack too.
    fn redraw_document(&self) {
        let imp = self.imp();
        let Some(reader) = imp.document_reader.borrow().clone() else {
            return;
        };
        let rows = imp.thread_rows.borrow().clone();
        if rows.is_empty() {
            return;
        }
        let bodies = imp.thread_bodies.borrow();
        let newest = rows.last().map(|row| row.id);
        // Every message, per FR-013 -- see `expanded_in_document`. The map is
        // still per message and still keeps what it decided, because folding
        // one shut by hand is a gesture the reader can make and the model
        // changing underneath must not undo it.
        let expanded: std::collections::HashSet<MessageId> = {
            let seen: Vec<bool> = rows.iter().map(|row| row.seen).collect();
            let open = expanded_in_document(&seen);
            let mut decided = imp.expanded_in_document.borrow_mut();
            for (row, open) in rows.iter().zip(open) {
                decided.entry(row.id).or_insert(open);
            }
            decided
                .iter()
                .filter(|(_, open)| **open)
                .map(|(id, _)| *id)
                .collect()
        };
        let now = chrono::Local::now();
        let messages: Vec<crate::reader::view::ThreadMessage> = rows
            .iter()
            .map(|row| {
                let from = row.from.as_ref();
                crate::reader::view::ThreadMessage {
                    scope: row.id.get().to_string(),
                    sender: from
                        .and_then(|from| from.name.clone())
                        .or_else(|| from.map(|from| from.address.clone()))
                        .unwrap_or_else(|| "Unknown sender".to_string()),
                    address: from.map(|from| from.address.clone()).unwrap_or_default(),
                    when: postio_ui::row::timestamp(row.received_at, now),
                    preview: row.preview.clone().unwrap_or_default(),
                    expanded: bodies.contains_key(&row.id) && expanded.contains(&row.id),
                    latest: newest == Some(row.id) && rows.len() > 1,
                    body: bodies.get(&row.id).cloned().unwrap_or_default(),
                }
            })
            .collect();
        drop(bodies);
        // A load that changes nothing is still a full teardown and reload,
        // and the reader's scroll position goes with it. Several things queue
        // a redraw -- a body arriving, a thread reopening, a timer armed
        // before either -- and they overlap, so the guard belongs here rather
        // than at each of them. This is #749's fourth cause, in a new pane.
        if reader.would_render_thread(&messages) {
            imp.thread_renders.set(imp.thread_renders.get() + 1);
            reader.render_thread(&messages);
        }
    }

    /// How many conversation documents this pane has handed to WebKit.
    ///
    /// Every one is a full teardown and reload — JavaScript is off, so there
    /// is no incremental path, and the scroll position goes with it. A thread
    /// fill should cost a small number of these, not one per message.
    ///
    /// Counted here rather than read off `Reader::loads`, which counts every
    /// load that reader ever did — including the ones that were not this
    /// pane's, and which made this number look four when the pane had drawn
    /// twice.
    pub fn thread_renders(&self) -> u32 {
        self.imp().thread_renders.get()
    }

    /// The document the one-document pane last handed to WebKit.
    ///
    /// The last artifact before the engine, which is where a wiring mistake
    /// shows: a pane that opened but never composed, or composed without the
    /// bodies. `None` when the pane is stacked, or before anything opened.
    pub fn thread_document(&self) -> Option<String> {
        self.imp()
            .document_reader
            .borrow()
            .as_ref()
            .map(|reader| reader.test_document())
    }

    /// Open `messages` as one document rather than as a stack.
    fn open_as_document(&self, messages: Vec<Row>) {
        let imp = self.imp();
        // A thread opens twice: once with the row the list had, and again
        // with the whole conversation once it is read. Those are the same
        // thread, and so is a re-read after a flag changed -- clearing on
        // each of them would refetch every body and decide every expansion
        // again, which is what folded a message shut under the reader.
        let opening = messages.first().and_then(|row| row.thread);
        if imp.thread_id.get() != opening {
            imp.thread_id.set(opening);
            imp.thread_bodies.borrow_mut().clear();
            imp.expanded_in_document.borrow_mut().clear();
        }
        imp.thread_rows.replace(messages.clone());

        if imp.document_reader.borrow().is_none() {
            // Through the same factory the stack uses, so this reader is
            // built and wired exactly as any other -- scheme handlers,
            // hardening, allow list.
            let built = messages
                .first()
                .and_then(|row| imp.factory.borrow().as_ref().map(|make| make(row.id)))
                .flatten();
            if let Some(reader) = built {
                // The reader's own single-message chrome is the chrome ADR
                // 0032 moves into the document: an empty header band above a
                // thread would be the stack's furniture with none of its use.
                reader.header().widget().set_visible(false);
                reader.set_actions_visible(false);
                // A message's own verbs, from inside the document (#1365).
                // The scope is the message id in decimal, which is what
                // `ThreadMessage` puts in the URI; the mapping back lives
                // here for the same reason it does for the stacked pane's
                // per-message bars -- the reader knows scopes, the
                // conversation knows messages.
                reader.connect_message_action({
                    let view = self.downgrade();
                    move |scope, verb| {
                        let Some(view) = view.upgrade() else {
                            return;
                        };
                        let Ok(id) = scope.parse::<i64>() else {
                            glib::g_warning!(
                                "postio",
                                "a message verb named a scope that is not a message id: {scope}"
                            );
                            return;
                        };
                        view.emit_action(
                            MessageId::new(id),
                            match verb {
                                crate::reader::view::MessageVerb::Reply => ReplyKind::Reply,
                                crate::reader::view::MessageVerb::Forward => ReplyKind::Forward,
                            },
                        );
                    }
                });
                let widget = reader.widget();
                widget.set_vexpand(true);
                imp.stack.append(&widget);
                imp.document_reader.replace(Some(reader));
            }
        }

        for handler in imp.on_thread_opened.borrow().iter() {
            handler(messages.clone());
        }
        self.queue_document_redraw();
    }

    /// What one message contributes to the rail.
    ///
    /// The sender the same way the document names it, so the rail and the
    /// message headers cannot disagree about who wrote something.
    fn rail_sender(row: &Row) -> String {
        let from = row.from.as_ref();
        from.and_then(|from| from.name.clone())
            .or_else(|| from.map(|from| from.address.clone()))
            .unwrap_or_else(|| "Unknown sender".to_string())
    }

    /// Give the rail the thread, and take the ladder's step for it.
    ///
    /// Lengths are all `None` for now. The count is stored (#1329, migration
    /// `0015_body_line_count.sql`) but is not carried on `list::Row`, so
    /// nothing here can see it yet -- and `rail::rows` draws a row with no
    /// number rather than a row with a wrong one, which is the right failure.
    fn fill_rail(&self, messages: &[Row]) {
        let imp = self.imp();
        let senders: Vec<String> = messages.iter().map(Self::rail_sender).collect();
        let initials: Vec<String> = messages
            .iter()
            .map(|row| postio_ui::row::initials(row.from.as_ref()))
            .collect();
        let now = chrono::Local::now();
        let whens: Vec<String> = messages
            .iter()
            .map(|row| postio_ui::row::timestamp(row.received_at, now))
            .collect();
        let lengths: Vec<Option<u32>> = vec![None; messages.len()];
        imp.rail
            .show_thread(&rows(&senders, &initials, &whens, &lengths));
        self.apply_rail_ladder(self.window_width(), messages.len());
    }

    pub fn open(&self, messages: Vec<Row>) {
        let imp = self.imp();
        self.fill_rail(&messages);
        for entry in imp.entries.borrow().iter() {
            imp.stack.remove(&entry.container());
        }
        if imp.one_document.get() {
            imp.entries.borrow_mut().clear();
            for divider in imp.dividers.borrow_mut().drain(..) {
                imp.stack.remove(&divider);
            }
            // FR-015: the most recent, through the same rule the stacked
            // pane uses. This said `messages.first()` -- the *oldest* -- so
            // the two panes gave opposite answers to one requirement.
            imp.focused
                .set(opening_focus(&messages).map(|index| messages[index].id));
            // The stacked pane marks the rail from `focus_message`, which
            // this path never calls -- it sets the focus itself and hands the
            // whole thread to one document. Without this the rail marked
            // nothing in the pane the rail was designed for.
            imp.thread_rows.replace(messages.clone());
            imp.rail.set_marked(self.focused_index());
            imp.header.set_conversation(&messages, chrono::Local::now());
            imp.header.actions().set_visible(true);
            // Always, whatever the length -- unlike the stacked pane below.
            //
            // There, a single message stands its footer down because the
            // message's own bar carries the verbs and drawing both put
            // `Reply` on screen twice with `e` on each (#1173). Here the
            // per-message chrome is HTML inside the document, so there is no
            // second bar to collide with and nothing to fall back on: the
            // footer standing down left a pane with no way to reply with the
            // mouse at all, which is #1259 (#1349).
            imp.footer.set_visible(false);
            self.open_as_document(messages);
            return;
        }
        imp.entries.borrow_mut().clear();
        for divider in imp.dividers.borrow_mut().drain(..) {
            imp.stack.remove(&divider);
        }
        imp.focused.set(None);
        self.cancel_dwell();

        imp.header.set_conversation(&messages, chrono::Local::now());
        // A footer for a *conversation*. One message is not one, and drawing
        // both bars put `Reply` on screen twice with `e` on each (#1173);
        // the lone message carries `Archive` in its own bar instead, so
        // nothing is lost by the footer standing down.
        imp.footer.set_visible(messages.len() > 1);
        imp.header.actions().set_visible(false);

        let focus = opening_focus(&messages);
        let expanded = match focus {
            Some(focus) => expanded_on_open(&messages, focus, EAGER_EXPANSION_CAP),
            None => Vec::new(),
        };

        for (index, row) in messages.iter().enumerate() {
            // Numbered from one. Nothing draws the number since #1003 took
            // the column away, but a screen reader still says "3 of 8", which
            // is the position a sighted reader gets from the stack itself.
            let entry = self.build_entry(row, index as u32 + 1, messages.len() == 1);
            imp.stack.append(&entry.container());
            imp.entries.borrow_mut().push(entry);
            if expanded.get(index).copied().unwrap_or(false) {
                self.expand(row.id);
            }
        }
        if let Some(focus) = focus.and_then(|index| messages.get(index)) {
            self.focus_message(focus.id);
        }
        self.refold();
    }

    /// Fold every run of three-or-more collapsed messages into one divider,
    /// and unfold any that no longer qualifies.
    ///
    /// Recomputed from scratch rather than patched, because the runs are not
    /// independent: collapsing the message between two runs joins them into
    /// one, and expanding inside a run splits it in two. Patching that
    /// correctly is harder than recounting, and the count is over a slice of
    /// bools — [`postio_ui::conversation::collapsed_runs`] — which is cheap
    /// and provable without a display.
    fn refold(&self) {
        let imp = self.imp();
        for divider in imp.dividers.borrow_mut().drain(..) {
            imp.stack.remove(&divider);
        }

        let collapsed: Vec<bool> = imp
            .entries
            .borrow()
            .iter()
            .map(|entry| !entry.expanded.get() && !entry.shown.get())
            .collect();
        let runs = postio_ui::conversation::collapsed_runs(
            &collapsed,
            postio_ui::conversation::RUN_MINIMUM,
        );

        for range in runs {
            let (senders, first) = {
                let entries = imp.entries.borrow();
                let senders: Vec<postio_model::address::EmailAddress> = entries[range.clone()]
                    .iter()
                    .filter_map(|entry| entry.row.from.clone())
                    .collect();
                (senders, entries[range.start].container())
            };
            let divider = self.build_divider(range.clone(), &senders);
            imp.stack
                .insert_child_after(&divider, first.prev_sibling().as_ref());
            for entry in imp.entries.borrow()[range.clone()].iter() {
                entry.container().set_visible(false);
            }
            imp.dividers.borrow_mut().push(divider);
        }
    }

    /// The hairline row standing in for one folded run.
    fn build_divider(
        &self,
        range: std::ops::Range<usize>,
        senders: &[postio_model::address::EmailAddress],
    ) -> gtk::Box {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("conversation-divider");

        let rule = || {
            let line = gtk::Separator::new(gtk::Orientation::Horizontal);
            line.set_hexpand(true);
            line.set_valign(gtk::Align::Center);
            line
        };
        row.append(&rule());

        let label = gtk::Label::new(Some(&postio_ui::conversation::run_summary(
            range.len(),
            senders,
        )));
        label.add_css_class("conversation-divider-label");
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        row.append(&label);

        // `Show`, not `Expand`: it puts the individual collapsed rows back,
        // it does not open them. One gesture, one step -- expanding five
        // messages because you wanted to see who they were from is not what
        // anybody asked for.
        let show = gtk::Button::with_label("Show");
        show.add_css_class("flat");
        show.add_css_class("postio-ghost");
        show.add_css_class("conversation-divider-show");
        let view = self.clone();
        let range_for_show = range.clone();
        show.connect_clicked(move |_| view.show_run(range_for_show.clone()));
        row.append(&show);
        row.append(&rule());

        row.update_property(&[gtk::accessible::Property::Label(&format!(
            "{}, collapsed",
            postio_ui::conversation::run_summary(range.len(), senders)
        ))]);

        row
    }

    /// Put a folded run's individual rows back, still collapsed.
    pub fn show_run(&self, range: std::ops::Range<usize>) {
        {
            let imp = self.imp();
            let entries = imp.entries.borrow();
            for entry in entries.get(range.clone()).unwrap_or(&[]) {
                entry.shown.set(true);
                entry.container().set_visible(true);
            }
        }
        self.refold();
    }

    /// Open every collapsed message, folded runs included — `O`.
    pub fn expand_all(&self) {
        let messages: Vec<MessageId> = self
            .imp()
            .entries
            .borrow()
            .iter()
            .map(|entry| entry.message)
            .collect();
        for message in messages {
            self.expand(message);
        }
        for entry in self.imp().entries.borrow().iter() {
            entry.shown.set(true);
            entry.container().set_visible(true);
        }
        self.refold();
    }

    /// How many messages are showing their bodies.
    ///
    /// Asks the entries rather than counting readers: a message can be
    /// expanded before its body arrives, and "how much is open" is the
    /// question `Expand all` and the fold rules are about.
    pub fn expanded_count(&self) -> usize {
        self.imp()
            .entries
            .borrow()
            .iter()
            .filter(|entry| entry.expanded.get())
            .count()
    }

    /// The pane's header, for a test that wants to read what it says.
    pub fn header(&self) -> &Header {
        &self.imp().header
    }

    /// The conversation's own action bar.
    /// Whichever action bar is currently on screen, if either is.
    ///
    /// The two panes carry different bars ([`CONVERSATION_ACTIONS`] and
    /// [`DOCUMENT_ACTIONS`]), and a test asking "can this be replied to" wants
    /// the one a person can see rather than the one a given branch built.
    pub fn visible_actions(&self) -> Option<std::rc::Rc<crate::widgets::ActionBar>> {
        let imp = self.imp();
        if imp.header.actions().is_visible() {
            Some(imp.header.actions())
        } else if imp.footer.is_visible() {
            Some(std::rc::Rc::clone(&imp.footer))
        } else {
            None
        }
    }

    pub fn footer(&self) -> std::rc::Rc<crate::widgets::ActionBar> {
        std::rc::Rc::clone(&self.imp().footer)
    }

    /// What the folded-run dividers currently say, in stack order.
    /// Test-facing.
    pub fn divider_labels(&self) -> Vec<String> {
        self.imp()
            .dividers
            .borrow()
            .iter()
            .filter_map(|divider| {
                divider
                    .first_child()
                    .and_then(|child| child.next_sibling())
                    .and_downcast::<gtk::Label>()
                    .map(|label| label.label().to_string())
            })
            .collect()
    }

    /// How many messages the pane is holding.
    pub fn len(&self) -> usize {
        if self.imp().one_document.get() {
            return self.imp().thread_rows.borrow().len();
        }
        self.imp().entries.borrow().len()
    }

    /// Whether the pane is holding nothing.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The current message: what both this pane and the drill-in column show,
    /// and what a per-message verb aims at.
    pub fn focused(&self) -> Option<MessageId> {
        self.imp().focused.get()
    }

    /// The current message's row.
    ///
    /// What the dwell timer marks read and what a reply is composed against,
    /// so the caller does not have to keep a second copy of the conversation
    /// and keep it in step.
    pub fn focused_row(&self) -> Option<Row> {
        let focused = self.imp().focused.get()?;
        self.imp()
            .entries
            .borrow()
            .iter()
            .find(|entry| entry.message == focused)
            .map(|entry| entry.row.clone())
    }

    /// Every message in the pane, in the order it is stacked.
    ///
    /// The drill-in column indexes this, and the two must agree on the order
    /// or jumping lands somewhere other than where it pointed.
    pub fn rows(&self) -> Vec<Row> {
        // One document has no entries, and the rows are still the pane's
        // answer to what it is showing -- `Window::conversation_on` asks this
        // to decide whether a thread read is still wanted, so a pane that
        // answered nothing would drop the rest of every conversation it
        // opened and keep the one row the list gave it.
        if self.imp().one_document.get() {
            return self.imp().thread_rows.borrow().clone();
        }
        self.imp()
            .entries
            .borrow()
            .iter()
            .map(|entry| entry.row.clone())
            .collect()
    }

    /// Whether the focused message is drawn as focused.
    ///
    /// Asks the widget rather than the field, because "there is a focused
    /// message" and "you can see which one" are different claims and only the
    /// second one matters to somebody about to press reply.
    pub fn is_focus_drawn(&self) -> bool {
        let focused = self.imp().focused.get();
        self.imp()
            .entries
            .borrow()
            .iter()
            .any(|entry| Some(entry.message) == focused && entry.header.is_selected())
    }

    /// Whether `message` is showing its body.
    /// Press a verb in `message`'s own bar, without a pointer. Test-facing.
    pub fn press_entry_command(&self, message: MessageId, command: postio_core::CommandId) {
        if let Some(entry) = self
            .imp()
            .entries
            .borrow()
            .iter()
            .find(|entry| entry.message == message)
        {
            entry.actions.press(command);
        }
    }

    pub fn is_expanded(&self, message: MessageId) -> bool {
        self.imp()
            .entries
            .borrow()
            .iter()
            .any(|entry| entry.message == message && entry.expanded.get())
    }

    /// Make `message` the current one: expand it, draw it as focused, and
    /// scroll it into view.
    ///
    /// This is what the drill-in column's cursor calls. Landing on a
    /// collapsed header would be a dead end — you went there to read it — so
    /// jumping expands.
    pub fn focus_message(&self, message: MessageId) {
        // The one-document pane has no entries -- the whole thread is one
        // `WebView` (ADR 0032) -- so everything below this, which is about
        // expanding an entry and scrolling to its widget, has nothing to work
        // on. It used to fall out of the guard beneath and return, which made
        // the rail's rows, `J` and `K` all inert in the pane the rail exists
        // for (#1386).
        //
        // Here the same gesture is a scroll of the one document to the
        // message's own anchor.
        if self.imp().one_document.get() {
            self.focus_in_document(message);
            return;
        }
        if !self
            .imp()
            .entries
            .borrow()
            .iter()
            .any(|e| e.message == message)
        {
            return;
        }
        // Focus first, then expand. `expand` releases the bodies furthest
        // from the focus to stay under `LIVE_BODY_CAP`, and with the *old*
        // focus still set it measured distance from where the reader used to
        // be -- so opening a message at the far end of a long thread released
        // the body it had just built, and scrolling back showed an expanded
        // entry with nothing in it.
        self.imp().focused.set(Some(message));
        self.expand(message);
        // After `focused` is set, and not in `expand`: opening a conversation
        // expands several messages before focus exists, and "the next one"
        // has no answer until it does. One warm per navigation, which is the
        // gesture -- read this one, move down.
        self.warm_the_next();
        for entry in self.imp().entries.borrow().iter() {
            entry.header.set_selected(entry.message == message);
        }
        self.scroll_to(message);
        // The rail follows the focus rather than being set beside it, so
        // there is no path that moves one without the other.
        self.imp().rail.set_marked(self.focused_index());
        self.start_dwell(message);
        for handler in self.imp().on_focus.borrow().iter() {
            handler(message);
        }
    }

    /// Focus the message at `index`, which is what activating a rail row
    /// does.
    ///
    /// Through `Rail::activate` rather than straight to `focus_message`, so a
    /// rail click and `J` take the same route to the same mark (#1372). That
    /// is the whole of the brief's *"one entry point"*: two ways in that each
    /// set the value are two ways to disagree.
    pub fn focus_at(&self, index: usize) -> bool {
        let mut rail = Rail::at(self.message_count(), self.focused_index());
        if rail.activate(index) == Effect::Nothing {
            return false;
        }
        let Some(message) = rail.marked().and_then(|at| self.message_at(at)) else {
            return false;
        };
        self.focus_message(message);
        true
    }

    /// Tell the pane how wide its window is, so the rail can take its step
    /// on the ladder.
    ///
    /// The window's business rather than the pane's: the ladder's numbers are
    /// window widths, and a pane that measured itself would give a different
    /// answer depending on what else was on screen.
    pub fn set_window_width(&self, width: i32) {
        let imp = self.imp();
        imp.rail_width.set(Some(width));
        let messages = self.message_count();
        self.apply_rail_ladder(width, messages);
    }

    fn apply_rail_ladder(&self, width: i32, messages: usize) {
        let imp = self.imp();
        let step = presentation(width, messages, imp.rail_hidden.get());
        // Where the one rail lives. Moved rather than duplicated: a second
        // `RailColumn` for the popover would be a second marked row, and it
        // would be wrong exactly when someone scrolled with the index open.
        self.house_the_rail(matches!(step, Some(Presentation::Popover)));
        match step {
            Some(Presentation::Full) => {
                imp.rail.widget().set_visible(true);
                imp.rail.set_narrow(false);
            }
            Some(Presentation::Narrow) => {
                imp.rail.widget().set_visible(true);
                imp.rail.set_narrow(true);
            }
            Some(Presentation::Popover) => {
                // Visible *within the popover*, which shows nothing until the
                // counter is pressed. The column beside the body is gone
                // because the rail is no longer in it.
                imp.rail.widget().set_visible(true);
                imp.rail.set_narrow(false);
            }
            None => imp.rail.widget().set_visible(false),
        }
        let position = match step {
            Some(Presentation::Popover) => {
                Some((imp.rail.marked_position().unwrap_or(1), messages))
            }
            _ => None,
        };
        imp.header.set_counter(position);
        imp.header
            .set_compact(matches!(step, Some(Presentation::Popover)));
    }

    /// Put the rail in the popover, or back beside the body.
    ///
    /// Idempotent, and it has to be: the ladder runs on every resize and on
    /// every conversation, and GTK will not let a widget be added to a second
    /// parent while the first still holds it.
    fn house_the_rail(&self, in_popover: bool) {
        let imp = self.imp();
        let rail = imp.rail.widget();
        let index = imp.header.index();
        let housed_in_popover = rail.ancestor(gtk::Popover::static_type()).is_some();
        if in_popover == housed_in_popover {
            return;
        }
        if in_popover {
            imp.body.remove(rail);
            index.set_child(Some(rail));
        } else {
            index.set_child(None::<&gtk::Widget>);
            imp.body.append(rail);
        }
    }

    /// `⇧R`: put the rail away, or bring it back.
    ///
    /// The choice belongs to the window and outlives the conversation open in
    /// it (FR-047), which is why nothing here touches the thread.
    pub fn toggle_rail(&self) {
        let imp = self.imp();
        imp.rail_hidden.set(!imp.rail_hidden.get());
        let width = self.window_width();
        let messages = self.message_count();
        self.apply_rail_ladder(width, messages);
    }

    /// Whether `⇧R` has the rail put away.
    pub fn rail_hidden(&self) -> bool {
        self.imp().rail_hidden.get()
    }

    /// The rail, so a test can read back what a person would see.
    pub fn rail(&self) -> &crate::reader::rail::RailColumn {
        &self.imp().rail
    }

    /// How wide the window is, asked of the window.
    ///
    /// The first version of this measured the *pane* and called it close
    /// enough, reasoning that the pane is never wider than the window so the
    /// ladder could only err quiet. It erred quiet every time: the reading
    /// pane is about 660px in a 1280px window, which is below the ladder's
    /// floor, so the rail unmounted itself in a window with ample room for it
    /// and the breakpoints never corrected it -- neither one applies above
    /// 1240, so neither one fires. Nothing was wrong that a screenshot did
    /// not show immediately, and nothing but a screenshot would have.
    fn window_width(&self) -> i32 {
        if let Some(width) = self.imp().rail_width.get() {
            return width;
        }
        self.root()
            .and_downcast::<gtk::Window>()
            .map(|window| window.width())
            .filter(|width| *width > 0)
            .unwrap_or(NARROW_BELOW)
    }

    /// Focus a message in the one-document pane.
    ///
    /// Sets the focus, marks the rail from it, scrolls the document to the
    /// message and runs the focus handlers — the same four things the stacked
    /// arm does, with the scroll being a fragment navigation rather than a
    /// widget being brought into view.
    fn focus_in_document(&self, message: MessageId) {
        let imp = self.imp();
        if !imp.thread_rows.borrow().iter().any(|row| row.id == message) {
            return;
        }
        imp.focused.set(Some(message));
        imp.rail.set_marked(self.focused_index());
        if let Some(reader) = imp.document_reader.borrow().as_ref() {
            reader.scroll_to_message(&message.get().to_string());
        }
        self.start_dwell(message);
        for handler in imp.on_focus.borrow().iter() {
            handler(message);
        }
    }

    /// Move focus to the next message in the stack — `J`.
    ///
    /// Stops at the end rather than wrapping: a conversation has a first and
    /// a last message, and wrapping from one to the other makes "am I at the
    /// end" a question you have to keep answering yourself. Steps *into* a
    /// folded run rather than over it — the run's messages are messages, and
    /// walking past five of them because they were drawn as one line would
    /// be the fold changing what the keyboard does.
    pub fn focus_next(&self) -> bool {
        self.step(1)
    }

    /// Move focus to the previous message in the stack — `K`.
    pub fn focus_previous(&self) -> bool {
        self.step(-1)
    }

    /// Fold or unfold the focused message — `space`.
    ///
    /// The only way to collapse the focused message: landing on one expands
    /// it ([`focus_message`](Self::focus_message)), so collapsed-and-focused
    /// is a state nothing else reaches.
    pub fn toggle_fold(&self) {
        let Some(focused) = self.focused() else {
            return;
        };
        if self.is_expanded(focused) {
            self.collapse(focused);
        } else {
            self.expand(focused);
        }
    }

    /// Where the focused message sits in the conversation.
    ///
    /// Both panes, which is not the same list: the stacked pane keeps an
    /// `Entry` per message and the one-document pane keeps `thread_rows` and
    /// no entries at all. Reading only `entries` answered `None` for every
    /// message of a one-document conversation -- and every caller treats
    /// `None` as "nothing is focused", so the rail marked nothing and `J`
    /// started from the beginning.
    pub fn focused_index(&self) -> Option<usize> {
        let focused = self.focused()?;
        let imp = self.imp();
        let entries = imp.entries.borrow();
        if !entries.is_empty() {
            return entries.iter().position(|entry| entry.message == focused);
        }
        imp.thread_rows
            .borrow()
            .iter()
            .position(|row| row.id == focused)
    }

    /// The most recent message of the conversation on screen.
    ///
    /// The thread is held oldest first in both panes, so this is the last of
    /// whichever list the pane keeps.
    fn latest_message(&self) -> Option<MessageId> {
        self.message_count()
            .checked_sub(1)
            .and_then(|last| self.message_at(last))
    }

    /// The message at `index`, in whichever pane is drawing.
    fn message_at(&self, index: usize) -> Option<MessageId> {
        let imp = self.imp();
        let entries = imp.entries.borrow();
        if !entries.is_empty() {
            return entries.get(index).map(|entry| entry.message);
        }
        imp.thread_rows.borrow().get(index).map(|row| row.id)
    }

    /// How many messages the conversation has, in whichever pane is drawing.
    fn message_count(&self) -> usize {
        let imp = self.imp();
        let entries = imp.entries.borrow().len();
        if entries > 0 {
            entries
        } else {
            imp.thread_rows.borrow().len()
        }
    }

    /// One step through the stack, in draw order.
    ///
    /// Answers whether it moved, so a caller can tell "there was nowhere to
    /// go" from "the pane is empty" — the first is a no-op the user will
    /// expect, the second means the key reached the wrong surface.
    /// Where `J` and `K` land, asked of the rail's rule rather than answered
    /// again here.
    ///
    /// The clamping, and the two different starting points for an unfocused
    /// conversation, used to be written out in this function. They are the
    /// same rule the conversation rail needs, and a rule in a widget cannot be
    /// proven without a display while the same rule in `postio-ui` is
    /// arithmetic — so this asks, and `rail::Rail` answers.
    fn step(&self, by: isize) -> bool {
        let count = self.message_count();
        if count == 0 {
            return false;
        }
        let mut rail = Rail::at(count, self.focused_index());
        let effect = if by > 0 {
            rail.next_message()
        } else {
            rail.previous_message()
        };
        if effect == Effect::Nothing {
            return false;
        }
        let Some(message) = rail.marked().and_then(|at| self.message_at(at)) else {
            return false;
        };
        self.focus_message(message);
        true
    }

    /// Give `message` a body, if it has not got one.
    ///
    /// Idempotent, and the only place the factory is called — which is what
    /// makes "one reader per expanded message, never per message" a property
    /// of this function rather than of every caller.
    pub fn expand(&self, message: MessageId) {
        let imp = self.imp();
        let entries = imp.entries.borrow();
        let Some(entry) = entries.iter().find(|entry| entry.message == message) else {
            return;
        };
        // Expanded *and* still holding its reader. An entry whose body was
        // released to stay under `LIVE_BODY_CAP` is expanded with nothing in
        // it, and asking for it again has to rebuild rather than return.
        if entry.expanded.get() && entry.reader.borrow().is_some() {
            return;
        }
        entry.expanded.set(true);
        entry.actions.set_visible(true);
        // The spare, if it was built for *this* message, and that is the
        // whole of the pre-warm: its web process has already started, so the
        // body paints rather than flashing black while one boots (#1216).
        let taken = match imp.spare.borrow_mut().take() {
            Some((warmed, reader)) if warmed == message => Some(reader),
            // Built for a different message: it cannot be used, and holding
            // it would keep a process for a body nobody asked for.
            Some(_) | None => None,
        };
        if let Some(reader) = taken.or_else(|| {
            imp.factory
                .borrow()
                .as_ref()
                .and_then(|factory| factory(message))
        }) {
            let widget = reader.widget();
            widget.set_hexpand(true);
            entry.body.append(&widget);
            *entry.reader.borrow_mut() = Some(reader);
        }
        entry.body.set_visible(true);
        drop(entries);
        self.release_distant_bodies();
    }

    /// Drop the live bodies furthest from the focus, down to
    /// [`LIVE_BODY_CAP`].
    ///
    /// Furthest rather than oldest: what a person is about to want is what is
    /// near where they are reading, and a thread is navigated up and down
    /// rather than in one direction. Released entries stay expanded, so
    /// scrolling back rebuilds them.
    fn release_distant_bodies(&self) {
        let imp = self.imp();
        let entries = imp.entries.borrow();
        let focus = imp
            .focused
            .get()
            .and_then(|id| entries.iter().position(|entry| entry.message == id))
            .unwrap_or(0);

        let mut live: Vec<usize> = entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.reader.borrow().is_some())
            .map(|(index, _)| index)
            .collect();
        if live.len() <= LIVE_BODY_CAP {
            return;
        }
        // Furthest from the focus first, and release exactly the excess.
        live.sort_by_key(|index| std::cmp::Reverse(index.abs_diff(focus)));
        let excess = live.len() - LIVE_BODY_CAP;
        for index in live.into_iter().take(excess) {
            let entry = &entries[index];
            let released = entry.reader.borrow_mut().take();
            if let Some(reader) = released {
                // Off the widget tree as well as out of the field: a parked
                // widget keeps its `WebView`, and the `WebView` is the web
                // process this exists to give back.
                entry.body.remove(&reader.widget());
            }
        }
    }

    /// Build and start a reader for the message most likely to open next.
    ///
    /// A `WebView` spawns its web process on the first *load*, not when it is
    /// built, so a reader made at the moment a message expands makes the
    /// person wait for a process to start, relocate and paint -- and it
    /// composites black until it has. That is the flicker moving through a
    /// conversation (#1216).
    ///
    /// The next message *down the stack*, because that is the gesture: read
    /// one, move to the next. Anything else falls back to building a reader
    /// the old way, so this is an optimisation for the common path and never
    /// a correctness question.
    ///
    /// On an idle callback rather than inline: the caller has just expanded a
    /// message, and starting a second web process there would spend on this
    /// keystroke exactly what the spare exists to save.
    fn warm_the_next(&self) {
        let Some(next) = self.next_unexpanded() else {
            return;
        };
        if matches!(*self.imp().spare.borrow(), Some((held, _)) if held == next) {
            return;
        }
        let pane = self.clone();
        glib::idle_add_local_once(move || {
            let imp = pane.imp();
            if matches!(*imp.spare.borrow(), Some((held, _)) if held == next) {
                return;
            }
            let built = imp
                .factory
                .borrow()
                .as_ref()
                .and_then(|factory| factory(next));
            if let Some(reader) = built {
                reader.warm();
                *imp.spare.borrow_mut() = Some((next, reader));
            }
        });
    }

    /// The nearest message to the focused one that has no body yet.
    ///
    /// **Behind first, then ahead.** This looked only ahead, which was right
    /// while the pane opened on the first unread and the gesture was "read
    /// this one, move down". FR-015 opens it on the *newest* (#1385), where
    /// there is nothing ahead at all — so the spare was never warmed on open
    /// and #1216's black flash came back for the first `K`, which is now the
    /// only way to go.
    ///
    /// Ahead is still checked, for the top of a thread and for a reader who
    /// has walked back and is coming down again.
    fn next_unexpanded(&self) -> Option<MessageId> {
        let imp = self.imp();
        let entries = imp.entries.borrow();
        let focused = imp.focused.get();
        let at = focused.and_then(|id| entries.iter().position(|entry| entry.message == id));
        let Some(at) = at else {
            return entries
                .iter()
                .find(|entry| !entry.expanded.get())
                .map(|entry| entry.message);
        };
        entries
            .iter()
            .take(at)
            .rev()
            .find(|entry| !entry.expanded.get())
            .or_else(|| {
                entries
                    .iter()
                    .skip(at + 1)
                    .find(|entry| !entry.expanded.get())
            })
            .map(|entry| entry.message)
    }

    /// How many message bodies are holding a live `WebKitWebView`.
    ///
    /// The number [`LIVE_BODY_CAP`] bounds, and the one worth asserting on:
    /// expanded entries and live bodies are no longer the same set, because a
    /// released entry stays expanded with nothing in it.
    pub fn live_body_count(&self) -> usize {
        self.imp()
            .entries
            .borrow()
            .iter()
            .filter(|entry| entry.reader.borrow().is_some())
            .count()
    }

    /// The reader already built for `message`'s entry, if it has one.
    ///
    /// The conversation pane's answer to a body or a payload landing for an
    /// already-expanded entry (#739): `expand` only ever fills an *empty*
    /// body, so nothing re-fills a full one without a caller that can find
    /// the reader again and re-render into it — this is that seam. `None`
    /// for a collapsed entry (there is no reader yet; that is `expand`'s
    /// job) and for a message the pane is not holding at all.
    pub fn reader_for(&self, message: MessageId) -> Option<crate::reader::Reader> {
        self.imp()
            .entries
            .borrow()
            .iter()
            .find(|entry| entry.message == message && entry.expanded.get())
            .and_then(|entry| entry.reader.borrow().clone())
    }

    /// Collapse `message` back to its one-line header.
    pub fn collapse(&self, message: MessageId) {
        let imp = self.imp();
        let entries = imp.entries.borrow();
        let Some(entry) = entries.iter().find(|entry| entry.message == message) else {
            return;
        };
        if !entry.expanded.get() {
            return;
        }
        entry.expanded.set(false);
        entry.body.set_visible(false);
        entry.actions.set_visible(false);
        // The body widget is kept rather than destroyed: a message collapsed
        // and reopened is the common gesture, and rebuilding a `WebKitWebView`
        // for it would make the cheap direction the expensive one.
    }

    /// Called when a message's reply button is used. `true` is reply-all.
    pub fn connect_reply(&self, handler: impl Fn(MessageId, bool) + 'static) {
        self.imp().on_reply.borrow_mut().push(Box::new(handler));
    }

    /// Called when a message's forward button is used.
    pub fn connect_forward(&self, handler: impl Fn(MessageId) + 'static) {
        self.imp().on_forward.borrow_mut().push(Box::new(handler));
    }

    /// Called when the current message changes, so the drill-in column can
    /// move its cursor to match.
    pub fn connect_focus_changed(&self, handler: impl Fn(MessageId) + 'static) {
        self.imp().on_focus.borrow_mut().push(Box::new(handler));
    }

    /// Called when a message has been focused long enough to have been read
    /// (#71).
    ///
    /// The same rule the list uses, one surface over: focus is what reading
    /// looks like here, and walking a conversation with the index passes over
    /// messages without reading them exactly as scrolling a mailbox does.
    /// Never fires for a conversation merely opened — the focused message has
    /// to be rested on.
    pub fn connect_dwelled(&self, handler: impl Fn(MessageId) + 'static) {
        self.imp().on_dwell.borrow_mut().push(Box::new(handler));
    }

    /// Re-cap every message's action bar from the live keymap.
    ///
    /// Called by `Window::apply_keymap` alongside every other surface that
    /// shows a key: a `[keys]` rebind has to reach the caps in the stack the
    /// same moment it reaches the keyboard, or the pane advertises a key
    /// that now runs something else.
    pub fn set_keymap(&self, keymap: &postio_core::Keymap) {
        for entry in self.imp().entries.borrow().iter() {
            entry.actions.set_keymap(keymap);
        }
        self.imp().header.set_keymap(keymap);
        self.imp().footer.set_keymap(keymap);
        self.imp().header.actions().set_keymap(keymap);
    }

    /// Shorten the dwell for a test that cannot wait a second.
    pub fn set_dwell_delay(&self, delay: std::time::Duration) {
        self.imp().dwell_delay.set(delay);
    }

    /// Stop any dwell in flight.
    ///
    /// Cancelled rather than dropped: a `glib` timeout whose handle goes away
    /// still fires, and this one would mark a message read after the pane had
    /// moved on from it.
    pub fn cancel_dwell(&self) {
        if let Some(source) = self.imp().dwell.borrow_mut().take() {
            source.remove();
        }
    }

    fn start_dwell(&self, message: MessageId) {
        self.cancel_dwell();
        let view = self.clone();
        let source = glib::timeout_add_local_once(self.imp().dwell_delay.get(), move || {
            view.imp().dwell.borrow_mut().take();
            // Named explicitly rather than read back off the focus: focus may
            // have moved between the timer firing and this running, and the
            // message that was read is the one the clock was started for.
            for handler in view.imp().on_dwell.borrow().iter() {
                handler(message);
            }
        });
        *self.imp().dwell.borrow_mut() = Some(source);
    }

    /// Press a message's reply button without a pointer.
    pub fn test_click_reply(&self, message: MessageId) {
        for handler in self.imp().on_reply.borrow().iter() {
            handler(message, false);
        }
    }

    /// Press a message's forward button without a pointer.
    pub fn test_click_forward(&self, message: MessageId) {
        for handler in self.imp().on_forward.borrow().iter() {
            handler(message);
        }
    }

    /// The widget the reader factory built for `message`, if it has been
    /// expanded — what a test downcasts to check what is actually on
    /// screen inside an entry, rather than only what the pane's own state
    /// says (#487).
    #[doc(hidden)]
    pub fn test_expanded_widget(&self, message: MessageId) -> Option<gtk::Widget> {
        self.imp()
            .entries
            .borrow()
            .iter()
            .find(|entry| entry.message == message)
            .and_then(|entry| entry.body.first_child())
    }

    // -- internals ---------------------------------------------------------

    fn scroll_to(&self, message: MessageId) {
        let imp = self.imp();
        let entries = imp.entries.borrow();
        let Some(entry) = entries.iter().find(|entry| entry.message == message) else {
            return;
        };
        // No animation: the motion budget says a jump is instant, and this is
        // the same swap the drill-in itself makes.
        let container = entry.container();
        let Some(bounds) = container.compute_bounds(&imp.stack) else {
            return;
        };
        imp.scroller.vadjustment().set_value(bounds.y() as f64);
    }

    /// One entry in the stack.
    ///
    /// `alone` is whether this message is the whole conversation, which
    /// decides which verbs it carries: on its own it is a view in its own
    /// right and takes `Archive` with it, because the footer that would
    /// otherwise have carried that verb is not drawn (#1173).
    fn build_entry(&self, row: &Row, index: u32, alone: bool) -> imp::Entry {
        let header = crate::thread_row::ThreadRowView::new();
        header.set_row(Some(row.clone()), index);
        header.set_mine(self.is_mine(row));

        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.set_visible(false);

        // Reply, reply-all and forward only. Every other verb is the
        // conversation's, in this pane exactly as in the list (ADR 0015 Q4),
        // and a delete button on every message in a stack is how people
        // delete the wrong one.
        //
        // The same `ActionBar` the reading pane's own bar is (#1002), so
        // these three carry their keys — they were three bare buttons with
        // no caps at all, in a pane whose whole point is that `e` acts on
        // whichever message you are looking at.
        let message = row.id;
        let actions = crate::widgets::ActionBar::new(
            match (row.draft, alone) {
                (true, true) => &LONE_DRAFT_ACTIONS[..],
                (true, false) => &DRAFT_ACTIONS[..],
                (false, true) => &LONE_MESSAGE_ACTIONS[..],
                (false, false) => &MESSAGE_ACTIONS[..],
            },
            "conversation-actions",
        );
        actions.set_visible(false);
        let view = self.clone();
        actions.connect_command(move |command| {
            // `Archive` is only ever in this bar when the message is the
            // whole thread, where archiving it and archiving the thread are
            // the same act — so it goes out on the conversation's own path
            // rather than growing a second one.
            if command.id() == postio_core::CommandId::ArchiveThread {
                view.emit_command(command);
                return;
            }
            // Named, not left to the cursor: a draft inside a longer thread
            // is not the row the list holds, and an untargeted open would
            // resume whatever the list is pointing at instead (#1212).
            if command.id() == postio_core::CommandId::OpenMessage {
                view.emit_command(postio_core::Command::OpenMessage {
                    message: Some(message),
                });
                return;
            }
            let kind = match command.id() {
                postio_core::CommandId::Reply => ReplyKind::Reply,
                postio_core::CommandId::ReplyAll => ReplyKind::ReplyAll,
                postio_core::CommandId::Forward => ReplyKind::Forward,
                _ => return,
            };
            view.emit_action(message, kind);
        });

        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.add_css_class("conversation-entry");
        container.append(&header);
        container.append(&body);
        // Under the message, not under its header. Above the body they read
        // as belonging to whatever comes next -- which for a stack of
        // messages is somebody else's mail.
        container.append(&actions.widget());

        // A click anywhere on the header makes that message current, which is
        // the mouse's half of what the column's cursor does.
        let gesture = gtk::GestureClick::new();
        let view = self.clone();
        gesture.connect_released(move |_, _, _, _| view.focus_message(message));
        header.add_controller(gesture);

        imp::Entry {
            message,
            row: row.clone(),
            header,
            body,
            reader: std::cell::RefCell::new(None),
            actions,
            expanded: std::cell::Cell::new(false),
            shown: std::cell::Cell::new(false),
            container,
        }
    }

    /// Tells whoever is listening to run `command`.
    ///
    /// The pane raises `CommandId`s and runs none of them, the same
    /// arrangement every other surface here has: `window.rs` owns the one
    /// dispatch a keystroke, a menu item and a palette entry all go through,
    /// and a second path from a button straight to the runtime is how two
    /// surfaces come to disagree about what a verb means.
    fn emit_command(&self, command: postio_core::Command) {
        for handler in self.imp().on_command.borrow().iter() {
            handler(command.clone());
        }
    }

    /// The addresses this account sends as.
    ///
    /// What lets a row draw the drawing's `mine` mark — an outlined square
    /// rather than a filled one, for the user's own side of the
    /// conversation. Folded once here rather than per row per redraw.
    ///
    /// Re-stating it redraws the rows: identities can change while a
    /// conversation is open, and a mark that was right when the pane opened
    /// is not a mark anybody checks again.
    pub fn set_own_addresses(&self, addresses: &[postio_model::EmailAddress]) {
        let folded: Vec<String> = addresses
            .iter()
            .map(|address| address.address.to_lowercase())
            .collect();
        if *self.imp().own_addresses.borrow() == folded {
            return;
        }
        *self.imp().own_addresses.borrow_mut() = folded;
        for entry in self.imp().entries.borrow().iter() {
            entry.header.set_mine(self.is_mine(&entry.row));
        }
    }

    /// The collapsed header drawn for `message`, if the stack holds it.
    ///
    /// The same shape as `reader_for`, and for the same reason: what a row
    /// draws is a fact about one entry, and the pane is what knows which
    /// entry that is.
    pub fn header_for(&self, message: MessageId) -> Option<crate::thread_row::ThreadRowView> {
        self.imp()
            .entries
            .borrow()
            .iter()
            .find(|entry| entry.message == message)
            .map(|entry| entry.header.clone())
    }

    /// Whether `row` came from one of the account's own addresses.
    fn is_mine(&self, row: &Row) -> bool {
        let own = self.imp().own_addresses.borrow();
        row.from.as_ref().is_some_and(|from| {
            let from = from.address.to_lowercase();
            own.contains(&from)
        })
    }

    /// Who to ask to run a command one of this pane's bars carries.
    pub fn connect_command(&self, handler: impl Fn(postio_core::Command) + 'static) {
        self.imp().on_command.borrow_mut().push(Box::new(handler));
    }

    fn emit_action(&self, message: MessageId, kind: ReplyKind) {
        match kind {
            ReplyKind::Reply => {
                for handler in self.imp().on_reply.borrow().iter() {
                    handler(message, false);
                }
            }
            ReplyKind::ReplyAll => {
                for handler in self.imp().on_reply.borrow().iter() {
                    handler(message, true);
                }
            }
            ReplyKind::Forward => {
                for handler in self.imp().on_forward.borrow().iter() {
                    handler(message);
                }
            }
        }
    }
}

/// Which per-message verb a button carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReplyKind {
    Reply,
    ReplyAll,
    Forward,
}
