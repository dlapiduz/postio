//! The conversation pane on a real display (ADR 0032, #1426).
//!
//! The decisions with consequences are pure and unit-tested -- in
//! `conversation.rs` for the pane's own rules and in
//! `postio_ui::reader::thread` for the document's markup. What needs a
//! display is what those cannot see: which controls are *on screen*, and
//! which message they carry when pressed.
//!
//! That is the one worth a display test. The header's verbs are scoped to the
//! conversation's latest message and a message's own verbs live in the
//! document, so "can this be replied to" is a question about two surfaces at
//! once, and answering the wrong message of a conversation is the mistake the
//! whole arrangement exists to prevent.
//!
//! Skips without a display. Nothing here touches the network.

use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::conversation::ConversationView;
use postio_gtk::list::Row;
use postio_gtk::reader::Reader;
use postio_gtk::{fonts, style};
use postio_model::EmailAddress;
use postio_model::ids::{MessageId, ThreadId};

/// A reader with nothing behind it — no blob source worth naming, since
/// nothing here asks it to resolve a `cid:`. Good enough to stand in for the
/// hardened one everywhere this file only cares that a reader was built, not
/// what it can render.
fn stub_reader() -> Reader {
    Reader::new(Rc::new(|_content_id: &str| None))
}

/// One message of the conversation, oldest first by id.
fn message(id: i64, seen: bool) -> Row {
    Row {
        id: MessageId::new(id),
        thread: Some(ThreadId::new(1)),
        from: Some(EmailAddress::new(Some("Ada Norwood"), "ada@example.com")),
        subject: Some(format!("Tide gate interlock {id}")),
        preview: Some(format!("Snippet {id}")),
        received_at: chrono::Utc::now() - chrono::Duration::minutes(100 - id),
        seen,
        flagged: false,
        answered: false,
        send_state: None,
        has_attachments: false,
        thread_count: 6,
        participants: Vec::new(),
    }
}

/// Pump the main loop for `how_long`, so a timer can fire.
fn settle_for(how_long: std::time::Duration) {
    let deadline = std::time::Instant::now() + postio_test_support::scaled(how_long);
    while std::time::Instant::now() < deadline {
        while gtk::glib::MainContext::default().iteration(false) {}
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Every widget under `root` whose CSS class ends in `-reply`, walking only
/// into subtrees that are actually on screen.
///
/// The class is what `widgets::Action` documents as "the CSS class a test
/// finds it by", and `-reply` excludes `-reply-all` because the class for
/// that one ends in `-all`. Counting what is *drawn* rather than asking the
/// two action tables what they hold is the whole point: the tables were each
/// right on their own, and the defect was the pair of them on screen at once.
fn reply_controls(root: &gtk::Widget) -> Vec<String> {
    let mut found = Vec::new();
    if !root.is_visible() {
        return found;
    }
    for class in root.css_classes() {
        if class.ends_with("-reply") {
            found.push(class.to_string());
        }
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        found.extend(reply_controls(&widget));
        child = widget.next_sibling();
    }
    found
}

/// One thread, one `Reply` on screen — at any length (#1173).
///
/// The one-message case was the visible half: both bars drew and `Reply`
/// appeared twice with `e` printed on each. The n>1 case is the same defect
/// with a pane's height between the two buttons, which is why it survived
/// three issues while the n=1 one was reported at once.
///
/// `Reply to conversation` runs `CommandId::Reply` aimed at the *focused*
/// message, so it is the same command and the same key as the bar drawn on
/// that message — under a label naming a scope Postio does not have. You
/// reply to a message, never to a thread, and a pinned control drawn a pane
/// away from the message it will answer is the mistake ADR 0015 Q4 gives as
/// its reason for making the reply verbs per-message: "answering the wrong
/// message of a conversation is a real and common mistake".
pub fn one_thread_offers_one_reply_however_long_it_is() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = gtk::Window::new();
    let pane = ConversationView::new();
    pane.set_reader_factory(|_message| Some(stub_reader()));
    window.set_child(Some(&pane.widget()));
    window.set_default_size(700, 600);
    window.present();
    crate::pump();

    // ── a thread of one ───────────────────────────────────────────────────
    pane.open(vec![message(1, false)]);
    crate::pump();
    let found = reply_controls(&pane.widget());
    assert_eq!(
        found.len(),
        1,
        "a one-message thread drew {} controls bound to reply: {found:?}",
        found.len()
    );

    // ── and a thread of eight ─────────────────────────────────────────────
    let mut messages: Vec<Row> = (1..=8).map(|id| message(id, true)).collect();
    messages[7].seen = false;
    pane.open(messages);
    crate::pump();
    let found = reply_controls(&pane.widget());
    assert_eq!(
        found.len(),
        1,
        "a thread drew {} controls bound to reply: {found:?}. Both run \
         `CommandId::Reply` on the focused message, so the second is a \
         duplicate that hides which message it answers",
        found.len()
    );

    window.close();
}

/// #1241: a row knows whether the message is one the user sent.
///
/// `Design/screens/18-conversation-row-states.png` gives four states a mark
/// each, and three of them — read, unread, focused — the row could already
/// tell apart from `Row::seen` and its own selection. `mine` it could not:
/// nothing in `postio-gtk` knows the account's addresses, so the pane is
/// told them and folds the comparison once.
///
/// The mark itself is drawn into a `snapshot()` and cannot be read back from
/// a widget tree, so what is asserted is the state the drawing depends on.
/// `scripts/screens.sh --only conversation` is what puts the pixels beside
/// `17-conversation-view.png`.
pub fn a_row_knows_whether_the_message_is_the_users_own() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = gtk::Window::new();
    let pane = ConversationView::new();
    window.set_child(Some(&pane.widget()));
    window.present();

    let from = |id: i64, address: &str| Row {
        from: Some(EmailAddress::new(Some("Someone"), address)),
        ..message(id, true)
    };

    pane.open(vec![
        from(1, "ada@example.com"),
        from(2, "me@example.net"),
        from(3, "ADA@EXAMPLE.COM"),
    ]);
    settle_for(std::time::Duration::from_millis(20));

    // Nobody has said which addresses are the user's, so nothing is theirs.
    assert_eq!(mine_flags(&pane), vec![false, false, false]);

    pane.set_own_addresses(&[EmailAddress::new(None::<String>, "me@example.net")]);
    settle_for(std::time::Duration::from_millis(20));
    assert_eq!(
        mine_flags(&pane),
        vec![false, true, false],
        "only the message from the account's own address is the user's"
    );

    // Addresses are case-insensitive, and a thread quotes them however the
    // sender typed them.
    pane.set_own_addresses(&[EmailAddress::new(None::<String>, "Ada@Example.com")]);
    settle_for(std::time::Duration::from_millis(20));
    assert_eq!(
        mine_flags(&pane),
        vec![true, false, true],
        "ADA@EXAMPLE.COM and ada@example.com are the same person"
    );

    // Identities can change while a conversation is open, and a mark that
    // was right when the pane opened is not one anybody checks again.
    pane.set_own_addresses(&[]);
    settle_for(std::time::Duration::from_millis(20));
    assert_eq!(mine_flags(&pane), vec![false, false, false]);

    window.destroy();
}

