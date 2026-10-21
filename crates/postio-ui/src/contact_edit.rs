//! The Edit and New contact form, in words (specs/011-focus-contacts S8,
//! handoff screen 06): what each part says, which segment is on, what the
//! Show as choices preview, and the hints under the form.
//!
//! The form's content and its rules are `postio_model::ContactForm`'s; its
//! state (what is changed, whether Esc is armed) is `postio-focus`'s. This
//! turns both into what a frontend draws, with no toolkit in it.

use postio_core::{CommandId, Keymap};
use postio_model::{ContactForm, ContactKind, ShowAs};

use crate::contact_popover::{Avatar, avatar};
use crate::hints::{Hint, fixed, hint};

/// One choice of a segmented control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    /// Its words.
    pub label: String,
    /// Whether it is the chosen one.
    pub on: bool,
}

/// One address row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressRowView {
    /// What is typed.
    pub text: String,
    /// The type menu's current words; empty when none is chosen.
    pub label: String,
    /// Whether the filled circle is here.
    pub primary: bool,
    /// Whether a bounce named it: its type reads "Old \u{b7} bounced".
    pub bounced: bool,
}

/// One thing the mail says that the person might keep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeenRow {
    /// "+1 555 0119".
    pub what: String,
    /// "phone number in her signature since August".
    pub from: String,
    /// "Ignore" or "Add phone".
    pub action: String,
}

/// A button with its key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditButton {
    /// "Save".
    pub label: String,
    /// `mod+s`, if bound.
    pub key: Option<String>,
}

/// Which field has the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FormField {
    /// First name, where an edit opens and a new person starts.
    #[default]
    Given,
    /// An address row's text, by index.
    Address(usize),
    /// The group name being typed.
    Group,
}

/// One line of the "+ Add to group" picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupOption {
    /// What the line says: a group's name, or `New group <name>`.
    pub label: String,
    /// The name it adds.
    pub name: String,
    /// Whether it makes the group.
    pub is_new: bool,
}

/// The picker under "+ Add to group": what is typed, and the groups it
/// matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupPicker {
    /// What is typed.
    pub query: String,
    /// The matching groups, then `New group <query>`.
    pub options: Vec<GroupOption>,
}

/// The groups `query` offers: those that exist, are not already on the form
/// and contain what is typed, in the order given; and `New group <query>`
/// last unless a group already has exactly that name.
pub fn group_options(known: &[String], chosen: &[String], query: &str) -> Vec<GroupOption> {
    let query = query.trim();
    let wanted = query.to_lowercase();
    let mut options: Vec<GroupOption> = known
        .iter()
        .filter(|name| !chosen.iter().any(|c| c.eq_ignore_ascii_case(name)))
        .filter(|name| name.to_lowercase().contains(&wanted))
        .map(|name| GroupOption {
            label: name.clone(),
            name: name.clone(),
            is_new: false,
        })
        .collect();
    let exists = known
        .iter()
        .chain(chosen.iter())
        .any(|name| name.eq_ignore_ascii_case(query));
    if !query.is_empty() && !exists {
        options.push(GroupOption {
            label: format!("New group {query}"),
            name: query.to_owned(),
            is_new: true,
        });
    }
    options
}

