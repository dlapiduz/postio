//! The classic app's settings wiring cases, ported to Focus (T234): the
//! shared settings window over a real store, through Focus's host, with
//! each change asserted in the store and on screen. Their classic
//! originals are `postio-app`'s `settings_accounts_wiring`,
//! `settings_credential_wiring`, `settings_reindex_wiring`, `egress_wiring`
//! and `read_receipt_wiring`; `signature_default_wiring` and
//! `sidebar_backfill_wiring` are ported in `settings.rs`, where a signature
//! made in Settings signs the next message and a folder's check skips its
//! backfill.
//!
//! An account row's own menu is opened through the panel's
//! `test_open_account_menu`, as the classic cases open it: GTK gives a test
//! no way to send a secondary click that a gesture's own state machine
//! accepts. Everything else is a real key or click.

use gtk::glib;
use gtk::prelude::*;
use postio_storage::repository::{AccountRepository, EgressLogRepository};

use crate::settings::{one_message_under, open_under, settings_shown};
use crate::support::{self, Fixture};

/// `mod+comma`, as the keyboard sends it.
fn mod_comma(window: &postio_focus::window::FocusWindow) {
    support::deliver_with(window, "comma", gtk::gdk::ModifierType::CONTROL_MASK);
}

/// The account rows Settings draws now, top to bottom.
fn rows(dialog: &adw::Dialog) -> Vec<gtk::ListBoxRow> {
    support::with_class(dialog, "postio-settings-account-row")
        .into_iter()
        .filter(|row| row.is_mapped())
        .filter_map(|row| row.downcast().ok())
        .collect()
}

/// Run the account menu's `verb` on the row at `index`, as its menu does:
/// on the row on screen when it runs, since a change redraws the rows.
fn account_verb(window: &postio_focus::window::FocusWindow, index: usize, verb: &str) {
    let settings = window.settings().expect("Settings is open");
    let dialog = settings.dialog().clone();
    let panel = settings.panel();
    let deadline =
        std::time::Instant::now() + postio_test_support::scaled(std::time::Duration::from_secs(10));
    loop {
        crate::settle();
        let place = rows(&dialog).into_iter().nth(index).and_then(|row| {
            let bounds = row.compute_bounds(&row.parent()?)?;
            (bounds.height() > 0.0).then(|| f64::from(bounds.y() + bounds.height() / 2.0))
        });
        if let Some(y) = place {
            panel.test_open_account_menu(1.0, y);
            let ran = panel.activate_action(verb, None).is_ok();
            panel.test_close_account_menu();
            if ran {
                return;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{verb} is on row {index}'s menu"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

async fn account(
    database: &postio_storage::Store,
    id: postio_model::ids::AccountId,
) -> postio_model::Account {
    let connection = database.connect().await.expect("a connection");
    AccountRepository::new(&connection)
        .get(id)
        .await
        .expect("the account reads")
        .expect("still there")
}

/// `settings_accounts_wiring`: both accounts are rows, the enable switch
/// persists, Set as default moves rather than adds, and Remove marks the
/// account (never deletes it) with an Undo that restores it.
pub fn account_rows_persist_enable_default_and_removal() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "A line.", 10)
            .await;
        let (second, _) = fixture.second_account().await;
        let (window, _directory, _path) = open_under(&fixture, "").await;
        mod_comma(&window);
        let dialog = settings_shown(&window).await.expect("Settings opened");
        assert!(
            crate::settle_until(async || rows(&dialog).len() == 2).await,
            "both accounts are rows: {}",
            rows(&dialog).len()
        );

        // The second account's switch persists to its own row.
        let switch = || {
            rows(&dialog)
                .get(1)
                .and_then(|row| {
                    support::descendants(row)
                        .into_iter()
                        .find_map(|widget| widget.downcast::<gtk::Switch>().ok())
                })
                .expect("every account row has a switch")
        };
        assert!(switch().is_active(), "a new account starts enabled");
        // Flipped as the classic case flips it: a switch's gesture toggles
        // only on a pointer sequence GTK made, which a test cannot make.
        switch().set_active(false);
        let database = fixture.database.clone();
        assert!(
            crate::settle_until(async || !account(&database, second.id).await.enabled).await,
            "the switch reached the store"
        );
        switch().set_active(true);
        assert!(crate::settle_until(async || account(&database, second.id).await.enabled).await);

        // Set as default moves the marker.
        account_verb(&window, 1, "account.set-default");
        assert!(
            crate::settle_until(async || account(&database, second.id).await.is_default).await,
            "Set as default reached the store"
        );
        account_verb(&window, 0, "account.set-default");
        assert!(
            crate::settle_until(
                async || account(&database, fixture.account.id).await.is_default
                    && !account(&database, second.id).await.is_default
            )
            .await,
            "one account is the default at a time"
        );

        // Remove marks it pending and takes its row away at once...
        account_verb(&window, 1, "account.remove");
        assert!(
            crate::settle_until(async || account(&database, second.id).await.pending_deletion)
                .await,
            "Remove marks the account, it does not delete it"
        );
        assert!(
            crate::settle_until(async || rows(&dialog).len() == 1).await,
            "a removed account stops showing"
        );
        // ...and the dialog's own toast offers to undo it.
        let undo = || {
            support::descendants(&dialog).into_iter().find(|widget| {
                widget.is::<gtk::Button>()
                    && widget.is_mapped()
                    && support::texts(widget) == ["Undo"]
            })
        };
        assert!(
            crate::settle_until(async || undo().is_some()).await,
            "the removal's toast is in the dialog, where it can be reached"
        );
        support::click(&window, &undo().expect("Undo"), 1);
        assert!(
            crate::settle_until(
                async || !account(&database, second.id).await.pending_deletion
                    && rows(&dialog).len() == 2
            )
            .await,
            "Undo restored the account and its row"
        );
    });
}