/// Each row's `mine`, in conversation order.
///
/// The stacked pane kept a `ThreadRowView` per message and this read the flag
/// off it. The one document has no per-message widget, so it asks the pane
/// the same question the drawing will (#1426).
fn mine_flags(pane: &ConversationView) -> Vec<bool> {
    pane.rows().iter().map(|row| pane.is_mine(row)).collect()
}

/// A draft's conversation offers `Continue editing`, not a reply (#1212).
///
/// The pane's own bar is scoped to the conversation's *latest* message
/// (`latest_message`), which for a thread you are part-way through answering
/// is the draft itself. Offered `Reply` there, the primary verb quotes your
/// own unsent text back at you, and the verb that is actually right --
/// resuming the composer -- is unannounced.
///
/// The stacked pane knew this and drew `DRAFT_ACTIONS` on the message's own
/// bar. Retiring it (#1426) left the one-document pane with a header bar
/// built once from `DOCUMENT_ACTIONS` and no way to swap the set, so the
/// behaviour was absent rather than wrong -- which is #1444.
///
/// The document's own per-message verbs are asserted in
/// `postio_ui::reader::thread`, where they are markup and need no display.
/// What needs one is this: which bar is *visible*, and what it emits.
pub fn a_conversation_ending_in_a_draft_offers_continue_editing() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = gtk::Window::new();
    let pane = ConversationView::new();
    pane.set_reader_factory(|_message| Some(stub_reader()));

    let ran: Rc<std::cell::RefCell<Vec<postio_core::Command>>> =
        Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = Rc::clone(&ran);
    pane.connect_command(move |command| seen.borrow_mut().push(command));

    window.set_child(Some(&pane.widget()));
    window.set_default_size(700, 600);
    window.present();
    crate::pump();

    // A message and the unsent reply to it: the draft is the latest, which is
    // what the header's verbs are aimed at.
    let mut messages: Vec<Row> = (1..=2).map(|id| message(id, true)).collect();
    messages[1].send_state = Some(postio_model::DraftState::Editing);
    let draft = messages[1].id;
    pane.open(messages);
    crate::pump();

    let found = reply_controls(&pane.widget());
    assert!(
        found.is_empty(),
        "a conversation ending in a draft drew {found:?}: you do not reply to \
         a message you wrote and never sent"
    );

    let actions = pane
        .visible_actions()
        .expect("a draft's conversation still has a bar");
    actions.press(postio_core::CommandId::OpenMessage);
    crate::pump();
    assert_eq!(
        ran.borrow().as_slice(),
        [postio_core::Command::OpenMessage {
            message: Some(draft)
        }],
        "`Continue editing` names the draft it resumes, so a thread whose \
         draft is not the row the list holds still opens the right one"
    );

    window.close();
}

/// And a conversation that does not end in one keeps its reply.
///
/// The half that keeps the case above honest: a pane that had simply stopped
/// drawing reply verbs would pass it.
pub fn an_ordinary_conversation_still_offers_a_reply() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = gtk::Window::new();
    let pane = ConversationView::new();
    pane.set_reader_factory(|_message| Some(stub_reader()));
    window.set_child(Some(&pane.widget()));
    window.set_default_size(700, 600);
    window.present();
    crate::pump();

    pane.open((1..=2).map(|id| message(id, true)).collect());
    crate::pump();

    assert!(
        !reply_controls(&pane.widget()).is_empty(),
        "a conversation of sent mail lost the reply verb that is right for it"
    );

    window.close();
}
