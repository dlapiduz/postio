//! The Edit and New contact form's state (specs/011-focus-contacts S8,
//! handoff screen 06): which form is open, what has changed, and the ladder
//! Escape climbs: the first Esc on an edited form arms, the second discards.
//!
//! The form's content and its rules are `postio_model::ContactForm`'s and
//! what it says is `postio_ui::contact_edit`'s; this is only what a person is
//! doing with it. A frontend draws what [`FocusController::contact_edit`]
//! hands back and reports each change as an [`EditChange`], so the same form
//! behaves the same on every platform. Saving is one command
//! (`ContactEditAction::Save`), never a write the form makes itself.

use postio_core::{Command, ContactEditAction};
use postio_model::{
    AddressLabel, CardSwitch, ContactForm, ContactId, ContactKind, FormAddress, ShowAs,
};
use postio_ui::contact_edit::{EditContext, EditView, FormField};

use crate::feed::Step;
use crate::{FocusController, Intent};

/// One change a person made to the form, reported as they make it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditChange {
    /// A kind chosen.
    Kind(ContactKind),
    /// The first name, as it now reads.
    Given(String),
    /// The last name, as it now reads.
    Family(String),
    /// A Show as style chosen.
    ShowAs(ShowAs),
    /// The job title, as it now reads.
    Title(String),
    /// The company, as it now reads.
    Company(String),
    /// An address row's text, as it now reads.
    Address {
        /// Which row.
        index: u32,
        /// What is typed.
        text: String,
    },
    /// An address row's type chosen.
    Label {
        /// Which row.
        index: u32,
        /// The type; `None` clears it.
        label: Option<AddressLabel>,
    },
    /// The filled circle moved to this row.
    Primary(u32),
    /// "+ Add address".
    AddAddress,
    /// The minus beside this row.
    RemoveAddress(u32),
    /// A group typed or picked, "+ Add to group".
    AddGroup(String),
    /// The cross on this group chip.
    RemoveGroup(u32),
}

/// A form open in front of the Contacts view.
#[derive(Debug)]
pub(crate) struct ContactEdit {
    /// Who is being edited; `None` for a new person.
    person: Option<ContactId>,
    /// The form as it was opened: what "changed" is measured from.
    original: ContactForm,
    /// The form as it is now.
    form: ContactForm,
    /// Whether the kind is the detector's word.
    kind_detected: bool,
    /// Whether the next Esc discards.
    armed: bool,
    /// Whether Save was asked and refused.
    show_problem: bool,
    /// Who has the keyboard.
    focus: FormField,
    /// The avatar's seed.
    id: i64,
}

impl ContactEdit {
    pub(crate) fn editing(person: ContactId, form: ContactForm, kind_detected: bool) -> Self {
        Self {
            person: Some(person),
            original: form.clone(),
            form,
            kind_detected,
            armed: false,
            show_problem: false,
            focus: FormField::Given,
            id: person.get(),
        }
    }

    pub(crate) fn new_person() -> Self {
        let form = ContactForm::blank();
        Self {
            person: None,
            original: form.clone(),
            form,
            kind_detected: false,
            armed: false,
            show_problem: false,
            focus: FormField::Given,
            id: 0,
        }
    }

    fn dirty(&self) -> bool {
        self.form.tidy() != self.original.tidy()
    }

    fn view(&self, keymap: &postio_core::Keymap) -> EditView {
        postio_ui::contact_edit::view(
            &EditContext {
                form: &self.form,
                is_new: self.person.is_none(),
                kind_detected: self.kind_detected,
                id: self.id,
                armed: self.armed,
                show_problem: self.show_problem,
                focus: self.focus,
            },
            keymap,
        )
    }

    fn apply(&mut self, change: EditChange) {
        let form = &mut self.form;
        match change {
            EditChange::Kind(kind) => {
                form.kind = kind;
                self.kind_detected = false;
            }
            EditChange::Given(text) => form.given = text,
            EditChange::Family(text) => form.family = text,
            EditChange::ShowAs(style) => form.show_as = style,
            EditChange::Title(text) => form.title = text,
            EditChange::Company(text) => form.company = text,
            EditChange::Address { index, text } => {
                if let Some(row) = form.addresses.get_mut(index as usize) {
                    row.address = text;
                }
            }
            EditChange::Label { index, label } => {
                if let Some(row) = form.addresses.get_mut(index as usize) {
                    row.label = label;
                    row.bounced = false;
                }
            }
            EditChange::Primary(index) => {
                if (index as usize) < form.addresses.len() {
                    form.primary = index as usize;
                }
            }
            EditChange::AddAddress => {
                form.addresses.push(FormAddress::blank());
                self.focus = FormField::Address(form.addresses.len() - 1);
            }
            EditChange::RemoveAddress(index) => {
                let index = index as usize;
                if index < form.addresses.len() {
                    form.addresses.remove(index);
                    if form.addresses.is_empty() {
                        form.addresses.push(FormAddress::blank());
                    }
                    form.primary = match form.primary {
                        primary if primary == index => 0,
                        primary if primary > index => primary - 1,
                        primary => primary,
                    }
                    .min(form.addresses.len() - 1);
                }
            }
            EditChange::AddGroup(name) => {
                let name = name.trim();
                if !name.is_empty() && !form.groups.iter().any(|g| g.eq_ignore_ascii_case(name)) {
                    form.groups.push(name.to_owned());
                }
            }
            EditChange::RemoveGroup(index) => {
                if (index as usize) < form.groups.len() {
                    form.groups.remove(index as usize);
                }
            }
        }
    }
}

