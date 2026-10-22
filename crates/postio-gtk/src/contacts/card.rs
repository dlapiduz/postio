//! A contact's card (handoff screens 01 and 03): the avatar, the name and
//! the role line with the head's buttons; then two columns. Left, the
//! addresses, how mail arrives with its switches, and the notes. Right, the
//! tiles, the twelve-month strip, what is open between you (or a
//! newsletter's recent issues) and the recent files. Every word is
//! `postio_ui::contacts_view::CardView`'s: a person's or a newsletter's alike.

use std::rc::Rc;

use gtk::prelude::*;
use postio_core::CommandId;
use postio_model::ContactArrival;
use postio_ui::contacts_view::{CardView, MonthStrip};
use postio_widgets::widgets::button::{self, Kind, Size};
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::segmented::SegmentedControl;
use postio_widgets::widgets::space::{S1, S2, S3, S4};

/// What a click on the card does: the same input a key would have been.
#[derive(Clone)]
pub struct CardHooks {
    /// A button on the card: the command its key sends.
    pub run: Rc<dyn Fn(CommandId)>,
    /// A switch under "How mail arrives", by its index.
    pub toggle: Rc<dyn Fn(u32)>,
    /// "Split" on an address.
    pub split: Rc<dyn Fn(String)>,
    /// A segment of the arrival control.
    pub arrival: Rc<dyn Fn(ContactArrival)>,
}

/// The card, whole.
pub fn build(card: &CardView, hooks: &CardHooks) -> gtk::Widget {
    let column = gtk::Box::new(gtk::Orientation::Vertical, S4);
    column.add_css_class("focus-contact-card");
    column.append(&head(card, hooks));

    let columns = gtk::Box::new(gtk::Orientation::Horizontal, S4 * 2);
    let left = gtk::Box::new(gtk::Orientation::Vertical, S4);
    left.set_hexpand(true);
    left.set_valign(gtk::Align::Start);
    left.append(&section(&card.addresses_title, &addresses(card, hooks)));
    left.append(&section(&card.arrival_title, &arrival(card, hooks)));
    left.append(&section(&card.notes_title, &notes(card)));
    let right = gtk::Box::new(gtk::Orientation::Vertical, S4);
    right.set_valign(gtk::Align::Start);
    right.set_size_request(380, -1);
    right.append(&section(&card.stats_title, &stats(card)));
    if !card.lines.is_empty() {
        right.append(&section(&card.lines_title, &lines(card)));
    }
    if let Some(title) = &card.files_title
        && !card.files.is_empty()
    {
        right.append(&section(title, &files(card)));
    }
    columns.append(&left);
    columns.append(&right);
    column.append(&columns);
    column.upcast()
}

fn section(title: &str, body: &impl IsA<gtk::Widget>) -> gtk::Box {
    let section = gtk::Box::new(gtk::Orientation::Vertical, S2);
    let heading = gtk::Label::new(Some(title));
    heading.add_css_class("focus-contact-heading");
    heading.set_xalign(0.0);
    section.append(&heading);
    section.append(body);
    section
}

fn head(card: &CardView, hooks: &CardHooks) -> gtk::Box {
    let head = gtk::Box::new(gtk::Orientation::Horizontal, S4);
    head.append(&super::avatar::widget(&card.avatar, 64));
    let names = gtk::Box::new(gtk::Orientation::Vertical, S1);
    names.set_hexpand(true);
    names.set_valign(gtk::Align::Center);
    let name = gtk::Label::new(Some(&card.name));
    name.add_css_class("focus-contact-name");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let role = gtk::Label::new(Some(&card.role));
    role.add_css_class("focus-contact-role");
    role.add_css_class("dim-label");
    role.set_xalign(0.0);
    role.set_wrap(true);
    names.append(&name);
    names.append(&role);
    head.append(&names);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, S2);
    buttons.set_valign(gtk::Align::Center);
    for spec in &card.buttons {
        let one = button::button(
            &spec.label,
            if spec.primary {
                Kind::Primary
            } else {
                Kind::Secondary
            },
            Size::Regular,
        );
        keyhint::dress(&one, &spec.label, spec.key.as_deref());
        one.set_sensitive(spec.enabled);
        let run = Rc::clone(&hooks.run);
        let command = spec.command;
        one.connect_clicked(move |_| run(command));
        buttons.append(&one);
    }
    head.append(&buttons);
    head
}

