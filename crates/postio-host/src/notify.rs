//! New mail, told to exactly one frontend.
//!
//! With two frontends on one store, an arrival must still raise one desktop
//! notification, not one per window (research R1c). So the host decides:
//! whether this folder's arrivals notify at all (`[sync]`), whether the
//! person is already looking at them, and what the notification says --
//! `postio_ui::notify::decide`, unchanged -- and then sends the answer to one
//! frontend, which only delivers it.
//!
//! **Which frontend** is an election held at each arrival: the first desktop
//! app still connected, since a click on its notification raises its window;
//! else the first terminal. A test client or the macOS frontend is never
//! elected. With nobody to deliver it, nothing is read and nothing is said.

use std::sync::Weak;

use postio_client::protocol::{ClientId, ClientKind};
use postio_config::SyncConfig;
use postio_core::Event;
use postio_core::bridge::EventStream;
use postio_model::listing::{MailStore, MessageSummary};
use postio_model::{AccountId, MailboxId, MailboxRole, MessageId};
use postio_storage::Store;
use postio_storage::repository::{AccountRepository, MailboxRepository};
use postio_ui::notify::{self, Attention, Decision, Notification, Wording};

use crate::Inner;

/// Who delivers notifications, among `clients`: the first connected
/// desktop app, else the first connected terminal.
///
/// "First" is the lowest id: ids are handed out in the order clients
/// connect, and never reused.
pub fn elect(clients: impl IntoIterator<Item = (ClientId, ClientKind)>) -> Option<ClientId> {
    let mut desktop = None;
    let mut terminal = None;
    for (id, kind) in clients {
        let first = match kind {
            ClientKind::Gtk => &mut desktop,
            ClientKind::Tui => &mut terminal,
            ClientKind::Ffi | ClientKind::Test => continue,
        };
        if first.is_none_or(|earlier: ClientId| id.0 < earlier.0) {
            *first = Some(id);
        }
    }
    desktop.or(terminal)
}

/// Hear every arrival for as long as the host lives, and tell the elected
/// frontend about each one worth a notification.
///
/// One arrival at a time, in the order they came: a notification per folder
/// replaces the one before it, so two raced would leave the older showing.
pub(crate) async fn run(inner: Weak<Inner>, arrivals: EventStream) {
    while let Some(event) = arrivals.next().await {
        let Event::NewMail {
            mailbox, messages, ..
        } = event
        else {
            continue;
        };
        let Some(inner) = inner.upgrade() else {
            return;
        };
        // Elected before anything is read: with nobody to tell, there is
        // nothing worth reading.
        let elected = {
            let clients = inner.clients.lock().expect("never poisoned");
            elect(clients.iter().map(|(id, entry)| (*id, entry.kind)))
                .and_then(|id| clients.get(&id).map(|entry| (id, entry.attention)))
        };
        let Some((client, attention)) = elected else {
            continue;
        };
        let config = inner.notify.lock().expect("never poisoned").clone();
        let decided = decide_arrival(
            &inner.wiring.database,
            inner.wiring.store.as_ref(),
            &config,
            mailbox,
            &messages,
            attention,
        )
        .await;
        let Some(notification) = decided else {
            continue;
        };
        // Whoever is elected now: the one elected above may have left while
        // the rows were read.
        let notices = inner
            .clients
            .lock()
            .expect("never poisoned")
            .get(&client)
            .map(|entry| entry.notices.clone());
        if let Some(notices) = notices {
            tracing::debug!(client = client.0, mailbox = mailbox.get(), "new mail told");
            let _ = notices.try_send(notification);
        }
    }
}

/// Whether `messages` arriving in `mailbox` is worth a notification, and
/// what it says: `[sync]`'s folder gate, then [`notify::decide`] over the
/// rows, worded with the newest arrival's sender and subject.
///
/// The one decision every frontend's notification comes from, whether this
/// host sends it over a socket or a frontend in this process asks it itself.
/// A store that cannot be read is no notification, never an error.
pub async fn decide_arrival(
    database: &Store,
    store: &dyn MailStore,
    config: &SyncConfig,
    mailbox: MailboxId,
    messages: &[MessageId],
    attention: Attention,
) -> Option<Notification> {
    if messages.is_empty() {
        return None;
    }
    let (role, account) = mailbox_info(database, mailbox).await?;
    if !notify::watched(config, role) {
        return None;
    }
    let account_name = account_label(database, account).await;
    let rows = store.message_rows(messages.to_vec()).await.ok()?;
    match decision(mailbox, &rows, attention, account_name.as_deref()) {
        Decision::Deliver(notification) => Some(notification),
        Decision::Suppress(_) => None,
    }
}

/// What a store this read cannot reach yields: `None`, never a reason to
/// fail the sync pass that called this.
async fn mailbox_info(database: &Store, mailbox: MailboxId) -> Option<(MailboxRole, AccountId)> {
    let connection = database
        .connect()
        .await
        .map_err(|error| tracing::warn!(%error, "could not read the mailbox to notify about"))
        .ok()?;
    MailboxRepository::new(&connection)
        .get(mailbox)
        .await
        .map_err(|error| tracing::warn!(%error, "could not read the mailbox to notify about"))
        .ok()?
        .map(|mailbox| (mailbox.role, mailbox.account_id))
}