/// What the form draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditView {
    /// "Editing Ada Moreno", or "New contact".
    pub title: String,
    /// "saved in Postio".
    pub note: String,
    /// Cancel and its key.
    pub cancel: EditButton,
    /// Save and its key.
    pub save: EditButton,
    /// The avatar beside "Photo".
    pub avatar: Avatar,
    /// "Initials until you add one. ..."
    pub photo_note: String,
    /// The five kinds.
    pub kinds: Vec<Segment>,
    /// "Detected from their mail. ..." or "Set by you. ..."
    pub kind_note: String,
    /// First name.
    pub given: String,
    /// Last name.
    pub family: String,
    /// The three styles, each as it would read.
    pub show_as: Vec<Segment>,
    /// "What the inbox shows for their mail, whatever name their mail client sends."
    pub show_note: String,
    /// Job title.
    pub work_title: String,
    /// Company.
    pub work_company: String,
    /// The address rows.
    pub addresses: Vec<AddressRowView>,
    /// "+ Add address".
    pub add_address: String,
    /// "The filled circle is where replies and new mail go. ..."
    pub address_note: String,
    /// What the mail says: details found in headers and signatures.
    pub seen: Vec<SeenRow>,
    /// The group chips.
    pub groups: Vec<String>,
    /// "+ Add to group".
    pub add_group: String,
    /// The picker, while one is open.
    pub group_picker: Option<GroupPicker>,
    /// "Groups work as recipients ..."
    pub group_note: String,
    /// Why Save would be refused, said once the person asks it.
    pub problem: Option<String>,
    /// "Esc again to discard your changes", while Esc is armed.
    pub armed: Option<String>,
    /// The hints under the form.
    pub hints: Vec<Hint>,
    /// Who has the keyboard.
    pub focus: FormField,
}

/// What the form needs beside the form itself.
#[derive(Debug, Clone, Copy)]
pub struct EditContext<'a> {
    /// The form as the person has it now.
    pub form: &'a ContactForm,
    /// Whether this is a person being made.
    pub is_new: bool,
    /// Whether the kind is the detector's word rather than the user's.
    pub kind_detected: bool,
    /// The stable id of the avatar's fill; any for a new person.
    pub id: i64,
    /// Whether Esc is armed.
    pub armed: bool,
    /// Whether the person asked for Save and the form said no.
    pub show_problem: bool,
    /// Who has the keyboard.
    pub focus: FormField,
    /// Every group that exists.
    pub known_groups: &'a [String],
    /// What is typed in the group picker, while it is open.
    pub group_query: Option<&'a str>,
}

/// The kind's words in the segmented control.
pub fn kind_title(kind: ContactKind) -> &'static str {
    match kind {
        ContactKind::Person => "Person",
        ContactKind::Company => "Company",
        ContactKind::List => "List",
        ContactKind::Newsletter => "Newsletter",
        ContactKind::Automated => "Automated",
    }
}

