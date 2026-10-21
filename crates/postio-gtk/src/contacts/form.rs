//! The Edit and New contact form (handoff screen 06), in the card's place:
//! an accent-tinted bar with Cancel and Save, then the form, its labels
//! right-aligned over fields. What it says, what may be saved and the Esc
//! ladder are the controller's (`postio_ui::contact_edit::EditView`); this
//! draws them and reports each change as the `EditChange` it is.
//!
//! The form is drawn whole on every change and the keyboard put back where it
//! was, with its caret: the controller owns the text, so a letter typed is a
//! change to it and the field is drawn from what it answers. Nothing here
//! keeps a copy that could differ.

use std::rc::Rc;

use gtk::prelude::*;
use postio_core::CommandId;
use postio_focus::EditChange;
use postio_model::{AddressLabel, ContactKind, ShowAs};
use postio_ui::contact_edit::{EditButton, EditView, FormField, Segment};
use postio_widgets::widgets::button::{self, Kind, Size};
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::segmented::SegmentedControl;
use postio_widgets::widgets::space::{S2, S3, S4};

/// What a change in the form does.
#[derive(Clone)]
pub struct FormHooks {
    /// A change to the form.
    pub change: Rc<dyn Fn(EditChange)>,
    /// Save and Cancel: the commands their keys send.
    pub command: Rc<dyn Fn(CommandId)>,
}

/// One field a person can be typing in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// First name.
    Given,
    /// Last name.
    Family,
    /// Job title.
    Title,
    /// Company.
    Company,
    /// An address row.
    Address(usize),
    /// The group name being typed.
    Group,
}

impl From<FormField> for Slot {
    fn from(field: FormField) -> Self {
        match field {
            FormField::Given => Slot::Given,
            FormField::Address(index) => Slot::Address(index),
            FormField::Group => Slot::Group,
        }
    }
}

/// The form, built, with the fields it holds.
pub struct Form {
    root: gtk::Box,
    fields: Vec<(Slot, gtk::Entry)>,
}

impl Form {
    /// The form's widget.
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Where the keyboard is in the form, and the caret in it.
    pub fn focus(&self) -> Option<(Slot, i32)> {
        self.fields
            .iter()
            .find(|(_, entry)| entry.focus_child().is_some() || entry.has_focus())
            .map(|(slot, entry)| (*slot, entry.position()))
    }

    /// Put the keyboard in `slot`, the caret at `position` (the end when
    /// `None`).
    pub fn put_focus(&self, slot: Slot, position: Option<i32>) {
        if let Some((_, entry)) = self.fields.iter().find(|(held, _)| *held == slot) {
            entry.grab_focus();
            entry.set_position(position.unwrap_or(-1));
        }
    }

    /// The text of the field in `slot`.
    pub fn text_of(&self, slot: Slot) -> Option<String> {
        self.fields
            .iter()
            .find(|(held, _)| *held == slot)
            .map(|(_, entry)| entry.text().to_string())
    }

    /// The field in `slot`, to type into as a person would.
    pub fn entry_of(&self, slot: Slot) -> Option<gtk::Entry> {
        self.fields
            .iter()
            .find(|(held, _)| *held == slot)
            .map(|(_, entry)| entry.clone())
    }
}

