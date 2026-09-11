//! Resuming a draft picked out of the Drafts folder.
//!
//! Its own file: GTK is single-threaded and initialised once, so one `#[test]`
//! per integration binary. See `gtk_composer.rs`.
//!
//! [`Composer::open`] deliberately refuses to replace what it is holding —
//! `c` a second time means "show me the draft", never "start another", which
//! is the one-composition-at-a-time rule. Resuming is the other request:
//! *this* draft, named, chosen out of a folder. It has to replace, and it may,
//! because a retained draft is autosaved and — since #166 — is itself a row in
//! that folder. Nothing is lost by swapping to another one and back.
//!
//! What this cannot prove is where the draft came from; that is `postio-app`'s
//! `tests/resume_draft.rs`, which activates a real row over a real store.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle;
use gtk::gdk;
use postio_gtk::composer;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::{AccountId, Draft, DraftId, EmailAddress};

fn a_draft(id: i64, subject: &str, to: &str) -> Draft {
    let mut draft = Draft::new(AccountId::UNASSIGNED);
    draft.id = DraftId::new(id);
    draft.subject = subject.to_owned();
    draft.to = vec![EmailAddress::new(None::<String>, to)];
    draft.body.text = Some(format!("About {subject}."));
    draft
}

pub fn resuming_replaces_the_draft_the_composer_was_holding() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

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
    let saves: std::rc::Rc<std::cell::RefCell<Vec<Draft>>> = Default::default();
    composer.connect_save({
        let saves = std::rc::Rc::clone(&saves);
        move |draft| saves.borrow_mut().push(draft.clone())
    });

    // Something in the composer, with an edit that has not been autosaved yet.
    composer.open(a_draft(1, "Tide gate interlock", "quinn@example.net"));
    settle();
    composer.test_set_subject("Tide gate interlock, revised");
    settle();
    assert!(
        saves.borrow().is_empty(),
        "the debounce has not elapsed; this is the edit resuming must not lose"
    );

    // ── `open` holds on, which is the rule resume is the exception to ────
    composer.open(a_draft(2, "Weir gauge", "grace@example.net"));
    settle();
    assert_eq!(
        composer.test_subject(),
        "Tide gate interlock, revised",
        "`c` a second time means show me the draft, never start another"
    );

    // ── resume replaces it, and flushes what was pending first ───────────
    composer.resume(a_draft(2, "Weir gauge", "grace@example.net"));
    settle();

    assert!(
        saves
            .borrow()
            .iter()
            .any(|draft| draft.subject == "Tide gate interlock, revised"),
        "swapping drafts must flush the pending edit rather than leave it in a \
         timer that is about to fire against the wrong draft"
    );
    assert_eq!(composer.test_subject(), "Weir gauge");
    assert!(composer.is_open());

    // ── and the one it swapped away from can be resumed back ─────────────
    composer.resume(a_draft(
        1,
        "Tide gate interlock, revised",
        "quinn@example.net",
    ));
    settle();
    assert_eq!(composer.test_subject(), "Tide gate interlock, revised");
}

/// #1196: `c` on a *closed* composer starts a blank message.
///
/// This is where #691's guarantee lives now. `composer::opening` used to
/// decide between the kept draft and the asked-for one, and three unit tests
/// held its exceptions in place; it returns `Fill` unconditionally since
/// #1196, so asserting on it proves nothing any more. What can still fail is
/// the widget.
///
/// Note what does *not* change: `c` while the composer is already open is
/// still "show me the draft" — that is `open`'s own first branch, and the
/// case above asserts it. The rule was always about the keyboard; it had
/// grown to cover the draft as well.
pub fn composing_after_a_kept_draft_starts_blank() {
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

    composer.open(a_draft(
        1,
        "Thank you for your enrollment",
        "quinn@example.net",
    ));
    settle();

    // Escape keeps it. That is the autosave's doing, not the restore's --
    // which is the whole reason the restore could go.
    assert_eq!(composer.close(), composer::Closing::Keep);
    settle();

    composer.open(Draft::new(AccountId::UNASSIGNED));
    settle();

    assert_eq!(
        composer.test_subject(),
        "",
        "compose means a new message. It used to reopen the last draft, \
         which reads as the composer refusing to start one"
    );

    // Nothing was lost by starting fresh: the displaced draft is a row in
    // Drafts, and naming it brings it back.
    composer.resume(a_draft(
        1,
        "Thank you for your enrollment",
        "quinn@example.net",
    ));
    settle();
    assert_eq!(
        composer.test_subject(),
        "Thank you for your enrollment",
        "the draft compose displaced is still resumable by name"
    );
}

