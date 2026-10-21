//! The review queue and the merge sheet (specs/011-focus-contacts S10, handoff
//! screen 07): `g w` reads the groups of records that look like one person
//! and opens the sheet on the first; `x` marks people in the list and `m`
//! merges the marked through the same sheet.
//!
//! The sheet holds the keyboard. Return merges into the record whose name and
//! photo are kept, `n` says they are two people for good, Escape is Later,
//! and the arrows choose whose name is kept. A merge is the engine's join
//! command, so undo is the engine's. What the sheet says is
//! `postio_ui::contact_merge`'s.

use postio_core::{Command, ContactJoinAction};
use postio_model::{ContactId, MergeGroup};

use crate::feed::Step;
use crate::{FocusController, Intent, Request};

/// What the controller keeps of the sheet.
#[derive(Debug)]
pub(crate) struct MergeSheet {
    /// The groups still to look at; the first is the one on screen.
    groups: Vec<MergeGroup>,
    /// How many the queue had when it opened, for "1 of 3".
    of: usize,
    /// Which record of the first group keeps its name and photo.
    kept: usize,
    /// Whether the user chose these records by marking them, not the queue.
    manual: bool,
}

impl MergeSheet {
    fn view(&self, controller: &FocusController) -> postio_ui::contact_merge::MergeSheetView {
        let position = (!self.manual).then(|| (self.of - self.groups.len(), self.of));
        postio_ui::contact_merge::view(
            &self.groups[0],
            self.kept,
            position,
            controller.bar.keymap(),
            controller.bar.now(),
        )
    }
}

impl FocusController {
    /// `g w`: the queue is read.
    pub(crate) fn contacts_review_ask(&mut self) -> Vec<Step> {
        if self.contacts.is_none() {
            return Vec::new();
        }
        vec![Step::Ask(Request::ContactReview)]
    }

    /// "Split" on one of the card's addresses: it becomes a person of its
    /// own, undoable with the engine's undo. A person's only address is not
    /// split off.
    pub(crate) fn contact_split(&mut self, address: &str) -> Vec<Step> {
        let Some(card) = self
            .contacts
            .as_ref()
            .and_then(|contacts| contacts.card.as_ref())
        else {
            return Vec::new();
        };
        if card.person.addresses.len() < 2 {
            return Vec::new();
        }
        let Some(owned) = card
            .person
            .addresses
            .iter()
            .find(|owned| owned.address.address.eq_ignore_ascii_case(address))
        else {
            return Vec::new();
        };
        self.verbs.post(Command::ContactDetachAddress {
            address: Some(owned.id),
        })
    }

    /// `x`: the person under the cursor is marked, or not.
    pub(crate) fn contacts_mark(&mut self) -> Vec<Step> {
        let Some(contacts) = self.contacts.as_mut() else {
            return Vec::new();
        };
        let Some(person) = contacts.person().map(|row| row.id) else {
            return Vec::new();
        };
        match contacts.marks.iter().position(|id| *id == person) {
            Some(at) => {
                contacts.marks.remove(at);
            }
            None => contacts.marks.push(person),
        }
        vec![Step::Show(Intent::ContactsRows { first: 0, count: 0 })]
    }

    /// `m`: the marked people are looked at together, once there are two.
    pub(crate) fn contacts_merge_marked(&mut self) -> Vec<Step> {
        let Some(contacts) = self.contacts.as_ref() else {
            return Vec::new();
        };
        if contacts.marks.len() < 2 {
            return Vec::new();
        }
        vec![Step::Ask(Request::ContactMergeOf(contacts.marks.clone()))]
    }

    /// The queue came back.
    pub(crate) fn contacts_review_read(&mut self, groups: Vec<MergeGroup>) -> Vec<Step> {
        if groups.is_empty() || self.contacts.is_none() {
            return Vec::new();
        }
        let of = groups.len();
        self.merge_open(MergeSheet {
            groups,
            of,
            kept: 0,
            manual: false,
        })
    }

    /// The group the marked people make came back.
    pub(crate) fn contacts_merge_of_read(&mut self, group: Option<MergeGroup>) -> Vec<Step> {
        let Some(group) = group else {
            return Vec::new();
        };
        if self.contacts.is_none() {
            return Vec::new();
        }
        self.merge_open(MergeSheet {
            groups: vec![group],
            of: 1,
            kept: 0,
            manual: true,
        })
    }

