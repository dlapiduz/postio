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
use postio_model::{MailboxId, MessageId};
use postio_runtime::store::MailStore;
use postio_storage::Store;
use postio_ui::notify::{Attention, Notification};

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
    postio_config::Config::load_from_path(path)
        .ok()
        .map(|config| config.sync)
        .unwrap_or_default()
}

/// Everything `notify` needs that does not change per call.
///
/// Empty when the store's owner is another process: that owner decides
/// every arrival and sends the one frontend it elects a `Notify`
/// ([`deliver_from`]), so the window's own event drain has nothing to do.
#[derive(Clone)]
pub struct Notifier(Option<Reads>);

#[derive(Clone)]
struct Reads {
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
        Self(Some(Reads {
            database,
            store,
            runtime,
            config,
        }))
    }

    /// A notifier for a window whose store's owner is another process,
    /// which decides and sends the notifications itself.
    pub fn from_the_owner() -> Self {
        Self(None)
    }

    /// Notifies about `messages` having arrived in `mailbox`, if `[sync]`
    /// says this mailbox's arrivals are worth one and `attention` says the
    /// user is not already looking at them.
    ///
    /// Every read is on `self.runtime`, never the main loop: building a
    /// notification is not on any interaction's budget.
    pub async fn notify(
        &self,
        window: &Window,
        mailbox: MailboxId,
        messages: &[MessageId],
        attention: Attention,
    ) {
        let Some(reads) = &self.0 else {
            return;
        };
        if messages.is_empty() {
            return;
        }
        // Every read on the runtime, in one task (#1608). The mailbox's role
        // and the account's label used to be awaited here, and `notify` runs
        // inside the single event drain, so every event queued behind a
        // `NewMail` waited on the GTK thread for two fresh store connections.
        // The decision is the host's (`postio_host::notify`), the same one a
        // daemon makes for a frontend over its socket.
        let ids: Vec<MessageId> = messages.to_vec();
        let store = reads.store.clone();
        let database = reads.database.clone();
        let config = reads.config.clone();
        let (sender, receiver) = async_channel::bounded(1);
        reads.runtime.spawn(async move {
            let decided = postio_host::notify::decide_arrival(
                &database,
                store.as_ref(),
                &config,
                mailbox,
                &ids,
                attention,
            )
            .await;
            if let Some(notification) = decided {
                let _ = sender.send(notification).await;
            }
        });

        let Some(application) = window.application() else {
            return;
        };
        glib::spawn_future_local(async move {
            if let Ok(notification) = receiver.recv().await {
                deliver(&application, &notification);
            }
        });
    }
}

/// Deliver every notification the store's owner sends `client`, for as long
/// as `application` runs, and keep the owner told what the window is
/// showing so it can hold back mail already on screen.
///
/// Only the frontend the owner elected is sent any (`postio_host::notify`),
/// so two windows on one store raise one notification.
pub fn deliver_from(
    application: &gtk::Application,
    window: &Window,
    feeds: &postio_gtk::feed::Feeds,
    client: &postio_client::Client,
) {
    let application = application.downgrade();
    glib::spawn_future_local({
        let client = client.clone();
        async move {
            // From whichever owner is there: after a reconnect, the new
            // connection's notifications.
            let mut reconnected = client.reconnected();
            loop {
                let notices = client.notifications();
                while let Ok(notification) = notices.recv().await {
                    let Some(application) = application.upgrade() else {
                        return;
                    };
                    deliver(&application, &notification);
                }
                if reconnected.changed().await.is_err() {
                    return;
                }
            }
        }
    });

    // Posted when either half changes, never asked at the arrival: the
    // owner decides without a round trip to this window.
    let tell = {
        let client = client.clone();
        let feeds = feeds.clone();
        let window = window.downgrade();
        move || {
            if let Some(window) = window.upgrade() {
                client.attention(Attention {
                    showing: feeds.messages.mailbox(),
                    active: window.is_active(),
                });
            }
        }
    };
    feeds.messages.connect_opened({
        let tell = tell.clone();
        move || tell()
    });
    window.connect_is_active_notify(move |_| tell());
}

/// Post a decided notification, replacing the one already showing for its
/// folder. The whole of this frontend's half: the host decided it.
pub fn deliver(application: &impl IsA<gio::Application>, notification: &Notification) {
    application.send_notification(Some(&notification.identifier), &build(notification));
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
}
