//! Spec 009: a verb in a message opened from a search hit aims at the
//! message on screen, not at the list's cursor row.
//!
//! The controller never hears which message a hit opened, so the cursor
//! stays on whatever row it was on. A person who searched, opened the hit
//! and pressed `e` means the message they are reading. The store is what is
//! asserted on: which message moved, and which did not.

use gtk::prelude::*;
use postio_model::{Flag, MailboxId, MailboxRole, MessageId};
use postio_storage::repository::{MailboxRepository, MessageRepository};

use crate::support::{self, Fixture};

struct Hit {
    fixture: Fixture,
    window: postio_gtk::window::FocusWindow,
    /// The list cursor's row: newest, matches nothing searched.
    cursor: MessageId,
    /// The message the search finds and the person opens.
    hit: MessageId,
}

/// Two messages: the cursor is on "Atlas budget" (newest); the search for
/// "harbor" finds only "Harbor budget", which is opened from the hit.
async fn open_a_hit_away_from_the_cursor() -> Hit {
    let fixture = Fixture::empty().await;
    let (cursor, _) = fixture
        .file(
            ("Ada Moreno", "ada@example.com"),
            "Atlas budget",
            "Numbers.",
            1,
        )
        .await;
    let (hit, _) = fixture
        .file(
            ("Lena Park", "lena@example.org"),
            "Harbor budget",
            "Numbers for harbor.",
            9,
        )
        .await;
    fixture.index().await;
    let (window, _client) = fixture.open().await;
    assert!(
        crate::settle_until(async || support::subjects(&window).len() == 2).await,
        "the inbox never reached the screen"
    );
    assert_eq!(
        window.cursor_row().map(|row| row.id()),
        Some(cursor),
        "the cursor starts on the newest row"
    );
    support::press(&window, "slash", gtk::gdk::ModifierType::empty());
    let bar = window.bar().expect("/ opened the bar");
    for end in 1..="harbor".len() {
        bar.set_text(&"harbor"[..end]);
        crate::settle();
    }
    assert!(
        crate::settle_until(async || !bar.result_subjects().is_empty()).await,
        "the search found nothing: {:?}",
        bar.texts()
    );
    assert_eq!(bar.result_subjects(), ["Harbor budget"]);
    assert!(bar.to_first_hit(), "no hit to step to");
    let chord: postio_ui::keymap::Chord = "Return".parse().expect("a chord");
    postio_widgets::storyboard::deliver::press(window.upcast_ref(), &chord)
        .expect("a deliverable chord");
    crate::settle();
    assert!(
        crate::settle_until(async || window.reading().is_some_and(|r| r.is_open())).await,
        "the hit never opened"
    );
    assert_eq!(
        window.reading().map(|r| r.title()).as_deref(),
        Some("Harbor budget"),
        "the hit opened the wrong message"
    );
    Hit {
        fixture,
        window,
        cursor,
        hit,
    }
}

impl Hit {
    async fn message(&self, id: MessageId) -> postio_model::Message {
        let connection = self.fixture.database.connect().await.expect("a connection");
        MessageRepository::new(&connection)
            .get(id)
            .await
            .expect("a read")
            .expect("the message")
    }

    async fn role(&self, role: MailboxRole) -> MailboxId {
        let connection = self.fixture.database.connect().await.expect("a connection");
        MailboxRepository::new(&connection)
            .by_role(self.fixture.account.id, role)
            .await
            .expect("a read")
            .expect("the mailbox")
            .id
    }
}

/// Press `key` in the message opened from the hit: it, and not the cursor's
/// row, ends in `role`'s mailbox, and the screen stops showing it.
async fn leaves_for(key: (&str, gtk::gdk::ModifierType), role: MailboxRole, what: &str) {
    let hit = open_a_hit_away_from_the_cursor().await;
    let target = hit.role(role).await;
    support::press(&hit.window, key.0, key.1);
    crate::settle_until(async || {
        let (shown, cursor) = (hit.message(hit.hit).await, hit.message(hit.cursor).await);
        shown.mailbox_id != hit.fixture.inbox || cursor.mailbox_id != hit.fixture.inbox
    })
    .await;
    let (shown, cursor) = (hit.message(hit.hit).await, hit.message(hit.cursor).await);
    assert_eq!(
        (shown.mailbox_id, cursor.mailbox_id),
        (target, hit.fixture.inbox),
        "{what} in the message opened from a hit: \"Harbor budget\" (the one on screen) is in \
         {:?}, \"Atlas budget\" (the cursor's row) is in {:?}; wanted the first in {target:?} \
         and the second left in the inbox",
        shown.mailbox_id,
        cursor.mailbox_id,
    );
    // What a person sees: the message that left is not the one still shown,
    // and the other is still in the list.
    assert!(
        crate::settle_until(async || {
            hit.window
                .reading()
                .is_none_or(|r| !r.is_open() || r.title() != "Harbor budget")
        })
        .await,
        "{what}: the message is still on screen after it left"
    );
    assert!(
        support::subjects(&hit.window).contains(&"Atlas budget".to_owned()),
        "{what}: the cursor's row vanished: {:?}",
        support::subjects(&hit.window)
    );
}

pub fn e_in_a_message_opened_from_a_hit_archives_that_message() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        leaves_for(
            ("e", gtk::gdk::ModifierType::empty()),
            MailboxRole::Archive,
            "archive",
        )
        .await;
    });
}

pub fn hash_in_a_message_opened_from_a_hit_deletes_that_message() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        leaves_for(
            ("numbersign", gtk::gdk::ModifierType::SHIFT_MASK),
            MailboxRole::Trash,
            "delete",
        )
        .await;
    });
}

pub fn s_in_a_message_opened_from_a_hit_flags_that_message() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let hit = open_a_hit_away_from_the_cursor().await;
        support::keys(&hit.window, &["s"]);
        crate::settle_until(async || {
            let (shown, cursor) = (hit.message(hit.hit).await, hit.message(hit.cursor).await);
            shown.flags.contains(&Flag::Flagged) || cursor.flags.contains(&Flag::Flagged)
        })
        .await;
        let (shown, cursor) = (hit.message(hit.hit).await, hit.message(hit.cursor).await);
        let (on_screen, on_cursor) = (
            shown.flags.contains(&Flag::Flagged),
            cursor.flags.contains(&Flag::Flagged),
        );
        assert_eq!(
            (on_screen, on_cursor),
            (true, false),
            "flag in the message opened from a hit: \"Harbor budget\" (on screen) flagged \
             {on_screen}, \"Atlas budget\" (the cursor's row) flagged {on_cursor}"
        );
    });
}
