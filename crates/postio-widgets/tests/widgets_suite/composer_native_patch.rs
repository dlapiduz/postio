use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;
use postio_body::{DocPosition, Document, EditIntent, project};

use super::composer_native_render::surface;

fn text(surface: &postio_widgets::composer::native::NativeSurface) -> String {
    let buffer = surface.view().buffer();
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), false)
        .to_string()
}

fn place(surface: &postio_widgets::composer::native::NativeSurface, path: &[u32], offset: u32) {
    let document = postio_widgets::composer::surface::ComposerSurface::document(surface);
    let at = project(&document, None).offset_of(&DocPosition::new(path, offset));
    let buffer = surface.view().buffer();
    buffer.place_cursor(&buffer.iter_at_offset(at as i32));
}

pub fn a_keystroke_is_a_patch_of_its_block_and_nothing_else_writes() {
    let lines: Vec<String> = (0..50).map(|n| format!("line {n} of the draft")).collect();
    let surface = surface(Document::from_text(&lines.join("\n\n")));
    let buffer = surface.view().buffer();
    let writes = Rc::new(Cell::new(0u32));
    for signal in ["insert-text", "delete-range"] {
        let writes = writes.clone();
        buffer.connect_local(signal, true, move |_| {
            writes.set(writes.get() + 1);
            None
        });
    }
    place(&surface, &[25], 4);

    assert!(surface.dispatch(EditIntent::Insert("!".into())));
    let stats = surface.last_patch();
    assert_eq!(
        writes.get(),
        stats.operations,
        "every buffer write was the patch's"
    );
    assert!(
        stats.operations <= 2,
        "one keystroke, at most a delete and an insert: {stats:?}"
    );
    assert_eq!(
        (stats.deleted, stats.inserted),
        (0, 1),
        "only the typed character"
    );
    assert!(stats.tags <= 4, "tags for one block only: {stats:?}");
    assert!(text(&surface).contains("line! 25 of"));
    assert_eq!(
        text(&surface),
        project(
            &postio_widgets::composer::surface::ComposerSurface::document(&surface),
            None
        )
        .text(),
        "the buffer is the projection of the document"
    );

    writes.set(0);
    assert!(surface.dispatch(EditIntent::SplitBlock));
    assert_eq!(writes.get(), surface.last_patch().operations);
    assert!(surface.last_patch().operations <= 2);
}
