use gtk::prelude::*;
use postio_core::{CommandId, Keymap};

use crate::support::{self, Fixture, only, texts};

fn button(root: &impl IsA<gtk::Widget>, class: &str) -> gtk::Button {
    only(root, class)
        .downcast::<gtk::Button>()
        .expect("a button")
}

pub fn the_bulk_bar_has_a_delete_button_that_deletes_the_selection() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        support::keys(&window, &["x", "j", "x", "j"]);
        let bar = only(&window, "focus-bulk-bar");
        let delete = button(&bar, "focus-bulk-delete");
        let key = postio_ui::hints::key(Keymap::defaults(), CommandId::Delete).expect("a key");
        assert!(
            texts(&delete).contains(&key),
            "the button shows Delete's key {key:?}: {:?}",
            texts(&delete)
        );
        assert!(
            texts(&delete).iter().any(|text| text == "Delete"),
            "the button says what it does: {:?}",
            texts(&delete)
        );
        support::click(&window, &delete, 1);
        assert!(
            crate::settle_until(async || support::subjects(&window) == ["Third", "Fourth", "Fifth"])
                .await,
            "the two selected conversations left the inbox: {:?}",
            support::subjects(&window)
        );
    });
}