/// `settings_credential_wiring`: Update credential opens the account form
/// over the window, filled in from the account's row, and the list behind
/// is untouched.
pub fn update_credential_opens_a_prefilled_form_over_the_window() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (fixture, window, _directory, _path) = one_message_under("").await;
        mod_comma(&window);
        let dialog = settings_shown(&window).await.expect("Settings opened");
        assert!(crate::settle_until(async || rows(&dialog).len() == 1).await);
        let form = || {
            support::descendants(&window)
                .into_iter()
                .find_map(|widget| {
                    widget
                        .downcast::<postio_widgets::onboarding::Onboarding>()
                        .ok()
                })
                .filter(|form| form.is_mapped())
        };
        assert!(form().is_none(), "no form opens unasked");

        account_verb(&window, 0, "account.update-credential");
        assert!(
            crate::settle_until(async || form().is_some()).await,
            "Update credential opened the account form"
        );
        let form = form().expect("the form");
        assert!(
            matches!(
                form.status(),
                postio_widgets::onboarding::Status::Reauthenticate(_)
            ),
            "the form arrives as a credential update: {:?}",
            form.status()
        );
        assert_eq!(
            form.address(),
            fixture.account.address.address,
            "with the address the store already has"
        );
        assert_eq!(
            support::subjects(&window),
            ["Budget"],
            "the list is untouched"
        );
    });
}

/// `settings_reindex_wiring`: Rebuild search index refills the account's
/// own index, and its row's progress line clears once it is over.
pub fn rebuilding_an_index_refills_it_and_clears_the_row() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "A line.", 10)
            .await;
        fixture
            .write_body(message, "The numbers are attached.")
            .await;
        fixture.index().await;
        let (window, _directory, _path) = open_under(&fixture, "").await;
        mod_comma(&window);
        let dialog = settings_shown(&window).await.expect("Settings opened");
        assert!(crate::settle_until(async || rows(&dialog).len() == 1).await);

        account_verb(&window, 0, "account.rebuild-index");
        let database = fixture.database.clone();
        let account = fixture.account.id;
        assert!(
            crate::settle_until(async || {
                let connection = database.connect().await.expect("a connection");
                postio_index::index::messages_missing_body_text_for_account(
                    &connection,
                    account.get(),
                    10,
                )
                .await
                .expect("candidates")
                .is_empty()
            })
            .await,
            "the rebuild refilled the account's index"
        );
        let reindexing = || {
            rows(&dialog).first().and_then(|row| {
                support::with_class(row, "postio-settings-account-reindexing")
                    .into_iter()
                    .find(|line| line.is_visible())
            })
        };
        assert!(
            crate::settle_until(async || reindexing().is_none()).await,
            "the row's progress line clears once the rebuild is over"
        );
    });
}

