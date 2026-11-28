use gtk::prelude::*;
use postio_model::ids::AccountId;
use postio_model::{Draft, MessageBody};

use crate::support_compose::{Window, composer, settle};

pub fn the_native_surface_holds_the_document_it_was_given() {
    let window = Window::default();
    let composer = composer::install_native(&window);
    let html = "<h2>Plan</h2><ul><li><p>milk</p><ul><li><p>oat</p></li></ul></li></ul>\
                <blockquote><p>said</p></blockquote><p>Hello <strong>there</strong></p>";
    let mut draft = Draft::new(AccountId::new(1));
    draft.body = MessageBody {
        text: None,
        html: Some(html.to_owned()),
    };
    composer.open(draft);
    settle();

    let view = composer
        .test_body_widget()
        .downcast::<gtk::TextView>()
        .expect("the body is a native text view");
    let buffer = view.buffer();
    let shown = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
    assert_eq!(
        shown, "Plan\n\u{2022} milk\n\u{25e6} oat\nsaid\nHello there",
        "the buffer shows the document, bullets drawn"
    );
    assert_eq!(
        composer.document(),
        postio_body::parse(html),
        "and holds it as given"
    );
    assert!(
        !composer
            .draft()
            .body
            .html
            .unwrap_or_default()
            .contains('\u{2022}'),
        "a drawn bullet is never in the draft"
    );
}
