//! The merge sheet over the Contacts view (handoff screen 07): a card per
//! record with the kept one ringed, "Why we think so", what the merge would
//! make, and Not the same, Later and Merge. Every word and every key is
//! `postio_ui::contact_merge::MergeSheetView`'s.

use std::rc::Rc;

use gtk::prelude::*;
use postio_core::CommandId;
use postio_ui::contact_edit::EditButton;
use postio_ui::contact_merge::{MergeCard, MergeSheetView};
use postio_widgets::widgets::button::{self, Kind, Size};
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::plate;
use postio_widgets::widgets::space::{S1, S2, S3};

/// What a click on the sheet does.
#[derive(Clone)]
pub struct MergeHooks {
    /// A card chosen to keep its name and photo.
    pub keep: Rc<dyn Fn(u32)>,
    /// A button: the command its key sends.
    pub command: Rc<dyn Fn(CommandId)>,
}

/// The sheet, over a scrim that fills what it covers.
pub fn build(sheet: &MergeSheetView, hooks: &MergeHooks) -> gtk::Widget {
    let scrim = gtk::Box::new(gtk::Orientation::Vertical, 0);
    scrim.add_css_class("focus-contacts-scrim");
    scrim.set_hexpand(true);
    scrim.set_vexpand(true);
    // The scrim takes a click: nothing under the sheet is reachable.
    scrim.append(&sheet_plate(sheet, hooks));
    scrim.upcast()
}

fn sheet_plate(sheet: &MergeSheetView, hooks: &MergeHooks) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    column.add_css_class("focus-merge");
    column.set_size_request(780, -1);
    column.set_halign(gtk::Align::Center);
    column.set_valign(gtk::Align::Start);
    plate::dress(&column, "focus-merge-plate", &sheet.title);
    column.set_halign(gtk::Align::Center);
    column.set_valign(gtk::Align::Start);

    let head = gtk::Box::new(gtk::Orientation::Vertical, S1);
    head.add_css_class("focus-merge-head");
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let title = gtk::Label::new(Some(&sheet.title));
    title.add_css_class("focus-merge-title");
    title.set_hexpand(true);
    title.set_xalign(0.0);
    top.append(&title);
    if let Some(position) = &sheet.position {
        let place = gtk::Label::new(Some(position));
        place.add_css_class("dim-label");
        top.append(&place);
    }
    let subtitle = gtk::Label::new(Some(&sheet.subtitle));
    subtitle.add_css_class("dim-label");
    subtitle.set_xalign(0.0);
    subtitle.set_wrap(true);
    head.append(&top);
    head.append(&subtitle);
    column.append(&head);

    let cards = gtk::Box::new(gtk::Orientation::Horizontal, S3);
    cards.add_css_class("focus-merge-cards");
    cards.set_homogeneous(true);
    for (index, card) in sheet.cards.iter().enumerate() {
        cards.append(&card_button(index, card, hooks));
    }
    column.append(&cards);

    let why = gtk::Box::new(gtk::Orientation::Vertical, S2);
    why.add_css_class("focus-merge-why");
    let heading = gtk::Label::new(Some(&sheet.why_title));
    heading.add_css_class("focus-contact-heading");
    heading.set_xalign(0.0);
    why.append(&heading);
    for line in &sheet.evidence {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        let tick = gtk::Label::new(Some("\u{2713}"));
        tick.add_css_class("dim-label");
        tick.set_valign(gtk::Align::Start);
        let said = gtk::Label::new(Some(line));
        said.set_xalign(0.0);
        said.set_wrap(true);
        said.set_hexpand(true);
        row.append(&tick);
        row.append(&said);
        why.append(&row);
    }
    column.append(&why);

    let result = gtk::Box::new(gtk::Orientation::Horizontal, S3);
    result.add_css_class("focus-merge-result");
    result.append(&super::avatar::widget(&sheet.result.avatar, 40));
    let words = gtk::Box::new(gtk::Orientation::Vertical, 2);
    words.set_hexpand(true);
    let name = gtk::Label::new(None);
    name.set_xalign(0.0);
    let role = if sheet.result.role.is_empty() {
        String::new()
    } else {
        format!("  \u{b7} {}", sheet.result.role)
    };
    name.set_markup(&format!(
        "<b>{}</b><span alpha=\"60%\">{}</span>",
        gtk::glib::markup_escape_text(&sheet.result.name),
        gtk::glib::markup_escape_text(&role)
    ));
    let addresses = gtk::Label::new(Some(&sheet.result.addresses));
    addresses.add_css_class("focus-contact-mono");
    addresses.add_css_class("dim-label");
    addresses.set_xalign(0.0);
    addresses.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let line = gtk::Label::new(Some(&sheet.result.line));
    line.add_css_class("dim-label");
    line.add_css_class("focus-contact-small");
    line.set_xalign(0.0);
    words.append(&name);
    words.append(&addresses);
    words.append(&line);
    result.append(&words);
    column.append(&result);

    let foot = gtk::Box::new(gtk::Orientation::Horizontal, S2);
    foot.add_css_class("focus-merge-foot");
    let said = gtk::Label::new(Some(&sheet.foot));
    said.add_css_class("dim-label");
    said.set_hexpand(true);
    said.set_xalign(0.0);
    said.set_wrap(true);
    foot.append(&said);
    for (spec, command, primary) in [
        (&sheet.not_same, CommandId::SuggestionDismiss, false),
        (&sheet.later, CommandId::Back, false),
        (&sheet.merge, CommandId::ContactShowMail, true),
    ] {
        foot.append(&foot_button(spec, command, primary, hooks));
    }
    column.append(&foot);
    column
}

fn card_button(index: usize, card: &MergeCard, hooks: &MergeHooks) -> gtk::Button {
    let one = gtk::Button::new();
    one.add_css_class("focus-merge-card");
    if card.kept {
        one.add_css_class("focus-merge-card-kept");
    }
    let column = gtk::Box::new(gtk::Orientation::Vertical, S1 + 2);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, S2);
    head.append(&super::avatar::widget(&card.avatar, 30));
    let names = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let name = gtk::Label::new(Some(&card.name));
    name.add_css_class("focus-contacts-name");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let source = gtk::Label::new(Some(&card.source));
    source.add_css_class("dim-label");
    source.add_css_class("focus-contact-small");
    source.set_xalign(0.0);
    names.append(&name);
    names.append(&source);
    head.append(&names);
    let address = gtk::Label::new(Some(&card.address));
    address.add_css_class("focus-contact-mono");
    address.set_xalign(0.0);
    address.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let stats = gtk::Label::new(Some(&card.stats));
    stats.add_css_class("dim-label");
    stats.add_css_class("focus-contact-small");
    stats.set_xalign(0.0);
    let keep = gtk::Label::new(Some(&card.keep));
    keep.add_css_class("focus-merge-keep");
    keep.set_xalign(0.0);
    column.append(&head);
    column.append(&address);
    column.append(&stats);
    column.append(&keep);
    one.set_child(Some(&column));
    one.update_property(&[gtk::accessible::Property::Label(&format!(
        "{}, {}",
        card.name, card.keep
    ))]);
    let keep_hook = Rc::clone(&hooks.keep);
    one.connect_clicked(move |_| keep_hook(index as u32));
    one
}

fn foot_button(
    spec: &EditButton,
    command: CommandId,
    primary: bool,
    hooks: &MergeHooks,
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

/// Every text the sheet shows, in order: what a test reads.
pub fn texts(sheet: &gtk::Widget) -> Vec<String> {
    super::view::texts_of(sheet)
}
