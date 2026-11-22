//! Download and attach, at the composer (spec 014 US8,
//! `contracts/remote-image.md`, "The composer"): the rows under the body,
//! the press, what each outcome says, and remove-to-link. The fetch is a
//! fake handler that counts, so nothing reaches the network.

use std::cell::RefCell;
use std::rc::Rc;

use crate::support_compose::{Window, app, fonts, settle, style};
use gtk::gdk;
use postio_body::markdown::to_document;
use postio_core::CommandId;
use postio_model::{AccountId, Attachment, Draft};
use postio_ui::compose::RemoteRefusal;
use postio_widgets::composer::{Composer, RemoteImageReady};

type Pending = Rc<RefCell<Vec<(String, RemoteImageReady)>>>;

fn ready() -> Option<(Window, Composer, Pending)> {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display");
        return None;
    }
    let display = gdk::Display::default().expect("a display");
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);
    let window = Window::default();
    let composer = window.composer();
    window.present();
    let pending: Pending = Rc::default();
    composer.connect_remote_image({
        let pending = Rc::clone(&pending);
        move |_draft, href, then| pending.borrow_mut().push((href.as_str().to_owned(), then))
    });
    Some((window, composer, pending))
}

fn with_chips(composer: &Composer, markdown: &str) {
    let mut draft = Draft::new(AccountId::new(1));
    draft.body = postio_body::message_body(&to_document(markdown));
    composer.resume(draft);
    settle();
}

fn a_png_part() -> Attachment {
    let mut part = Attachment::new(
        postio_model::ids::MessageId::UNASSIGNED,
        "image/png",
        40_960,
    );
    part.content_id = Some("x1@postio.invalid".to_owned());
    part.disposition = postio_model::attachment::Disposition::Inline;
    part
}

/// The rows, the warning before Send, and no request until the press.
pub fn a_remote_image_is_a_row_and_nothing_is_fetched_until_the_press() {
    let Some((_window, composer, pending)) = ready() else {
        return;
    };
    with_chips(&composer, "See ![chart](https://example.com/x.png).");
    assert_eq!(
        composer.test_remote_rows(),
        vec![("chart".to_owned(), "example.com".to_owned())]
    );
    assert_eq!(
        composer.test_remote_warning().as_deref(),
        Some("1 image isn't attached and will be sent as a link \u{b7} ctrl+alt+i to attach")
    );
    composer.save();
    settle();
    assert!(
        pending.borrow().is_empty(),
        "something but the press asked for the image"
    );

    composer.dispatch(CommandId::AttachRemoteImage);
    settle();
    assert_eq!(pending.borrow().len(), 1, "one press is one request");
    assert_eq!(pending.borrow()[0].0, "https://example.com/x.png");
    assert_eq!(composer.test_remote_rows()[0].1, "Downloading\u{2026}");
}

/// A picture that arrives takes the chip's place, and is an attachment.
pub fn an_attached_image_takes_the_chips_place() {
    let Some((_window, composer, pending)) = ready() else {
        return;
    };
    with_chips(&composer, "See ![chart](https://example.com/x.png).");
    composer.dispatch(CommandId::AttachRemoteImage);
    settle();
    let (_, then) = pending.borrow_mut().pop().expect("a request");
    then(Ok(a_png_part()));
    settle();
    assert!(
        composer.test_remote_rows().is_empty(),
        "the row is still there"
    );
    assert_eq!(composer.test_remote_warning(), None);
    let html = composer.draft().body.html.expect("an HTML body");
    assert!(
        html.contains("<img src=\"cid:x1@postio.invalid\" alt=\"chart\">"),
        "{html}"
    );
    assert_eq!(composer.draft().attachments.len(), 1);
}

/// A refusal says why on the row, and the image stays a link.
pub fn a_refusal_says_why_and_the_image_stays_a_link() {
    let Some((_window, composer, pending)) = ready() else {
        return;
    };
    with_chips(&composer, "See ![chart](https://example.com/x.png).");
    for (refusal, words) in [
        (
            RemoteRefusal::TooLarge,
            "Larger than 10 MB \u{b7} not attached \u{b7} it will be sent as a link",
        ),
        (
            RemoteRefusal::Unreachable("it answered 404".into()),
            "Couldn't download from example.com \u{b7} it answered 404 \u{b7} Try again ctrl+alt+i",
        ),
    ] {
        composer.dispatch(CommandId::AttachRemoteImage);
        settle();
        let (_, then) = pending.borrow_mut().pop().expect("a request");
        then(Err(refusal));
        settle();
        assert_eq!(composer.test_remote_rows()[0].1, words);
        let html = composer.draft().body.html.expect("an HTML body");
        assert!(
            html.contains("<a href=\"https://example.com/x.png\""),
            "{html}"
        );
        assert!(!html.contains("<img"), "{html}");
    }
    assert!(
        pending.borrow().is_empty(),
        "a failure was retried on its own"
    );
}

/// Remove sends it as the link it would be, and stops offering it.
pub fn remove_turns_the_chip_into_a_link() {
    let Some((_window, composer, pending)) = ready() else {
        return;
    };
    with_chips(&composer, "See ![chart](https://example.com/x.png).");
    composer.test_unlink_remote(0);
    settle();
    assert!(composer.test_remote_rows().is_empty());
    let html = composer.draft().body.html.expect("an HTML body");
    assert!(
        html.contains("<a href=\"https://example.com/x.png\">chart</a>"),
        "{html}"
    );
    assert!(pending.borrow().is_empty());
}

/// With no image in the body, the press says so and asks for nothing.
pub fn a_press_with_no_image_says_so() {
    let Some((_window, composer, pending)) = ready() else {
        return;
    };
    with_chips(&composer, "Just words.");
    composer.dispatch(CommandId::AttachRemoteImage);
    settle();
    assert_eq!(composer.status(), postio_ui::compose::NO_REMOTE_IMAGE);
    assert!(pending.borrow().is_empty());
}