/// `egress_wiring`: opening Focus makes no connection, and Settings' Privacy
/// lists what the egress log holds.
pub fn opening_makes_no_connection_and_privacy_lists_the_log() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (fixture, window, _directory, _path) = one_message_under("").await;
        let connection = fixture.database.connect().await.expect("a connection");
        let log = EgressLogRepository::new(&connection);
        assert_eq!(
            log.count().await.expect("a count"),
            0,
            "opening Focus made an outbound connection"
        );
        log.record(&postio_model::egress::EgressEvent {
            at: chrono::Utc::now(),
            subsystem: postio_model::egress::EgressSubsystem::Imap,
            account: Some(fixture.account.id),
            host: "imap.example.com".to_owned(),
            port: 993,
            outcome: postio_model::egress::EgressOutcome::Connected,
        })
        .await
        .expect("a logged connection");
        drop(connection);

        mod_comma(&window);
        let dialog = settings_shown(&window).await.expect("Settings opened");
        open_section(&window, &dialog, "Privacy").await;
        assert!(
            crate::settle_until(async || {
                support::with_class(&dialog, "postio-settings-egress-row")
                    .iter()
                    .any(|row| {
                        support::texts(row)
                            .iter()
                            .any(|text| text.contains("imap.example.com"))
                    })
            })
            .await,
            "Privacy lists the logged connection"
        );
    });
}

/// `read_receipt_wiring`: Privacy says how many messages asked for a read
/// receipt, and that none were sent.
pub fn privacy_counts_the_receipts_asked_for() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        const ASKED: &[u8] = b"From: Newsletter <news@example.org>\r\n\
To: Ada Lovelace <ada@example.com>\r\n\
Subject: Please confirm receipt\r\n\
Disposition-Notification-To: news@example.org\r\n\
MIME-Version: 1.0\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Let us know you got this\r\n";
        let fixture = Fixture::empty().await;
        {
            let connection = fixture.database.connect().await.expect("a connection");
            let mut asked = postio_model::mime::parse(ASKED).into_message(
                fixture.account.id,
                fixture.inbox,
                chrono::Utc::now(),
            );
            postio_storage::repository::MessageRepository::new(&connection)
                .create(&mut asked)
                .await
                .expect("a message");
        }
        let (window, _directory, _path) = open_under(&fixture, "").await;
        mod_comma(&window);
        let dialog = settings_shown(&window).await.expect("Settings opened");
        open_section(&window, &dialog, "Privacy").await;
        let said = || {
            support::with_class(&dialog, "postio-settings-read-receipt-count")
                .first()
                .map(support::texts)
                .unwrap_or_default()
                .join(" ")
        };
        assert!(
            crate::settle_until(
                async || said().contains('1') && said().contains("none have been sent")
            )
            .await,
            "one message asked for a receipt, and none were sent: {:?}",
            said()
        );
    });
}

/// Click the section row named `name`, and wait for its pane.
async fn open_section(
    window: &postio_focus::window::FocusWindow,
    dialog: &adw::Dialog,
    name: &str,
) {
    let row = || {
        crate::settings::section_rows(dialog)
            .into_iter()
            .find(|(said, _)| said == name)
            .map(|(_, row)| row)
    };
    assert!(
        crate::settle_until(async || row().is_some()).await,
        "{name} is listed"
    );
    support::click(window, &row().expect("its row"), 1);
    assert!(
        crate::settle_until(async || {
            support::with_class(dialog, "postio-settings-pane-title")
                .first()
                .is_some_and(|title| support::texts(title) == [name])
        })
        .await,
        "{name}'s pane is on screen"
    );
    glib::MainContext::default().iteration(false);
}
