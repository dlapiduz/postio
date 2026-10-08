//! The two-line row (T044; contracts/focus-surface.md, "Rows"): a row with
//! a marker is 72 px -- its kind decides its height, never its content,
//! focus or selection (FR-013) -- and its second line says the marker's
//! kind, its sentence, and the action that answers it with its key.

use gtk::prelude::*;
use postio_core::{CommandId, Keymap};

use crate::support::{self, Fixture};

pub fn a_marked_row_is_two_lines_whatever_its_state() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Atlas staffing",
                "Sharing the draft.",
                30,
            )
            .await;
        let (asked, _) = fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Re: Atlas Q3 budget",
                "Can you approve these by Friday so finance can close the quarter?",
                10,
            )
            .await;
        fixture
            .ask(
                asked,
                "Can you approve these by Friday so finance can close the quarter?",
            )
            .await;
        let (window, _client) = fixture.open().await;
        let pane = window.pane().expect("the inbox");
        let marked = || {
            pane.rows_on_screen()
                .into_iter()
                .find(|row| row.item().is_some_and(|item| item.two_lines()))
        };
        assert!(
            crate::settle_until(async || marked().is_some_and(|row| row.height() == 72)).await,
            "the marked row never took its two lines"
        );
        let row = marked().expect("just seen");
        let plain = pane
            .rows_on_screen()
            .into_iter()
            .find(|row| row.item().is_some_and(|item| !item.two_lines()))
            .expect("the unmarked row");
        assert_eq!(plain.height(), 40, "an unmarked row keeps its one line");

        // What the second line says.
        assert!(
            crate::settle_until(async || row.drawn().texts.iter().any(|text| text == "Question"))
                .await,
            "the chip was never drawn: {:?}",
            row.drawn().texts
        );
        let drawn = row.drawn().texts;
        let reply = postio_ui::hints::key(Keymap::defaults(), CommandId::Reply).expect("a key");
        for said in [
            "Question",
            "\u{201c}Can you approve these by Friday so finance can close the quarter?\u{201d}",
            "Reply",
            reply.as_str(),
        ] {
            assert!(
                drawn.iter().any(|text| text == said),
                "the second line draws {said:?}: {drawn:?}"
            );
        }

        // Neither the cursor nor keyboard focus moves its height.
        let position = (0..pane.feed().list().n_items())
            .find(|position| {
                pane.feed()
                    .list()
                    .item(*position)
                    .and_downcast::<postio_gtk::list::RowObject>()
                    .and_then(|object| object.item())
                    .is_some_and(|item| item.two_lines())
            })
            .expect("the marked row's position");
        pane.cursor().set_selected(position);
        crate::settle();
        assert_eq!(row.height(), 72, "with the cursor on it");
        row.parent().expect("the list item").grab_focus();
        crate::settle();
        assert_eq!(row.height(), 72, "with keyboard focus on it");
    });
}

/// A selected marked row shows one mark in its gutter, the checked box. The
/// box and the marker's dot both stood at the gutter's centre on the first
/// line, so selecting a row with an action drew the check over the dot; the
/// box takes the dot's place, as it takes a digest's stack, and the second
/// line still says what the marker is.
pub fn a_selected_marked_row_shows_its_check_not_its_dot() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (asked, _) = fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Re: Atlas Q3 budget",
                "Can you approve these by Friday?",
                10,
            )
            .await;
        fixture.ask(asked, "Can you approve these by Friday?").await;
        let (window, _client) = fixture.open().await;
        let pane = window.pane().expect("the inbox");
        let marked = || {
            pane.rows_on_screen()
                .into_iter()
                .find(|row| row.item().is_some_and(|item| item.two_lines()))
        };
        assert!(
            crate::settle_until(async || {
                marked().is_some_and(|row| row.height() == 72 && row.drawn().accent.is_some())
            })
            .await,
            "the marked row never drew its marker"
        );
        let row = marked().expect("just seen");
        assert_eq!(
            gutter_marks(&row),
            1,
            "unselected, the gutter holds the dot"
        );

        support::keys(&window, &["x"]);
        row.queue_draw();
        assert!(
            crate::settle_until(async || row.drawn().picked).await,
            "`x` never selected the marked row"
        );
        assert_eq!(
            gutter_marks(&row),
            1,
            "selected, the gutter holds the checked box alone, not the box over the dot"
        );
        assert!(
            row.drawn().texts.iter().any(|text| text == "Question"),
            "and the second line still says what the marker is: {:?}",
            row.drawn().texts
        );
    });
}

/// How many marks `row` paints in its gutter on the first line: the dot, the
/// checked box, a digest's stack -- anything small standing there.
fn gutter_marks(row: &postio_gtk::list::RowWidget) -> u32 {
    let paintable = gtk::WidgetPaintable::new(Some(row));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, f64::from(row.width()), f64::from(row.height()));
    let mut found = 0;
    if let Some(node) = snapshot.to_node() {
        in_gutter(&node, &mut found);
    }
    found
}

fn in_gutter(node: &gtk::gsk::RenderNode, found: &mut u32) {
    use gtk::gsk::{ClipNode, ContainerNode};
    let at = node.bounds();
    let small = at.width() <= 20.0 && at.height() <= 20.0;
    let centre = (at.x() + at.width() / 2.0, at.y() + at.height() / 2.0);
    if small && (14.0..=46.0).contains(&centre.0) && (6.0..=38.0).contains(&centre.1) {
        *found += 1;
    } else if let Some(container) = node.downcast_ref::<ContainerNode>() {
        for index in 0..container.n_children() {
            in_gutter(&container.child(index), found);
        }
    } else if let Some(clip) = node.downcast_ref::<ClipNode>() {
        in_gutter(&clip.child(), found);
    }
}
