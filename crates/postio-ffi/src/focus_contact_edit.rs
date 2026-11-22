//! The Edit and New contact form: its content, rules and Esc ladder are Rust's. The frontend draws
//! `ContactEditViewFfi`,
//! reports each `EditChangeFfi`, and presses Cmd-S and Escape as commands.

use postio_focus::EditChange;
use postio_model::{AddressLabel, ShowAs};
use postio_ui::contact_edit::{EditView, FormField};

use crate::ContactKindFfi;
use crate::Session;
use crate::focus_contacts::AvatarFfi;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ShowAsFfi {
    Full,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AddressLabelFfi {
    Work,
    Personal,
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

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum EditChangeFfi {
    Kind {
        kind: ContactKindFfi,
    },
    Given {
        text: String,
    },
    Family {
        text: String,
    },
    ShowAs {
        style: ShowAsFfi,
    },
    Title {
        text: String,
    },
    Company {
        text: String,
    },
    Address {
        index: u32,
        text: String,
    },
    Label {
        index: u32,
        /// The type, or none.
        label: Option<AddressLabelFfi>,
    },
    Primary {
        index: u32,
    },
    AddAddress,
    RemoveAddress {
        index: u32,
    },
    AddGroup {
        name: String,
    },
    GroupQuery {
        /// What is typed; none puts it away.
        query: Option<String>,
    },
    GroupPick {
        index: u32,
    },
    Seen {
        index: u32,
        /// Whether it is Ignore rather than the main action.
        secondary: bool,
    },
    RemoveGroup {
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
            EditChangeFfi::GroupQuery { query } => Self::GroupQuery(query),
            EditChangeFfi::GroupPick { index } => Self::GroupPick(index),
            EditChangeFfi::Seen { index, secondary } => Self::Seen { index, secondary },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct EditSegmentFfi {
    pub label: String,
    pub on: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct EditAddressFfi {
    pub text: String,
    pub label: String,
    pub primary: bool,
    /// Whether a bounce named it.
    pub bounced: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct EditSeenFfi {
    pub what: String,
    pub from: String,
    /// "Add phone", "Add website" or "Remove".
    pub action: String,
    /// "Ignore", for one only offered.
    pub secondary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct EditButtonFfi {
    pub label: String,
    /// Its key, as the registry spells it.
    pub key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct EditHintFfi {
    pub key: String,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FormFieldFfi {
    Given,
    Address { index: u32 },
    Group,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GroupOptionFfi {
    pub label: String,
    pub is_new: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GroupPickerFfi {
    pub query: String,
    pub options: Vec<GroupOptionFfi>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactEditViewFfi {
    /// "Editing Ada Moreno".
    pub title: String,
    /// "saved in Postio".
    pub note: String,
    pub cancel: EditButtonFfi,
    pub save: EditButtonFfi,
    pub avatar: AvatarFfi,
    pub photo_note: String,
    pub kinds: Vec<EditSegmentFfi>,
    pub kind_note: String,
    pub given: String,
    pub family: String,
    pub show_as: Vec<EditSegmentFfi>,
    pub show_note: String,
    pub work_title: String,
    pub work_company: String,
    pub addresses: Vec<EditAddressFfi>,
    pub add_address: String,
    pub address_note: String,
    pub seen: Vec<EditSeenFfi>,
    pub groups: Vec<String>,
    pub add_group: String,
    pub group_picker: Option<GroupPickerFfi>,
    pub group_note: String,
    pub problem: Option<String>,
    pub armed: Option<String>,
    pub hints: Vec<EditHintFfi>,
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
                    secondary: row.secondary,
                })
                .collect(),
            groups: view.groups,
            add_group: view.add_group,
            group_picker: view.group_picker.map(|picker| GroupPickerFfi {
                query: picker.query,
                options: picker
                    .options
                    .into_iter()
                    .map(|option| GroupOptionFfi {
                        label: option.label,
                        is_new: option.is_new,
                    })
                    .collect(),
            }),
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
    pub fn focus_contact_edit(&self) -> Option<ContactEditViewFfi> {
        self.focus_driver().contact_edit().map(Into::into)
    }

    pub fn focus_contact_edit_change(&self, change: EditChangeFfi) {
        self.focus_driver()
            .input(postio_focus::Input::ContactEditChange(change.into()));
    }

    pub fn focus_contact_card_switch(&self, index: u32) {
        self.focus_driver()
            .input(postio_focus::Input::ContactCardSwitch(index));
    }
}
