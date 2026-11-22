use gtk::prelude::*;
use postio_core::{ConnectionState, Event, FailureReason};

use crate::support::{self, Fixture};

pub fn each_sync_state_shows_its_banner_and_label() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "Numbers.", 5)
            .await;
        let (host, sink) = fixture.host_telling();
        let window = postio_gtk::window::FocusWindow::new(None);
        window.present();
        support::keep(postio_gtk::startup::adopt(
            &window,
            host,
            &postio_config::Config::default(),
        ));
        assert!(
            crate::settle_until(async || !window.rows_on_screen().is_empty()).await,
            "the inbox never reached the screen"
        );
        let account = fixture.account.id;
        let tell = |event| assert!(sink.emit(event), "the hub took the event");

        tell(Event::ConnectionChanged {
            account,
            state: ConnectionState::Online,
        });
        tell(Event::SyncProgress {
            account,
            done: 12_408,
            total: 18_204,
        });
        assert!(
            crate::settle_until(async || window.banner_showing().is_some()).await,
            "no banner for a first sync"
        );
        let (title, button, progress) = window.banner_showing().expect("a banner");
        assert!(
            title.starts_with("First sync. 12,408 of 18,204 messages"),
            "{title}"
        );
        assert_eq!(button, None);
        assert!(progress.is_some_and(|p| (p - 12_408.0 / 18_204.0).abs() < 1e-6));
        assert_eq!(window.sync_said(), "Syncing 12,408 of 18,204");

        tell(Event::ConnectionChanged {
            account,
            state: ConnectionState::Offline,
        });
        assert!(
            crate::settle_until(async || window
                .banner_showing()
                .is_some_and(|(title, ..)| title.starts_with("You're offline")))
            .await,
            "no offline banner: {:?}",
            window.banner_showing()
        );
        let (_, button, progress) = window.banner_showing().expect("a banner");
        assert_eq!(button.as_deref(), Some("Retry now"));
        assert_eq!(progress, None, "no progress bar while offline");
        assert_eq!(window.sync_said(), "Offline");

        tell(Event::ConnectionChanged {
            account,
            state: ConnectionState::Failing {
                reason: FailureReason::Auth,
            },
        });
        assert!(
            crate::settle_until(async || window
                .banner_showing()
                .is_some_and(|(title, ..)| title.starts_with("Can't sign in")))
            .await,
            "no sign-in banner: {:?}",
            window.banner_showing()
        );
        let (title, button, _) = window.banner_showing().expect("a banner");
        assert!(
            title.contains("imap.example.com") && title.contains(&fixture.account.address.address),
            "the banner names the server and the address: {title}"
        );
        assert_eq!(button.as_deref(), Some("Update password\u{2026}"));
        assert_eq!(window.sync_said(), "Sync failed");

        tell(Event::ConnectionChanged {
            account,
            state: ConnectionState::Online,
        });
        tell(Event::SyncProgress {
            account,
            done: 40,
            total: 40,
        });
        assert!(
            crate::settle_until(async || window.banner_showing().is_none()).await,
            "the banner stayed: {:?}",
            window.banner_showing()
        );
        assert!(
            crate::settle_until(async || window.sync_said().starts_with("Synced ")).await,
            "the label says when it synced: {}",
            window.sync_said()
        );
    });
}

pub fn update_password_on_the_sign_in_banner_opens_the_credential_dialog() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "Numbers.", 5)
            .await;
        let (host, sink) = fixture.host_telling();
        let window = postio_gtk::window::FocusWindow::new(None);
        window.present();
        support::keep(postio_gtk::startup::adopt(
            &window,
            host,
            &postio_config::Config::default(),
        ));
        assert!(
            crate::settle_until(async || !window.rows_on_screen().is_empty()).await,
            "the inbox never reached the screen"
        );
        assert!(sink.emit(Event::ConnectionChanged {
            account: fixture.account.id,
            state: ConnectionState::Failing {
                reason: FailureReason::Auth,
            },
        }));
        assert!(
            crate::settle_until(async || {
                window.banner_showing().is_some_and(|(_, button, _)| {
                    button.as_deref() == Some("Update password\u{2026}")
                })
            })
            .await,
            "no sign-in banner: {:?}",
            window.banner_showing()
        );

        let banner = support::only(&window, "focus-banner");
        let button = support::descendants(&banner)
            .into_iter()
            .find(|widget| widget.is::<gtk::Button>() && widget.is_mapped())
            .expect("the banner's button is on screen");
        support::click(&window, &button, 1);

        let form = || form_in(window.upcast_ref());
        assert!(
            crate::settle_until(async || form().is_some()).await,
            "Update password\u{2026} opened no credential dialog"
        );
        let form = form().expect("the credential form");
        assert_eq!(form.address(), fixture.account.address.address);
        assert!(
            matches!(
                form.status(),
                postio_widgets::onboarding::Status::Reauthenticate(ref settings)
                    if settings.imap.host == fixture.account.incoming.host
            ),
            "the form is not a repair of the refused account: {:?}",
            form.status()
        );
    });
}