fn addresses(card: &CardView, hooks: &CardHooks) -> gtk::Box {
    let list = gtk::Box::new(gtk::Orientation::Vertical, S3);
    for line in &card.addresses {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        row.add_css_class("focus-contact-address");
        let star = gtk::Label::new(Some(if line.primary { "\u{2605}" } else { "" }));
        star.set_width_chars(1);
        star.set_valign(gtk::Align::Start);
        star.set_accessible_role(gtk::AccessibleRole::Presentation);
        row.append(&star);
        let words = gtk::Box::new(gtk::Orientation::Vertical, 0);
        words.set_hexpand(true);
        let address = gtk::Label::new(Some(&line.address));
        address.add_css_class("focus-contact-mono");
        address.set_xalign(0.0);
        address.set_selectable(true);
        let note = gtk::Label::new(Some(&line.note));
        note.add_css_class("dim-label");
        note.add_css_class("focus-contact-small");
        note.set_xalign(0.0);
        words.append(&address);
        words.append(&note);
        row.append(&words);
        if let Some(bounced) = &line.bounced {
            let bounced = gtk::Label::new(Some(bounced));
            bounced.add_css_class("focus-contact-bounced");
            bounced.set_valign(gtk::Align::Start);
            row.append(&bounced);
        }
        if line.can_split {
            let split = button::button("Split", Kind::Ghost, Size::Small);
            split.set_valign(gtk::Align::Start);
            split.update_property(&[gtk::accessible::Property::Label(&format!(
                "Split {} into its own contact",
                line.address
            ))]);
            let hook = Rc::clone(&hooks.split);
            let address = line.address.clone();
            split.connect_clicked(move |_| hook(address.clone()));
            row.append(&split);
        }
        list.append(&row);
    }
    if let Some(seen) = &card.also_seen {
        let seen = gtk::Label::new(Some(seen));
        seen.add_css_class("dim-label");
        seen.set_xalign(0.0);
        seen.set_wrap(true);
        list.append(&seen);
    }
    list
}

fn arrival(card: &CardView, hooks: &CardHooks) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, S3);
    let labels: Vec<&str> = card
        .arrival
        .iter()
        .map(|option| option.label.as_str())
        .collect();
    let segmented = SegmentedControl::new(&card.arrival_title, &labels);
    if let Some(on) = card.arrival.iter().position(|option| option.on) {
        segmented.set_selected(on);
    }
    let options: Vec<ContactArrival> = card.arrival.iter().map(|option| option.arrival).collect();
    let hook = Rc::clone(&hooks.arrival);
    segmented.connect_selected(move |index| {
        if let Some(arrival) = options.get(index) {
            hook(*arrival);
        }
    });
    segmented.widget().set_halign(gtk::Align::Start);
    column.append(segmented.widget());
    for (index, line) in card.switches.iter().enumerate() {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, S3);
        row.add_css_class("focus-contact-switch");
        let words = gtk::Box::new(gtk::Orientation::Vertical, 0);
        words.set_hexpand(true);
        let title = gtk::Label::new(Some(&line.title));
        title.add_css_class("focus-contact-switch-title");
        title.set_xalign(0.0);
        let note = gtk::Label::new(Some(&line.note));
        note.add_css_class("dim-label");
        note.add_css_class("focus-contact-small");
        note.set_xalign(0.0);
        note.set_wrap(true);
        words.append(&title);
        words.append(&note);
        row.append(&words);
        let switch = gtk::Switch::new();
        switch.set_active(line.on);
        switch.set_valign(gtk::Align::Center);
        switch.set_sensitive(line.switch.is_some());
        switch.update_property(&[gtk::accessible::Property::Label(&line.title)]);
        let hook = Rc::clone(&hooks.toggle);
        let on = line.on;
        // A click asks; what is drawn is the controller's answer, so a
        // switch that did not turn goes back to what it said.
        switch.connect_state_set(move |switch, wanted| {
            if wanted != on {
                hook(index as u32);
            }
            switch.set_state(on);
            glib_stop()
        });
        row.append(&switch);
        column.append(&row);
    }
    column
}

