//! Joining the window to the command bus.
//!
//! The window produces invocations and repaints from events; the bus consumes
//! invocations and produces events. Neither knows the other exists — that is
//! what keeps `postio-gtk` free of SQL and `postio-core` free of GTK — so the
//! join is here.
//!
//! # App state mirrors the window, and does it at send time
//!
//! Every message verb defaults to `MessageTarget::Selection`, and
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

use postio_core::bridge::{CommandSender, EventSink, EventStream, event_channel};
use postio_core::state::{Selection, SharedState, ViewScope};
use postio_core::{Command, CommandId, Event};
use postio_gtk::feed::Feeds;
use postio_gtk::window::Window;
use postio_model::MessageId;

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
    scope: Option<ViewScope>,
    selection: &Selection,
    focus: Option<MessageId>,
) {
    state.update(quiet, |app| {
        let mut events = Vec::new();
        if let Some(scope) = scope {
            // Opening a different view drops the selection with it, on both
            // sides — the list does the same. So this goes first, or it
            // would throw away the selection just mirrored.
            //
            // The *scope*, not the mailbox it may or may not name: a smart
            // folder has no mailbox, and telling app state only about
            // mailboxes is what left `Ctrl+A` in Flagged with nothing to be
            // relative to (#52).
            events.extend(app.open_view(scope));
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

/// The app-state scope a feed scope stands for.
///
/// `None` for a thread drill-in: a conversation is a *destination for a
/// verb*, not a view a whole-view selection is relative to — `Ctrl+A` inside
/// a thread is not a gesture, and `MessageTarget::Thread` is how a thread
/// gets acted on instead.
fn view_scope(scope: postio_gtk::feed::FeedScope) -> Option<ViewScope> {
    match scope {
        postio_gtk::feed::FeedScope::Mailbox(mailbox) => Some(ViewScope::Mailbox(mailbox)),
        postio_gtk::feed::FeedScope::Flagged(account) => Some(ViewScope::Flagged(account)),
        postio_gtk::feed::FeedScope::Thread(_) => None,
    }
}

/// Whether this invocation is the bus's business.
///
/// The bus is one consumer among several — the composer answers reply and
/// compose, the config module answers `edit_config`, and the window answers
/// `Esc` itself when there is something to close — and it sees every gesture,
/// because [`Window::connect_action`] is the seam that carries whole
/// invocations. Sending it commands it does not handle would answer a stray
/// `Esc` with "`back` is not wired up in this build".
///
/// `wired` comes from [`Dispatcher::wired`], so this cannot drift from what
/// the bus actually answers.
///
/// [`Dispatcher::wired`]: postio_core::dispatch::Dispatcher::wired
fn is_for_the_bus(wired: &[CommandId], command: &Command) -> bool {
    wired.contains(&command.id())
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
            if !is_for_the_bus(&wired, &command) {
                return;
            }
            let list = window.list();
            mirror(
                &state,
                &quiet,
                feeds.messages.scope().and_then(view_scope),
                &list.selection().selection(),
                list.cursor_id(),
            );
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
    use postio_core::MessageTarget;

    fn quiet() -> EventSink {
        let (sink, _) = event_channel();
        sink
    }

    use postio_model::MailboxId;

    fn mailbox() -> MailboxId {
        MailboxId::new(4)
    }

    #[test]
    fn only_the_verbs_the_bus_owns_are_sent_to_it() {
        // The bus is one consumer among several: the composer answers reply
        // and compose, the config module answers edit_config, and the window
        // answers `Esc` itself when there is something to close. Sending it
        // everything would put "`back` is not wired up in this build" on
        // screen every time a stray `Esc` found nothing to close.
        let wired = [CommandId::Archive, CommandId::Undo];

        assert!(is_for_the_bus(
            &wired,
            &Command::Archive {
                target: MessageTarget::Selection
            }
        ));
        assert!(is_for_the_bus(&wired, &Command::Undo));
        assert!(!is_for_the_bus(&wired, &Command::Back));
        assert!(!is_for_the_bus(&wired, &Command::Reply { message: None }));
    }

    #[test]
    fn a_bus_with_nothing_wired_is_told_nothing() {
        // What an unopenable store leaves behind. Every key would otherwise
        // answer with a rejection it can do nothing about.
        assert!(!is_for_the_bus(
            &[],
            &Command::Archive {
                target: MessageTarget::Selection
            }
        ));
    }

    #[test]
    fn an_empty_selection_aims_a_verb_at_the_cursor() {
        // The daily case, and the one that fails silently: click a message —
        // which *clears* the selection in this list — and press `a`.
        let state = SharedState::default();

        mirror(
            &state,
            &quiet(),
            Some(ViewScope::Mailbox(mailbox())),
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
            Some(ViewScope::Mailbox(mailbox())),
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
            Some(ViewScope::Mailbox(mailbox())),
            &Selection::Everything {
                except: vec![MessageId::new(7)],
            },
            Some(MessageId::new(7)),
        );

        assert_eq!(
            state.read(|app| app.resolve(&MessageTarget::Selection)),
            Some(postio_core::state::Resolved::Everything {
                scope: ViewScope::Mailbox(mailbox()),
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
            Some(ViewScope::Mailbox(mailbox())),
            &Selection::These(vec![MessageId::new(1)]),
            Some(MessageId::new(1)),
        );

        mirror(
            &state,
            &quiet(),
            Some(ViewScope::Mailbox(MailboxId::new(5))),
            &Selection::These(Vec::new()),
            None,
        );

        assert_eq!(
            state.read(|app| app.resolve(&MessageTarget::Selection)),
            None
        );
    }
}
