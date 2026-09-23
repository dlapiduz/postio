//! Desktop notifications for new mail: the delivery half.
//!
//! `Event::NewMail` existed with a doc comment naming it "the trigger for a
//! desktop notification", was already consumed by `postio_gtk::feed` for the
//! insert-at-top scroll behaviour, and nothing ever turned it into a
//! notification (`postio-du6`, another `postio-bl2` instance). This module
//! is that other half.
//!
//! **The decision is not made here.** Whether an arrival is worth
//! interrupting somebody for, which id coalesces it, where a click lands and
//! what the words are all come from [`postio_ui::notify`], the one rule the
//! macOS app calls too. This module reads the rows the wording needs, asks,
//! and posts the answer — the same split the macOS `MailNotifications` has,
//! for the same reason: `gio::Notification` has no getters, so the half that
//! can be asserted on has to be the half that has no toolkit in it.
//!
//! # Through `gio::Notification`, not a lower-level portal call
//!
//! `gio::Application::send_notification` is the GNOME-idiomatic path
//! rather than talking to `org.freedesktop.portal.Notification` directly: on
//! a sandboxed build it already goes through that portal without this module
//! needing to know, and either way it is the desktop shell — not this
//! process — that decides whether Do Not Disturb suppresses it. Nothing here
//! re-implements either.
//!
//! # What a click does
//!
//! Presents the window, then switches to the mailbox the mail landed in and
//! puts the cursor on the message the notification named.
//! `Window::open_mailbox` and `Window::open_message` (`postio-gtk`) are what
//! make this possible from outside a click on an already-visible row;
//! [`build`] is where the click's target is encoded onto the notification,
//! in [`RAISE_ACTION`]'s own string parameter, and [`install_action`] is
//! where it is read back.

use std::sync::Arc;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;

use postio_config::SyncConfig;
use postio_gtk::window::Window;
use postio_model::ids::AccountId;
use postio_model::{MailboxId, MailboxRole, MessageId};
use postio_runtime::store::{MailStore, MessageSummary};
use postio_storage::Store;
use postio_storage::repository::{AccountRepository, MailboxRepository};
use postio_ui::notify::{self, Attention, Decision, Notification, Wording};

/// The action a click on a notification runs. Application-scoped because a
/// notification's default action activates whether or not any window
/// currently has focus.
const RAISE_ACTION: &str = "raise-for-mail";

/// Registers [`RAISE_ACTION`] on `application`, so a notification's click
/// target exists before the first one is ever sent.
pub fn install_action(application: &impl IsA<gio::ActionMap>, window: &Window) {
    let raise = gio::SimpleAction::new(RAISE_ACTION, Some(glib::VariantTy::STRING));
    raise.connect_activate(glib::clone!(
        #[weak]
        window,
        move |_, parameter| {
            window.present();
            let Some((mailbox, message)) = parameter
                .and_then(|value| value.str())
                .and_then(parse_target)
            else {
                return;
            };
            match message {
                Some(message) => window.open_message(mailbox, message),
                None => window.open_mailbox(mailbox),
            }
        }
    ));
    application.add_action(&raise);
}

/// Loads `[sync]` from `path`, or the defaults for a first run or a file
/// that will not parse.
///
/// Read once at startup rather than kept live: unlike `[logging]`, a setting
/// this small does not need to change without restarting the app, and
/// `logging::config_at` is the pattern this mirrors for the same reason.
pub fn config_at(path: &std::path::Path) -> SyncConfig {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| postio_config::Config::from_toml_str(&text).ok())
        .map(|config| config.sync)
        .unwrap_or_default()
}

/// Everything `notify` needs that does not change per call.
#[derive(Clone)]
pub struct Notifier {
    database: Store,
    store: Arc<dyn MailStore>,
    runtime: tokio::runtime::Handle,
    config: SyncConfig,
}

impl Notifier {
    /// Builds a notifier over `wiring`'s store and `config`'s `[sync]`
    /// settings.
    pub fn new(
        database: Store,
        store: Arc<dyn MailStore>,
        runtime: tokio::runtime::Handle,
        config: SyncConfig,
    ) -> Self {
        Self {
            database,
            store,
            runtime,
            config,
        }
    }

