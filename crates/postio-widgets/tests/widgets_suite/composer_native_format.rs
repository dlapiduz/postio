use gtk::prelude::*;
use postio_model::ids::AccountId;
use postio_model::{Draft, MessageBody};

use crate::support_compose::{Window, composer};

fn place(view: &gtk::TextView, offset: i32) {
    let buffer = view.buffer();
    buffer.place_cursor(&buffer.iter_at_offset(offset));
    postio_widgets::storyboard::deliver::drain();
}

pub fn the_toolbar_lights_what_the_caret_is_in() {
    let window = Window::default();
    window.present();
    let composer = composer::install_native(&window);
    let mut draft = Draft::new(AccountId::new(1));
    draft.body = MessageBody {
        text: None,
        html: Some(
            "<h1>Plan</h1><p>a <strong>bold</strong> b</p><ul><li><p>milk</p></li></ul>".into(),
        ),
    };
    composer.open(draft);
    postio_widgets::storyboard::deliver::drain();
    let view = composer
        .test_body_widget()
        .downcast::<gtk::TextView>()
        .unwrap();

    place(&view, 2);
    assert_eq!(composer.marks_lit(), ["heading_1"], "in the heading");
    place(&view, 8);
    assert_eq!(composer.marks_lit(), ["bold"], "inside the bold word");
    place(&view, 13);
    assert_eq!(
        composer.marks_lit(),
        Vec::<&str>::new(),
        "on the plain word after it"
    );
    place(&view, 18);
    assert_eq!(composer.marks_lit(), ["bullet_list"], "in the list item");

    let buffer = view.buffer();
    buffer.select_range(&buffer.iter_at_offset(12), &buffer.iter_at_offset(14));
    composer.dispatch(postio_core::CommandId::Bold);
    assert_eq!(composer.marks_lit(), ["bold"], "a toggle lights itself");
}
