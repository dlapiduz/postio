//! #1894 on the GTK app: archiving the cursor's row never draws a frame
//! that differs from the settled one.
//!
//! On the Mac an archive reset the list model, and for one frame the rows
//! drew unmeasured, their headings empty, before snapping back. GTK's list
//! splices a removal in place instead, but "a splice keeps the rows" is a
//! claim about a layer; this case films every frame of the real window from
//! the key press to the settled list and compares each with the settled
//! one: the same row height, a heading over every day that is showing, no
//! row still waiting for its page, the cursor on the archived row or the one
//! that took its place, and the scroll where it was.

use gtk::prelude::*;

use crate::support::{self, Fixture};

/// What one frame of the list showed.
#[derive(Debug, Clone)]
struct Frame {
    /// Each row on screen: its subject, its drawn height, whether it had
    /// its page.
    rows: Vec<(String, i32, bool)>,
    /// The day headings on screen, as they read.
    headings: Vec<String>,
    scroll: f64,
    cursor: Option<String>,
}

fn subject_of(item: &postio_gtk::list::FocusRow) -> String {
    item.as_conversation()
        .and_then(|row| row.summary.representative.subject.clone())
        .unwrap_or_default()
}

fn frame(window: &postio_gtk::window::FocusWindow) -> Frame {
    let pane = window.pane().expect("the inbox");
    let rows = pane
        .rows_on_screen()
        .iter()
        .map(|row| {
            (
                row.item().map(|item| subject_of(&item)).unwrap_or_default(),
                gtk::prelude::WidgetExt::height(row),
                row.item().is_some(),
            )
        })
        .collect();
    let headings = support::with_class(pane.view(), "focus-day-heading")
        .iter()
        .filter(|heading| heading.is_mapped())
        .filter_map(|heading| {
            heading
                .downcast_ref::<gtk::Label>()
                .map(|l| l.text().to_string())
        })
        .collect();
    Frame {
        rows,
        headings,
        scroll: pane.widget().vadjustment().value(),
        cursor: window.cursor_row().map(|row| subject_of(&row)),
    }
}

/// Fifty conversations, eight hours apart: a few to a day, a heading each.
async fn fifty() -> Fixture {
    let fixture = Fixture::empty().await;
    for n in 1..=50 {
        fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                &format!("Mail {n:02}"),
                "A line of text.",
                i64::from(n) * 480,
            )
            .await;
    }
    fixture
}

pub fn archiving_the_cursors_row_draws_no_frame_unlike_the_settled_one() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = fifty().await;
        let (window, _client) = fixture.open().await;
        let pane = window.pane().expect("the inbox");
        // The list has to be laid out before the keys: rows allocated at a
        // height, and a scroll range longer than the page. A cursor moved
        // before then scrolls to a row that has no place yet, and stays put.
        assert!(
            crate::settle_until(async || {
                let adjustment = pane.widget().vadjustment();
                let rows = pane.rows_on_screen();
                !rows.is_empty()
                    && rows
                        .iter()
                        .all(|row| gtk::prelude::WidgetExt::height(row) > 0)
                    && adjustment.upper() > adjustment.page_size() + 1.0
            })
            .await,
            "the list was never laid out: {:?}",
            pane.rows_on_screen()
                .iter()
                .map(gtk::prelude::WidgetExt::height)
                .collect::<Vec<_>>()
        );
        // Down the list far enough that it has scrolled: the cursor is in
        // the middle of a screen of rows, under a heading.
        for _ in 0..20 {
            support::keys(&window, &["j"]);
            crate::settle();
        }
        assert!(
            crate::settle_until(async || window.cursor_row().is_some()).await,
            "no cursor"
        );
        // The scroll follows the cursor on a later turn of the loop.
        crate::settle_until(async || pane.widget().vadjustment().value() > 0.0).await;
        crate::settle_for(std::time::Duration::from_millis(300)).await;
        let before = frame(&window);
        assert!(before.scroll > 0.0, "the list never scrolled: {before:?}");
        let archived = before.cursor.clone().expect("the cursor is on a row");
        assert!(
            before
                .rows
                .iter()
                .any(|(subject, _, _)| *subject == archived),
            "the cursor's row is on screen: {before:?}"
        );

        // Film every frame from here until the list has settled.
        let frames = std::rc::Rc::new(std::cell::RefCell::new(Vec::<Frame>::new()));
        let id = pane.view().add_tick_callback({
            let frames = frames.clone();
            let window = window.clone();
            move |_, _| {
                frames.borrow_mut().push(frame(&window));
                gtk::glib::ControlFlow::Continue
            }
        });
        support::keys(&window, &["e"]);
        // Also what the very next turn of the loop shows.
        frames.borrow_mut().push(frame(&window));
        assert!(
            crate::settle_until(async || { !support::subjects(&window).contains(&archived) }).await,
            "the archived row is still on screen: {:?}",
            support::subjects(&window)
        );
        crate::settle_for(postio_test_support::scaled(
            std::time::Duration::from_millis(500),
        ))
        .await;
        // The frame clock ticks when the compositor lets it; on a loaded
        // machine that can be late. Film until it has, rather than count
        // on the dwell above to have been long enough.
        crate::settle_until(async || frames.borrow().len() >= 3).await;
        id.remove();
        let after = frame(&window);
        let frames = frames.borrow();
        assert!(
            frames.len() >= 2,
            "the case saw {} frames: it filmed nothing",
            frames.len()
        );

        let settled_height = after.rows[0].1;
        for (n, shot) in frames.iter().chain(std::iter::once(&after)).enumerate() {
            assert!(
                shot.rows
                    .iter()
                    .all(|(_, height, _)| *height == settled_height),
                "frame {n} drew a row at a height other than the settled \
                 {settled_height}: {shot:?}"
            );
            assert!(
                shot.rows.iter().all(|(_, _, loaded)| *loaded),
                "frame {n} drew a row still waiting for its page: {shot:?}"
            );
            assert!(
                !shot.headings.is_empty() && shot.headings.iter().all(|h| !h.is_empty()),
                "frame {n} drew a day heading with no day, or none: {shot:?}"
            );
            assert!(
                (shot.scroll - before.scroll).abs() <= 1.0
                    || (shot.scroll - after.scroll).abs() <= 1.0,
                "frame {n} jumped the scroll to {} (was {}, settles at {})",
                shot.scroll,
                before.scroll,
                after.scroll
            );
            let cursor = shot.cursor.as_deref();
            assert!(
                cursor == Some(archived.as_str()) || cursor == after.cursor.as_deref(),
                "frame {n} had the cursor on {cursor:?}, between {archived:?} \
                 and {:?}",
                after.cursor
            );
        }
        assert_ne!(after.cursor.as_deref(), Some(archived.as_str()));
        assert!(
            (after.scroll - before.scroll).abs() <= 1.0,
            "archiving moved the scroll from {} to {}",
            before.scroll,
            after.scroll
        );
    });
}
