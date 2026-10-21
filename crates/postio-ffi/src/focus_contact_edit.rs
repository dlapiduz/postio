//! The Edit and New contact form across the boundary (specs/011-focus-contacts
//! S8). The form's content, rules and Esc ladder are Rust's: the frontend
//! draws [`ContactEditViewFfi`], reports each change as an
//! [`EditChangeFfi`], and presses Cmd-S and Escape as commands.

use postio_focus::EditChange;
use postio_model::{AddressLabel, ShowAs};
use postio_ui::contact_edit::{EditView, FormField};

use crate::ContactKindFfi;
use crate::Session;
use crate::focus_contacts::AvatarFfi;

/// How the inbox names a person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ShowAsFfi {
    /// First and last.
    Full,
    /// First name.
    First,
    /// "Last, first".
    LastFirst,
}

impl From<ShowAsFfi> for ShowAs {
    fn from(style: ShowAsFfi) -> Self {
        match style {
            ShowAsFfi::Full => Self::Full,
            ShowAsFfi::First => Self::First,
            ShowAsFfi::LastFirst => Self::LastFirst,
        }
    }
}

/// What an address is called.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AddressLabelFfi {
    /// Work.
    Work,
    /// Personal.
    Personal,
    /// Old.
    Old,
}

impl From<AddressLabelFfi> for AddressLabel {
    fn from(label: AddressLabelFfi) -> Self {
        match label {
            AddressLabelFfi::Work => Self::Work,
            AddressLabelFfi::Personal => Self::Personal,
            AddressLabelFfi::Old => Self::Old,
        }
    }
}

/// One change to the open form.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum EditChangeFfi {
    /// A kind chosen.
    Kind {
        /// Which.
        kind: ContactKindFfi,
    },
    /// The first name as it now reads.
    Given {
        /// The text.
        text: String,
    },
    /// The last name as it now reads.
    Family {
        /// The text.
        text: String,
    },
    /// A Show as style chosen.
    ShowAs {
        /// Which.
        style: ShowAsFfi,
    },
    /// The job title as it now reads.
    Title {
        /// The text.
        text: String,
    },
    /// The company as it now reads.
    Company {
        /// The text.
        text: String,
    },
    /// An address row's text.
    Address {
        /// The row.
        index: u32,
        /// The text.
        text: String,
    },
    /// An address row's type.
    Label {
        /// The row.
        index: u32,
        /// The type, or none.
        label: Option<AddressLabelFfi>,
    },
    /// The filled circle moved.
    Primary {
        /// The row.
        index: u32,
    },
    /// "+ Add address".
    AddAddress,
    /// The minus on a row.
    RemoveAddress {
        /// The row.
        index: u32,
    },
    /// A group added.
    AddGroup {
        /// Its name.
        name: String,
    },
    /// A chip's cross.
    RemoveGroup {
        /// The chip.
        index: u32,
    },
}

impl From<EditChangeFfi> for EditChange {
    fn from(change: EditChangeFfi) -> Self {
        match change {
            EditChangeFfi::Kind { kind } => Self::Kind(kind.into()),
            EditChangeFfi::Given { text } => Self::Given(text),
            EditChangeFfi::Family { text } => Self::Family(text),
            EditChangeFfi::ShowAs { style } => Self::ShowAs(style.into()),
            EditChangeFfi::Title { text } => Self::Title(text),
            EditChangeFfi::Company { text } => Self::Company(text),
            EditChangeFfi::Address { index, text } => Self::Address { index, text },
            EditChangeFfi::Label { index, label } => Self::Label {
                index,
                label: label.map(Into::into),
            },
            EditChangeFfi::Primary { index } => Self::Primary(index),
            EditChangeFfi::AddAddress => Self::AddAddress,
            EditChangeFfi::RemoveAddress { index } => Self::RemoveAddress(index),
            EditChangeFfi::AddGroup { name } => Self::AddGroup(name),
            EditChangeFfi::RemoveGroup { index } => Self::RemoveGroup(index),
        }
    }
}

/// A segment of a segmented control.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct EditSegmentFfi {
    /// Its words.
    pub label: String,
    /// Whether it is chosen.
    pub on: bool,
}

/// An address row.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct EditAddressFfi {
    /// What is typed.
    pub text: String,
    /// The type menu's words.
    pub label: String,
    /// Whether the filled circle is here.
    pub primary: bool,
    /// Whether a bounce named it.
    pub bounced: bool,
}

/// A detail the mail shows.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct EditSeenFfi {
    /// The detail.
    pub what: String,
    /// Where it was seen.
    pub from: String,
    /// "Ignore" or "Add phone".
    pub action: String,
}

