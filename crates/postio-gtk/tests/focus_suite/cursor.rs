use crate::support::{self, Fixture};

pub fn the_list_view_moving_the_cursor_is_where_j_walks_on_from() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        let pane = window.pane().expect("the inbox");
        assert!(
            crate::settle_until(async || pane.cursor().selected() == 0).await,
            "the list opens with the cursor on the first row"
        );

        pane.cursor().set_selected(2);
        support::keys(&window, &["j"]);
        assert_eq!(
            pane.cursor().selected(),
            3,
            "j walks on from the row the list view chose"
        );
        pane.cursor().set_selected(1);
        support::keys(&window, &["k"]);
        assert_eq!(pane.cursor().selected(), 0, "and k back from it");
        assert!(window.selection().is_empty(), "moving selected nothing");
    });
}
