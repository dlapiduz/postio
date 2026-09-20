//! What the reading pane offers a message that is on its way out (#1525).
//!
//! Reply, Reply all, Forward and Archive are all answers to somebody else's
//! mail. Until spec 003 there was no folder holding a message that had not
//! arrived, so offering them unconditionally was never wrong; the Outbox
//! made it wrong, on the message a person is most likely to want to act on.
//!
//! Its own file with one test function: GTK is initialised once, per process,
//! from one thread. Skips without a display. Nothing here touches the network.

use gtk::prelude::*;
use std::rc::Rc;

use postio_gtk::reader::Reader;
use postio_model::{DraftState, MessageBody};

pub fn a_message_being_sent_offers_sending_verbs_and_not_replies() {
    if adw::init().is_err() || gtk::gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }

    let reader = Reader::new(Rc::new(|_content_id: &str| None));
    // Mounted and presented: an unmapped bar reports itself invisible
    // whatever its own flag says, so a test that skipped this would pass
    // for the wrong reason on every assertion below.
    let window = gtk::Window::new();
    window.set_child(Some(&reader.widget()));
    window.set_default_size(900, 600);
    window.present();
    crate::pump();
    let body = MessageBody {
        text: Some("Confirmed on 0.4.1 — sending the trace now.".to_owned()),
        html: None,
    };

    // ── ordinary mail is untouched, which is the regression that matters ──
    reader.render(&body, Some("lena@example.com"));
    reader.set_unsubscribe(Some("example.com"));
    reader.set_send_state(None);
    crate::pump();
    let verbs = reader.visible_verbs();
    for expected in ["Reply", "Reply all", "Forward", "Archive"] {
        assert!(
            verbs.iter().any(|verb| verb == expected),
            "ordinary mail lost {expected:?}: {verbs:?}"
        );
    }
    assert!(
        reader.unsubscribe_banner_visible(),
        "a real list message keeps its unsubscribe"
    );

    // ── a send that stopped offers the way out of it ──────────────────────
    for stopped in [DraftState::Failed, DraftState::Unconfirmed] {
        reader.render(&body, Some("me@example.com"));
        // The banner the sender's own domain would raise (#971), which is
        // exactly what must not stand on outgoing mail.
        reader.set_unsubscribe(Some("example.com"));
        reader.set_send_state(Some(stopped));
        crate::pump();

        let verbs = reader.visible_verbs();
        assert_eq!(
            verbs,
            vec!["Send again".to_string()],
            "{stopped:?} should offer only the retry: {verbs:?}"
        );
        assert!(
            !reader.unsubscribe_banner_visible(),
            "{stopped:?} still offers to unsubscribe the user from their own domain"
        );
    }

    // ── one still waiting offers to stop it ───────────────────────────────
    reader.render(&body, Some("me@example.com"));
    reader.set_send_state(Some(DraftState::Queued));
    crate::pump();
    assert_eq!(
        reader.visible_verbs(),
        vec!["Cancel send".to_string()],
        "a queued send should offer to be stopped"
    );

    // ── one mid-submission offers nothing, on purpose ─────────────────────
    //
    // Cancelling is refused once the drainer has it (ADR 0021) and retrying
    // would risk a second copy, so an empty bar is the honest answer rather
    // than a button that will be turned down.
    reader.render(&body, Some("me@example.com"));
    reader.set_send_state(Some(DraftState::Sending));
    crate::pump();
    assert!(
        reader.visible_verbs().is_empty(),
        "a submission in flight offered a verb it would refuse: {:?}",
        reader.visible_verbs()
    );

    // ── and a repaint does not put Reply back ─────────────────────────────
    //
    // `render` clears the state, so the caller sets it again — but the bar
    // must not flash the wrong verbs in between, and a message drawn over an
    // outgoing one must not inherit its bar either.
    reader.render(&body, Some("lena@example.com"));
    crate::pump();
    let verbs = reader.visible_verbs();
    assert!(
        verbs.iter().any(|verb| verb == "Reply"),
        "a message drawn after an outgoing one is ordinary mail again: {verbs:?}"
    );

    window.close();
}