fn form_in(widget: &gtk::Widget) -> Option<postio_widgets::onboarding::Onboarding> {
    if let Ok(form) = widget
        .clone()
        .downcast::<postio_widgets::onboarding::Onboarding>()
    {
        return Some(form);
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(form) = form_in(&current) {
            return Some(form);
        }
        child = current.next_sibling();
    }
    None
}

pub fn offline_an_archive_takes_effect_at_once_and_queues() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "Numbers.", 5)
            .await;
        fixture
            .file(("Lena Park", "lena@example.org"), "Draft", "Comments.", 9)
            .await;
        let (host, sink) = fixture.host_telling();
        let window = postio_gtk::window::FocusWindow::new(None);
        window.present();
        support::keep(postio_gtk::startup::adopt(
            &window,
            host,
            &postio_config::Config::default(),
        ));
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 2).await,
            "the inbox never reached the screen"
        );
        assert!(sink.emit(Event::ConnectionChanged {
            account: fixture.account.id,
            state: ConnectionState::Offline,
        }));
        assert!(
            crate::settle_until(async || window.sync_said() == "Offline").await,
            "the window never heard it was offline"
        );

        support::keys(&window, &["e"]);
        assert!(
            crate::settle_until(async || support::subjects(&window) == ["Draft"]).await,
            "the archived row stayed while offline: {:?}",
            support::subjects(&window)
        );
        let connection = fixture.database.connect().await.expect("a connection");
        let queued = postio_storage::repository::OperationQueueRepository::new(&connection)
            .pending(fixture.account.id, chrono::Utc::now())
            .await
            .expect("the queue reads");
        assert_eq!(
            queued.len(),
            1,
            "the archive waits in the queue: {queued:?}"
        );
        assert!(
            window
                .banner_showing()
                .is_some_and(|(title, ..)| title.starts_with("You're offline")),
            "the offline banner stays while the change waits"
        );
    });
}

pub fn during_a_first_sync_what_has_arrived_is_listed() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "Numbers.", 5)
            .await;
        let (host, sink) = fixture.host_telling();
        let window = postio_gtk::window::FocusWindow::new(None);
        window.present();
        support::keep(postio_gtk::startup::adopt(
            &window,
            host,
            &postio_config::Config::default(),
        ));
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 1).await,
            "the inbox never reached the screen"
        );
        let account = fixture.account.id;
        assert!(sink.emit(Event::ConnectionChanged {
            account,
            state: ConnectionState::Online,
        }));
        assert!(sink.emit(Event::SyncProgress {
            account,
            done: 100,
            total: 5_000,
        }));

        let (arrived, _) = fixture
            .file(("Lena Park", "lena@example.org"), "Harbor draft", "v3.", 1)
            .await;
        assert!(sink.emit(Event::NewMail {
            account,
            mailbox: fixture.inbox,
            messages: vec![arrived],
        }));
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 2).await,
            "mail that arrived during the first sync is not listed: {:?}",
            support::subjects(&window)
        );
        assert!(
            crate::settle_until(async || counts(&window).starts_with("2 ")).await,
            "the strip does not count it: {}",
            counts(&window)
        );
        assert!(
            window
                .banner_showing()
                .is_some_and(|(title, ..)| title.starts_with("First sync")),
            "the first sync's banner is still up"
        );
    });
}

fn counts(window: &postio_gtk::window::FocusWindow) -> String {
    support::texts(&support::only(window, "focus-counts")).join(" ")
}

