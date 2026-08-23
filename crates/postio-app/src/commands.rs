//! Joining the window to the command bus.
//!
//! The window produces invocations and repaints from events; the bus consumes
//! invocations and produces events. Neither knows the other exists — that is
//! what keeps `postio-gtk` free of SQL and `postio-core` free of GTK — so the
//! join is here.
//!
//! # App state mirrors the window, and does it at send time
//!
//! Every message verb defaults to [`MessageTarget::Selection`], and
//! `AppState::resolve` is what turns that into rows. But the *selection* lives
//! in the list widget: it is what the user built with `x`, Ctrl-click and
//! `Ctrl+A`, and the list is the only thing that knows it.
//!
//! So app state is brought into step with the window in the instant before a
//! command is sent, rather than being kept in step by a signal. Two reasons.
//! A pull cannot drift: there is no ordering in which the bus resolves against
//! a selection one gesture out of date. And a push would have to fire on every
//! `j`, which is the interaction that happens most and the one with the
//! tightest budget.
//!
//! # The quiet sink
//!
//! Mirroring emits [`Event::SelectionChanged`] like any other state change,
//! and those events have nowhere to go: the window is where they came from,
//! and telling it back would be a round trip to nowhere. They go into a sink
//! whose reader was dropped on purpose.

use postio_core::Event;
use postio_core::bridge::{CommandSender, EventSink, EventStream, event_channel};
use postio_core::state::{Selection, SharedState};
use postio_gtk::feed::Feeds;
use postio_gtk::window::Window;
use postio_model::{MailboxId, MessageId};

use adw::prelude::*;
use gtk::glib;

/// Point app state at what the user is looking at.
///
/// `focus` is the cursor — where the keyboard is — and it is load-bearing:
/// `AppState::resolve` falls back to it when the selection is empty, which is
/// the difference between "click a message, press `a`" archiving that message
/// and archiving nothing at all. See `crates/postio-gtk/src/selection.rs`.
pub fn mirror(
    state: &SharedState,
    quiet: &EventSink,
    mailbox: Option<MailboxId>,
    selection: &Selection,
    focus: Option<MessageId>,
) {
    state.update(quiet, |app| {
        let mut events = Vec::new();
        if let Some(mailbox) = mailbox {
            // Opening a different mailbox drops the selection with it, on
            // both sides — the list does the same. So this goes first, or it
            // would throw away the selection just mirrored.
            events.extend(app.open_mailbox(mailbox));
        }
        match selection {
            Selection::These(messages) => events.extend(app.select(messages.clone(), focus)),
            // "Everything" stays a predicate the whole way: the exceptions
            // are re-applied one by one rather than the predicate being
            // resolved into the ids it stands for, because resolving it is
            // exactly the mailbox-sized read it exists to avoid.
            Selection::Everything { except } => {
                events.extend(app.select_all());
                for message in except {
                    events.extend(app.toggle_selection(*message));
                }
                events.extend(app.focus_on(focus));
            }
        }
        events
    });
}

/// Send every invocation the window produces to the bus.
pub fn install(window: &Window, feeds: &Feeds, state: SharedState, commands: CommandSender) {
    // No reader, on purpose: see the module docs.
    let (quiet, _) = event_channel();
    let feeds = feeds.clone();

    window.connect_action(glib::clone!(
        #[weak(rename_to = window)]
        window,
        move |command| {
            let list = window.list();
            mirror(
                &state,
                &quiet,
                feeds.messages.mailbox(),
                &list.selection().selection(),
                list.cursor_id(),
            );
            if commands.send(command).is_err() {
                // Only ever during teardown: the bridge has stopped and there
                // is nothing left to run the verb on.
                eprintln!("postio: the runtime has stopped and did not run that");
            }
        }
    ));
}

/// Apply one event to everything on screen.
pub fn apply(window: &Window, feeds: &Feeds, event: &Event) {
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
            eprintln!("postio: {message}");
            window.show_action_completed(message, false);
        }
        // Rows that have left the mailbox cannot stay selected: the next
        // action would be aimed at mail that is no longer there.
        Event::MessagesRemoved { .. } => window.list().clear_selection(),
        _ => {}
    }
}

/// Drain `stream` onto the panes, for as long as the window is alive.
///
/// Awaited on the GTK main context, so what the UI does is take an event off
/// a queue — never wait for one to be produced.
pub fn drain(window: &Window, feeds: &Feeds, stream: EventStream) {
    let window = window.downgrade();
    let feeds = feeds.clone();
    glib::spawn_future_local(async move {
        while let Some(event) = stream.next().await {
            let Some(window) = window.upgrade() else {
                return;
            };
            apply(&window, &feeds, &event);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_core::MessageTarget;

    fn quiet() -> EventSink {
        let (sink, _) = event_channel();
        sink
    }

    fn mailbox() -> MailboxId {
        MailboxId::new(4)
    }

    #[test]
    fn an_empty_selection_aims_a_verb_at_the_cursor() {
        // The daily case, and the one that fails silently: click a message —
        // which *clears* the selection in this list — and press `a`.
        let state = SharedState::default();

        mirror(
            &state,
            &quiet(),
            Some(mailbox()),
            &Selection::These(Vec::new()),
            Some(MessageId::new(9)),
        );

        assert_eq!(
            state.read(|app| app.resolve(&MessageTarget::Selection)),
            Some(postio_core::state::Resolved::Messages(vec![
                MessageId::new(9)
            ]))
        );
    }

    #[test]
    fn a_deliberate_selection_survives_the_mirror() {
        let state = SharedState::default();

        mirror(
            &state,
            &quiet(),
            Some(mailbox()),
            &Selection::These(vec![MessageId::new(1), MessageId::new(2)]),
            Some(MessageId::new(9)),
        );

        assert_eq!(
            state.read(|app| app.resolve(&MessageTarget::Selection)),
            Some(postio_core::state::Resolved::Messages(vec![
                MessageId::new(1),
                MessageId::new(2)
            ])),
            "what the user marked, not where they happen to be looking"
        );
    }

    #[test]
    fn select_all_arrives_as_a_predicate_and_keeps_its_exceptions() {
        let state = SharedState::default();

        mirror(
            &state,
            &quiet(),
            Some(mailbox()),
            &Selection::Everything {
                except: vec![MessageId::new(7)],
            },
            Some(MessageId::new(7)),
        );

        assert_eq!(
            state.read(|app| app.resolve(&MessageTarget::Selection)),
            Some(postio_core::state::Resolved::Everything {
                mailbox: mailbox(),
                except: vec![MessageId::new(7)],
            }),
            "resolving it here would be the mailbox-sized read it exists to avoid"
        );
    }

    #[test]
    fn changing_mailbox_does_not_carry_the_old_selection_over() {
        // An action landing on mail the user cannot see is the failure this
        // prevents; the list drops its selection on the same boundary.
        let state = SharedState::default();
        mirror(
            &state,
            &quiet(),
            Some(mailbox()),
            &Selection::These(vec![MessageId::new(1)]),
            Some(MessageId::new(1)),
        );

        mirror(
            &state,
            &quiet(),
            Some(MailboxId::new(5)),
            &Selection::These(Vec::new()),
            None,
        );

        assert_eq!(
            state.read(|app| app.resolve(&MessageTarget::Selection)),
            None
        );
    }
}