/// The name to put on a notification for `account`, or `None` when only one
/// account is enabled — naming the only account there is would be noise, not
/// information (ADR 0005 Q13).
pub async fn account_label(database: &Store, account: AccountId) -> Option<String> {
    let connection = database
        .connect()
        .await
        .map_err(|error| tracing::warn!(%error, "could not read the accounts to notify about"))
        .ok()?;
    let repository = AccountRepository::new(&connection);
    let enabled = repository
        .list_enabled()
        .await
        .map_err(|error| tracing::warn!(%error, "could not read the accounts to notify about"))
        .ok()?;
    if enabled.len() < 2 {
        return None;
    }
    repository
        .get(account)
        .await
        .map_err(|error| tracing::warn!(%error, "could not read the account to notify about"))
        .ok()?
        .map(|account| account.display_name)
}

/// What [`postio_ui::notify`] says about `rows` having landed in `mailbox`,
/// worded with the newest arrival's sender and subject ([`Wording::Newest`]).
///
/// The newest is whichever `received_at` is latest. A position in `rows`
/// says nothing about arrival order — `rows` is whatever
/// `store.message_rows` returned for the ids `Event::NewMail` carried, and
/// nothing along that path promises newest-last (or first).
pub fn decision(
    mailbox: MailboxId,
    rows: &[MessageSummary],
    attention: Attention,
    account: Option<&str>,
) -> Decision {
    let arrival = notify::Arrival {
        mailbox,
        messages: rows.iter().map(|row| row.id).collect(),
    };
    let Some(newest) = rows.iter().max_by_key(|row| row.received_at) else {
        return notify::decide(&arrival, attention, Wording::Counts { mailbox_name: None });
    };
    let from = newest
        .from
        .as_ref()
        .map(|address| address.display().to_owned());
    notify::decide(
        &arrival,
        attention,
        Wording::Newest {
            message: newest.id,
            from: from.as_deref(),
            subject: newest.subject.as_deref(),
            account,
        },
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use std::time::Duration;

    use postio_client::Client;
    use postio_ui::notify::{Attention, Notification};

    use postio_model::EmailAddress;
    use postio_ui::notify::Suppressed;

    use super::*;
    use crate::tests::World;

    fn summary(id: i64, from: &str, subject: &str) -> MessageSummary {
        MessageSummary {
            id: MessageId::new(id),
            thread: None,
            from: Some(EmailAddress::new(Some(from), format!("{from}@example.com"))),
            subject: Some(subject.to_owned()),
            preview: None,
            received_at: chrono::Utc::now(),
            seen: false,
            flagged: false,
            answered: false,
            send_state: None,
            send_at: None,
            has_attachments: false,
            thread_count: 1,
        }
    }

    /// `row`, arrived at `when` — a burst's "newest" is a fact about
    /// `received_at`, not about a row's position in the slice, so a test
    /// that wants to prove that has to control it.
    fn at(row: MessageSummary, when: chrono::DateTime<chrono::Utc>) -> MessageSummary {
        MessageSummary {
            received_at: when,
            ..row
        }
    }

    fn delivered(decision: Decision) -> Notification {
        match decision {
            Decision::Deliver(notification) => notification,
            Decision::Suppress(reason) => panic!("expected a notification, got {reason:?}"),
        }
    }

    /// The world's message arrives, as the engine says so.
    pub(crate) fn arrive(world: &World) {
        world.host().wiring().events.emit(Event::NewMail {
            account: world.account,
            mailbox: world.inbox(),
            messages: vec![world.message()],
        });
    }

    /// The next notification `client` is asked to deliver, if one comes.
    fn told(world: &World, client: &Client) -> Option<Notification> {
        let notices = client.notifications();
        world.rt.block_on(async {
            tokio::time::timeout(Duration::from_millis(500), notices.recv())
                .await
                .ok()
                .and_then(Result::ok)
        })
    }

    #[test]
    fn the_first_desktop_app_is_elected_over_an_earlier_terminal() {
        let clients = [
            (ClientId(1), ClientKind::Tui),
            (ClientId(3), ClientKind::Gtk),
            (ClientId(2), ClientKind::Gtk),
        ];
        assert_eq!(elect(clients), Some(ClientId(2)));
    }

    #[test]
    fn with_no_desktop_app_the_first_terminal_is_elected_and_nobody_else_ever() {
        assert_eq!(
            elect([
                (ClientId(4), ClientKind::Tui),
                (ClientId(1), ClientKind::Test),
                (ClientId(2), ClientKind::Ffi),
                (ClientId(3), ClientKind::Tui),
            ]),
            Some(ClientId(3))
        );
        assert_eq!(
            elect([
                (ClientId(1), ClientKind::Test),
                (ClientId(2), ClientKind::Ffi)
            ]),
            None
        );
    }

    #[test]
    fn an_arrival_is_told_to_the_desktop_app_once_then_to_the_terminal_when_it_leaves() {
        let world = World::new();
        let (terminal, _) = world.frontend(ClientKind::Tui);
        let (desktop, _) = world.frontend(ClientKind::Gtk);

        arrive(&world);
        let notification = told(&world, &desktop).expect("the desktop app is told");
        assert_eq!(notification.mailbox, world.inbox());
        assert_eq!(notification.message, Some(world.message()));
        assert_eq!(
            told(&world, &desktop),
            None,
            "one arrival, one notification"
        );
        assert_eq!(
            told(&world, &terminal),
            None,
            "and the terminal says nothing"
        );

        drop(desktop);
        arrive(&world);
        let notification = told(&world, &terminal).expect("the terminal is told now");
        assert_eq!(notification.mailbox, world.inbox());
    }

    #[test]
    fn mail_landing_where_the_elected_frontend_is_looking_is_not_told() {
        let world = World::new();
        let (desktop, _) = world.frontend(ClientKind::Gtk);
        desktop.attention(Attention {
            showing: Some(world.inbox()),
            active: true,
        });

        arrive(&world);
        assert_eq!(told(&world, &desktop), None);

        // Behind another application, the same folder is not being watched.
        desktop.attention(Attention {
            showing: Some(world.inbox()),
            active: false,
        });
        arrive(&world);
        assert!(told(&world, &desktop).is_some());
    }

    #[test]
    fn a_folder_sync_does_not_watch_is_not_told() {
        let world = World::new();
        world.host().notify_with(postio_config::SyncConfig {
            notify: false,
            ..Default::default()
        });
        let (desktop, _) = world.frontend(ClientKind::Gtk);
        arrive(&world);
        assert_eq!(told(&world, &desktop), None);

        world
            .host()
            .notify_with(postio_config::SyncConfig::default());
        arrive(&world);
        assert!(
            told(&world, &desktop).is_some(),
            "the inbox is watched by default"
        );
    }

    #[test]
    fn a_burst_is_named_after_its_newest_arrival_by_received_at() {
        // Deliberately out of arrival order: the newest is the middle one,
        // so a fix that just reads rows[0] or rows.last() cannot pass this.
        let base = chrono::Utc::now();
        let notification = delivered(decision(
            MailboxId::new(7),
            &[
                at(summary(1, "Ada Lovelace", "One"), base),
                at(
                    summary(99, "Carol", "Three"),
                    base + chrono::Duration::minutes(2),
                ),
                at(
                    summary(2, "Bob", "Two"),
                    base + chrono::Duration::minutes(1),
                ),
            ],
            Attention::default(),
            None,
        ));
        assert_eq!(
            notification.title, "Carol",
            "the newest arrival's sender, not the first one that arrived"
        );
        assert_eq!(notification.body, "\"Three\" and 2 more");
        assert_eq!(
            notification.message,
            Some(MessageId::new(99)),
            "the click should land on the message the notification actually named"
        );
    }

    #[test]
    fn this_frontend_draws_the_sender_and_names_the_account() {
        let notification = delivered(decision(
            MailboxId::new(7),
            &[summary(42, "Ada Lovelace", "Quarterly report")],
            Attention::default(),
            Some("Work"),
        ));
        assert_eq!(notification.title, "Ada Lovelace — Work");
        assert_eq!(notification.body, "Quarterly report");
        assert_eq!(notification.identifier, "new-mail-7");
    }

    #[test]
    fn mail_landing_in_the_open_mailbox_of_the_active_window_is_not_posted() {
        let attention = Attention {
            showing: Some(MailboxId::new(7)),
            active: true,
        };
        assert_eq!(
            decision(
                MailboxId::new(7),
                &[summary(42, "Ada Lovelace", "Quarterly report")],
                attention,
                None
            ),
            Decision::Suppress(Suppressed::AlreadyOnScreen)
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn account_label_is_none_with_exactly_one_enabled_account() {
        let database = postio_storage::test_support::memory().await;
        let account = {
            let connection = database.connect().await.expect("a connection");
            postio_storage::test_support::account(&connection).await
        };
        assert_eq!(
            account_label(&database, account.id).await,
            None,
            "a single-account install must read exactly as it did before #189"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn account_label_names_the_account_once_a_second_is_enabled() {
        let database = postio_storage::test_support::memory().await;
        let (first, second) = {
            let connection = database.connect().await.expect("a connection");
            let first = postio_storage::test_support::account(&connection).await;
            let mut second = postio_model::Account::new(
                "Work",
                EmailAddress::new(None::<String>, "grace@example.com"),
            );
            AccountRepository::new(&connection)
                .create(&mut second)
                .await
                .expect("create the second account");
            (first, second)
        };
        assert_eq!(
            account_label(&database, second.id).await,
            Some("Work".to_owned())
        );
        assert_eq!(
            account_label(&database, first.id).await,
            Some(first.display_name.clone()),
            "both accounts get named once there is more than one"
        );
    }
}