/// #1240: two unsaved drafts are not the same draft.
///
/// `resume` decides it is being handed the draft it already has by comparing
/// ids — and every unsaved draft carries `DraftId::UNASSIGNED`, so
/// `UNASSIGNED == UNASSIGNED` read as "you are already looking at this" and
/// it silently declined to swap.
///
/// Nothing in production hits it: `resume` exists for the Drafts folder and
/// a row there always has a real id. It cost real confusion twice in this
/// suite, though, which is the shape of the bug — a guard that silently
/// declines leaves the caller believing it swapped and fails an assertion
/// three steps later.
pub fn resuming_an_unsaved_draft_over_another_unsaved_one_replaces_it() {
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

    let unsaved = |subject: &str| {
        let mut draft = Draft::new(AccountId::UNASSIGNED);
        draft.subject = subject.to_owned();
        draft
    };

    composer.open(unsaved("the first one"));
    settle();
    assert_eq!(composer.test_subject(), "the first one");

    composer.resume(unsaved("the second one"));
    settle();
    assert_eq!(
        composer.test_subject(),
        "the second one",
        "two unsaved drafts both carry UNASSIGNED; that is the absence of an \
         identity, not evidence they are the same draft"
    );

    // And the guard still does its job where an id means something: asking
    // for the draft already open returns the keyboard and replaces nothing.
    let named = a_draft(7, "named", "quinn@example.net");
    composer.resume(named.clone());
    settle();
    composer.test_set_subject("edited since");
    settle();
    composer.resume(named);
    settle();
    assert_eq!(
        composer.test_subject(),
        "edited since",
        "resuming the draft already open must not throw away an edit"
    );
}

/// FR-064: reopening restores text, formatting, recipients **and
/// attachments**.
///
/// The two the existing cases above do not cover, and the two most worth
/// covering. `postio-storage` round-trips both through the database, and the
/// composer round-trips the body through a WebView and a parse — so a draft
/// can survive the disk perfectly and still come back to the person with its
/// emphasis flattened and its files gone, with every storage test green.
/// That is the join, and it is what this asserts.
pub fn reopening_restores_the_formatting_and_the_attachments_too() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display");
        return;
    }
    let display = gdk::Display::default().expect("a display");
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    let composer = window.composer();
    window.present();

    let mut draft = Draft::new(AccountId::new(1));
    draft.id = DraftId::new(41);
    draft.subject = "the weir gauge".to_owned();
    draft.to = vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")];
    draft.cc = vec![EmailAddress::new(None::<String>, "list@example.org")];
    draft.body = postio_model::MessageBody {
        text: Some("The reading is high.".to_owned()),
        html: Some("<p>The reading is <strong>high</strong>.</p>".to_owned()),
    };
    let mut part = postio_model::Attachment::new(
        postio_model::ids::MessageId::UNASSIGNED,
        "application/pdf",
        4_096,
    );
    part.filename = Some("gauge.pdf".to_owned());
    draft.attachments = vec![part];

    composer.resume(draft.clone());
    settle();

    // ── On screen ────────────────────────────────────────────────────────
    assert_eq!(composer.test_subject(), "the weir gauge");
    assert_eq!(
        composer.test_attachment_count(),
        1,
        "the attachment row is not showing, so the person cannot tell the \
         file is still on the draft"
    );
    assert!(
        composer.test_attachments_visible(),
        "the attachment list is hidden on a draft that has one"
    );

    // ── And in what would be sent ────────────────────────────────────────
    let reopened = composer.draft();
    assert_eq!(
        reopened.attachments.len(),
        1,
        "the attachment survived the row and not the draft: {:?}",
        reopened.attachments
    );
    assert_eq!(
        reopened.attachments[0].filename.as_deref(),
        Some("gauge.pdf")
    );
    assert_eq!(reopened.to.len(), 1, "a recipient was lost");
    assert_eq!(reopened.cc.len(), 1, "a Cc was lost");

    let html = reopened.body.html.unwrap_or_default();
    assert!(
        html.contains("<strong>") || html.contains("<b>"),
        "the emphasis was flattened by the round trip through the editor, so \
         reopening a draft quietly rewrites it: {html:?}"
    );
    assert!(
        html.contains("high"),
        "the emphasised words did not survive at all: {html:?}"
    );
}
