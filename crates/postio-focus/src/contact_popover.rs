//! The known-sender popover: who a message is from, at a glance, and where
//! their mail arrives (specs/011-focus-contacts S6, handoff screen 04).
//!
//! `i` on a row or the open message, or a click on a sender's name, asks the
//! host for the person; what comes back is drawn as `postio_ui`'s view and
//! the popover holds the keyboard until it is put away. A change to where
//! their mail arrives is a command, sent at once, and shown at once: the
//! toast with its Undo is the engine's own, as for any other undo unit.

use postio_core::{Command, CommandId};
use postio_model::{ContactArrival, ContactGlance};
use postio_ui::contact_popover::{self as words, Facts};

use crate::bar::BarMode;
use crate::cursor::Rows;
use crate::feed::Step;
use crate::{FocusController, Intent, Request, SurfaceKind};

/// What the controller keeps of the popover.
#[derive(Debug)]
pub(crate) struct ContactPopover {
    /// The address it was asked about: what its commands name.
    address: String,
    /// Which asking this is; an answer to an earlier one is dropped.
    stamp: u64,
    /// What the host answered, once it has.
    glance: Option<ContactGlance>,
}

impl FocusController {
    /// `i` or a click: ask about `address`, or about the sender of the open
    /// message, else of the cursor's row. Nothing is drawn until the host
    /// answers; a row with no sender to ask about does nothing.
    pub(crate) fn open_contact_popover(
        &mut self,
        address: Option<String>,
        rows: &dyn Rows,
    ) -> Vec<Step> {
        let Some(address) = address.or_else(|| self.popover_sender(rows)) else {
            return Vec::new();
        };
        let stamp = self.stamp();
        let steps = self
            .surfaces
            .opened(SurfaceKind::ContactPopover, self.policy.caps.stacking);
        self.contact_popover = Some(ContactPopover {
            address: address.clone(),
            stamp,
            glance: None,
        });
        let mut steps = steps;
        steps.push(Step::Ask(Request::ContactGlance { address, stamp }));
        steps
    }

    /// Whose name `i` means: the sender of the message on screen, else of the
    /// cursor's row.
    fn popover_sender(&self, rows: &dyn Rows) -> Option<String> {
        let at = match self.surfaces.reading() {
            Some(message) => rows.position_of(message)?,
            None => self.cursor.position()?,
        };
        let row = rows.row(at)?;
        let from = row
            .as_conversation()?
            .summary
            .representative
            .from
            .as_ref()?;
        Some(from.address.clone())
    }

    /// Whether `id` is the popover's own while it is on top.
    pub(crate) fn contact_popover_answers(id: CommandId) -> bool {
        matches!(
            id,
            CommandId::Back
                | CommandId::ContactPopover
                | CommandId::ContactOpenCard
                | CommandId::ContactArrivalPrev
                | CommandId::ContactArrivalNext
                | CommandId::ContactSetArrival
                | CommandId::ContactShowMail
                | CommandId::ContactCompose
        )
    }

    /// A command while the popover is on top. It holds the keyboard: what is
    /// not its own does nothing, but the key map and Quit.
    pub(crate) fn contact_popover_command(
        &mut self,
        id: CommandId,
        rows: &dyn Rows,
    ) -> Option<Vec<Step>> {
        let popover = self.contact_popover.as_ref()?;
        let address = popover.address.clone();
        // Before the host has answered, only putting it away is possible.
        let Some(glance) = popover.glance.as_ref() else {
            return match id {
                CommandId::Back => Some(self.close_contact_popover()),
                CommandId::CheatSheet => None,
                _ => Some(Vec::new()),
            };
        };
        match id {
            CommandId::Back => Some(self.close_contact_popover()),
            CommandId::ContactArrivalPrev | CommandId::ContactArrivalNext => {
                let by = if id == CommandId::ContactArrivalNext {
                    1
                } else {
                    -1
                };
                let to =
                    words::step_arrival(Some(glance.arrival.unwrap_or(glance.default_arrival)), by);
                Some(self.set_contact_arrival(Some(to)))
            }
            CommandId::ContactSetArrival => Some(self.set_contact_arrival(None)),
            CommandId::ContactShowMail => {
                let addresses: Vec<String> = glance
                    .person
                    .addresses
                    .iter()
                    .map(|owned| owned.address.address.clone())
                    .collect();
                let query = if addresses.is_empty() {
                    postio_ui::contacts::show_mail_query(&[address])
                } else {
                    postio_ui::contacts::show_mail_query(&addresses)
                };
                let mut steps = self.close_contact_popover();
                steps.extend(self.open_bar(BarMode::Search, &query, rows));
                Some(steps)
            }
            CommandId::ContactCompose => {
                let mut steps = self.close_contact_popover();
                steps.insert(0, Step::Show(Intent::WriteTo(address)));
                Some(steps)
            }
            // The card is the Contacts view with this person found: the view
            // opens over the list filtered to their address, so the card
            // beside it is theirs.
            CommandId::ContactOpenCard if self.policy.caps.contacts_view => {
                let mut steps = self.close_contact_popover();
                steps.extend(self.open_contacts());
                steps.extend(self.contacts_filter_typed(address));
                Some(steps)
            }
            CommandId::CheatSheet => None,
            _ => Some(Vec::new()),
        }
    }