/// The form for `view`.
pub fn build(view: &EditView, hooks: &FormHooks) -> Form {
    let mut fields = Vec::new();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("focus-contact-form");
    root.append(&bar(view, hooks));

    let rows = gtk::Box::new(gtk::Orientation::Vertical, S4);
    rows.add_css_class("focus-contact-form-rows");
    rows.append(&row("Photo", &photo(view)));
    rows.append(&row(
        "Kind",
        &segments(&view.kinds, &view.kind_note, {
            let change = Rc::clone(&hooks.change);
            move |index| {
                if let Some(kind) = KINDS.get(index) {
                    change(EditChange::Kind(*kind));
                }
            }
        }),
    ));
    let names = gtk::Box::new(gtk::Orientation::Horizontal, S2);
    names.append(&field(
        &mut fields,
        Slot::Given,
        &view.given,
        false,
        hooks,
        EditChange::Given,
    ));
    names.append(&field(
        &mut fields,
        Slot::Family,
        &view.family,
        false,
        hooks,
        EditChange::Family,
    ));
    rows.append(&row("Name", &names));
    rows.append(&row(
        "Show as",
        &segments(&view.show_as, &view.show_note, {
            let change = Rc::clone(&hooks.change);
            move |index| {
                if let Some(style) = STYLES.get(index) {
                    change(EditChange::ShowAs(*style));
                }
            }
        }),
    ));
    let work = gtk::Box::new(gtk::Orientation::Horizontal, S2);
    work.append(&field(
        &mut fields,
        Slot::Title,
        &view.work_title,
        false,
        hooks,
        EditChange::Title,
    ));
    work.append(&field(
        &mut fields,
        Slot::Company,
        &view.work_company,
        false,
        hooks,
        EditChange::Company,
    ));
    rows.append(&row("Work", &work));
    rows.append(&row("Addresses", &addresses(&mut fields, view, hooks)));
    if !view.seen.is_empty() {
        rows.append(&row("Seen in mail", &seen(view, hooks)));
    }
    rows.append(&row("Groups", &groups(&mut fields, view, hooks)));
    if let Some(problem) = &view.problem {
        let said = gtk::Label::new(Some(problem));
        said.add_css_class("focus-contact-problem");
        said.set_xalign(0.0);
        said.set_wrap(true);
        rows.append(&row("", &said));
    }
    let scroller = gtk::ScrolledWindow::builder()
        .child(&rows)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    root.append(&scroller);
    Form { root, fields }
}

const KINDS: [ContactKind; 5] = [
    ContactKind::Person,
    ContactKind::Company,
    ContactKind::List,
    ContactKind::Newsletter,
    ContactKind::Automated,
];
const STYLES: [ShowAs; 3] = [ShowAs::Full, ShowAs::First, ShowAs::LastFirst];
const LABELS: [(AddressLabel, &str); 3] = [
    (AddressLabel::Work, "Work"),
    (AddressLabel::Personal, "Personal"),
    (AddressLabel::Old, "Old"),
];

fn bar(view: &EditView, hooks: &FormHooks) -> gtk::Box {
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, S3);
    bar.add_css_class("focus-contact-form-bar");
    let title = gtk::Label::new(Some(&view.title));
    title.add_css_class("focus-contact-form-title");
    let note = gtk::Label::new(Some(&view.note));
    note.add_css_class("dim-label");
    bar.append(&title);
    bar.append(&note);
    if let Some(armed) = &view.armed {
        let armed = gtk::Label::new(Some(armed));
        armed.add_css_class("focus-contact-armed");
        bar.append(&armed);
    }
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    bar.append(&spacer);
    for (button, command, primary) in [
        (&view.cancel, CommandId::Back, false),
        (&view.save, CommandId::ContactSave, true),
    ] {
        bar.append(&bar_button(button, command, primary, hooks));
    }
    bar
}

fn bar_button(
    spec: &EditButton,
    command: CommandId,
    primary: bool,
    hooks: &FormHooks,
) -> gtk::Button {
    let one = button::button(
        &spec.label,
        if primary {
            Kind::Primary
        } else {
            Kind::Secondary
        },
        Size::Regular,
    );
    keyhint::dress(&one, &spec.label, spec.key.as_deref());
    let run = Rc::clone(&hooks.command);
    one.connect_clicked(move |_| run(command));
    one
}

fn row(label: &str, content: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, S4);
    let name = gtk::Label::new(Some(label));
    name.add_css_class("dim-label");
    name.add_css_class("focus-contact-form-label");
    name.set_xalign(1.0);
    name.set_yalign(0.0);
    name.set_width_chars(12);
    row.append(&name);
    content.set_hexpand(true);
    content.set_halign(gtk::Align::Fill);
    row.append(content);
    row
}

fn photo(view: &EditView) -> gtk::Box {
    let photo = gtk::Box::new(gtk::Orientation::Horizontal, S3);
    photo.append(&super::avatar::widget(&view.avatar, 48));
    let note = gtk::Label::new(Some(&view.photo_note));
    note.add_css_class("dim-label");
    note.add_css_class("focus-contact-small");
    note.set_wrap(true);
    note.set_xalign(0.0);
    note.set_hexpand(true);
    note.set_valign(gtk::Align::Center);
    photo.append(&note);
    photo
}