    /// Notifies about `messages` having arrived in `mailbox`, if `[sync]`
    /// says this mailbox's arrivals are worth one and `attention` says the
    /// user is not already looking at them.
    ///
    /// The mailbox lookup is one indexed row, done synchronously like
    /// `compose.rs`'s small bounded reads — not the message read, which
    /// goes through `store.message_rows` on `self.runtime` the way every
    /// other read from this crate does, because building a notification body
    /// is not on any interaction's budget and must never hold the main loop.
    pub async fn notify(
        &self,
        window: &Window,
        mailbox: MailboxId,
        messages: &[MessageId],
        attention: Attention,
    ) {
        if messages.is_empty() {
            return;
        }
        // Every read on the runtime, in one task (#1608). The mailbox's role
        // and the account's label used to be awaited here, and `notify` runs
        // inside the single event drain, so every event queued behind a
        // `NewMail` waited on the GTK thread for two fresh store connections.
        // The store is cloned in -- it is an `Arc` inside -- which is what
        // lets a `'static` task carry it.
        let ids: Vec<MessageId> = messages.to_vec();
        let store = self.store.clone();
        let database = self.database.clone();
        let config = self.config.clone();
        let (sender, receiver) = async_channel::bounded(1);
        self.runtime.spawn(async move {
            let Some((role, account)) = mailbox_info(&database, mailbox).await else {
                return;
            };
            if !notify::watched(&config, role) {
                return;
            }
            let account_name = account_label(&database, account).await;
            let rows = store.message_rows(ids).await;
            let _ = sender.send((rows, account_name)).await;
        });

        let Some(application) = window.application() else {
            return;
        };
        glib::spawn_future_local(async move {
            let Ok((Ok(rows), account_name)) = receiver.recv().await else {
                return;
            };
            let Decision::Deliver(notification) =
                decision(mailbox, &rows, attention, account_name.as_deref())
            else {
                return;
            };
            application.send_notification(Some(&notification.identifier), &build(&notification));
        });
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
async fn account_label(database: &Store, account: AccountId) -> Option<String> {
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
/// worded the way this frontend words it: the newest arrival's sender and
/// subject ([`Wording::Newest`] — the module docs there say why the macOS
/// app chooses differently).
///
/// The newest is whichever `received_at` is latest. A position in `rows`
/// says nothing about arrival order — `notify` hands this whatever
/// `store.message_rows` returned for the ids `Event::NewMail` carried, and
/// nothing along that path promises newest-last (or first).
fn decision(
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

/// The `gio::Notification` for a decided [`Notification`], with its click
/// target encoded onto [`RAISE_ACTION`].
fn build(notification: &Notification) -> gio::Notification {
    let built = gio::Notification::new(&notification.title);
    built.set_body(Some(&notification.body));
    built.set_default_action_and_target_value(
        &format!("app.{RAISE_ACTION}"),
        Some(&encode_target(notification.mailbox, notification.message).to_variant()),
    );
    built
}

/// `RAISE_ACTION`'s parameter: a mailbox, and optionally the one message the
/// notification named.
///
/// A plain string rather than a `(x, mx)` tuple variant: this crate has no
/// other use for GVariant's maybe-type machinery, and a delimited string is
/// exactly as much of it as the action actually needs.
fn encode_target(mailbox: MailboxId, message: Option<MessageId>) -> String {
    match message {
        Some(message) => format!("{}:{}", mailbox.get(), message.get()),
        None => mailbox.get().to_string(),
    }
}

/// The other half of [`encode_target`]. `None` for a parameter this build
/// does not recognise — a notification from a future version of Postio,
/// say — which the caller treats as "just present the window", exactly
/// what a click did before this action carried a target at all.
fn parse_target(value: &str) -> Option<(MailboxId, Option<MessageId>)> {
    let mut parts = value.splitn(2, ':');
    let mailbox: i64 = parts.next()?.parse().ok()?;
    let message = match parts.next() {
        Some(text) => Some(MessageId::new(text.parse().ok()?)),
        None => None,
    };
    Some((MailboxId::new(mailbox), message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_model::EmailAddress;

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

    #[test]
    fn a_target_round_trips_through_encode_and_parse() {
        assert_eq!(
            parse_target(&encode_target(MailboxId::new(7), Some(MessageId::new(42)))),
            Some((MailboxId::new(7), Some(MessageId::new(42))))
        );
        assert_eq!(
            parse_target(&encode_target(MailboxId::new(7), None)),
            Some((MailboxId::new(7), None)),
            "a folder-only target has to round-trip too"
        );
    }

    #[test]
    fn an_unparseable_target_is_none_rather_than_a_panic() {
        // A future build could send a shape this one does not recognise; the
        // click has to fall back to just presenting the window, not crash.
        assert_eq!(parse_target(""), None);
        assert_eq!(parse_target("not-a-number"), None);
        assert_eq!(parse_target("7:not-a-number"), None);
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
            Decision::Suppress(notify::Suppressed::AlreadyOnScreen)
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