    fn merge_open(&mut self, sheet: MergeSheet) -> Vec<Step> {
        let Some(contacts) = self.contacts.as_mut() else {
            return Vec::new();
        };
        contacts.merge = Some(sheet);
        self.draw_contact_merge()
    }

    /// The sheet, whole, or put away.
    pub(crate) fn draw_contact_merge(&self) -> Vec<Step> {
        let view = self
            .contacts
            .as_ref()
            .and_then(|contacts| contacts.merge.as_ref())
            .map(|sheet| Box::new(sheet.view(self)));
        vec![Step::Show(Intent::ContactMerge(view))]
    }

    /// Whose name is kept: one step, or a click.
    pub(crate) fn contact_merge_keep(&mut self, to: usize) -> Vec<Step> {
        let Some(sheet) = self
            .contacts
            .as_mut()
            .and_then(|contacts| contacts.merge.as_mut())
        else {
            return Vec::new();
        };
        let last = sheet.groups[0].people.len() - 1;
        sheet.kept = to.min(last);
        self.draw_contact_merge()
    }

    /// A step along the cards, by the arrows.
    pub(crate) fn contact_merge_step(&mut self, by: i64) -> Vec<Step> {
        let Some(sheet) = self
            .contacts
            .as_ref()
            .and_then(|contacts| contacts.merge.as_ref())
        else {
            return Vec::new();
        };
        let to = (sheet.kept as i64 + by).max(0) as usize;
        self.contact_merge_keep(to)
    }

    /// Later: the sheet goes and nothing changes.
    pub(crate) fn contact_merge_later(&mut self) -> Vec<Step> {
        if let Some(contacts) = self.contacts.as_mut() {
            contacts.merge = None;
        }
        vec![Step::Show(Intent::ContactMerge(None))]
    }

    /// Return: the group is merged into the kept record.
    pub(crate) fn contact_merge_now(&mut self) -> Vec<Step> {
        let Some(contacts) = self.contacts.as_mut() else {
            return Vec::new();
        };
        let Some(sheet) = contacts.merge.as_mut() else {
            return Vec::new();
        };
        let group = sheet.groups.remove(0);
        let manual = sheet.manual;
        let into = group.people[sheet.kept.min(group.people.len() - 1)].clone();
        let others: Vec<ContactId> = group
            .people
            .iter()
            .filter(|p| p.id != into.id)
            .map(|p| p.id)
            .collect();
        let organization = into.organization.clone().or_else(|| {
            group
                .people
                .iter()
                .find_map(|p| p.organization.clone().filter(|o| !o.trim().is_empty()))
        });
        let command = Command::ContactJoin {
            action: ContactJoinAction::Join {
                into: into.id,
                others,
                name: into.shown_as().into_owned(),
                organization,
            },
        };
        if manual {
            contacts.marks.clear();
        }
        let mut steps = self.merge_advance();
        steps.extend(self.verbs.post(command));
        steps
    }

    /// `n`: they are two people, for good.
    pub(crate) fn contact_merge_not_the_same(&mut self) -> Vec<Step> {
        let Some(contacts) = self.contacts.as_mut() else {
            return Vec::new();
        };
        let Some(sheet) = contacts.merge.as_mut() else {
            return Vec::new();
        };
        let group = sheet.groups.remove(0);
        let mut commands = Vec::new();
        for (index, a) in group.people.iter().enumerate() {
            for b in &group.people[index + 1..] {
                commands.push(Command::SuggestionDismiss {
                    pair: Some((a.id, b.id)),
                });
            }
        }
        let mut steps = self.merge_advance();
        for command in commands {
            steps.extend(self.verbs.post(command));
        }
        steps
    }

    /// The next group, or the sheet put away when there is none.
    fn merge_advance(&mut self) -> Vec<Step> {
        let Some(contacts) = self.contacts.as_mut() else {
            return Vec::new();
        };
        if let Some(sheet) = contacts.merge.as_mut() {
            sheet.kept = 0;
            if !sheet.groups.is_empty() {
                return self.draw_contact_merge();
            }
        }
        contacts.merge = None;
        vec![
            Step::Show(Intent::ContactMerge(None)),
            Step::Show(Intent::ContactsRows { first: 0, count: 0 }),
        ]
    }
}
