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
                    .and_downcast::<postio_focus::list::RowObject>()
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
