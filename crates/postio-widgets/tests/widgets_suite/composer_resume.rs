use crate::support_compose::Window;
use crate::support_compose::composer;
use crate::support_compose::settle;
use crate::support_compose::{app, fonts, style};
use gtk::gdk;
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

    composer.open(a_draft(1, "Tide gate interlock", "quinn@example.net"));
    settle();
    composer.test_set_subject("Tide gate interlock, revised");
    settle();
    assert!(
        saves.borrow().is_empty(),
        "the debounce has not elapsed; this is the edit resuming must not lose"
    );

    composer.open(a_draft(2, "Weir gauge", "grace@example.net"));
    settle();
    assert_eq!(
        composer.test_subject(),
        "Tide gate interlock, revised",
        "`c` a second time means show me the draft, never start another"
    );

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

    composer.resume(a_draft(
        1,
        "Tide gate interlock, revised",
        "quinn@example.net",
    ));
    settle();
    assert_eq!(composer.test_subject(), "Tide gate interlock, revised");

    let saved = a_draft(3, "Sluice log", "quinn@example.net");
    composer.resume(saved.clone());
    settle();
    assert_eq!(
        composer.status(),
        postio_ui::compose::saved_at(saved.updated_at),
        "a reopened draft exists in Drafts; it must not claim to be in the \
         composer only"
    );
}

/// A draft can survive the disk and still come back from the WebView round trip with its emphasis
/// flattened and files gone.
pub fn reopening_restores_the_formatting_and_the_attachments_too() {
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
