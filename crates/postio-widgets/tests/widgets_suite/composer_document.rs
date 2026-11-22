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
