use gtk::prelude::*;

use crate::support::{self, Fixture};

#[derive(Debug, Clone)]
struct Frame {
    rows: Vec<(String, i32, bool)>,
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

async fn fifty() -> Fixture {
    let fixture = Fixture::empty().await;
    for n in 1..=50 {
        fixture
            .file_from_ada(
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
        let fixture = fifty().await;
        let (window, _client) = fixture.open().await;
        let pane = window.pane().expect("the inbox");
        // The list has to be laid out before the keys: a cursor moved before then scrolls to a row
        // with no place yet, and stays put.
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
        for _ in 0..20 {
            support::keys(&window, &["j"]);
            crate::settle();
        }
        assert!(
            crate::settle_until(async || window.cursor_row().is_some()).await,
            "no cursor"
        );
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
        // The frame clock ticks when the compositor lets it, late on a loaded machine: film until
        // it has.
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
