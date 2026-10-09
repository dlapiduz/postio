//! Questions asked before something no single undo takes back whole: the
//! sweep of the inbox (spec 007 FR-118) and stopping a digest for a sender
//! (US10 scenario 5). GTK asked each in an `adw::AlertDialog`; the words
//! are `postio_ui`'s, and the controller keeps what a yes does.

use postio_core::Command;

/// A question, drawable as it is: an alert with Cancel and one button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirm {
    /// What [`crate::Input::Confirmed`] names it by. Never reused.
    pub token: u64,
    /// The alert's heading.
    pub heading: String,
    /// The sentence under it.
    pub body: String,
    /// The button that says yes: "Stop digesting", "Move 2 to Filtered".
    pub confirm: String,
    /// Drawn as destructive rather than as the default.
    pub destructive: bool,
}

/// The question asked last, and what a yes sends.
#[derive(Debug)]
pub(crate) struct Asked {
    pub(crate) token: u64,
    pub(crate) yes: Command,
}

impl crate::FocusController {
    /// Ask `heading` before sending `yes`: the question replaces any asked
    /// before it, whose answer then does nothing.
    pub(crate) fn ask_first(
        &mut self,
        heading: String,
        body: String,
        confirm: String,
        destructive: bool,
        yes: Command,
    ) -> crate::feed::Step {
        let token = self.stamp();
        self.confirm = Some(Asked { token, yes });
        crate::feed::Step::Show(crate::Intent::Confirm(Confirm {
            token,
            heading,
            body,
            confirm,
            destructive,
        }))
    }

    /// Yes, to the question with `token`: what it was about is sent, once.
    pub(crate) fn confirmed(&mut self, token: u64) -> Vec<crate::feed::Step> {
        match self.confirm.take() {
            Some(asked) if asked.token == token => {
                vec![crate::feed::Step::Ask(crate::Request::Post(asked.yes))]
            }
            other => {
                self.confirm = other;
                Vec::new()
            }
        }
    }
}
