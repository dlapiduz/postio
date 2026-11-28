//! The Edit and New contact form's state: which form is open, what has changed, and the ladder
//! Escape climbs (the first Esc on an edited form arms, the second discards).
//! Saving is one command (`ContactEditAction::Save`), never a write the form makes itself.

use postio_core::{Command, ContactEditAction};
use postio_model::{
    AddressLabel, CardSwitch, ContactForm, ContactId, ContactKind, FormAddress, ShowAs,
};
use postio_ui::contact_edit::{EditContext, EditView, FormField};

use crate::feed::Step;
use crate::{FocusController, Intent};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditChange {
    Kind(ContactKind),
    Given(String),
    Family(String),
    ShowAs(ShowAs),
    Title(String),
    Company(String),
    Address {
        index: u32,
        text: String,
    },
    Label {
        index: u32,
        /// The type; `None` clears it.
        label: Option<AddressLabel>,
    },
    Primary(u32),
    AddAddress,
    RemoveAddress(u32),
    AddGroup(String),
    /// The group picker opened (`Some`, with what is typed so far) or put
    /// away (`None`).
    GroupQuery(Option<String>),
    GroupPick(u32),
    Seen {
        index: u32,
        secondary: bool,
    },
    RemoveGroup(u32),
}

#[derive(Debug)]
pub(crate) struct ContactEdit {
    /// Who is being edited; `None` for a new person.
    person: Option<ContactId>,
    original: ContactForm,
    form: ContactForm,
    kind_detected: bool,
    armed: bool,
    show_problem: bool,
    focus: FormField,
    id: i64,
    pub(crate) known_groups: Vec<String>,
    pub(crate) seen: Vec<postio_model::SeenItem>,
    pending_detail: Option<(postio_model::DetailKind, String, Option<bool>)>,
    group_query: Option<String>,
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
            known_groups: Vec::new(),
            seen: Vec::new(),
            pending_detail: None,
            group_query: None,
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
            known_groups: Vec::new(),
            seen: Vec::new(),
            pending_detail: None,
            group_query: None,
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
                known_groups: &self.known_groups,
                seen: &self.seen,
                group_query: self.group_query.as_deref(),
                synced_with: None,
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
                self.group_query = None;
            }
            EditChange::Seen { index, secondary } => {
                if (index as usize) < self.seen.len() {
                    let item = self.seen.remove(index as usize);
                    let keep = match (item.kept, secondary) {
                        (false, false) => Some(true),
                        (false, true) => Some(false),
                        (true, _) => None,
                    };
                    if keep == Some(true) {
                        self.seen.push(postio_model::SeenItem {
                            kept: true,
                            ..item.clone()
                        });
                    }
                    self.pending_detail = Some((item.kind, item.value, keep));
                }
            }
            EditChange::GroupQuery(query) => {
                if query.is_some() {
                    self.focus = FormField::Group;
                }
                self.group_query = query;
            }
            EditChange::GroupPick(index) => {
                let options = postio_ui::contact_edit::group_options(
                    &self.known_groups,
                    &form.groups,
                    self.group_query.as_deref().unwrap_or(""),
                );
                if let Some(option) = options.get(index as usize) {
                    form.groups.push(option.name.clone());
                    if option.is_new {
                        self.known_groups.push(option.name.clone());
                    }
                }
                self.group_query = None;
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
        let person = card.person.id;
        contacts.edit = Some(ContactEdit::editing(person, form, detected));
        let mut steps = self.draw_contact_edit();
        steps.push(Step::Ask(crate::Request::ContactGroups));
        steps.push(Step::Ask(crate::Request::ContactSeen(person)));
        steps
    }

    pub(crate) fn contact_new_open(&mut self) -> Vec<Step> {
        let Some(contacts) = self.contacts.as_mut() else {
            return Vec::new();
        };
        if contacts.edit.is_some() {
            return Vec::new();
        }
        contacts.edit = Some(ContactEdit::new_person());
        let mut steps = self.draw_contact_edit();
        steps.push(Step::Ask(crate::Request::ContactGroups));
        steps
    }

    pub(crate) fn draw_contact_edit(&self) -> Vec<Step> {
        let view = self
            .contacts
            .as_ref()
            .and_then(|contacts| contacts.edit.as_ref())
            .map(|edit| Box::new(edit.view(self.bar.keymap())));
        vec![Step::Show(Intent::ContactEdit(view))]
    }

    pub fn contact_edit(&self) -> Option<EditView> {
        let edit = self.contacts.as_ref()?.edit.as_ref()?;
        Some(edit.view(self.bar.keymap()))
    }

    pub(crate) fn contact_edit_changed(&mut self, change: EditChange) -> Vec<Step> {
        let Some(edit) = self
            .contacts
            .as_mut()
            .and_then(|contacts| contacts.edit.as_mut())
        else {
            return Vec::new();
        };
        let is_seen = matches!(change, EditChange::Seen { .. });
        edit.apply(change);
        // A detail kept or refused is its own command, undoable apart from
        // the form; it is not a change the form must be saved for.
        let detail = edit.pending_detail.take();
        let address = edit
            .form
            .addresses
            .get(edit.form.primary)
            .map(|row| row.address.clone())
            .filter(|address| !address.trim().is_empty());
        if !is_seen {
            // Anything typed puts the discard back out of reach, and a refusal
            // is said again only if Save is asked again.
            edit.armed = false;
            edit.show_problem = false;
        }
        let mut steps = self.draw_contact_edit();
        if let (Some((kind, value, keep)), Some(address)) = (detail, address) {
            steps.extend(self.verbs.post(Command::ContactEdit {
                action: ContactEditAction::Detail {
                    address,
                    kind,
                    value,
                    keep,
                },
            }));
        }
        steps
    }

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
