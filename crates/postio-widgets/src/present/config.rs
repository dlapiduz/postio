//! `config.toml`, applied live: the last hop, from the watcher's thread to
//! the main loop.
//!
//! `postio-config` has the watcher and `postio-core` the resolution onto the
//! command registry, and they meet across a thread boundary neither can
//! cross. The watcher reparses and validates on **its own thread**, so a
//! broken file never costs the interface a frame, and hands back a
//! `Checked`, which is `Send`. Every widget is main-thread only. [`follow`]
//! is the bridge: an `async_channel` whose sender goes to the watcher thread
//! and whose receiver is awaited on the main context, where the widgets are.
//!
//! A reload that fails validation leaves the last good configuration -- and
//! so the last good keymap -- exactly as it was, and says why: the user keeps
//! a working keyboard to fix the file with. That is `ConfigService`'s rule;
//! this only has to not undo it.

use std::ops::ControlFlow;

use gtk::glib;
use postio_config::Checked;
use postio_config::watch::ConfigWatcher;
use postio_core::Event;
use postio_core::config::{ConfigService, ConfigUpdate};

/// Follow `service`'s file: every reload the watcher validates is applied to
/// `service` on the main loop, then handed to `on_update` with what changed.
///
/// `on_update` answers whether to keep following: an app stops when the
/// window it applies to has gone. Returns `false`, having followed nothing,
/// when the file cannot be watched -- edits then need a restart, which is
/// logged here, and the configuration loaded at start stands.
pub fn follow(
    mut service: ConfigService,
    mut on_update: impl FnMut(&ConfigService, &ConfigUpdate) -> ControlFlow<()> + 'static,
) -> bool {
    // Unbounded because the sender is a file watcher that has already
    // debounced a burst of save events down to one message, and because
    // blocking that thread would be worse than queueing.
    let (sender, receiver) = async_channel::unbounded::<Checked>();
    let watcher = match ConfigWatcher::new(service.path(), move |checked| {
        // `send_blocking` on the watcher's own thread, which is allowed to
        // block and has nothing else to do.
        let _ = sender.send_blocking(checked);
    }) {
        Ok(watcher) => watcher,
        Err(error) => {
            tracing::warn!(
                path = %service.path().display(),
                %error,
                "config will not be watched; edits need a restart"
            );
            return false;
        }
    };
    glib::spawn_future_local(async move {
        // The watcher is moved in so that it lives exactly as long as the task
        // reading from it. Dropping it stops the thread.
        let _watcher = watcher;
        while let Ok(checked) = receiver.recv().await {
            // One span per reload, so the problems a file produced are
            // attributable to *that* reload rather than to whichever of the
            // day's edits happened to be nearest in the log. Nothing here
            // awaits, so entering it for the body is sound.
            let reload = tracing::info_span!("config_reload", path = %service.path().display());
            let _entered = reload.enter();

            let update = service.apply(checked);
            for event in &update.events {
                if let Event::Error { message, .. } = event {
                    tracing::warn!(message, "rejected");
                }
            }
            tracing::debug!(keys = update.changed.keys, "applied");
            if on_update(&service, &update).is_break() {
                break;
            }
        }
    });
    true
}