fn segments(items: &[Segment], note: &str, pick: impl Fn(usize) + 'static) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, S2);
    let labels: Vec<&str> = items.iter().map(|item| item.label.as_str()).collect();
    let control = SegmentedControl::new(note, &labels);
    if let Some(on) = items.iter().position(|item| item.on) {
        control.set_selected(on);
    }
    control.connect_selected(pick);
    control.widget().set_halign(gtk::Align::Start);
    column.append(control.widget());
    let note = gtk::Label::new(Some(note));
    note.add_css_class("dim-label");
    note.add_css_class("focus-contact-small");
    note.set_xalign(0.0);
    note.set_wrap(true);
    column.append(&note);
    column
}

/// A text field whose every edit is reported.
fn field(
    fields: &mut Vec<(Slot, gtk::Entry)>,
    slot: Slot,
    text: &str,
    mono: bool,
    hooks: &FormHooks,
    change: impl Fn(String) -> EditChange + 'static,
) -> gtk::Entry {
    let entry = gtk::Entry::new();
    entry.add_css_class("focus-contact-field");
    if mono {
        entry.add_css_class("focus-contact-mono");
    }
    entry.set_text(text);
    entry.set_hexpand(true);
    let report = Rc::clone(&hooks.change);
    entry.connect_changed(move |entry| report(change(entry.text().to_string())));
    fields.push((slot, entry.clone()));
    entry
}

fn addresses(fields: &mut Vec<(Slot, gtk::Entry)>, view: &EditView, hooks: &FormHooks) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, S2);
    // The primary address is one of a group, drawn as the filled circle the
    // handoff names: a check button that joins the first one's group is a
    // radio button.
    let mut first: Option<gtk::CheckButton> = None;
    for (index, address) in view.addresses.iter().enumerate() {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        let primary = gtk::CheckButton::new();
        match &first {
            Some(leader) => primary.set_group(Some(leader)),
            None => first = Some(primary.clone()),
        }
        primary.set_active(address.primary);
        primary.add_css_class("focus-contact-primary");
        primary.update_property(&[gtk::accessible::Property::Label(if address.primary {
            "Primary address"
        } else {
            "Make primary"
        })]);
        let change = Rc::clone(&hooks.change);
        let was = address.primary;
        primary.connect_toggled(move |button| {
            if !was && button.is_active() {
                change(EditChange::Primary(index as u32));
            }
        });
        line.append(&primary);
        let typed = field(fields, Slot::Address(index), &address.text, true, hooks, {
            move |text| EditChange::Address {
                index: index as u32,
                text,
            }
        });
        line.append(&typed);
        line.append(&label_menu(index, &address.label, hooks));
        let remove = button::button("\u{2212}", Kind::Ghost, Size::Small);
        remove.update_property(&[gtk::accessible::Property::Label("Remove address")]);
        let change = Rc::clone(&hooks.change);
        remove.connect_clicked(move |_| change(EditChange::RemoveAddress(index as u32)));
        line.append(&remove);
        column.append(&line);
    }
    let add = button::button(&view.add_address, Kind::Ghost, Size::Small);
    add.set_halign(gtk::Align::Start);
    let change = Rc::clone(&hooks.change);
    add.connect_clicked(move |_| change(EditChange::AddAddress));
    column.append(&add);
    let note = gtk::Label::new(Some(&view.address_note));
    note.add_css_class("dim-label");
    note.add_css_class("focus-contact-small");
    note.set_xalign(0.0);
    note.set_wrap(true);
    column.append(&note);
    column
}

/// The address's type: Work, Personal, Old or none.
fn label_menu(index: usize, current: &str, hooks: &FormHooks) -> gtk::MenuButton {
    let menu = gtk::MenuButton::new();
    menu.add_css_class("focus-contact-type");
    menu.set_label(if current.is_empty() { "Type" } else { current });
    menu.update_property(&[gtk::accessible::Property::Label("Address type")]);
    let popover = gtk::Popover::new();
    let options = gtk::Box::new(gtk::Orientation::Vertical, 0);
    for (label, title) in LABELS {
        let one = gtk::Button::with_label(title);
        one.add_css_class("flat");
        let change = Rc::clone(&hooks.change);
        let popover = popover.clone();
        one.connect_clicked(move |_| {
            popover.popdown();
            change(EditChange::Label {
                index: index as u32,
                label: Some(label),
            });
        });
        options.append(&one);
    }
    let other = gtk::Button::with_label("Other");
    other.add_css_class("flat");
    let change = Rc::clone(&hooks.change);
    let closing = popover.clone();
    other.connect_clicked(move |_| {
        closing.popdown();
        change(EditChange::Label {
            index: index as u32,
            label: None,
        });
    });
    options.append(&other);
    popover.set_child(Some(&options));
    menu.set_popover(Some(&popover));
    menu
}

