//! Digest rules by list or search, and "Digest mail like this" (US14,
//! T155; screen 24). No model runs here: the query path is exercised with
//! a typed query, through the same executor a sender's `from:` rule
//! previews with (`Client::digest_preview`); "Digest mail like this" is
//! exercised for whether it is offered and for a graceful answer with no
//! model configured -- what `Client::digest_like_this` itself does with one
//! is postio-ai's and postio-host's to test.

use gtk::gdk;

use crate::support::{self, Fixture};

/// US14 scenarios 1 and 2: "Match a list or a search instead…" swaps the
/// senders' "From" for a typed query, previewed the same way a sender's
/// `from:` is (the one query language, ADR 0008); Create saves the typed
/// query verbatim, not the sender it replaced.
pub fn match_a_list_or_a_search_instead_previews_and_saves_the_typed_query() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
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
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 3).await,
            "the inbox never reached the screen"
        );
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

/// Editing a list rule keeps the rule's own name: opening it fills the
/// query entry, and that is not the person typing a query, so Create
/// saves the rule under the name it had (the terminal and Focus agree).
pub fn editing_a_list_rule_keeps_its_name_until_a_query_is_typed() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Atlas budget",
                "Numbers.",
                5,
            )
            .await;
        fixture.index().await;
        let directory = tempfile::tempdir().expect("a config directory");
        let path = directory.path().join("config.toml");
        std::fs::write(
            &path,
            "[[focus.digests]]\nname = \"Weekly lists\"\nmatch = [\"list:weekly.example.org\"]\ncadence = \"daily\"\nat = \"08:00\"\n",
        )
        .expect("a config");
        let config = postio_config::Config::load_from_path(&path).expect("it reads");
        let rule = config.focus.digests[0].clone();
        let window = postio_gtk::window::FocusWindow::new(None);
        gtk::prelude::GtkWindowExt::present(&window);
        support::keep(postio_gtk::startup::adopt_at(
            &window,
            fixture.host(),
            &config,
            Some(&path),
        ));
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 1).await,
            "the inbox never reached the screen"
        );
        support::keys(&window, &["j"]);
        support::press(&window, "d", gdk::ModifierType::empty());
        let dialog = window.rule_dialog().expect("d opened the rule dialog");
        dialog.open_edit(&window, &rule);
        support::keep(directory);
        dialog.create();
        assert!(
            crate::settle_until(async || window.rule_dialog().is_none()).await,
            "Create closed the dialog"
        );
        let written = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            written.contains("name = \"Weekly lists\""),
            "the edited rule lost its name:\n{written}"
        );
    });
}

/// FR-171: without a model, "Digest mail like this" is absent from the
/// dialog `d` opens over an ordinary message.
pub fn digest_mail_like_this_is_absent_with_no_model_configured() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = support::three_in_the_inbox().await;
        support::keys(&window, &["j"]);
        support::press(&window, "d", gdk::ModifierType::empty());
        let dialog = window.rule_dialog().expect("d opened the rule dialog");
        assert!(
            dialog
                .texts()
                .iter()
                .all(|text| text != "Digest mail like this"),
            "the control is offered with no model configured: {:?}",
            dialog.texts()
        );
    });
}

/// FR-171: given a message to check (as the window only would once the
/// user has brought a model with `like_this` on), the control is present;
/// clicking it against a client with no model configured says so rather
/// than opening query mode on nothing.
pub fn digest_mail_like_this_present_with_a_message_says_so_with_no_model() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Atlas budget",
                "Numbers.",
                5,
            )
            .await;
        let (window, client) = fixture.open().await;
        let dialog = postio_gtk::rule_dialog::RuleDialog::new(client, &window.keymap());
        dialog.open_new(
            &window,
            &[postio_model::EmailAddress::new(
                Some("Ada Moreno"),
                "ada@example.com",
            )],
            Some(message),
        );
        assert!(
            dialog
                .texts()
                .iter()
                .any(|text| text == "Digest mail like this"),
            "the control is missing with a message given: {:?}",
            dialog.texts()
        );
        // The preview answers first: its line and the model's share the
        // dialog's one sentence, and a failed preview would write over the
        // model's.
        assert!(
            crate::settle_until(async || {
                dialog
                    .texts()
                    .iter()
                    .any(|text| text.starts_with("Would have caught"))
            })
            .await,
            "the preview never answered: {:?}",
            dialog.texts()
        );
        dialog.like_this();
        assert!(
            crate::settle_until(async || {
                dialog
                    .texts()
                    .iter()
                    .any(|text| text.contains("nothing alike"))
            })
            .await,
            "no model configured said nothing: {:?}",
            dialog.texts()
        );
    });
}
