//! A reply from a search hit must not count as the message closing, which would bring the search
//! bar back over the composer (#1895).

use gtk::prelude::*;

use crate::support::{self, Fixture};

async fn open_a_hit(
    fixture: &Fixture,
) -> (
    postio_gtk::window::FocusWindow,
    std::rc::Rc<postio_gtk::bar::Bar>,
) {
    fixture
        .file_from_ada("Harbor budget", "Numbers for harbor.", 5)
        .await;
    fixture.index().await;
    let (window, _client) = fixture.open().await;
    support::rows_shown(&window, 1).await;
    let bar = support::open_hit(&window, "harbor", "Harbor budget").await;
    assert!(!bar.is_open(), "opening a hit puts the bar away");
    (window, bar)
}

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
        let fixture = Fixture::empty().await;
        let (window, bar) = open_a_hit(&fixture).await;
        support::keys(&window, &["r"]);
        composer_without_the_bar(&window, &bar, "reply").await;
    });
}
