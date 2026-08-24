//! Desktop notifications for new mail.
//!
//! `Event::NewMail` existed with a doc comment naming it "the trigger for a
//! desktop notification", was already consumed by `postio_gtk::feed` for the
//! insert-at-top scroll behaviour, and nothing ever turned it into a
//! notification (`postio-du6`, another `postio-bl2` instance). This module
//! is that other half.
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
//! # Coalescing
//!
//! Every notification for one mailbox reuses the same id
//! (`"new-mail-<mailbox>"`), which is what `gio::Application::send_notification`
//! treats as "replace the one already showing" rather than "queue another
//! popup beside it" — so several `IDLE` wake-ups in a row settle into the one
//! notification on screen actually saying, rather than a burst of them.
//!
//! # What a click does today
//!
//! Presents the window. It does not yet switch to the mailbox the mail
//! landed in or select the message: `postio-gtk` has no call that does
//! either from outside a click on an already-visible row — `window.list()`'s
//! activation is the reverse direction, a signal the list emits, not
//! something this module can drive. Filed as `postio-du6`'s own follow-up
//! rather than guessed at here.

use std::sync::Arc;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;

use postio_config::SyncConfig;
use postio_gtk::window::Window;
use postio_model::{MailboxId, MailboxRole, MessageId};
use postio_runtime::store::MailStore;
use postio_storage::Database;
use postio_storage::repository::MailboxRepository;

/// The action a click on a notification runs. Application-scoped because a
/// notification's default action activates whether or not any window
/// currently has focus.
const RAISE_ACTION: &str = "raise-for-mail";

