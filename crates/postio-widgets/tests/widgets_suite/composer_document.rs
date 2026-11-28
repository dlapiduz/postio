use crate::support_compose::Window;
use crate::support_compose::composer;
use postio_model::ids::AccountId;
use postio_model::{Draft, MessageBody};

pub fn the_body_round_trips_through_the_neutral_document() {
    // `open` is a no-op on a composer that is already visible, and a bare widget is.
    let window = Window::default();

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

    // A composer that tidied a reply's blank lines on first read would move the cursor out from
    // under the user.
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

    // A multipart/alternative whose HTML half repeats its text half is bytes nobody asked for.
    assert_eq!(
        composer.draft().body.html,
        None,
        "a plain-text message grew an HTML alternative"
    );

    // The HTML half is regenerated; passing it through left a stale one after editing a reply.
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

    // `postio_model::reply` sits below `postio-body` and quotes only plain text, so an HTML-only
    // message replied with an empty attribution line.
    let window = Window::default();
    let composer = composer::install(&window);
    let source = html_only_message();
    let draft = composer.test_reply_draft(&source).expect("a reply draft");
    let quoted = draft.body.text.expect("a quoted body");

    assert!(
        quoted.contains("> Hello there"),
        "an HTML-only message replied with an empty quote: {quoted:?}"
    );
    assert!(
        quoted.contains("wrote:"),
        "the attribution line went missing: {quoted:?}"
    );
    assert!(
        !quoted.contains('<'),
        "markup reached the composer as markup: {quoted:?}"
    );
}

fn html_only_message() -> postio_model::Message {
    use postio_model::ids::{AccountId, MailboxId};

    let mut message =
        postio_model::Message::new(AccountId::new(1), MailboxId::new(1), chrono::Utc::now());
    message.rfc_message_id = Some(postio_model::ids::RfcMessageId::new(
        "html-only@example.com",
    ));
    message.subject = Some("An HTML message".to_owned());
    message.from = vec![postio_model::EmailAddress::new(
        Some("Ada Lovelace"),
        "ada@example.com",
    )];
    message.body = MessageBody {
        text: None,
        html: Some("<p>Hello <strong>there</strong></p><p>Second line.</p>".to_owned()),
    };
    message
}

/// GTK and the Mac run one editing bridge (spec 014 T001). Two copies of
/// `editor.js` are two dialects of rich text, and the spec changes the
/// bridge, so the one GTK injects must be the one `postio-ui` assembles
/// for every frontend.
pub fn gtk_injects_the_one_bridge_postio_ui_assembles() {
    assert_eq!(
        postio_widgets::composer::editor::editor_script(),
        postio_ui::compose::editor_script(),
        "the GTK composer runs a different editing bridge from the Mac's"
    );
}

/// Spec 014 R14: the rich editor reports its caret as a place in the
/// document -- a block path and a character offset -- and puts it back from
/// one, so switching to the Markdown view and back keeps the person's place
/// even where a word repeats.
pub fn the_caret_crosses_the_bridge_as_a_place_in_the_document() {
    use gtk::prelude::*;
    use postio_body::markdown::DocPosition;
    use postio_widgets::composer::editor::Editor;
    use std::rc::Rc;
    use webkit6::prelude::*;
    if adw::init().is_err() || gtk::gdk::Display::default().is_none() {
        eprintln!("skipping: no display");
        return;
    }
    let window = gtk::Window::new();
    window.set_default_size(600, 400);
    let editor =
        Editor::with_coalesce(Rc::new(|_: &str| None), std::time::Duration::from_millis(1));
    window.set_child(Some(editor.widget()));
    window.present();

    let document = postio_body::markdown::to_document("Some **bold** words\n\nwords again");
    editor.load(document.clone());
    // The second text node, "bold", two characters in.
    editor.test_select(1, 2, 2);
    assert_eq!(
        editor.test_caret(),
        Some(DocPosition {
            path: vec![0],
            offset: 7
        }),
        "the caret in \"bold\" is not reported where it is"
    );
    assert_eq!(
        editor.document(),
        document,
        "reporting the caret edited the body"
    );

    // Put back in the second "words", after "wo": text typed lands there.
    editor.show_at(
        document,
        Some(&DocPosition {
            path: vec![1],
            offset: 2,
        }),
    );
    crate::support_compose::settle_until("the marked page to load", || {
        !editor.widget().is_loading()
            && editor.test_eval(crate::support_compose::EDITOR_LISTENING) == "true"
    });
    editor.test_eval("document.execCommand('insertText', false, 'X'); 'typed'");
    crate::support_compose::settle_until("the typed X to cross the bridge", || {
        editor.document().to_text().contains("woXrds again")
    });
    assert_eq!(
        editor.document().to_text(),
        "Some bold words\n\nwoXrds again",
        "the caret was not put back where the document said"
    );
}
