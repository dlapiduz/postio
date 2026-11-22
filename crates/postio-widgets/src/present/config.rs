//! `config.toml` applied live: the watcher reparses and validates on its own thread, so a broken
//! file never costs a frame,
//! and [`follow`] hands the `Checked` result to the main loop. A failed reload leaves the last good
//! configuration and keymap.

use std::ops::ControlFlow;

use gtk::glib;
use postio_config::Checked;
use postio_config::watch::ConfigWatcher;
use postio_core::Event;
use postio_core::config::{ConfigService, ConfigUpdate};

/// `on_update` answers whether to keep following. Returns `false`, having followed nothing, when
/// the file cannot be
/// watched: edits then need a restart.
pub fn follow(
    mut service: ConfigService,
    mut on_update: impl FnMut(&ConfigService, &ConfigUpdate) -> ControlFlow<()> + 'static,
) -> bool {
    // Unbounded: the sender is a file watcher that already debounced a burst to one message, and
    // blocking it would be worse than queueing.
    let (sender, receiver) = async_channel::unbounded::<Checked>();
    let watcher = match ConfigWatcher::new(service.path(), move |checked| {
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
        // Moved in so the watcher lives exactly as long as the task reading from it; dropping it
        // stops the thread.
        let _watcher = watcher;
        while let Ok(checked) = receiver.recv().await {
            // One span per reload, so a file's problems are attributable to that reload. Nothing
            // here awaits, so entering it is sound.
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