    /// A click on a segment.
    pub(crate) fn contact_arrival_picked(&mut self, arrival: ContactArrival) -> Vec<Step> {
        if self
            .contact_popover
            .as_ref()
            .is_none_or(|popover| popover.glance.is_none())
        {
            return Vec::new();
        }
        self.set_contact_arrival(Some(arrival))
    }

    /// Where their mail arrives, from now: sent as a command, drawn at once.
    /// What is already so changes nothing and sends nothing.
    fn set_contact_arrival(&mut self, arrival: Option<ContactArrival>) -> Vec<Step> {
        let Some(popover) = self.contact_popover.as_mut() else {
            return Vec::new();
        };
        let Some(glance) = popover.glance.as_mut() else {
            return Vec::new();
        };
        if glance.arrival == arrival {
            return Vec::new();
        }
        glance.arrival = arrival;
        let command = Command::ContactSetArrival {
            address: popover.address.clone(),
            arrival,
        };
        let mut steps = self.draw_contact_popover();
        steps.extend(self.verbs.post(command));
        steps
    }

    /// The popover goes, and the keyboard goes home once nothing is over the
    /// list.
    fn close_contact_popover(&mut self) -> Vec<Step> {
        self.contact_popover = None;
        self.surfaces.dismiss(SurfaceKind::ContactPopover);
        let mut steps = vec![Step::Show(Intent::CloseSurface(
            SurfaceKind::ContactPopover,
        ))];
        if !self.has_surface() {
            steps.push(Step::Show(Intent::KeyboardHome));
        }
        steps
    }

    /// The host's answer. One for a popover put away or asked again is
    /// dropped; nobody at that address is no popover.
    pub(crate) fn contact_popover_reply(
        &mut self,
        stamp: u64,
        glance: Result<Option<Box<ContactGlance>>, String>,
    ) -> Vec<Step> {
        let Some(popover) = self.contact_popover.as_mut() else {
            return Vec::new();
        };
        if popover.stamp != stamp {
            return Vec::new();
        }
        match glance {
            Ok(Some(glance)) => {
                popover.glance = Some(*glance);
                self.draw_contact_popover()
            }
            // Someone the mail never saw, or the user's own address; or the
            // read failed, and there is nothing to say a click could mend.
            Ok(None) | Err(_) => self.close_contact_popover(),
        }
    }

    /// The popover, whole.
    fn draw_contact_popover(&self) -> Vec<Step> {
        let Some(popover) = self.contact_popover.as_ref() else {
            return Vec::new();
        };
        let Some(glance) = popover.glance.as_ref() else {
            return Vec::new();
        };
        let facts = Facts {
            person: &glance.person,
            address: &popover.address,
            stats: &glance.stats,
            open: &glance.open,
            arrival: glance.arrival,
            default_arrival: glance.default_arrival,
            // The card is the Contacts view's, where there is one.
            card: self.policy.caps.contacts_view,
        };
        let view = words::view(&facts, self.bar.keymap(), self.bar.now());
        vec![Step::Show(Intent::ContactPopover(Box::new(view)))]
    }
}