impl FocusController {
    /// ⌘E: the form on the person under the cursor, once their card is read.
    pub(crate) fn contact_edit_open(&mut self) -> Vec<Step> {
        let Some(contacts) = self.contacts.as_mut() else {
            return Vec::new();
        };
        if contacts.edit.is_some() {
            return Vec::new();
        }
        let Some(card) = contacts.card.as_ref() else {
            return Vec::new();
        };
        let form = ContactForm::of(card, &card.address_notes);
        let detected = card.person.kind_source == postio_model::KindSource::Detected;
        contacts.edit = Some(ContactEdit::editing(card.person.id, form, detected));
        self.draw_contact_edit()
    }

    /// ⌘⇧N: an empty form for a new person.
    pub(crate) fn contact_new_open(&mut self) -> Vec<Step> {
        let Some(contacts) = self.contacts.as_mut() else {
            return Vec::new();
        };
        if contacts.edit.is_some() {
            return Vec::new();
        }
        contacts.edit = Some(ContactEdit::new_person());
        self.draw_contact_edit()
    }

    /// The form's frame, whole.
    pub(crate) fn draw_contact_edit(&self) -> Vec<Step> {
        let view = self
            .contacts
            .as_ref()
            .and_then(|contacts| contacts.edit.as_ref())
            .map(|edit| Box::new(edit.view(self.bar.keymap())));
        vec![Step::Show(Intent::ContactEdit(view))]
    }

    /// The form, in words, while one is open.
    pub fn contact_edit(&self) -> Option<EditView> {
        let edit = self.contacts.as_ref()?.edit.as_ref()?;
        Some(edit.view(self.bar.keymap()))
    }

    /// A change a person made to the open form.
    pub(crate) fn contact_edit_changed(&mut self, change: EditChange) -> Vec<Step> {
        let Some(edit) = self
            .contacts
            .as_mut()
            .and_then(|contacts| contacts.edit.as_mut())
        else {
            return Vec::new();
        };
        edit.apply(change);
        // Anything typed puts the discard back out of reach, and a refusal
        // is said again only if Save is asked again.
        edit.armed = false;
        edit.show_problem = false;
        self.draw_contact_edit()
    }

    /// Escape in the form: arm when something changed, then discard.
    pub(crate) fn contact_edit_back(&mut self) -> Vec<Step> {
        let Some(contacts) = self.contacts.as_mut() else {
            return Vec::new();
        };
        let Some(edit) = contacts.edit.as_mut() else {
            return Vec::new();
        };
        if edit.dirty() && !edit.armed {
            edit.armed = true;
            return self.draw_contact_edit();
        }
        contacts.edit = None;
        vec![Step::Show(Intent::ContactEdit(None))]
    }

    /// ⌘S: one command, or the reason there is not one yet.
    pub(crate) fn contact_edit_save(&mut self) -> Vec<Step> {
        let Some(contacts) = self.contacts.as_mut() else {
            return Vec::new();
        };
        let Some(edit) = contacts.edit.as_mut() else {
            return Vec::new();
        };
        if edit.form.problem().is_some() {
            edit.show_problem = true;
            edit.armed = false;
            return self.draw_contact_edit();
        }
        let command = Command::ContactEdit {
            action: ContactEditAction::Save {
                person: edit.person,
                form: Box::new(edit.form.tidy()),
            },
        };
        contacts.edit = None;
        let mut steps = vec![Step::Show(Intent::ContactEdit(None))];
        steps.extend(self.verbs.post(command));
        steps
    }

    /// A click on the card's switch `index`: sent as a command, drawn at once.
    pub(crate) fn contact_card_switch(&mut self, index: u32) -> Vec<Step> {
        let Some(contacts) = self.contacts.as_mut() else {
            return Vec::new();
        };
        let Some(card) = contacts.card.as_mut() else {
            return Vec::new();
        };
        let keymap = self.bar.keymap();
        let now = self.bar.now();
        let view = postio_ui::contacts_view::card(card, keymap, now);
        let Some(switch) = view
            .switches
            .get(index as usize)
            .and_then(|line| line.switch)
        else {
            return Vec::new();
        };
        let Some(address) = card
            .person
            .preferred_address()
            .or_else(|| card.person.addresses.first())
            .map(|owned| owned.address.address.clone())
        else {
            return Vec::new();
        };
        let on = !CardSwitch::is_on(switch, &card.settings);
        switch.set(&mut card.settings, on);
        let drawn = postio_ui::contacts_view::card(card, keymap, now);
        let command = Command::ContactEdit {
            action: ContactEditAction::Switch {
                address,
                switch,
                on,
            },
        };
        let mut steps = vec![Step::Show(Intent::ContactsCard(Some(Box::new(drawn))))];
        steps.extend(self.verbs.post(command));
        steps
    }
}
