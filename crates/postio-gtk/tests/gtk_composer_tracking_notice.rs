//! Issue #116: a reply that quotes an HTML-only message says so when the
//! quoted text links to a domain other than the sender's own — on a real
//! display, since this is the composer's own widget being shown and hidden,
//! not just the pure domain-comparison logic `src/composer.rs` already
//! unit-tests with no display.
//!
//! Its own file: GTK is single-threaded and initialised once, so one
//! `#[test]` per integration binary. See `gtk_composer.rs`.

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::composer;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::{Account, AccountId, EmailAddress, Message, MessageBody};

fn settle() {
    while glib::MainContext::default().iteration(false) {}
}

fn press(window: &Window, key: &str) {
    window.handle_key(
        gdk::Key::from_name(key).unwrap(),
        gdk::ModifierType::empty(),
    );
    settle();
}

fn an_account() -> Account {
    let mut account = Account::new(
        "Test",
        EmailAddress::new(None::<String>, "grace@example.com"),
    );
    account.id = AccountId::new(1);
    account
}

fn html_only_message(from_domain: &str, link_host: &str) -> Message {
    let mut message = Message::new(
        AccountId::new(1),
        postio_model::ids::MailboxId::new(1),
        chrono::Utc::now(),
    );
    message.id = postio_model::ids::MessageId::new(7);
    message.from = vec![EmailAddress::new(
        Some("Cooperage Supply"),
        format!("orders@{from_domain}"),
    )];
    message.to = vec![EmailAddress::new(None::<String>, "grace@example.com")];
    message.subject = Some("Your order".to_owned());
    message.body = MessageBody {
        text: None,
        html: Some(format!(
            "<p><a href=\"https://{link_host}/r?c=aa71\">Shop now</a></p>"
        )),
    };
    message
}

/// Depth-first search of a widget tree for the first one carrying `class`.
fn find(widget: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
    if widget.has_css_class(class) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(found) = find(&current, class) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}

fn notice(composer: &postio_gtk::composer::Composer) -> gtk::Label {
    find(
        composer.clone().upcast_ref::<gtk::Widget>(),
        "postio-compose-tracking-notice",
    )
    .expect("the composer builds the notice label unconditionally")
    .downcast()
    .expect("the notice is a label")
}

#[test]
fn replying_to_a_tracking_link_shows_the_notice_and_a_same_domain_link_does_not() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let window = Window::default();
    window.present();
    settle();

    let composer = composer::install(&window);
    let notice = notice(&composer);
    assert!(
        !notice.is_visible(),
        "nothing has been replied to yet, so there is nothing to say"
    );

    // ── a foreign tracking link is flagged ─────────────────────────────────
    composer.connect_reply_source({
        let source = html_only_message("shop.example.org", "click.tracker.example.org");
        move || Some((source.clone(), an_account()))
    });
    press(&window, "e");
    assert!(composer.is_open());
    assert!(notice.is_visible());
    assert!(
        notice.text().contains("click.tracker.example.org"),
        "{}",
        notice.text()
    );
    composer.discard();
    settle();
    assert!(
        !notice.is_visible(),
        "discarding the reply must not leave a stale notice for the next composition"
    );

    // ── a link to the sender's own domain is not flagged ───────────────────
    composer.connect_reply_source({
        let source = html_only_message("shop.example.org", "shop.example.org");
        move || Some((source.clone(), an_account()))
    });
    press(&window, "e");
    assert!(composer.is_open());
    assert!(
        !notice.is_visible(),
        "a link to the sender's own domain is not a foreign tracker"
    );
}