fn glib_stop() -> gtk::glib::Propagation {
    gtk::glib::Propagation::Stop
}

fn notes(card: &CardView) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, S2);
    if let Some(text) = &card.notes {
        let note = gtk::Label::new(Some(text));
        note.add_css_class("focus-contact-note");
        note.set_xalign(0.0);
        note.set_wrap(true);
        note.set_selectable(true);
        column.append(&note);
    }
    let foot = gtk::Label::new(Some(&card.notes_foot));
    foot.add_css_class("dim-label");
    foot.add_css_class("focus-contact-small");
    foot.set_xalign(0.0);
    column.append(&foot);
    column
}

fn stats(card: &CardView) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, S2);
    let tiles = gtk::Box::new(gtk::Orientation::Horizontal, S2);
    tiles.set_homogeneous(true);
    for tile in &card.tiles {
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
        caption.set_wrap(true);
        one.append(&value);
        one.append(&caption);
        tiles.append(&one);
    }
    column.append(&tiles);
    column.append(&months(&card.months));
    column
}

/// The twelve-month strip and its axis.
fn months(strip: &MonthStrip) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, S1);
    let bars = strip.bars.clone();
    let area = gtk::DrawingArea::new();
    area.set_content_height(44);
    area.set_hexpand(true);
    area.add_css_class("focus-contact-months");
    area.update_property(&[gtk::accessible::Property::Label(&format!(
        "{}, {} to {}",
        strip.caption, strip.from, strip.to
    ))]);
    area.set_draw_func(move |area, cairo, width, height| {
        let count = bars.len().max(1) as f64;
        let gap = 3.0;
        let each = (f64::from(width) - gap * (count - 1.0)) / count;
        let colour = area.color();
        cairo.set_source_rgba(
            f64::from(colour.red()),
            f64::from(colour.green()),
            f64::from(colour.blue()),
            0.22,
        );
        for (at, bar) in bars.iter().enumerate() {
            let high = (f64::from(height) * f64::from(*bar)).max(3.0);
            let left = at as f64 * (each + gap);
            cairo.rectangle(left, f64::from(height) - high, each, high);
        }
        let _ = cairo.fill();
    });
    column.append(&area);
    let axis = gtk::CenterBox::new();
    for (place, words) in [&strip.from, &strip.caption, &strip.to]
        .into_iter()
        .enumerate()
    {
        let label = gtk::Label::new(Some(words));
        label.add_css_class("dim-label");
        label.add_css_class("focus-contact-small");
        match place {
            0 => axis.set_start_widget(Some(&label)),
            1 => axis.set_center_widget(Some(&label)),
            _ => axis.set_end_widget(Some(&label)),
        }
    }
    column.append(&axis);
    column
}

fn lines(card: &CardView) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, S2);
    for line in &card.lines {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, S3);
        row.add_css_class("focus-contact-line");
        let label = gtk::Label::new(Some(&line.label));
        label.add_css_class("focus-contact-line-label");
        if line.yours {
            label.add_css_class("focus-contact-line-yours");
        }
        label.set_xalign(0.0);
        label.set_width_chars(12);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let text = gtk::Label::new(Some(&line.text));
        text.set_xalign(0.0);
        text.set_hexpand(true);
        text.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let state = gtk::Label::new(Some(&line.state));
        state.add_css_class("dim-label");
        state.add_css_class("focus-contact-small");
        row.append(&label);
        row.append(&text);
        row.append(&state);
        column.append(&row);
    }
    column
}

fn files(card: &CardView) -> gtk::FlowBox {
    let flow = gtk::FlowBox::new();
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_column_spacing(S2 as u32);
    flow.set_row_spacing(S2 as u32);
    flow.set_homogeneous(false);
    for file in &card.files {
        let chip = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        chip.add_css_class("focus-contact-file");
        let kind = gtk::Label::new(Some(&file.kind));
        kind.add_css_class("focus-contact-file-kind");
        let name = gtk::Label::new(Some(&file.name));
        chip.append(&kind);
        chip.append(&name);
        flow.append(&chip);
    }
    flow
}
