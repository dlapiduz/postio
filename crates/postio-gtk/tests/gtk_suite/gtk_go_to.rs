//! A destination key that has nowhere to go says so.
//!
//! FR-041. A key that appears to do nothing is read as a broken key, and the
//! next thing a person tries is the same key again. An account without a
//! drafts folder is an ordinary thing -- not every provider offers one, and
//! one that has not synced yet has none of them.
//!
//! The assertion is on what the window *said*, deliberately. "It did not
//! navigate" cannot tell being told apart from being ignored, which is the
//! whole distinction this requirement exists to make.
//!
//! Skips without a display. Nothing here touches the network.

use crate::pump;

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::ids::{AccountId, MailboxId};
use postio_model::mailbox::{Mailbox, MailboxCounts, MailboxRole};

fn only_an_inbox() -> Vec<Mailbox> {
    let mut inbox = Mailbox::new(AccountId::new(1), "INBOX", Some('/'));
    inbox.id = MailboxId::new(1);
    inbox.role = MailboxRole::Inbox;
    inbox.counts = MailboxCounts {
        total: 3,
        unread: 1,
        flagged: 0,
        snoozed: 0,
        // A question about drafts waiting on a person, and this is an inbox.
        attention: 0,
    };
    vec![inbox]
}

pub fn a_destination_this_account_does_not_have_is_reported() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    window.present();
    pump();

    window.sidebar().set_mailboxes(&only_an_inbox());
    pump();

    assert_eq!(
        window.announced(),
        None,
        "nothing has been said yet, so anything below is this test's doing"
    );

    window.act(postio_core::Command::GoToDrafts);
    pump();

    let said = window
        .announced()
        .expect("`g d` on an account with no drafts folder said nothing at all");
    assert!(
        said.contains("drafts"),
        "the window said {said:?}, which does not tell the reader which folder \
         is missing"
    );

    // And a destination this account *does* have is not reported missing, so
    // the message above is about the folder rather than about every `g` key
    // having given up.
    //
    // Only what is said is checked here. Arriving is `open_mailbox`, which
    // documents itself as a no-op until `install_feeds` has run, and a bare
    // window has no feeds -- the journey is proven at the composition root
    // instead, by `app_suite`'s `go_to_keystroke`.
    window.act(postio_core::Command::GoToInbox);
    pump();
    assert!(
        !window
            .announced()
            .is_some_and(|said| said.contains("inbox")),
        "the inbox is right here and the window claimed otherwise"
    );
}

/// The state the window actually starts in: focus lands in the search field
/// before any folder has loaded.
///
/// `focusing_the_field_offers_the_modes` sets mailboxes first, so it never
/// sees this -- and this is the state a person meets at launch, where an
/// offer with nothing in it is the empty plate hanging under the field.
pub fn an_empty_box_at_startup_is_not_a_blank_plate() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    window.present();
    pump();

    // No folders, no contacts, no labels -- nothing has loaded yet.
    let field = window.finder().field().expect("the header field");
    field.text.grab_focus();
    pump();

    let finder = window.finder();
    assert!(finder.is_open(), "focus opens the box");
    let hints = finder.mode_hints();
    assert!(
        !hints.is_empty(),
        "an open box with nothing to say draws a plate with nothing on it"
    );
    assert!(
        finder.is_visible(),
        "the plate is up, and it has rows: {hints:?}"
    );
}