/// A button with its key.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct EditButtonFfi {
    /// Its words.
    pub label: String,
    /// Its key, as the registry spells it.
    pub key: Option<String>,
}

/// A footer hint.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct EditHintFfi {
    /// The key.
    pub key: String,
    /// What it does.
    pub label: String,
}

/// Which field has the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FormFieldFfi {
    /// First name.
    Given,
    /// An address row.
    Address {
        /// Which.
        index: u32,
    },
    /// The group field.
    Group,
}

/// The form, drawn whole.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactEditViewFfi {
    /// "Editing Ada Moreno".
    pub title: String,
    /// "saved in Postio".
    pub note: String,
    /// Cancel.
    pub cancel: EditButtonFfi,
    /// Save.
    pub save: EditButtonFfi,
    /// The avatar.
    pub avatar: AvatarFfi,
    /// The photo row's note.
    pub photo_note: String,
    /// The five kinds.
    pub kinds: Vec<EditSegmentFfi>,
    /// The kind's note.
    pub kind_note: String,
    /// First name.
    pub given: String,
    /// Last name.
    pub family: String,
    /// The three styles.
    pub show_as: Vec<EditSegmentFfi>,
    /// The style's note.
    pub show_note: String,
    /// Job title.
    pub work_title: String,
    /// Company.
    pub work_company: String,
    /// The addresses.
    pub addresses: Vec<EditAddressFfi>,
    /// "+ Add address".
    pub add_address: String,
    /// The addresses' note.
    pub address_note: String,
    /// What the mail shows.
    pub seen: Vec<EditSeenFfi>,
    /// The group chips.
    pub groups: Vec<String>,
    /// "+ Add to group".
    pub add_group: String,
    /// The groups' note.
    pub group_note: String,
    /// Why Save was refused.
    pub problem: Option<String>,
    /// What Esc says while armed.
    pub armed: Option<String>,
    /// The footer's hints.
    pub hints: Vec<EditHintFfi>,
    /// Who has the keyboard.
    pub focus: FormFieldFfi,
}

impl From<EditView> for ContactEditViewFfi {
    fn from(view: EditView) -> Self {
        let segments = |segments: Vec<postio_ui::contact_edit::Segment>| -> Vec<EditSegmentFfi> {
            segments
                .into_iter()
                .map(|segment| EditSegmentFfi {
                    label: segment.label,
                    on: segment.on,
                })
                .collect()
        };
        let button = |button: postio_ui::contact_edit::EditButton| EditButtonFfi {
            label: button.label,
            key: button.key,
        };
        ContactEditViewFfi {
            title: view.title,
            note: view.note,
            cancel: button(view.cancel),
            save: button(view.save),
            avatar: view.avatar.into(),
            photo_note: view.photo_note,
            kinds: segments(view.kinds),
            kind_note: view.kind_note,
            given: view.given,
            family: view.family,
            show_as: segments(view.show_as),
            show_note: view.show_note,
            work_title: view.work_title,
            work_company: view.work_company,
            addresses: view
                .addresses
                .into_iter()
                .map(|row| EditAddressFfi {
                    text: row.text,
                    label: row.label,
                    primary: row.primary,
                    bounced: row.bounced,
                })
                .collect(),
            add_address: view.add_address,
            address_note: view.address_note,
            seen: view
                .seen
                .into_iter()
                .map(|row| EditSeenFfi {
                    what: row.what,
                    from: row.from,
                    action: row.action,
                })
                .collect(),
            groups: view.groups,
            add_group: view.add_group,
            group_note: view.group_note,
            problem: view.problem,
            armed: view.armed,
            hints: view
                .hints
                .into_iter()
                .map(|hint| EditHintFfi {
                    key: hint.key,
                    label: hint.label,
                })
                .collect(),
            focus: match view.focus {
                FormField::Given => FormFieldFfi::Given,
                FormField::Address(index) => FormFieldFfi::Address {
                    index: index as u32,
                },
                FormField::Group => FormFieldFfi::Group,
            },
        }
    }
}

#[uniffi::export]
impl Session {
    /// The Edit contact form, while one is open.
    pub fn focus_contact_edit(&self) -> Option<ContactEditViewFfi> {
        self.focus_driver().contact_edit().map(Into::into)
    }

    /// A change the person made to the open form.
    pub fn focus_contact_edit_change(&self, change: EditChangeFfi) {
        self.focus_driver()
            .input(postio_focus::Input::ContactEditChange(change.into()));
    }

    /// A click on the card's switch at `index`.
    pub fn focus_contact_card_switch(&self, index: u32) {
        self.focus_driver()
            .input(postio_focus::Input::ContactCardSwitch(index));
    }
}
