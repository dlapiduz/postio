//! No model runs here: "Digest mail like this" is exercised only for being offered and for a
//! graceful answer with no model configured.

use gtk::gdk;

use crate::support::{self, Fixture};

pub fn match_a_list_or_a_search_instead_previews_and_saves_the_typed_query() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        fixture
            .file_from_list(
                ("Ledger", "news@ledger.test"),
                "The rate decision",
                "weekly.example.org",
                30,
            )
            .await;
        fixture
            .file_from_list(
                ("Ledger", "news@ledger.test"),
                "The weekly numbers",
                "weekly.example.org",
                40,
            )
            .await;
        fixture
            .file_from_list(
                ("Other List", "other@example.test"),
                "Not this one",
                "other.example.org",
                20,
            )
            .await;
        fixture.index().await;
        let directory = tempfile::tempdir().expect("a config directory");
        let path = directory.path().join("config.toml");
        std::fs::write(&path, "# Mine.\n").expect("a config");
        let config = postio_config::Config::load_from_path(&path).expect("it reads");
        let window = postio_gtk::window::FocusWindow::new(None);
        gtk::prelude::GtkWindowExt::present(&window);
        support::keep(postio_gtk::startup::adopt_at(
            &window,
            fixture.host(),
            &config,
            Some(&path),
        ));
        support::keep(directory);
        support::rows_shown(&window, 3).await;
        support::keys(&window, &["j"]);
        support::press(&window, "d", gdk::ModifierType::empty());
        let dialog = window.rule_dialog().expect("d opened the rule dialog");
        assert!(
            dialog
                .texts()
                .iter()
                .any(|text| text == "Match a list or a search instead…"),
            "no link to a typed query: {:?}",
            dialog.texts()
        );
        dialog.match_instead();
        dialog.set_query("list:weekly.example.org");
        assert!(
            crate::settle_until(async || {
                dialog
                    .texts()
                    .iter()
                    .any(|text| text == "Would have caught 2 messages in the last 90 days")
            })
            .await,
            "the query's preview is not the executor's two: {:?}",
            dialog.texts()
        );
        dialog.create();
        assert!(
            crate::settle_until(async || {
                std::fs::read_to_string(&path)
                    .unwrap_or_default()
                    .contains("list:weekly.example.org")
            })
            .await,
            "Create wrote no query rule:\n{}",
            std::fs::read_to_string(&path).unwrap_or_default()
        );
        assert!(
            !std::fs::read_to_string(&path)
                .unwrap_or_default()
                .contains("from:news@ledger.test"),
            "the sender clause stayed instead of the typed query"
        );
        assert!(
            crate::settle_until(async || window.rule_dialog().is_none()).await,
            "Create closed the dialog"
        );
    });
}
