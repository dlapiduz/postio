use gtk::prelude::*;
use postio_body::{DocPosition, Document, EditIntent, project};
use postio_model::ids::AccountId;
use postio_model::{Draft, MessageBody};
use postio_widgets::composer::native::NativeSurface;
use postio_widgets::composer::surface::ComposerSurface;

use crate::support_compose::{Window, composer, settle};

fn five_thousand_words() -> String {
    (0..500)
        .map(|n| format!("paragraph {n} holds ten words of a long draft here"))
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn a_keystroke_in_five_thousand_words_touches_one_block() {
    let surface = NativeSurface::new(std::rc::Rc::new(|_: &str| None));
    surface.load(Document::from_text(&five_thousand_words()));
    assert_eq!(
        surface.document().to_text().split_whitespace().count(),
        5000
    );
    let at = project(&surface.document(), None).offset_of(&DocPosition::new([250], 9));
    let buffer = surface.view().buffer();
    buffer.place_cursor(&buffer.iter_at_offset(at as i32));

    for (intent, blocks) in [
        (EditIntent::Insert("x".into()), 1),
        (EditIntent::DeleteBackward(postio_body::Unit::Char), 1),
        (EditIntent::SplitBlock, 2),
        (EditIntent::DeleteBackward(postio_body::Unit::Char), 1),
    ] {
        assert!(surface.dispatch(intent.clone()));
        let stats = surface.last_patch();
        assert!(
            stats.blocks <= blocks,
            "{intent:?} rewrote {} blocks",
            stats.blocks
        );
        assert!(stats.operations <= 2, "{intent:?}: {stats:?}");
        assert!(
            stats.deleted + stats.inserted <= 2,
            "{intent:?} wrote only what changed: {stats:?}"
        );
        assert!(
            stats.tags <= 4,
            "{intent:?} tagged one block's runs: {stats:?}"
        );
    }
}

pub fn a_long_draft_opens_with_no_web_process() {
    let window = Window::default();
    window.present();
    let composer = composer::install_native(&window);
    let mut draft = Draft::new(AccountId::new(1));
    draft.body = MessageBody {
        text: Some(five_thousand_words()),
        html: None,
    };
    composer.open(draft);
    settle();
    fn engines(widget: &gtk::Widget) -> usize {
        let mut count = usize::from(widget.type_().name().starts_with("WebKit"));
        let mut child = widget.first_child();
        while let Some(current) = child {
            count += engines(&current);
            child = current.next_sibling();
        }
        count
    }
    assert_eq!(
        engines(composer.upcast_ref()),
        0,
        "no browser engine in the composer"
    );
    let view = composer
        .test_body_widget()
        .downcast::<gtk::TextView>()
        .unwrap();
    assert_eq!(
        view.buffer().line_count(),
        500,
        "every paragraph one line of the buffer"
    );
}
