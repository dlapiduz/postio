//! US1 scenario 7: only the visible window of rows is ever read (FR-014).
//!
//! A mailbox is never loaded into memory: the list is a window over the
//! paged store. Spike S3 (T009) measured what a `GtkListView` asks for --
//! about 205 rows around wherever it is, however far it jumps -- so a jump
//! to the bottom of a long inbox reads the pages around the top and the
//! pages around the bottom, and nothing between. Counted, not timed: the
//! client's round trips by kind, which are the same on any machine.

use postio_ui::list::{CACHE_PAGES, PAGE_SIZE};
use postio_widgets::list_model::WindowedModel;

use crate::support::{self, Fixture};

pub fn a_jump_to_the_bottom_reads_the_ends_and_nothing_between() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::large(6_000).await;
        let (window, client) = fixture.open().await;
        let pane = window.pane().expect("the inbox is showing");
        let list = pane.feed().list().clone();
        let total = pane.feed().total();
        let pages = total.div_ceil(PAGE_SIZE);
        assert!(
            pages >= 30,
            "the fixture is long enough to have a middle: {total} rows"
        );
        let at_the_top = client.counts().of("Page");
        let top = pane.feed().pages_asked();
        assert!(
            top.iter().all(|page| *page < 6),
            "opening reads the pages around the top, not the inbox: {top:?} of {pages}"
        );

        pane.view()
            .scroll_to(total - 1, gtk::ListScrollFlags::NONE, None);
        assert!(
            crate::settle_until(async || list.resident_pages().contains(&(pages - 1))).await,
            "the jump never brought the last page in"
        );
        crate::settle();
        let asked = pane.feed().pages_asked();
        let middle: Vec<u32> = asked
            .iter()
            .copied()
            .filter(|page| (6..pages - 6).contains(page))
            .collect();
        assert!(
            middle.is_empty(),
            "a jump to the bottom reads the pages it shows, not the {} between: \
             it read {middle:?} of them (all asked: {asked:?})",
            pages - 12
        );
        assert_eq!(
            client.counts().of("Page") - at_the_top,
            (asked.len() - top.len()) as u64,
            "every page read is one the list asked for"
        );
        assert!(
            list.resident_rows() <= CACHE_PAGES * PAGE_SIZE as usize,
            "at most {CACHE_PAGES} pages are held: {} rows are",
            list.resident_rows()
        );
        assert_eq!(
            client.counts().of("Body"),
            0,
            "drawing rows reads no body (FR-020): {:?}",
            client.counts().snapshot()
        );
    });
}

/// Spike S3's other half: a jump rebinds about 205 rows in one frame, most
/// of them before their page has landed, so a row with nothing yet draws a
/// skeleton from what it has -- never a blank the eye reads as an empty
/// inbox, and never the last row that widget showed.
pub fn a_row_whose_page_has_not_landed_draws_a_skeleton() {
    use gtk::prelude::*;
    use postio_gtk::list::{RowObject, RowWidget};
    use postio_widgets::list_model::ModelRow;

    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let row = RowWidget::default();
        row.bind(&RowObject::placeholder());
        let window = gtk::Window::new();
        window.set_default_size(800, 40);
        window.set_child(Some(&row));
        window.present();
        assert!(
            crate::settle_until(async || row.is_mapped() && row.width() > 0).await,
            "the row never reached the screen"
        );
        assert_eq!(
            row.spoken(),
            "Loading",
            "a screen reader hears it is loading"
        );
        let paintable = gtk::WidgetPaintable::new(Some(&row));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, f64::from(row.width()), f64::from(row.height()));
        assert!(
            snapshot.to_node().is_some(),
            "a row still waiting for its page draws its skeleton, not nothing"
        );
    });
}

/// #1534: opening a folder costs a screen's worth of pages, whatever it
/// holds, and switching folders faster than the store answers does not leave
/// the list asking forever.
///
/// The list keeps `CACHE_PAGES` resident, so a working set wider than that
/// evicts itself and every evicted position is asked for again: the cache is
/// what turns whatever asks widely into an unbounded loop instead of a slow
/// open. Counted at the client -- every page read is one round trip -- rather
/// than timed, which is the same number on any machine.
pub fn opening_and_switching_large_folders_asks_a_bounded_number_of_pages() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        // Twelve thousand messages leaves the biggest folder far past the
        // eight pages the cache holds, and seeds in a few seconds.
        let fixture = Fixture::large(12_000).await;
        let (window, client) = fixture.open().await;
        let pane = window.pane().expect("the inbox is showing");
        assert!(
            pane.feed().total() > 2_000,
            "the fixture needs a folder far past the cache, got {} rows",
            pane.feed().total()
        );
        let patience = || {
            crate::settle_for(postio_test_support::scaled(
                std::time::Duration::from_millis(1_000),
            ))
        };
        patience().await;
        let budget = CACHE_PAGES as u64;
        let opened = client.counts().of("Page");
        assert!(
            opened <= budget,
            "opening a {}-row folder read {opened} pages, and only {budget} fit \
             in the cache: at least {} were evicted and asked for again",
            pane.feed().total(),
            opened.saturating_sub(budget)
        );

        // Moving through it: the rows the cursor lands on are on screen.
        let before = client.counts().of("Page");
        support::keys(&window, &["j", "j", "j", "j", "j"]);
        patience().await;
        let walked = client.counts().of("Page") - before;
        assert!(
            walked <= 1,
            "five j presses near the top of the folder read {walked} pages"
        );

        // Switching while it is still loading: each switch resets the window,
        // so replies for the folder just left arrive into one that has moved
        // on. No settling between: that is the gesture.
        support::keys(&window, &["g", "o"]);
        let places = window.places().expect("g o opened the folders popover");
        assert!(
            crate::settle_until(async || places.names().len() >= 2).await,
            "the popover listed no folders: {:?}",
            places.names()
        );
        let names = places.names();
        let before = client.counts().of("Page");
        for name in names.iter().cycle().take(8) {
            if !places.is_open() {
                places.open();
            }
            places.set_filter(name);
            places.activate();
            crate::settle();
        }
        patience().await;
        let churned = client.counts().of("Page") - before;
        let switch_budget = 8 * budget;
        assert!(
            churned <= switch_budget,
            "switching folders eight times read {churned} pages, past the \
             {switch_budget} eight screenfuls cost: the list did not settle"
        );
        // The folder the last switch left may be an empty one; the way back
        // to the inbox shows the list still knows where it is.
        support::keys(&window, &["g", "i"]);
        assert!(
            crate::settle_until(async || window.place_name() == "Inbox"
                && window
                    .pane()
                    .is_some_and(|pane| !pane.rows_on_screen().is_empty()))
            .await,
            "after switching the list shows nothing: the app lost track of \
             which folder it is on (place {:?})",
            window.place_name()
        );
    });
}
