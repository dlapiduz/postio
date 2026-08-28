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
//! as an [`Aim`](postio_core::aim::Aim), and send back what comes out. A
//! second frontend writes the same three lines against its own list and gets
//! the same behaviour by construction rather than by agreement.
//!
//! [`view_scope`] is the one rule still here, and only because it cannot move
//! yet: `postio-core` sits below the runtime and cannot see the scope type the
//! store speaks. #670 collapses it into `aim`.

use postio_core::aim::{self, Aim};
use postio_core::bridge::{CommandSender, EventStream, event_channel};
use postio_core::state::SharedState;
use postio_core::state::ViewScope;
use postio_core::{CommandId, Event};
use postio_gtk::feed::Feeds;
use postio_gtk::window::Window;

use adw::prelude::*;
use gtk::glib;

/// The app-state scope a feed scope stands for.
///
/// `None` for a thread drill-in: a conversation is a *destination for a
/// verb*, not a view a whole-view selection is relative to — `Ctrl+A` inside
/// a thread is not a gesture, and `MessageTarget::Thread` is how a thread
/// gets acted on instead.
///
/// Also `None` for Snoozed (#493): unlike Flagged, nothing here needs
/// `Ctrl+A` to select every snoozed message at once yet, so it is not worth
/// a `MessageSet::Snoozed` predicate of its own until something does. A
/// person can still snooze, unsnooze, open and read individual rows in that
/// view — this only means a whole-view bulk gesture there is a rejection
/// rather than a no-op that silently claims to have done something.
///
/// # Why this one rule is still frontend-side
///
/// `ViewScope` has two variants, `FeedScope` four, `ScopeFfi` four different
/// ones, and `postio_runtime::store::ListScope` is a fourth spelling that
/// `postio-core` cannot see, because core sits below the runtime. Moving a
/// shared scope type is a bigger change than this module — #670 — and doing
/// it badly would put mailbox knowledge into `ListWindow`, which its own
/// module doc refuses. Today there is exactly one such conversion in the
/// workspace, so there is nothing for it to disagree with; the moment there
/// are two, #670 is what stops them drifting.
fn view_scope(scope: postio_gtk::feed::FeedScope) -> Option<ViewScope> {
    match scope {
        postio_gtk::feed::FeedScope::Mailbox(mailbox) => Some(ViewScope::Mailbox(mailbox)),
        postio_gtk::feed::FeedScope::Flagged(account) => Some(ViewScope::Flagged(account)),
        postio_gtk::feed::FeedScope::Snoozed(_) | postio_gtk::feed::FeedScope::Thread(_) => None,
    }
}

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
            let aim = Aim {
                scope: feeds.messages.scope().and_then(view_scope),
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
pub fn apply(
    window: &Window,
    feeds: &Feeds,
    event: &Event,
    notifier: &crate::notifications::Notifier,
) {
    feeds.apply(event);
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
) {
    let window = window.downgrade();
    let feeds = feeds.clone();
    glib::spawn_future_local(async move {
        while let Some(event) = stream.next().await {
            let Some(window) = window.upgrade() else {
                return;
            };
            apply(&window, &feeds, &event, &notifier);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_core::state::ViewScope;
    use postio_gtk::feed::FeedScope;
    use postio_model::{AccountId, MailboxId, ThreadId};

    /// The four shapes, and whether a whole-view gesture means anything in
    /// each. The rule itself is `postio_core::aim`'s; what is asserted here
    /// is this frontend's translation into it.
    #[test]
    fn only_a_folder_and_flagged_are_something_ctrl_a_can_be_relative_to() {
        assert_eq!(
            view_scope(FeedScope::Mailbox(MailboxId::new(4))),
            Some(ViewScope::Mailbox(MailboxId::new(4))),
        );
        assert_eq!(
            view_scope(FeedScope::Flagged(AccountId::new(1))),
            Some(ViewScope::Flagged(AccountId::new(1))),
        );
        assert_eq!(
            view_scope(FeedScope::Snoozed(AccountId::new(1))),
            None,
            "nothing needs a `MessageSet::Snoozed` predicate yet (#493)",
        );
        assert_eq!(
            view_scope(FeedScope::Thread(ThreadId::new(3))),
            None,
            "`Ctrl+A` inside a conversation is not a gesture",
        );
    }
}
