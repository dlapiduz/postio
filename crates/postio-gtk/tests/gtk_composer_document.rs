//! The composer's body is a document, not a `GtkTextBuffer`.
//!
//! Issue #30's second acceptance criterion, and the reason it is P0 for a
//! change that ships no feature: a rich text editor is the least portable
//! widget in any cross-platform application. `GtkTextBuffer`, `NSTextStorage`
//! and a `contenteditable` DOM disagree about what a rich text document *is*,
//! so a composer whose state is "whatever is in the buffer" makes a second
//! frontend's composer a rewrite rather than a port — and makes the two
//! produce different HTML from the same gestures.
//!
//! Its own file with one test function: GTK is initialised once, per process,
//! from one thread.

use postio_gtk::composer;
use postio_gtk::window::Window;
use postio_model::ids::AccountId;
use postio_model::{Draft, MessageBody};

#[test]
fn the_body_round_trips_through_the_neutral_document() {
    if adw::init().is_err() || gtk::gdk::Display::default().is_none() {
        eprintln!("skipping: no display");
        return;
    }
    // Mounted into a window, like every other composer test: `open` is a
    // no-op on a composer that is already visible, and a bare widget is.
    let window = Window::default();

    // ── an HTML-only message is editable, which it was not before ───────
    // `reply.rs` said "HTML-only content is not quoted" because there was
    // nothing to turn markup into text with. There is now.
    let composer = composer::install(&window);
    let mut draft = Draft::new(AccountId::new(1));
    draft.body = MessageBody {
        text: None,
        html: Some(
            "<p>Hello <strong>there</strong></p>\
             <blockquote><p>they wrote this</p></blockquote>"
                .to_owned(),
        ),
    };
    composer.open(draft);

    let document = composer.document();
    let text = document.to_text();
    assert!(
        text.contains("Hello there"),
        "an HTML body did not reach the editor as text: {text:?}"
    );
    assert!(
        text.contains("> they wrote this"),
        "the quote did not survive as a quote: {text:?}"
    );

    // ── the user's whitespace is theirs ─────────────────────────────────
    // A reply opens with blank lines above the signature to type into. A
    // composer that tidied them on the first read would move the cursor out
    // from under the user.
    let composer = composer::install(&window);
    let mut draft = Draft::new(AccountId::new(1));
    let typed = "\n\nThanks.\n\n-- \nLena\n";
    draft.body = MessageBody {
        text: Some(typed.to_owned()),
        html: None,
    };
    composer.open(draft);

    assert_eq!(
        composer.draft().body.text.as_deref(),
        Some(typed),
        "the composer normalised the body out from under the user"
    );

    // ── a plain message stays plain on the wire ─────────────────────────
    // A multipart/alternative whose HTML half says exactly what its text
    // half says is bytes nobody asked for, and what mailing lists ask
    // people not to send.
    assert_eq!(
        composer.draft().body.html,
        None,
        "a plain-text message grew an HTML alternative"
    );

    // ── and the HTML half is regenerated, never stale ────────────────────
    // Before this the html field was passed through from whatever the draft
    // was opened with, so editing the text of a reply left an HTML half
    // describing the text *before* the edit.
    let composer = composer::install(&window);
    let mut draft = Draft::new(AccountId::new(1));
    draft.body = MessageBody {
        text: None,
        html: Some("<p>original</p>".to_owned()),
    };
    composer.open(draft);
    composer.test_set_body("replaced");

    let body = composer.draft().body;
    assert_eq!(body.text.as_deref(), Some("replaced"));
    assert!(
        !body.html.is_some_and(|html| html.contains("original")),
        "the HTML half still described the text before the edit"
    );
}