pub fn offline_a_label_shows_at_once_and_queues_and_search_answers() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "Numbers.", 5)
            .await;
        fixture
            .file(("Lena Park", "lena@example.org"), "Draft", "Comments.", 9)
            .await;
        fixture.index().await;
        let (host, sink) = fixture.host_telling();
        let window = postio_gtk::window::FocusWindow::new(None);
        window.present();
        support::keep(postio_gtk::startup::adopt(
            &window,
            host,
            &postio_config::Config::default(),
        ));
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 2).await,
            "the inbox never reached the screen"
        );
        assert!(sink.emit(Event::ConnectionChanged {
            account: fixture.account.id,
            state: ConnectionState::Offline,
        }));
        assert!(
            crate::settle_until(async || window.sync_said() == "Offline").await,
            "the window never heard it was offline"
        );

        support::keys(&window, &["l"]);
        let picker = window.open_picker().expect("l opened the label picker");
        assert!(crate::settle_until(async || picker.is_shown()).await);
        picker.entry().set_text("Receipts");
        assert!(
            crate::settle_until(async || picker
                .texts()
                .iter()
                .any(|line| line == "Create label \u{201c}Receipts\u{201d}"))
            .await,
            "no Create label row: {:?}",
            picker.texts()
        );
        picker.entry().emit_activate();
        let pills = || {
            window
                .pane()
                .map(|pane| {
                    pane.rows_on_screen()
                        .iter()
                        .filter(|row| {
                            row.item().is_some_and(|item| {
                                item.as_conversation().is_some_and(|row| {
                                    row.summary.representative.subject.as_deref() == Some("Budget")
                                })
                            })
                        })
                        .flat_map(|row| row.drawn().pills)
                        .map(|(name, _)| name)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        assert!(
            crate::settle_until(async || pills() == ["Receipts"]).await,
            "offline, the label is not on the row at once: {:?}",
            pills()
        );
        let connection = fixture.database.connect().await.expect("a connection");
        let queued = postio_storage::repository::OperationQueueRepository::new(&connection)
            .pending(fixture.account.id, chrono::Utc::now())
            .await
            .expect("the queue reads");
        assert_eq!(
            queued.len(),
            1,
            "the label waits in the queue for sync: {queued:?}"
        );

        support::press(&window, "slash", gtk::gdk::ModifierType::empty());
        let bar = window.bar().expect("/ opened the bar");
        bar.set_text("draft");
        assert!(
            crate::settle_until(async || bar.result_subjects() == ["Draft"]).await,
            "offline, the bar found nothing: {:?}",
            bar.result_subjects()
        );
        assert!(
            window
                .banner_showing()
                .is_some_and(|(title, ..)| title.starts_with("You're offline")),
            "the offline banner stays while the change waits"
        );
    });
}

pub fn during_a_first_sync_what_has_arrived_opens_and_is_found() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "Numbers.", 5)
            .await;
        fixture.index().await;
        let (host, sink) = fixture.host_telling();
        let window = postio_gtk::window::FocusWindow::new(None);
        window.present();
        support::keep(postio_gtk::startup::adopt(
            &window,
            host,
            &postio_config::Config::default(),
        ));
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 1).await,
            "the inbox never reached the screen"
        );
        let account = fixture.account.id;
        assert!(sink.emit(Event::ConnectionChanged {
            account,
            state: ConnectionState::Online,
        }));
        assert!(sink.emit(Event::SyncProgress {
            account,
            done: 100,
            total: 5_000,
        }));

        let (arrived, _) = fixture
            .file(
                ("Lena Park", "lena@example.org"),
                "Harbor draft",
                "Version three.",
                1,
            )
            .await;
        fixture
            .write_body(arrived, "Version three is up for comments.")
            .await;
        assert!(sink.emit(Event::NewMail {
            account,
            mailbox: fixture.inbox,
            messages: vec![arrived],
        }));
        assert!(
            crate::settle_until(
                async || support::subjects(&window).first() == Some(&"Harbor draft".to_owned())
            )
            .await,
            "the arrived message is not listed: {:?}",
            support::subjects(&window)
        );

        support::keys(&window, &["k", "Return"]);
        let reading = window.reading().expect("Enter opened the message");
        assert!(reading.is_open(), "the dialog is up");
        assert_eq!(reading.title(), "Harbor draft");
        assert!(
            crate::settle_until(async || reading.body_text().contains("up for comments")).await,
            "the body of what arrived was not shown: {:?}",
            reading.body_text()
        );
        support::press(&window, "Escape", gtk::gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || !reading.is_open()).await,
            "Escape closed the message"
        );

        support::press(&window, "slash", gtk::gdk::ModifierType::empty());
        let bar = window.bar().expect("/ opened the bar");
        bar.set_text("harbor");
        assert!(
            crate::settle_until(async || bar.result_subjects() == ["Harbor draft"]).await,
            "during the first sync, the bar did not find what arrived: {:?}",
            bar.result_subjects()
        );
        assert!(
            window
                .banner_showing()
                .is_some_and(|(title, ..)| title.starts_with("First sync")),
            "the first sync's banner is still up"
        );
    });
}

