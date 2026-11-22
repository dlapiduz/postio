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

/// `c` on a closed composer starts blank (#1196); `c` while open still means "show me the draft".
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

/// `resume` compared ids and every unsaved draft is `DraftId::UNASSIGNED`, so it silently declined
/// to swap (#1240).
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

/// A draft can survive the disk and still come back from the WebView round trip with its emphasis
/// flattened and files gone.
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
