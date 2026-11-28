//! The known-sender popover: who a message is from, and where their mail arrives. It holds the
//! keyboard until put away; a change of arrival is a command, sent and shown at once.

use postio_core::{Command, CommandId};
use postio_model::{ContactArrival, ContactGlance};
use postio_ui::contact_popover::{self as words, Facts};

use crate::bar::BarMode;
use crate::cursor::Rows;
use crate::feed::Step;
use crate::{FocusController, Intent, Request, SurfaceKind};

#[derive(Debug)]
pub(crate) struct ContactPopover {
    address: String,
    /// Which asking this is; an answer to an earlier one is dropped.
    stamp: u64,
    glance: Option<ContactGlance>,
}

impl FocusController {
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

    /// Asked about the sender again after the people change, so it says what is true.
    pub(crate) fn contact_popover_heard(&mut self, event: &postio_core::Event) -> Vec<Step> {
        if !matches!(event, postio_core::Event::ContactsChanged { .. }) {
            return Vec::new();
        }
        match &self.contact_popover {
            Some(popover) => vec![Step::Ask(Request::ContactGlance {
                address: popover.address.clone(),
                stamp: popover.stamp,
            })],
            None => Vec::new(),
        }
    }

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
                | CommandId::ContactAdd
                | CommandId::ContactIgnore
        )
    }

    pub(crate) fn contact_popover_command(
        &mut self,
        id: CommandId,
        rows: &dyn Rows,
    ) -> Option<Vec<Step>> {
        let popover = self.contact_popover.as_ref()?;
        let address = popover.address.clone();
        let Some(glance) = popover.glance.as_ref() else {
            return match id {
                CommandId::Back => Some(self.close_contact_popover()),
                // The app's Undo stays the app's wherever it is.
                CommandId::CheatSheet | CommandId::Undo => None,
                _ => Some(Vec::new()),
            };
        };
        match id {
            CommandId::Undo => None,
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
            CommandId::ContactAdd | CommandId::ContactIgnore => {
                let Some(introduction) = glance.introduction.as_ref() else {
                    return Some(Vec::new());
                };
                let action = if id == CommandId::ContactAdd {
                    postio_core::ContactEditAction::AddSender {
                        address,
                        details: introduction.details.clone(),
                    }
                } else {
                    postio_core::ContactEditAction::Ignore {
                        address,
                        ignored: true,
                    }
                };
                let mut steps = self.close_contact_popover();
                steps.extend(self.verbs.post(Command::ContactEdit { action }));
                Some(steps)
            }
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

    /// A reply for a popover put away or asked again is dropped.
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
            card: self.policy.caps.contacts_view,
            introduction: glance.introduction.as_ref(),
        };
        let view = words::view(&facts, self.bar.keymap(), self.bar.now());
        vec![Step::Show(Intent::ContactPopover(Box::new(view)))]
    }
}
