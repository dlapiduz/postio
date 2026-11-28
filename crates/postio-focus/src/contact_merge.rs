//! The review queue and the merge sheet: `g w` reads the groups of records that look like one
//! person; `x` marks people and `m` merges them through the same sheet. The sheet holds the
//! keyboard. A merge is the engine's join command, so undo is the engine's.

use postio_core::{Command, ContactJoinAction};
use postio_model::{ContactId, MergeGroup};

use crate::feed::Step;
use crate::{FocusController, Intent, Request};

#[derive(Debug)]
pub(crate) struct MergeSheet {
    groups: Vec<MergeGroup>,
    kept: usize,
    manual: bool,
}

impl MergeSheet {
    fn view(&self, controller: &FocusController) -> postio_ui::contact_merge::MergeSheetView {
        postio_ui::contact_merge::view(
            &self.groups[0],
            self.kept,
            (!self.manual).then_some(self.groups.len()),
            controller.bar.keymap(),
            controller.bar.now(),
            None,
        )
    }
}

impl FocusController {
    pub(crate) fn contacts_review_ask(&mut self) -> Vec<Step> {
        let first = std::mem::take(&mut self.import_found);
        vec![Step::Ask(Request::ContactReview { first })]
    }

    /// The position waits for the counts: asked at once, its answer could land before them, and the
    /// restart they cause would put the cursor back on the first row (#2027).
    pub(crate) fn contacts_imported(
        &mut self,
        made: Vec<ContactId>,
        touched: Vec<ContactId>,
    ) -> Vec<Step> {
        self.import_found = touched;
        self.import_cursor = made.first().copied();
        if self.contacts.is_none() || self.import_cursor.is_none() {
            return Vec::new();
        }
        vec![Step::Ask(Request::ContactsOverview)]
    }

    pub(crate) fn contacts_go_to_import(&mut self) -> Vec<Step> {
        let Some(contacts) = self.contacts.as_ref() else {
            return Vec::new();
        };
        let Some(person) = self.import_cursor else {
            return Vec::new();
        };
        vec![Step::Ask(Request::ContactPosition {
            person,
            tab: contacts.tab_now(),
        })]
    }

    pub(crate) fn contacts_position_read(
        &mut self,
        person: ContactId,
        at: Option<u32>,
    ) -> Vec<Step> {
        if self.import_cursor != Some(person) {
            return Vec::new();
        }
        self.import_cursor = None;
        let (Some(at), Some(contacts)) = (at, self.contacts.as_mut()) else {
            // Not on the tab being shown (or the view is down): the cursor
            // stays, and the list is read once more so the new people are in
            // whatever it shows.
            return if self.contacts.is_some() {
                vec![Step::Show(Intent::ContactsRows { first: 0, count: 0 })]
            } else {
                Vec::new()
            };
        };
        contacts.cursor = Some(at);
        let mut steps = vec![
            Step::Show(Intent::ContactsRows { first: 0, count: 0 }),
            Step::Show(Intent::ContactsCursor(Some(at))),
        ];
        steps.extend(self.contacts_card_asked());
        steps
    }

    /// A person's only address is not split off.
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

    pub(crate) fn contacts_merge_marked(&mut self) -> Vec<Step> {
        let Some(contacts) = self.contacts.as_ref() else {
            return Vec::new();
        };
        if contacts.marks.len() < 2 {
            return Vec::new();
        }
        vec![Step::Ask(Request::ContactMergeOf(contacts.marks.clone()))]
    }

    pub(crate) fn merge_queue_asked(&self) -> Vec<Step> {
        let Some(sheet) = self
            .contacts
            .as_ref()
            .and_then(|contacts| contacts.merge.as_ref())
            .filter(|sheet| !sheet.manual)
        else {
            return Vec::new();
        };
        let first = sheet.groups[0].people.iter().map(|p| p.id).collect();
        vec![Step::Ask(Request::ContactReview { first })]
    }

    pub(crate) fn contacts_review_read(&mut self, groups: Vec<MergeGroup>) -> Vec<Step> {
        let ids = |group: &MergeGroup| group.people.iter().map(|p| p.id).collect::<Vec<_>>();
        if let Some(sheet) = self
            .contacts
            .as_mut()
            .and_then(|contacts| contacts.merge.as_mut())
            .filter(|sheet| !sheet.manual)
        {
            if groups.is_empty() {
                return self.merge_advance_to(Vec::new());
            }
            if ids(&groups[0]) != ids(&sheet.groups[0]) {
                sheet.kept = 0;
            }
            sheet.groups = groups;
            return self.draw_contact_merge();
        }
        if groups.is_empty() || self.contacts.is_none() {
            return Vec::new();
        }
        self.merge_open(MergeSheet {
            groups,
            kept: 0,
            manual: false,
        })
    }

    pub(crate) fn contacts_merge_of_read(&mut self, group: Option<MergeGroup>) -> Vec<Step> {
        let Some(group) = group else {
            return Vec::new();
        };
        if self.contacts.is_none() {
            return Vec::new();
        }
        self.merge_open(MergeSheet {
            groups: vec![group],
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

    pub(crate) fn draw_contact_merge(&self) -> Vec<Step> {
        let view = self
            .contacts
            .as_ref()
            .and_then(|contacts| contacts.merge.as_ref())
            .map(|sheet| Box::new(sheet.view(self)));
        vec![Step::Show(Intent::ContactMerge(view))]
    }

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

    pub(crate) fn contact_merge_later(&mut self) -> Vec<Step> {
        if let Some(contacts) = self.contacts.as_mut() {
            contacts.merge = None;
        }
        vec![Step::Show(Intent::ContactMerge(None))]
    }

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

    fn merge_advance_to(&mut self, groups: Vec<MergeGroup>) -> Vec<Step> {
        if let Some(sheet) = self
            .contacts
            .as_mut()
            .and_then(|contacts| contacts.merge.as_mut())
        {
            sheet.groups = groups;
        }
        self.merge_advance()
    }

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
