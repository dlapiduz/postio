//! Focus's behaviour, decided once (ADR 0045, `specs/009-focus-macos`).
//!
//! A frontend turns what happens to it — a key, an engine event, a client's
//! reply, a fact about its own window — into an [`Input`], hands it to the
//! [`FocusController`], and does what the returned [`Effect`]s say: draw an
//! [`Intent`], run a [`Request`] through [`perform()`] and feed the [`Reply`]
//! back, or set a timer. The controller does no I/O and never awaits, so the
//! GTK app can drive it from glib's main loop and the FFI from tokio, and
//! every rule in it is a unit test that runs on either host (research R1).
//!
//! Rules arrive here a slice at a time, moved out of `postio-gtk`'s window
//! (research R2); each slice brings its inputs, intents and requests with it.

use std::time::Duration;

use postio_client::protocol::FocusCounts;
use postio_config::paths::Platform;
use postio_core::CommandId;

mod perform;

pub use perform::perform;

/// What differs between platforms, as policy rather than as a fork
/// (ADR 0045 rule 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// Which platform's layout and conventions apply.
    pub platform: Platform,
    /// What this frontend can draw.
    pub caps: Capabilities,
}

/// What a frontend can draw that another cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// A message can be read beside the list (spec 007 FR-038). Linux has
    /// it; the Mac comes to it after parity (spec 009 M4).
    pub reading_pane: bool,
    /// Secondary surfaces stack over each other: Linux's dialogs do, the
    /// Mac's windows replace each other (spec 009 M4).
    pub stacking: bool,
}

impl Policy {
    /// The policy each platform's Focus app runs with.
    pub const fn for_platform(platform: Platform) -> Self {
        let linux = matches!(platform, Platform::Freedesktop);
        Policy {
            platform,
            caps: Capabilities {
                reading_pane: linux,
                stacking: linux,
            },
        }
    }
}

/// One request in flight. The generation is the controller's when it asked;
/// a reply carrying an older one changes nothing (contract invariant 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ticket {
    /// Unique among this controller's requests.
    pub id: u64,
    /// The controller's generation when it asked.
    pub generation: u64,
}

/// Something that happened, told to the controller.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Input {
    /// A command, from a key, a button, the bar or a menu.
    Command(CommandId),
    /// The answer to an earlier [`Effect::Ask`].
    Reply(Ticket, Reply),
}

/// What the frontend does next.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Effect {
    /// Draw this.
    Show(Intent),
    /// Run this through [`perform()`] and hand the answer back as
    /// [`Input::Reply`] with the same ticket.
    Ask(Ticket, Request),
    /// Call back with [`Input`] once `after` has passed. (Arrives with the
    /// first slice that needs one.)
    Timer {
        /// Handed back when the timer fires.
        token: u64,
        /// How long to wait.
        after: Duration,
    },
}

/// What the frontend draws. Each one is drawable without asking the
/// controller anything else.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Intent {
    /// The header strip's counts changed.
    Counts(FocusCounts),
    /// Leave the app.
    Quit,
}

/// What the controller needs from the engine. [`perform()`] is the one place
/// each becomes a client call.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Request {
    /// The header strip's counts.
    FocusCounts,
}

/// The engine's answer to a [`Request`]. A failure is carried as its
/// sentence: the controller decides what to show, not how to recover.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Reply {
    /// The answer to [`Request::FocusCounts`].
    FocusCounts(Result<FocusCounts, String>),
}

/// Focus's behaviour for one window. `Send`, and plain data: no toolkit
/// object, no executor, no client.
#[derive(Debug)]
pub struct FocusController {
    policy: Policy,
    generation: u64,
    next_ticket: u64,
}

impl FocusController {
    /// A controller for one window, under its platform's policy.
    pub fn new(policy: Policy) -> Self {
        FocusController {
            policy,
            generation: 0,
            next_ticket: 0,
        }
    }

    /// The policy this controller was made with.
    pub fn policy(&self) -> Policy {
        self.policy
    }

    /// Tell the controller what happened; get back what to do.
    pub fn handle(&mut self, input: Input) -> Vec<Effect> {
        match input {
            Input::Command(CommandId::Quit) => vec![Effect::Show(Intent::Quit)],
            Input::Command(_) => Vec::new(),
            Input::Reply(ticket, _) if ticket.generation != self.generation => Vec::new(),
            Input::Reply(_, Reply::FocusCounts(Ok(counts))) => {
                vec![Effect::Show(Intent::Counts(counts))]
            }
            Input::Reply(_, Reply::FocusCounts(Err(error))) => {
                tracing::debug!(%error, "focus counts unavailable");
                Vec::new()
            }
        }
    }

    /// Ask for the header strip's counts.
    pub fn refresh_counts(&mut self) -> Effect {
        Effect::Ask(self.ticket(), Request::FocusCounts)
    }

    /// Forget every request in flight: their replies, when they come, change
    /// nothing. Called when what they were for is gone, a place left or a
    /// list replaced.
    pub fn invalidate(&mut self) {
        self.generation += 1;
    }

    fn ticket(&mut self) -> Ticket {
        self.next_ticket += 1;
        Ticket {
            id: self.next_ticket,
            generation: self.generation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(has_action: u32) -> FocusCounts {
        FocusCounts {
            has_action,
            ..FocusCounts::default()
        }
    }

    #[test]
    fn the_controller_can_cross_threads() {
        fn send<T: Send>() {}
        send::<FocusController>();
    }

    #[test]
    fn linux_stacks_and_reads_beside_and_the_mac_does_neither() {
        let linux = Policy::for_platform(Platform::Freedesktop);
        let mac = Policy::for_platform(Platform::Apple);
        assert!(linux.caps.stacking && linux.caps.reading_pane);
        assert!(!mac.caps.stacking && !mac.caps.reading_pane);
    }

    #[test]
    fn counts_asked_for_are_drawn_when_they_come() {
        for platform in [Platform::Freedesktop, Platform::Apple] {
            let mut focus = FocusController::new(Policy::for_platform(platform));
            let Effect::Ask(ticket, Request::FocusCounts) = focus.refresh_counts() else {
                panic!("refreshing asks for the counts");
            };
            assert_eq!(
                focus.handle(Input::Reply(ticket, Reply::FocusCounts(Ok(counts(7))))),
                vec![Effect::Show(Intent::Counts(counts(7)))],
            );
        }
    }

    #[test]
    fn a_reply_asked_before_an_invalidation_changes_nothing() {
        let mut focus = FocusController::new(Policy::for_platform(Platform::Apple));
        let Effect::Ask(stale, _) = focus.refresh_counts() else {
            panic!("refreshing asks");
        };
        focus.invalidate();
        assert!(
            focus
                .handle(Input::Reply(stale, Reply::FocusCounts(Ok(counts(3)))))
                .is_empty()
        );
    }

    #[test]
    fn tickets_are_never_reused() {
        let mut focus = FocusController::new(Policy::for_platform(Platform::Apple));
        let Effect::Ask(first, _) = focus.refresh_counts() else {
            panic!()
        };
        let Effect::Ask(second, _) = focus.refresh_counts() else {
            panic!()
        };
        assert_ne!(first.id, second.id);
    }

    #[test]
    fn quit_is_drawn_as_quit() {
        let mut focus = FocusController::new(Policy::for_platform(Platform::Freedesktop));
        assert_eq!(
            focus.handle(Input::Command(CommandId::Quit)),
            vec![Effect::Show(Intent::Quit)],
        );
    }
}
