//! Questions asked before something no single undo takes back whole: the sweep of the inbox and
//! stopping a digest for a sender.

use postio_core::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirm {
    /// What [`crate::Input::Confirmed`] names it by. Never reused.
    pub token: u64,
    pub heading: String,
    pub body: String,
    pub confirm: String,
    pub destructive: bool,
}

#[derive(Debug)]
pub(crate) struct Asked {
    pub(crate) token: u64,
    pub(crate) yes: Command,
}

impl crate::FocusController {
    /// The question replaces any asked before it, whose answer then does nothing.
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