/// The form in words.
pub fn view(ctx: &EditContext<'_>, keymap: &Keymap) -> EditView {
    let form = ctx.form;
    let name = form.full_name();
    let named = if name.is_empty() && !form.company.trim().is_empty() {
        form.company.trim().to_owned()
    } else {
        name.clone()
    };
    let they = "their";
    let title = if ctx.is_new {
        "New contact".to_owned()
    } else {
        format!(
            "Editing {}",
            if named.is_empty() { "contact" } else { &named }
        )
    };
    let button = |label: &str, command| EditButton {
        label: label.to_owned(),
        key: keymap.binding(command).map(str::to_owned),
    };
    let kinds = ContactKind::ALL
        .iter()
        .map(|kind| Segment {
            label: kind_title(*kind).to_owned(),
            on: *kind == form.kind,
        })
        .collect();
    let kind_note = if ctx.kind_detected {
        format!("Detected from {they} mail. Only people and companies sync to Contacts.")
    } else {
        "Set by you. Only people and companies sync to Contacts.".to_owned()
    };
    let show_as = [ShowAs::Full, ShowAs::First, ShowAs::LastFirst]
        .iter()
        .map(|style| Segment {
            label: {
                let text = form.shown_as(*style);
                if text.is_empty() {
                    match style {
                        ShowAs::Full => "Full name".to_owned(),
                        ShowAs::First => "First name".to_owned(),
                        _ => "Last, first".to_owned(),
                    }
                } else {
                    text
                }
            },
            on: *style == form.show_as,
        })
        .collect();
    let addresses = form
        .addresses
        .iter()
        .enumerate()
        .map(|(index, row)| crate::contact_edit::AddressRowView {
            text: row.address.clone(),
            label: row.label_text(),
            primary: index == form.primary,
            bounced: row.bounced,
        })
        .collect();
    let problem = ctx
        .show_problem
        .then(|| form.problem().map(|problem| problem.text()))
        .flatten();
    let hints = [
        Some(fixed(
            "Tab",
            "next field",
            "moving between a form's fields is the toolkit's",
        )),
        hint(keymap, CommandId::ContactSave, "save"),
        hint(keymap, CommandId::Back, "cancel"),
    ]
    .into_iter()
    .flatten()
    .collect();
    EditView {
        title,
        note: "saved in Postio".to_owned(),
        cancel: button("Cancel", CommandId::Back),
        save: button("Save", CommandId::ContactSave),
        avatar: avatar(
            postio_model::ContactId::new(ctx.id),
            if named.is_empty() { "?" } else { &named },
            form.kind,
        ),
        photo_note: "Initials until you add one.".to_owned(),
        kinds,
        kind_note,
        given: form.given.clone(),
        family: form.family.clone(),
        show_as,
        show_note: "What the inbox shows for their mail, whatever name their mail client sends."
            .to_owned(),
        work_title: form.title.clone(),
        work_company: form.company.clone(),
        addresses,
        add_address: "+ Add address".to_owned(),
        address_note: format!(
            "The filled circle is where replies and new mail go. Mail from any of them counts as {}.",
            if form.given.trim().is_empty() {
                "this contact"
            } else {
                form.given.trim()
            }
        ),
        seen: Vec::new(),
        groups: form.groups.clone(),
        add_group: "+ Add to group".to_owned(),
        group_picker: ctx.group_query.map(|query| GroupPicker {
            query: query.to_owned(),
            options: group_options(ctx.known_groups, &form.groups, query),
        }),
        group_note:
            "Groups work as recipients (to:Atlas core) and as search filters (from:@Finance)."
                .to_owned(),
        problem,
        armed: ctx
            .armed
            .then(|| "Press Esc again to discard your changes.".to_owned()),
        hints,
        focus: ctx.focus,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_model::{AddressLabel, FormAddress};

    fn keymap() -> Keymap {
        Keymap::resolve_on(
            &postio_config::KeyBindings::default(),
            postio_config::paths::Platform::Apple,
        )
    }

    fn ada() -> ContactForm {
        ContactForm {
            given: "Ada".into(),
            family: "Moreno".into(),
            title: "Finance lead".into(),
            company: "Northwind Partners".into(),
            addresses: vec![
                FormAddress {
                    address: "ada@example.org".into(),
                    label: Some(AddressLabel::Work),
                    ..FormAddress::blank()
                },
                FormAddress {
                    address: "a.moreno@example.net".into(),
                    bounced: true,
                    ..FormAddress::blank()
                },
            ],
            groups: vec!["Atlas core".into(), "Finance".into()],
            ..ContactForm::blank()
        }
    }

    fn ctx(form: &ContactForm) -> EditContext<'_> {
        EditContext {
            form,
            is_new: false,
            kind_detected: true,
            id: 7,
            armed: false,
            show_problem: false,
            focus: FormField::Given,
            known_groups: &[],
            group_query: None,
        }
    }

    #[test]
    fn the_bar_names_the_person_and_where_the_edit_is_saved() {
        let form = ada();
        let view = view(&ctx(&form), &keymap());
        assert_eq!(view.title, "Editing Ada Moreno");
        assert_eq!(view.note, "saved in Postio");
        assert_eq!(view.cancel.label, "Cancel");
        assert_eq!(view.cancel.key.as_deref(), Some("Escape"));
        assert_eq!(view.save.label, "Save");
        assert_eq!(view.save.key.as_deref(), Some("cmd+s"));
    }

    #[test]
    fn a_new_contact_says_so() {
        let form = ContactForm::blank();
        let mut context = ctx(&form);
        context.is_new = true;
        assert_eq!(view(&context, &keymap()).title, "New contact");
    }

    #[test]
    fn the_five_kinds_show_the_chosen_one_and_say_who_decided() {
        let form = ada();
        let shown = view(&ctx(&form), &keymap());
        let kinds: Vec<(&str, bool)> = shown
            .kinds
            .iter()
            .map(|k| (k.label.as_str(), k.on))
            .collect();
        assert_eq!(
            kinds,
            [
                ("Person", true),
                ("Company", false),
                ("List", false),
                ("Newsletter", false),
                ("Automated", false)
            ]
        );
        assert!(shown.kind_note.starts_with("Detected from their mail."));
        let mut context = ctx(&form);
        context.kind_detected = false;
        assert!(
            view(&context, &keymap())
                .kind_note
                .starts_with("Set by you.")
        );
    }

    #[test]
    fn show_as_previews_each_style_with_the_names_as_typed() {
        let mut form = ada();
        form.show_as = ShowAs::First;
        let shown = view(&ctx(&form), &keymap());
        let labels: Vec<(&str, bool)> = shown
            .show_as
            .iter()
            .map(|s| (s.label.as_str(), s.on))
            .collect();
        assert_eq!(
            labels,
            [("Ada Moreno", false), ("Ada", true), ("Moreno, Ada", false)]
        );
    }

    #[test]
    fn addresses_mark_the_primary_and_say_old_and_bounced() {
        let form = ada();
        let shown = view(&ctx(&form), &keymap());
        assert!(shown.addresses[0].primary);
        assert_eq!(shown.addresses[0].label, "Work");
        assert!(!shown.addresses[1].primary);
        assert_eq!(shown.addresses[1].label, "Old \u{b7} bounced");
    }

    #[test]
    fn the_footer_names_tab_save_and_cancel() {
        let form = ada();
        let hints: Vec<(String, String)> = view(&ctx(&form), &keymap())
            .hints
            .into_iter()
            .map(|h| (h.key, h.label))
            .collect();
        assert_eq!(
            hints,
            [
                ("Tab".to_owned(), "next field".to_owned()),
                ("cmd+s".to_owned(), "save".to_owned()),
                ("Escape".to_owned(), "cancel".to_owned())
            ]
        );
    }

    fn names(options: &[GroupOption]) -> Vec<&str> {
        options.iter().map(|o| o.label.as_str()).collect()
    }

    #[test]
    fn the_picker_filters_the_existing_groups_as_you_type_and_offers_a_new_one_last() {
        let known: Vec<String> = ["Atlas core", "Finance", "Quarry crew"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let chosen = vec!["Finance".to_owned()];
        assert_eq!(
            names(&group_options(&known, &chosen, "")),
            ["Atlas core", "Quarry crew"],
            "nothing typed: what exists, minus what is on the form, no New group"
        );
        assert_eq!(
            names(&group_options(&known, &chosen, "ar")),
            ["Quarry crew", "New group ar"]
        );
        assert_eq!(
            names(&group_options(&known, &chosen, "atlas CORE")),
            ["Atlas core"],
            "a name that exists is not offered again as new"
        );
        assert_eq!(
            names(&group_options(&known, &chosen, "Finance")),
            Vec::<&str>::new(),
            "one already on the form offers nothing"
        );
        let new = group_options(&known, &chosen, " Book club ");
        assert_eq!(
            new.last().map(|o| (o.name.as_str(), o.is_new)),
            Some(("Book club", true))
        );
    }

    #[test]
    fn the_picker_is_in_the_view_only_while_it_is_open() {
        let form = ada();
        let known = vec!["Atlas core".to_owned(), "Zeta".to_owned()];
        let mut context = ctx(&form);
        context.known_groups = &known;
        assert!(view(&context, &keymap()).group_picker.is_none());
        context.group_query = Some("z");
        let picker = view(&context, &keymap()).group_picker.expect("open");
        assert_eq!(picker.query, "z");
        assert_eq!(names(&picker.options), ["Zeta", "New group z"]);
    }

    #[test]
    fn a_problem_is_said_once_save_is_asked_and_esc_says_it_is_armed() {
        let form = ContactForm::blank();
        let mut context = ctx(&form);
        assert_eq!(view(&context, &keymap()).problem, None);
        context.show_problem = true;
        context.armed = true;
        let shown = view(&context, &keymap());
        assert_eq!(shown.problem.as_deref(), Some("Add at least one address."));
        assert!(shown.armed.unwrap().contains("Esc again"));
    }
}
