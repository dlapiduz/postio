use gtk::AccessibleRole;
use gtk::prelude::*;
use postio_body::{Block, ContentId, Document, Inline};
use postio_model::ids::AccountId;
use postio_model::{Draft, MessageBody};

use super::composer_native_input::shown;
use crate::support_compose::{Window, composer};

pub fn the_body_is_a_named_text_box_with_gtks_accessible_text() {
    let shown = shown(Document {
        blocks: vec![
            Block::Paragraph(vec![
                Inline::Text("a".into()),
                Inline::Image {
                    content_id: ContentId::parse("cat@example.com").unwrap(),
                    alt: "a cat".into(),
                },
            ]),
            Block::Rule,
            Block::Paragraph(Vec::new()),
        ],
    });
    let view = shown.surface.view();
    assert_eq!(view.accessible_role(), AccessibleRole::TextBox);
    assert!(
        view.is::<gtk::AccessibleText>(),
        "text, caret and selection reach a screen reader through GtkAccessibleText"
    );
    let buffer = view.buffer();
    let object = |offset: i32| {
        buffer
            .iter_at_offset(offset)
            .child_anchor()
            .and_then(|anchor| anchor.widgets().first().cloned())
            .unwrap_or_else(|| panic!("an object at {offset}"))
    };
    let picture = object(1).downcast::<gtk::Picture>().expect("the image");
    assert_eq!(picture.alternative_text().as_deref(), Some("a cat"));
    assert_eq!(
        object(3).accessible_role(),
        AccessibleRole::Separator,
        "the rule"
    );
}

pub fn every_formatting_command_on_the_toolbar_has_a_name() {
    let window = Window::default();
    window.present();
    let composer = composer::install_native(&window);
    let mut draft = Draft::new(AccountId::new(1));
    draft.body = MessageBody {
        text: Some("hello".into()),
        html: None,
    };
    composer.open(draft);
    let toolbar = crate::support_compose::find(composer.upcast_ref(), "postio-compose-toolbar")
        .expect("the toolbar");
    let mut child = toolbar.first_child();
    let mut named = 0;
    while let Some(button) = child {
        if button.is::<gtk::ToggleButton>() || button.is::<gtk::Button>() {
            assert!(button.tooltip_text().is_some(), "{button:?} has no name");
            named += 1;
        }
        child = button.next_sibling();
    }
    assert!(named >= 6, "the toolbar's commands were found");
}