fn seen(view: &EditView, hooks: &FormHooks) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, S2);
    for (index, line) in view.seen.iter().enumerate() {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, S3);
        row.add_css_class("focus-contact-seen");
        let what = gtk::Label::new(Some(&line.what));
        what.set_xalign(0.0);
        let from = gtk::Label::new(Some(&line.from));
        from.add_css_class("dim-label");
        from.add_css_class("focus-contact-small");
        from.set_hexpand(true);
        from.set_xalign(0.0);
        row.append(&what);
        row.append(&from);
        if let Some(secondary) = &line.secondary {
            let one = button::button(secondary, Kind::Ghost, Size::Small);
            let change = Rc::clone(&hooks.change);
            one.connect_clicked(move |_| {
                change(EditChange::Seen {
                    index: index as u32,
                    secondary: true,
                })
            });
            row.append(&one);
        }
        let main = button::button(&line.action, Kind::Ghost, Size::Small);
        let change = Rc::clone(&hooks.change);
        main.connect_clicked(move |_| {
            change(EditChange::Seen {
                index: index as u32,
                secondary: false,
            })
        });
        row.append(&main);
        column.append(&row);
    }
    column
}

fn groups(fields: &mut Vec<(Slot, gtk::Entry)>, view: &EditView, hooks: &FormHooks) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, S2);
    let chips = gtk::FlowBox::new();
    chips.set_selection_mode(gtk::SelectionMode::None);
    chips.set_column_spacing(S2 as u32 - 1);
    chips.set_row_spacing(S2 as u32 - 1);
    chips.set_homogeneous(false);
    for (index, name) in view.groups.iter().enumerate() {
        let chip = button::button(&format!("{name} \u{d7}"), Kind::Secondary, Size::Small);
        chip.update_property(&[gtk::accessible::Property::Label(&format!(
            "{name}, remove from group"
        ))]);
        let change = Rc::clone(&hooks.change);
        chip.connect_clicked(move |_| change(EditChange::RemoveGroup(index as u32)));
        chips.append(&chip);
    }
    if let Some(picker) = &view.group_picker {
        let typed = field(fields, Slot::Group, &picker.query, false, hooks, |text| {
            EditChange::GroupQuery(Some(text))
        });
        typed.set_placeholder_text(Some("Group name"));
        typed.set_hexpand(false);
        typed.set_width_chars(16);
        let change = Rc::clone(&hooks.change);
        let has_options = !picker.options.is_empty();
        typed.connect_activate(move |_| {
            if has_options {
                change(EditChange::GroupPick(0));
            }
        });
        chips.append(&typed);
    } else {
        let add = button::button(&view.add_group, Kind::Ghost, Size::Small);
        let change = Rc::clone(&hooks.change);
        add.connect_clicked(move |_| change(EditChange::GroupQuery(Some(String::new()))));
        chips.append(&add);
    }
    column.append(&chips);
    if let Some(picker) = &view.group_picker
        && !picker.options.is_empty()
    {
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.add_css_class("focus-contact-group-options");
        list.set_halign(gtk::Align::Start);
        for (index, option) in picker.options.iter().enumerate() {
            let one = gtk::Button::with_label(&option.label);
            one.add_css_class("flat");
            if option.is_new {
                one.add_css_class("focus-contact-group-new");
            }
            let change = Rc::clone(&hooks.change);
            one.connect_clicked(move |_| change(EditChange::GroupPick(index as u32)));
            list.append(&one);
        }
        column.append(&list);
    }
    let note = gtk::Label::new(Some(&view.group_note));
    note.add_css_class("dim-label");
    note.add_css_class("focus-contact-small");
    note.set_xalign(0.0);
    note.set_wrap(true);
    column.append(&note);
    column
}

/// Every text the form shows, in order: what a test reads.
pub fn texts(form: &Form) -> Vec<String> {
    super::view::texts_of(form.widget())
}