/// Registers [`RAISE_ACTION`] on `application`, so a notification's click
/// target exists before the first one is ever sent.
pub fn install_action(application: &impl IsA<gio::ActionMap>, window: &Window) {
    let raise = gio::SimpleAction::new(RAISE_ACTION, None);
    raise.connect_activate(glib::clone!(
        #[weak]
        window,
        move |_, _| window.present()
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
    database: Database,
    store: Arc<dyn MailStore>,
    runtime: tokio::runtime::Handle,
    config: SyncConfig,
}

impl Notifier {
    /// Builds a notifier over `wiring`'s store and `config`'s `[sync]`
    /// settings.
    pub fn new(
        database: Database,
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
    /// says this mailbox's arrivals are worth one.
    ///
    /// The mailbox lookup is one indexed row, done synchronously like
    /// `compose.rs`'s small bounded reads — not the message read, which
    /// goes through `store.message_rows` on `self.runtime` the way every
    /// other read from this crate does, because building a notification body
    /// is not on any interaction's budget and must never hold the main loop.
    pub fn notify(&self, window: &Window, mailbox: MailboxId, messages: &[MessageId]) {
        if messages.is_empty() {
            return;
        }
        let role = match mailbox_role(&self.database, mailbox) {
            Some(role) => role,
            None => return,
        };
        if !role_may_notify(&self.config, role) {
            return;
        }

        let Some(application) = window.application() else {
            return;
        };
        let ids: Vec<MessageId> = messages.to_vec();
        let store = self.store.clone();
        let (sender, receiver) = async_channel::bounded(1);
        self.runtime.spawn(async move {
            let rows = store.message_rows(ids).await;
            let _ = sender.send(rows).await;
        });

        glib::spawn_future_local(async move {
            let Ok(Ok(rows)) = receiver.recv().await else {
                return;
            };
            if rows.is_empty() {
                return;
            }
            application.send_notification(Some(&notification_id(mailbox)), &build(&rows));
        });
    }
}

/// A notification id scoped to one mailbox, so a second batch of arrivals
/// replaces the first rather than stacking beside it. See the module docs.
fn notification_id(mailbox: MailboxId) -> String {
    format!("new-mail-{}", mailbox.get())
}

/// Whether `[sync]` says `role`'s arrivals are worth a notification.
fn role_may_notify(config: &SyncConfig, role: MailboxRole) -> bool {
    config.notify && config.notify_roles.iter().any(|name| name == role.as_str())
}

/// What one arrived mailbox's role is, or `None` for a store this read
/// cannot reach — never a reason to fail the sync pass that called this.
fn mailbox_role(database: &Database, mailbox: MailboxId) -> Option<MailboxRole> {
    let connection = database
        .connection()
        .map_err(|error| tracing::warn!(%error, "could not read the mailbox to notify about"))
        .ok()?;
    MailboxRepository::new(&connection)
        .get(mailbox)
        .map_err(|error| tracing::warn!(%error, "could not read the mailbox to notify about"))
        .ok()?
        .map(|mailbox| mailbox.role)
}

/// The title and body a batch of arrivals reads as: one sender and subject
/// for a single arrival, a count for a burst.
///
/// Pure on purpose — `gio::Notification` has no getters to assert against
/// (it is a write-only description, sent rather than introspected), so this
/// is the half of [`build`] a test can actually check.
fn content(rows: &[postio_runtime::store::MessageSummary]) -> (String, String) {
    if let [only] = rows {
        let from = only
            .from
            .as_ref()
            .map(|address| address.display().to_owned())
            .unwrap_or_else(|| "Someone".to_owned());
        let subject = only
            .subject
            .clone()
            .unwrap_or_else(|| "(no subject)".to_owned());
        (from, subject)
    } else {
        (
            "New mail".to_owned(),
            format!("{} new messages", rows.len()),
        )
    }
}

/// The notification itself, from [`content`].
fn build(rows: &[postio_runtime::store::MessageSummary]) -> gio::Notification {
    let (title, body) = content(rows);
    let notification = gio::Notification::new(&title);
    notification.set_body(Some(&body));
    notification.set_default_action(&format!("app.{RAISE_ACTION}"));
    notification
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_model::EmailAddress;
    use postio_runtime::store::MessageSummary;

    fn summary(from: &str, subject: &str) -> MessageSummary {
        MessageSummary {
            id: MessageId::new(1),
            thread: None,
            from: Some(EmailAddress::new(Some(from), format!("{from}@example.com"))),
            subject: Some(subject.to_owned()),
            preview: None,
            received_at: chrono::Utc::now(),
            seen: false,
            flagged: false,
            answered: false,
            draft: false,
            has_attachments: false,
            thread_count: 1,
        }
    }

    #[test]
    fn notify_settings_gate_on_both_the_switch_and_the_role() {
        let mut config = SyncConfig {
            notify: true,
            notify_roles: vec!["inbox".to_owned()],
            ..SyncConfig::default()
        };
        assert!(role_may_notify(&config, MailboxRole::Inbox));
        assert!(
            !role_may_notify(&config, MailboxRole::Archive),
            "archive was never asked for"
        );

        config.notify = false;
        assert!(
            !role_may_notify(&config, MailboxRole::Inbox),
            "the master switch must override an explicitly listed role"
        );
    }

    #[test]
    fn a_role_this_build_does_not_recognise_is_just_never_matched() {
        let config = SyncConfig {
            notify: true,
            notify_roles: vec!["not-a-real-role".to_owned()],
            ..SyncConfig::default()
        };
        assert!(!role_may_notify(&config, MailboxRole::Inbox));
    }

    #[test]
    fn each_mailbox_gets_one_stable_notification_id() {
        assert_eq!(notification_id(MailboxId::new(7)), "new-mail-7");
        assert_eq!(
            notification_id(MailboxId::new(7)),
            notification_id(MailboxId::new(7)),
            "a second arrival in the same mailbox must reuse the id, or it \
             stacks a second popup instead of replacing the first"
        );
        assert_ne!(
            notification_id(MailboxId::new(7)),
            notification_id(MailboxId::new(8))
        );
    }

    #[test]
    fn a_single_arrival_names_the_sender_and_the_subject() {
        let (title, body) = content(&[summary("Ada Lovelace", "Quarterly report")]);
        assert_eq!(title, "Ada Lovelace");
        assert_eq!(body, "Quarterly report");
    }

    #[test]
    fn a_burst_is_a_count_rather_than_one_popup_per_message() {
        let (_, body) = content(&[
            summary("Ada Lovelace", "One"),
            summary("Bob", "Two"),
            summary("Carol", "Three"),
        ]);
        assert_eq!(body, "3 new messages");
    }

    #[test]
    fn a_missing_subject_still_reads_as_a_sentence() {
        let mut only = summary("Ada Lovelace", "");
        only.subject = None;
        let (_, body) = content(&[only]);
        assert_eq!(body, "(no subject)");
    }
}
