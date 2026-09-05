//! Joining the window to the command bus.
//!
//! The window produces invocations and repaints from events; the bus consumes
//! invocations and produces events. Neither knows the other exists — that is
//! what keeps `postio-gtk` free of SQL and `postio-core` free of GTK — so the
//! join is here.
//!
//! # What is left here, and what moved
//!
//! The *rules* — which rows a verb acts on, what app state is told about the
//! selection, whether the bus owns a command at all — live in
//! [`postio_core::aim`], because they are semantics and every frontend has to
//! reach the same answer (#589, ADR 0019). What is left in this module is the
//! **adapter**: read GTK's focus, scope, selection and cursor, hand them over
//! as an [`Aim`], and send back what comes out. A
//! second frontend writes the same three lines against its own list and gets
//! the same behaviour by construction rather than by agreement.
//!
//! `view_scope` used to be the one rule still here, because `postio-core`
//! had no scope type of its own for a `FeedScope` to convert into. #670 gave
//! it one — `postio_model::ListScope`, which every reader of the list now
//! shares — so `aim::view_scope` is where the rule lives, and this module is
//! the adapter in full: nothing here decides anything.

use postio_core::aim::{self, Aim};
use postio_core::bridge::{CommandSender, EventSink, EventStream, event_channel};
use postio_core::state::SharedState;
use postio_core::{CommandId, Event};
use postio_gtk::feed::Feeds;
use postio_gtk::window::Window;

use adw::prelude::*;
use gtk::glib;

/// Send the invocations the bus owns to it, as the window produces them.
pub fn install(
    window: &Window,
    feeds: &Feeds,
    state: SharedState,
    commands: CommandSender,
    wired: Vec<CommandId>,
) {
    // No reader, on purpose: see the module docs.
    let (quiet, _) = event_channel();
    let feeds = feeds.clone();

    window.connect_action(glib::clone!(
        #[weak(rename_to = window)]
        window,
        move |command| {
            if !aim::is_wired(&wired, &command) {
                return;
            }
            // The adapter, and the whole of it: everything below is a fact
            // GTK already holds, handed over as one snapshot. Nothing here
            // decides anything -- `postio_core::aim` does, so that this
            // frontend and the next cannot answer differently.
            let list = window.list();
            let rows = list.model();
            let selection = list.selection().selection();
            // The accounts this selection was scoped to when it was made,
            // not the ones reachable now. The list froze them at the gesture;
            // reading them again here is the time-of-check/time-of-use hole
            // #811 exists to close.
            let reach = list.selection().reach();
            let aim = Aim {
                scope: feeds
                    .messages
                    .scope()
                    .and_then(|scope| aim::view_scope(scope, &reach.accounts)),
                selection: &selection,
                cursor: list.cursor_id(),
                rows: &rows,
            };
            let command = aim::refine(command, &aim);
            aim::mirror(&state, &quiet, &aim);
            if commands.send(command).is_err() {
                // Only ever during teardown: the bridge has stopped and there
                // is nothing left to run the verb on.
                tracing::debug!("the runtime has stopped and did not run that");
            }
        }
    ));

    // The cursor rested on a message long enough to have been read (#71).
    //
    // Through `Window::act`, so it takes the same road every other gesture
    // does and one seam still carries the lot. The message is named
    // explicitly rather than left to the selection: the cursor may have moved
    // on in the moment between the timer firing and this running, and the
    // message that was read is the one the clock was started for.
    window.list().connect_dwelled(glib::clone!(
        #[weak(rename_to = window)]
        window,
        move |message| window.act(postio_core::Command::MarkReadOnDwell { message })
    ));
}

/// Apply one event to everything on screen.
///
/// `state`/`quiet` are `AppState`'s side of it: [`postio_core::state::AppState::set_connection`]
/// had no production caller (#974), so `AppState::accounts()` always answered
/// empty and `g a` could never cycle scopes however many accounts existed.
/// `feeds.apply` above already folded the same event into `Trackers`, which
/// is why `set_connection`'s own diff goes to `quiet` rather than back onto
/// the real sink — nothing downstream needs a second `ConnectionChanged` for
/// news this call already delivered, the same reason `install`'s
/// `aim::mirror` call discards its receiver.
pub fn apply(
    window: &Window,
    feeds: &Feeds,
    event: &Event,
    notifier: &crate::notifications::Notifier,
    state: &SharedState,
    quiet: &EventSink,
) {
    feeds.apply(event);
    if let Event::ConnectionChanged {
        account,
        state: connection,
    } = event
    {
        state.update(quiet, |app_state| {
            app_state.set_connection(*account, *connection)
        });
        // A unified search's caveat is fixed at the moment its answer comes
        // back (ADR 0005 Q10, #812) -- so without this, an account dropping
        // out or coming back while the result sits on screen left the
        // readout saying something that had stopped being true, until the
        // query was asked again (#1060). `feeds.apply` above has already
        // folded this event into `Trackers`, so `unreachable_accounts` reads
        // the connection state this event caused rather than the one before
        // it. A no-op when nothing is on screen, or when the caveat has not
        // changed -- see `Live::set_unreachable`.
        if let Some(live) = window.finder().live() {
            let scope = window.scope();
            live.set_unreachable(crate::search::unreachable_accounts(
                window,
                &feeds.folders,
                scope,
            ));
        }
    }
    match event {
        Event::ActionCompleted {
            description,
            undoable,
        } => window.show_action_completed(description, *undoable),
        Event::UndoPerformed { description } => window.show_undo_performed(description),
        // A rejection is ordinary — nothing selected, nothing to undo — so it
        // gets the same brief sentence with nothing to press, rather than a
        // dialog. Silence is the one answer it must not get: a key that does
        // nothing and says nothing reads as the application ignoring you.
        Event::CommandRejected { reason, .. } => window.show_action_completed(reason, false),
        Event::Error { message } => {
            tracing::error!(
                message = %postio_model::address::redact_addresses(message),
                "command failed"
            );
            window.show_action_completed(message, false);
        }
        // Rows that have left the mailbox cannot stay selected: the next
        // action would be aimed at mail that is no longer there.
        Event::MessagesRemoved { .. } => window.list().clear_selection(),
        Event::NewMail {
            mailbox, messages, ..
        } => notifier.notify(window, *mailbox, messages),
        _ => {}
    }
}

/// Drain `stream` onto the panes, for as long as the window is alive.
///
/// Awaited on the GTK main context, so what the UI does is take an event off
/// a queue — never wait for one to be produced.
pub fn drain(
    window: &Window,
    feeds: &Feeds,
    stream: EventStream,
    notifier: crate::notifications::Notifier,
    state: SharedState,
) {
    let window = window.downgrade();
    let feeds = feeds.clone();
    // No reader, same reason `install`'s does not: see `apply`'s own doc.
    let (quiet, _) = event_channel();
    glib::spawn_future_local(async move {
        while let Some(event) = stream.next().await {
            let Some(window) = window.upgrade() else {
                return;
            };
            apply(&window, &feeds, &event, &notifier, &state, &quiet);
        }
    });
}
