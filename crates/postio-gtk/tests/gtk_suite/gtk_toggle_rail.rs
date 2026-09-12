//! `⇧I` puts the conversation rail away, and brings it back (#1375).
//!
//! `ConversationView::toggle_rail` has worked since #1374 and `gtk_rail`
//! covers what it does. What nothing covered is that anything *invokes* it:
//! every case there calls the method directly, and the only way a person had
//! to reach it was the footer button. That is the shape `gtk_toggle_sidebar`
//! was written for -- a registry command resolving correctly and then doing
//! nothing, because no `Window::handled_here` arm ever ran it.
//!
//! The second press matters as much as the first. The hide control lives
//! *inside* the rail, so once the rail is away there is nothing on screen to
//! bring it back and the key is the only route there is. A binding that hid
//! the rail and could not restore it would make hiding irreversible for the
//! rest of the session.
//!
//! Why `⇧I` and not the `⇧R` screen 28 draws: `R` is `Refresh`'s alternate on
//! every message surface, so the brief's key would shadow "check for new mail
//! now" precisely where the rail lives. Maintainer's call, on #1375.
//!
//! Skips without a display. Nothing here touches the network.

use std::time::{Duration, Instant};

use gtk::gdk;
use gtk::prelude::*;
use postio_core::Context;
use postio_gtk::feed::{
    MailboxFuture, MailboxSource, MessageSource, Page, PageFuture, PageRequest,
};
use postio_gtk::list::Row;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::EmailAddress;
use postio_model::ids::{AccountId, MailboxId, MessageId, ThreadId};
use postio_model::mailbox::{Mailbox, MailboxCounts, MailboxRole};

/// One thread of six. Six rather than two because the rail is what is under
/// test: `presentation` gives no rail to a conversation of one, and a longer
/// thread is what the column is for.
struct SixMessageThread;

impl MessageSource for SixMessageThread {
    fn fetch(&self, request: PageRequest) -> PageFuture {
        let rows: Vec<Row> = (1..=6)
            .map(|index| Row {
                id: MessageId::new(index),
                thread: Some(ThreadId::new(1)),
                from: Some(EmailAddress::new(
                    Some("Ada Norwood".to_owned()),
                    "ada@example.com".to_owned(),
                )),
                subject: Some("Tide gate interlock".to_owned()),
                preview: Some("\u{2026}".to_owned()),
                received_at: chrono::Utc::now(),
                seen: true,
                flagged: false,
                answered: false,
                send_state: None,
                has_attachments: false,
                thread_count: 6,
                participants: Vec::new(),
            })
            .collect();
        let start = (request.offset as usize).min(rows.len());
        let end = (start + request.limit as usize).min(rows.len());
        let page = rows[start..end].to_vec();
        let total = rows.len() as u32;
        Box::pin(async move { Ok(Page { total, rows: page }) })
    }
}

impl MailboxSource for SixMessageThread {
    fn mailboxes(&self, account: AccountId) -> MailboxFuture {
        let mut inbox = Mailbox::new(account, "INBOX", Some('/'));
        inbox.id = MailboxId::new(1);
        inbox.role = MailboxRole::Inbox;
        inbox.counts = MailboxCounts {
            total: 6,
            unread: 0,
            flagged: 0,
            snoozed: 0,
            attention: 0,
        };
        Box::pin(async move { Ok(vec![inbox]) })
    }
}

fn settle(window: &Window, what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + postio_test_support::scaled(Duration::from_secs(20));
    while Instant::now() < deadline {
        crate::pump();
        if done() {
            return;
        }
    }
    panic!(
        "timed out waiting for {what}; window context {:?}",
        window.context()
    );
}

fn press_shifted(window: &Window, key: &str) {
    window.handle_key(
        gdk::Key::from_name(key).unwrap(),
        gdk::ModifierType::SHIFT_MASK,
    );
    crate::pump();
}

pub fn shift_i_puts_the_rail_away_and_brings_it_back() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    window.set_default_size(1400, 900);
    let account = AccountId::new(1);
    let source = std::rc::Rc::new(SixMessageThread);
    let _feeds = window.install_feeds(account, "lena@example.com", source.clone(), source);
    window.present();
    crate::pump();
    settle(&window, "the list to have a row to land on", || {
        window.list().model().n_items() > 0
    });

    // The real gesture: landing on a thread row fills the reading pane, and
    // the rail is drawn as part of that. Building a pane by hand instead
    // would leave it unmounted, and an unmounted rail is invisible for
    // reasons that have nothing to do with the key.
    window.list().first_row();
    let row = window.list().cursor_row().expect("a row to land on");
    window.open_conversation(&row);
    window.set_context(Context::Conversation);
    crate::pump();

    let pane = window.conversation();
    settle(&window, "the rail to be drawn", || {
        pane.rail().widget().is_visible()
    });

    // The control: there is a rail to put away in the first place.
    assert!(
        !pane.rail_hidden() && pane.rail().widget().is_visible(),
        "the rail should be up before anything hides it, or the assertions \
         below pass against a rail that was never there"
    );

    press_shifted(&window, "I");
    assert!(
        pane.rail_hidden() && !pane.rail().widget().is_visible(),
        "shift-I did not put the rail away -- if the command resolved but no \
         `handled_here` arm ran it, this is exactly what that looks like"
    );

    press_shifted(&window, "I");
    assert!(
        !pane.rail_hidden() && pane.rail().widget().is_visible(),
        "shift-I did not bring the rail back. The hide control lives inside \
         the rail, so this key is the only way back and hiding would be \
         irreversible for the session"
    );

    window.destroy();
}
