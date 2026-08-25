//! ADR 0012 Q4-Q6: showing the first-run keyboard orientation once, after
//! the first successful sync, and remembering that it has been shown.
//!
//! `postio-gtk`'s [`OrientationPlate`](postio_gtk::orientation::OrientationPlate)
//! owns the widget's shape and text; this module owns the two questions it
//! cannot answer for itself without SQLite, which it may not depend on:
//! *has this been shown before*, and *when did it first make sense to show
//! it at all*.

use gtk::glib;
use postio_gtk::feed::Feeds;
use postio_gtk::window::Window;
use postio_storage::repository::SettingsRepository;

use crate::Wiring;

/// The `settings` row this feature owns. Global, not per account — ADR 0012
/// Q6: a second account joining a running app must not show this again.
const SEEN_KEY: &str = "orientation_seen";

/// Wire the plate to the sync engine and to every command the window runs.
pub fn install(window: &Window, wiring: &Wiring, feeds: &Feeds) {
    // Owned from here on: the closures below outlive this call, and `Wiring`
    // is cheap to clone (every field is a handle -- a connection pool, a
    // runtime handle -- not the data behind it).
    let wiring = wiring.clone();

    // The first successful sync, and only the first: `SyncStatus::last_sync`
    // goes from `None` to `Some` exactly once per mailbox set and stays
    // `Some` forever after, so a plain "has it arrived" check on every
    // status update would ask the database again on every later sync too.
    // The plate's own visibility is a cheaper guard against that than a
    // second flag would be -- if it is already showing, or already
    // dismissed, there is nothing new to decide.
    feeds.folders.connect_status(glib::clone!(
        #[weak]
        window,
        #[strong]
        wiring,
        move |status| {
            if status.last_sync.is_some() {
                maybe_show(&window, &wiring);
            }
        }
    ));

    // ADR 0012 Q6: the trigger is the first *command* the window runs, not
    // any GDK key event -- a modifier press or typing into search must not
    // retire something the user has not actually demonstrated knowledge of.
    window.connect_action(glib::clone!(
        #[weak]
        window,
        #[strong]
        wiring,
        move |_command| retire(&window, &wiring)
    ));

    window.orientation().connect_dismissed(glib::clone!(
        #[weak]
        window,
        #[strong]
        wiring,
        move || retire(&window, &wiring)
    ));
}

/// Show the plate, unless it has already been seen in some earlier run.
///
/// A cheap, synchronous check (`is_visible`) would not see across a restart;
/// the database is where "seen" actually lives (ADR 0012 Q6), so this reads
/// it on the blocking pool and only then, on the main context, decides.
fn maybe_show(window: &Window, wiring: &Wiring) {
    if window.orientation().is_visible() {
        return;
    }
    let answer = crate::search::ask(&wiring.database, &wiring.runtime, |connection| {
        SettingsRepository::new(connection).get_flag(SEEN_KEY).ok()
    });
    glib::spawn_future_local(glib::clone!(
        #[weak]
        window,
        async move {
            // A read that failed outright answers `None`; treated as "already
            // seen" rather than "never seen" -- a plate that cannot be
            // dismissed cleanly because the write path is also broken is a
            // worse failure than one that quietly never appears.
            let seen = answer.recv().await.ok().flatten().unwrap_or(true);
            if !seen {
                window.orientation().set_visible(true);
            }
        }
    ));
}

/// Hide the plate and persist that it has been seen, if it was showing.
///
/// The `is_visible` guard is what keeps this from writing the flag on every
/// single command a user runs for the rest of the session — it is already
/// hidden after the first call, so every later one is a no-op before it
/// touches the database at all.
fn retire(window: &Window, wiring: &Wiring) {
    let plate = window.orientation();
    if !plate.is_visible() {
        return;
    }
    plate.set_visible(false);

    let database = wiring.database.clone();
    wiring.runtime.spawn_blocking(move || {
        let Ok(connection) = database.connection() else {
            return;
        };
        if let Err(error) = SettingsRepository::new(&connection)
            .set_flag(SEEN_KEY, chrono::Utc::now())
        {
            tracing::warn!(%error, "could not remember that the keyboard orientation was seen");
        }
    });
}
