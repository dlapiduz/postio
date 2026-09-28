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
        assert!(
            at_the_top <= 6,
            "opening reads the pages around the top, not the inbox: {at_the_top} of {pages}"
        );

        pane.view()
            .scroll_to(total - 1, gtk::ListScrollFlags::NONE, None);
        assert!(
            crate::settle_until(async || list.resident_pages().contains(&(pages - 1))).await,
            "the jump never brought the last page in"
        );
        crate::settle();
        let jumped = client.counts().of("Page") - at_the_top;
        assert!(
            jumped <= 6,
            "a jump to the bottom reads the pages it shows, not the {} between: \
             it read {jumped} (resident now {:?})",
            pages - 12,
            list.resident_pages()
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
    use postio_focus::list::{RowObject, RowWidget};
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
