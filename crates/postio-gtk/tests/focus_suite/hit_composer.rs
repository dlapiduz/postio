//! #1895 on the GTK app: a message opened from a search hit, then answered
//! or forwarded, puts the composer over it with the search bar nowhere in
//! sight and the keyboard in the composer.
//!
//! A hit puts the bar away and holds its words to bring the bar back when
//! the message closes. A reply must not count as the message closing. The
//! shared controller got the fix (postio-focus `bar.rs`); GTK's bar has its
//! own held words and its own reopening (`window::reading_changed`), so the
//! window is what is asserted on: what a person sees, not what a layer
//! was handed.

use gtk::prelude::*;

use crate::support::{self, Fixture};

/// Search "harbor", open the first hit and wait for it to be read.
async fn open_a_hit(
    fixture: &Fixture,
) -> (
    postio_gtk::window::FocusWindow,
    std::rc::Rc<postio_gtk::bar::Bar>,
) {
    fixture
        .file(
            ("Ada Moreno", "ada@example.com"),
            "Harbor budget",
            "Numbers for harbor.",
            5,
        )
        .await;
    fixture.index().await;
    let (window, _client) = fixture.open().await;
    assert!(
        crate::settle_until(async || support::subjects(&window).len() == 1).await,
        "the inbox never reached the screen"
    );
    support::press(&window, "slash", gtk::gdk::ModifierType::empty());
    let bar = window.bar().expect("/ opened the bar");
    for end in 1..="harbor".len() {
        bar.set_text(&"harbor"[..end]);
        crate::settle();
    }
    assert!(
        crate::settle_until(async || !bar.result_subjects().is_empty()).await,
        "the search found nothing: {:?}",
        bar.texts()
    );
    bar.run_search();
    let chord: postio_ui::keymap::Chord = "Return".parse().expect("a chord");
    postio_widgets::storyboard::deliver::press(window.upcast_ref(), &chord)
        .expect("a deliverable chord");
    crate::settle();
    assert!(
        crate::settle_until(async || window.reading().is_some_and(|r| r.is_open())).await,
        "the hit never opened"
    );
    assert!(!bar.is_open(), "opening a hit puts the bar away");
    (window, bar)
}

/// Whether anything of the search bar is on screen: its dropdown or its
/// entry.
fn bar_showing(bar: &postio_gtk::bar::Bar) -> bool {
    bar.is_open() || bar.input().is_mapped() || bar.widget().is_visible() && bar.hints() > 0
}

async fn composer_without_the_bar(
    window: &postio_gtk::window::FocusWindow,
    bar: &postio_gtk::bar::Bar,
    what: &str,
) {
    assert!(
        crate::settle_until(async || window.compose_dialog().is_some()).await,
        "{what} from the hit opened no composer"
    );
    // Whatever would come back does so within a moment of the composer.
    crate::settle_for(postio_test_support::scaled(
        std::time::Duration::from_millis(400),
    ))
    .await;
    assert!(
        window.compose_dialog().is_some(),
        "the composer is still up"
    );
    assert!(
        !bar_showing(bar),
        "the search bar came back over the composer after {what}: open {}, \
         texts {:?}",
        bar.is_open(),
        bar.texts()
    );
    let dialog = window.compose_dialog().expect("the composer");
    let focus = gtk::prelude::GtkWindowExt::focus(window)
        .unwrap_or_else(|| panic!("nothing has the keyboard after {what}"));
    assert!(
        focus.is_ancestor(&dialog),
        "the keyboard is not in the composer after {what}: {}",
        support::focus_path(window)
    );
}

pub fn replying_from_a_hit_leaves_the_composer_clear_of_the_search_bar() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, bar) = open_a_hit(&fixture).await;
        support::keys(&window, &["r"]);
        composer_without_the_bar(&window, &bar, "reply").await;
    });
}

pub fn forwarding_from_a_hit_leaves_the_composer_clear_of_the_search_bar() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, bar) = open_a_hit(&fixture).await;
        support::keys(&window, &["f"]);
        composer_without_the_bar(&window, &bar, "forward").await;
    });
}