pub fn a_failing_account_is_named_and_the_others_mail_stays_listed() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "Numbers.", 5)
            .await;
        let (second, inbox) = fixture.second_account().await;
        fixture
            .file_as(second.id, inbox, "Lunch on Friday", 10)
            .await;
        let (host, sink) = fixture.host_telling();
        let window = postio_gtk::window::FocusWindow::new(None);
        window.present();
        support::keep(postio_gtk::startup::adopt(
            &window,
            host,
            &postio_config::Config::default(),
        ));
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 2).await,
            "both accounts' mail should be listed: {:?}",
            support::subjects(&window)
        );
        let tell = |event| assert!(sink.emit(event), "the hub took the event");
        tell(Event::ConnectionChanged {
            account: fixture.account.id,
            state: ConnectionState::Online,
        });
        tell(Event::SyncProgress {
            account: fixture.account.id,
            done: 40,
            total: 40,
        });
        tell(Event::ConnectionChanged {
            account: second.id,
            state: ConnectionState::Failing {
                reason: FailureReason::Config,
            },
        });
        assert!(
            crate::settle_until(async || window
                .banner_showing()
                .is_some_and(|(title, ..)| title.starts_with("Second can't sync")))
            .await,
            "the failing account is not named: {:?}",
            window.banner_showing()
        );
        let (title, button, _) = window.banner_showing().expect("a banner");
        assert!(
            title.contains("settings are wrong"),
            "the banner gives the reason: {title}"
        );
        assert!(
            !title.contains("Can't sign in"),
            "it is not the password: {title}"
        );
        assert_eq!(button.as_deref(), Some("Retry now"));
        let mut mail = support::subjects(&window);
        mail.sort();
        assert_eq!(
            mail,
            ["Budget", "Lunch on Friday"],
            "the mail of the account that works, and what the other already synced, \
             stays listed"
        );

        let banner = support::only(&window, "focus-banner");
        let retry = support::descendants(&banner)
            .into_iter()
            .find(|widget| widget.is::<gtk::Button>() && widget.is_mapped())
            .expect("the banner's Retry is on screen");
        support::click(&window, &retry, 1);
        crate::settle();
        tell(Event::ConnectionChanged {
            account: second.id,
            state: ConnectionState::Online,
        });
        assert!(
            crate::settle_until(async || window.banner_showing().is_none()).await,
            "the banner stayed after the account came back: {:?}",
            window.banner_showing()
        );
    });
}

pub fn a_failing_banner_says_the_syncs_own_reason() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "Numbers.", 5)
            .await;
        let (second, inbox) = fixture.second_account().await;
        fixture
            .file_as(second.id, inbox, "Lunch on Friday", 10)
            .await;
        let (host, sink) = fixture.host_telling();
        let window = postio_gtk::window::FocusWindow::new(None);
        window.present();
        support::keep(postio_gtk::startup::adopt(
            &window,
            host,
            &postio_config::Config::default(),
        ));
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 2).await,
            "both accounts' mail should be listed: {:?}",
            support::subjects(&window)
        );
        let tell = |event| assert!(sink.emit(event), "the hub took the event");
        tell(Event::ConnectionChanged {
            account: fixture.account.id,
            state: ConnectionState::Online,
        });
        tell(Event::SyncProgress {
            account: fixture.account.id,
            done: 40,
            total: 40,
        });
        tell(Event::ConnectionChanged {
            account: second.id,
            state: ConnectionState::Failing {
                reason: FailureReason::Server,
            },
        });
        tell(Event::Error {
            message: "the server said: mailbox is over quota".to_owned(),
            account: Some(second.id),
        });
        assert!(
            crate::settle_until(async || window
                .banner_showing()
                .is_some_and(|(title, ..)| title.contains("mailbox is over quota")))
            .await,
            "the banner does not carry the sync's own reason: {:?}",
            window.banner_showing()
        );
        let (title, ..) = window.banner_showing().expect("a banner");
        assert!(
            title.starts_with("Second can't sync"),
            "and it names the account: {title}"
        );
        let mut mail = support::subjects(&window);
        mail.sort();
        assert_eq!(mail, ["Budget", "Lunch on Friday"], "the mail stays listed");
    });
}
