//! The contact popover (handoff screens 04 and 05): what a person is to you,
//! at the sender's name, without leaving the list. A known sender has three
//! tiles, what is open between you, where their mail arrives and the
//! actions; a sender not yet saved has how they reached you and what their
//! signature said. Every word is `postio_ui::contact_popover::PopoverView`'s.
//!
//! It is a card in the window's own layer, not a separate surface: the
//! window's keys reach it, so the controller's commands (the arrival
//! arrows, Return for their mail) work as they do everywhere else.

use std::rc::Rc;

use gtk::prelude::*;
use postio_core::CommandId;
use postio_model::ContactArrival;
use postio_ui::contact_popover::{IntroView, PopoverView};
use postio_widgets::widgets::button::{self, Kind, Size};
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::plate;
use postio_widgets::widgets::segmented::SegmentedControl;
use postio_widgets::widgets::space::{S1, S2, S3};

/// What a click on the popover does.
#[derive(Clone)]
pub struct PopoverHooks {
    /// An action: the command its key sends.
    pub command: Rc<dyn Fn(CommandId)>,
    /// A segment of the arrival control.
    pub arrival: Rc<dyn Fn(ContactArrival)>,
}

/// The card for `view`.
pub fn build(view: &PopoverView, hooks: &PopoverHooks) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.add_css_class("focus-contact-popover");
    card.set_size_request(400, -1);
    plate::dress(&card, "focus-contact-popover-plate", "Contact");
    card.set_halign(gtk::Align::Start);
    card.set_valign(gtk::Align::Start);

    let head = gtk::Box::new(gtk::Orientation::Horizontal, S3 + 2);
    head.add_css_class("focus-contact-popover-head");
    head.append(&super::avatar::widget(&view.avatar, 44));
    let names = gtk::Box::new(gtk::Orientation::Vertical, S1);
    names.set_valign(gtk::Align::Center);
    let name = gtk::Label::new(Some(&view.name));
    name.add_css_class("focus-contact-popover-name");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let subline = gtk::Label::new(Some(&view.subline));
    subline.add_css_class("focus-contact-mono");
    subline.add_css_class("dim-label");
    subline.set_xalign(0.0);
    subline.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let role = gtk::Label::new(Some(
        view.intro
            .as_ref()
            .map_or(view.role.as_str(), |intro| intro.sent.as_str()),
    ));
    role.add_css_class("dim-label");
    role.set_xalign(0.0);
    role.set_ellipsize(gtk::pango::EllipsizeMode::End);
    names.append(&name);
    names.append(&subline);
    names.append(&role);
    head.append(&names);
    card.append(&head);

    if let Some(intro) = &view.intro {
        card.append(&introduction(intro));
    } else {
        card.append(&tiles(view));
        if !view.open.is_empty() {
            card.append(&open_lines(view));
        }
    }
    card.append(&arrival(view, hooks));
    card.append(&actions(view, hooks));
    card
}

fn introduction(intro: &IntroView) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, S3);
    column.add_css_class("focus-contact-popover-body");
    let status = gtk::Label::new(Some(&intro.status));
    status.set_xalign(0.0);
    status.set_wrap(true);
    column.append(&status);
    if let Some(title) = &intro.signature_title {
        let signature = gtk::Box::new(gtk::Orientation::Vertical, S1);
        signature.add_css_class("focus-contact-note");
        let heading = gtk::Label::new(Some(title));
        heading.add_css_class("focus-contact-heading");
        heading.set_xalign(0.0);
        signature.append(&heading);
        for (index, line) in intro.signature_lines.iter().enumerate() {
            let said = gtk::Label::new(Some(line));
            said.set_xalign(0.0);
            said.set_wrap(true);
            if index > 0 {
                said.add_css_class("focus-contact-mono");
            }
            signature.append(&said);
        }
        if let Some(foot) = &intro.signature_foot {
            let foot = gtk::Label::new(Some(foot));
            foot.add_css_class("dim-label");
            foot.add_css_class("focus-contact-small");
            foot.set_xalign(0.0);
            foot.set_wrap(true);
            signature.append(&foot);
        }
        column.append(&signature);
    }
    column
}

fn tiles(view: &PopoverView) -> gtk::Box {
    let tiles = gtk::Box::new(gtk::Orientation::Horizontal, S2);
    tiles.add_css_class("focus-contact-popover-body");
    tiles.set_homogeneous(true);
    for tile in &view.tiles {
        let one = gtk::Box::new(gtk::Orientation::Vertical, 0);
        one.add_css_class("focus-contact-tile");
        let value = gtk::Label::new(Some(&tile.value));
        value.add_css_class("focus-contact-tile-value");
        value.set_xalign(0.0);
        value.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let caption = gtk::Label::new(Some(&tile.caption));
        caption.add_css_class("dim-label");
        caption.add_css_class("focus-contact-small");
        caption.set_xalign(0.0);
        one.append(&value);
        one.append(&caption);
        tiles.append(&one);
    }
    tiles
}

fn open_lines(view: &PopoverView) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, S1);
    column.add_css_class("focus-contact-popover-body");
    for line in &view.open {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        let label = gtk::Label::new(Some(&line.label));
        label.add_css_class("focus-contact-line-label");
        if line.yours {
            label.add_css_class("focus-contact-line-yours");
        }
        label.set_xalign(0.0);
        label.set_width_chars(14);
        let text = gtk::Label::new(Some(&line.text));
        text.set_xalign(0.0);
        text.set_hexpand(true);
        text.set_ellipsize(gtk::pango::EllipsizeMode::End);
        row.append(&label);
        row.append(&text);
        column.append(&row);
    }
    column
}

fn arrival(view: &PopoverView, hooks: &PopoverHooks) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, S2);
    column.add_css_class("focus-contact-popover-arrival");
    let top = gtk::Box::new(gtk::Orientation::Horizontal, S3);
    let title = gtk::Label::new(Some(&view.arrival_title));
    title.add_css_class("focus-contact-heading");
    title.set_hexpand(true);
    title.set_xalign(0.0);
    top.append(&title);
    for hint in &view.hints {
        top.append(&keyhint::chip(hint, "focus-contact-popover-hint"));
    }
    column.append(&top);
    let labels: Vec<&str> = view
        .arrival
        .iter()
        .map(|option| option.label.as_str())
        .collect();
    let control = SegmentedControl::new(&view.arrival_title, &labels);
    if let Some(on) = view.arrival.iter().position(|option| option.on) {
        control.set_selected(on);
    }
    let options: Vec<ContactArrival> = view.arrival.iter().map(|option| option.arrival).collect();
    let hook = Rc::clone(&hooks.arrival);
    control.connect_selected(move |index| {
        if let Some(arrival) = options.get(index) {
            hook(*arrival);
        }
    });
    control.widget().set_halign(gtk::Align::Start);
    column.append(control.widget());
    column
}

fn actions(view: &PopoverView, hooks: &PopoverHooks) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, S2);
    row.add_css_class("focus-contact-popover-actions");
    for action in &view.actions {
        let one = button::button(
            &action.label,
            if action.primary {
                Kind::Primary
            } else {
                Kind::Secondary
            },
            Size::Regular,
        );
        keyhint::dress(&one, &action.label, action.key.as_deref());
        one.set_sensitive(action.enabled);
        let run = Rc::clone(&hooks.command);
        let command = action.command;
        one.connect_clicked(move |_| run(command));
        row.append(&one);
    }

    row
}
