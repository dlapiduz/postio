//! The shared settings window over a real store through Focus's host. An account row's menu is
//! opened through the panel's `test_open_account_menu`, since a test cannot send a secondary click
//! that a gesture's state machine accepts.

use gtk::glib;
use gtk::prelude::*;
use postio_storage::repository::{AccountRepository, EgressLogRepository};

use crate::settings::{one_message_under, open_under, settings_shown};
use crate::support::{self, Fixture};

fn mod_comma(window: &postio_gtk::window::FocusWindow) {
    support::deliver_with(window, "comma", gtk::gdk::ModifierType::CONTROL_MASK);
}

fn rows(dialog: &adw::Dialog) -> Vec<gtk::ListBoxRow> {
    support::with_class(dialog, "postio-settings-account-row")
        .into_iter()
        .filter(|row| row.is_mapped())
        .filter_map(|row| row.downcast().ok())
        .collect()
}

fn account_verb(
    window: &postio_gtk::window::FocusWindow,
    account: postio_model::ids::AccountId,
    verb: &str,
) {
    let settings = window.settings().expect("Settings is open");
    let dialog = settings.dialog().clone();
    let panel = settings.panel();
    let deadline =
        std::time::Instant::now() + postio_test_support::scaled(std::time::Duration::from_secs(10));
    loop {
        crate::settle();
        let name = format!("postio-account-{}", account.get());
        let row = rows(&dialog)
            .into_iter()
            .find(|row| row.widget_name() == name.as_str());
        let place = row.as_ref().and_then(|row| {
            let list = row.parent()?.downcast::<gtk::ListBox>().ok()?;
            let bounds = row.compute_bounds(&list)?;
            let y = bounds.y() + bounds.height() / 2.0;
            (bounds.height() > 0.0 && list.row_at_y(y as i32).as_ref() == Some(row))
                .then_some(f64::from(y))
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
            "{verb} is on the account's menu"
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
        // A switch's gesture toggles only on a pointer sequence GTK made, which a test cannot make.
        switch().set_active(false);
        let database = fixture.database.clone();
        assert!(
            crate::settle_until(async || !account(&database, second.id).await.enabled).await,
            "the switch reached the store"
        );
        switch().set_active(true);
        assert!(crate::settle_until(async || account(&database, second.id).await.enabled).await);

        account_verb(&window, second.id, "account.set-default");
        assert!(
            crate::settle_until(async || account(&database, second.id).await.is_default).await,
            "Set as default reached the store"
        );
        account_verb(&window, fixture.account.id, "account.set-default");
        assert!(
            crate::settle_until(
                async || account(&database, fixture.account.id).await.is_default
                    && !account(&database, second.id).await.is_default
            )
            .await,
            "one account is the default at a time"
        );

        account_verb(&window, second.id, "account.remove");
        assert!(
            crate::settle_until(async || account(&database, second.id).await.pending_deletion)
                .await,
            "Remove marks the account, it does not delete it"
        );
        assert!(
            crate::settle_until(async || rows(&dialog).len() == 1).await,
            "a removed account stops showing"
        );
        let undo = || {
            support::descendants(&dialog).into_iter().find(|widget| {
                widget.is::<gtk::Button>()
                    && widget.is_mapped()
                    && support::texts(widget)
                        .first()
                        .is_some_and(|word| word == "Undo")
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

/// A verb redraws Settings' rows as new widgets, and the keyboard on the old row went with it, so
/// the next key reached no account (#1787). It stays on the row.
pub fn an_account_verb_leaves_the_keyboard_on_its_row() {
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
        assert!(crate::settle_until(async || rows(&dialog).len() == 2).await);
        let database = fixture.database.clone();
        focus_row(&window, &dialog, second.id);
        let name = format!("postio-account-{}", second.id.get());
        let row_now = || {
            rows(&dialog)
                .into_iter()
                .find(|row| row.widget_name() == name.as_str())
        };
        let before = row_now().expect("the account's row");
        support::deliver(&window, "m");
        assert!(
            crate::settle_until(async || account(&database, second.id).await.is_default).await,
            "`m` on the second account's row made it the default"
        );
        assert!(
            crate::settle_until(async || row_now().is_some_and(|row| row != before)).await,
            "the rows were never drawn again, so this proves nothing"
        );
        assert!(
            crate::settle_while(async || {
                window
                    .settings()
                    .expect("Settings")
                    .panel()
                    .focused_account()
                    == Some(second.id)
            })
            .await,
            "the keyboard left the account's row when it was drawn again: {}",
            support::focus_path(&window)
        );
    });
}

fn focus_row(
    window: &postio_gtk::window::FocusWindow,
    dialog: &adw::Dialog,
    account: postio_model::ids::AccountId,
) {
    let name = format!("postio-account-{}", account.get());
    let row = rows(dialog)
        .into_iter()
        .find(|row| row.widget_name() == name.as_str())
        .expect("the account's row is on screen");
    row.grab_focus();
    let focused = || {
        window
            .settings()
            .expect("Settings")
            .panel()
            .focused_account()
    };
    let deadline =
        std::time::Instant::now() + postio_test_support::scaled(std::time::Duration::from_secs(10));
    while focused() != Some(account) && std::time::Instant::now() < deadline {
        crate::settle();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        focused(),
        Some(account),
        "the keyboard is on the account's row"
    );
}

pub fn the_account_verbs_have_keys_on_the_focused_row() {
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
        let (second, _) = fixture.second_account().await;
        let (window, _directory, _path) = open_under(&fixture, "").await;
        mod_comma(&window);
        let dialog = settings_shown(&window).await.expect("Settings opened");
        assert!(crate::settle_until(async || rows(&dialog).len() == 2).await);
        let database = fixture.database.clone();

        window
            .settings()
            .expect("Settings")
            .panel()
            .search_field()
            .grab_focus();
        crate::settle();
        let before = account(&database, second.id).await.is_default;
        support::deliver(&window, "Delete");
        support::deliver(&window, "m");
        crate::settle();
        assert_eq!(account(&database, second.id).await.is_default, before);
        assert!(!account(&database, second.id).await.pending_deletion);

        focus_row(&window, &dialog, second.id);
        support::deliver(&window, "m");
        assert!(
            crate::settle_until(async || account(&database, second.id).await.is_default).await,
            "`m` on the second account's row made it the default"
        );

        focus_row(&window, &dialog, second.id);
        support::deliver(&window, "Return");
        assert!(
            crate::settle_until(async || !account(&database, second.id).await.enabled).await,
            "`Return` on the row disabled the account"
        );
        focus_row(&window, &dialog, second.id);
        support::deliver(&window, "Return");
        assert!(
            crate::settle_until(async || account(&database, second.id).await.enabled).await,
            "and enabled it again"
        );

        fixture.index().await;
        focus_row(&window, &dialog, fixture.account.id);
        support::deliver(&window, "r");
        let first = fixture.account.id;
        assert!(
            crate::settle_until(async || {
                let connection = database.connect().await.expect("a connection");
                postio_index::index::messages_missing_body_text_for_account(
                    &connection,
                    first.get(),
                    10,
                )
                .await
                .expect("candidates")
                .is_empty()
            })
            .await,
            "`r` rebuilt the account's index"
        );

        focus_row(&window, &dialog, second.id);
        support::deliver(&window, "Delete");
        assert!(
            crate::settle_until(async || account(&database, second.id).await.pending_deletion)
                .await,
            "`Delete` marked the account for removal"
        );
        support::deliver_with(&window, "z", gtk::gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || !account(&database, second.id).await.pending_deletion)
                .await,
            "`mod+z` restored it"
        );

        focus_row(&window, &dialog, second.id);
        support::deliver(&window, "M");
        let detail = || {
            support::with_class(&dialog, "postio-settings-account-detail-mailboxes")
                .into_iter()
                .any(|group| group.is_mapped())
        };
        assert!(
            crate::settle_until(async || detail()).await,
            "`M` opened the account's roles"
        );
    });
}

pub fn the_command_bar_offers_the_account_verbs() {
    use postio_core::CommandId::*;
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (fixture, window, _directory, _path) = one_message_under("").await;
        let _keep = &fixture;
        let bar = window.command_bar_rows();
        for verb in [
            RemoveAccount,
            RebuildAccountIndex,
            SetDefaultAccount,
            ToggleAccountEnabled,
            MapMailboxRole,
        ] {
            assert!(bar.contains(&verb), "the command bar lists {verb}");
        }
        window.take_unanswered();
        window.act(SetDefaultAccount);
        assert!(
            settings_shown(&window).await.is_some(),
            "with Settings shut, the verb opens it to pick an account"
        );
        assert!(window.take_unanswered().is_empty());
    });
}

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

        account_verb(&window, fixture.account.id, "account.update-credential");
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

        account_verb(&window, fixture.account.id, "account.rebuild-index");
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

async fn open_section(window: &postio_gtk::window::FocusWindow, dialog: &adw::Dialog, name: &str) {
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
