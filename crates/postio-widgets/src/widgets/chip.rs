//! The text chip, built one way.
//!
//! A chip is a pill of mono text: a search refinement, the did-you-mean
//! offer, a finder filter token, the settings tag. The two chip-sizing bugs
//! this crate has already paid for — an offer wider than its siblings
//! because the count rode on its face, an offer stretched to the column
//! because nothing set its `halign` — were both call sites owning geometry
//! that belongs to the control. These constructors are that ownership moving
//! here: the face is the text alone, the box comes from `.postio-chip-base`
//! and the `--postio-chip-*` tokens, and hugging is set where the widget is
//! made, so no parent `Box` can stretch one again.

use gtk::prelude::*;

/// A clickable chip: the term on its face, the full sentence in its tooltip
/// and accessible name.
///
/// The face never carries counts or consequences — the shortlist column is
/// 212px wide and a scannable row of terms beats a wide one. What taking the
/// chip does belongs in `spoken`, where the tooltip and a screen reader
/// deliver it.
pub fn chip_button(face: &str, spoken: &str) -> gtk::Button {
    let button = gtk::Button::with_label(face);
    button.add_css_class("postio-chip-base");
    button.add_css_class("postio-refine-chip");
    button.set_tooltip_text(Some(spoken));
    button.update_property(&[gtk::accessible::Property::Label(spoken)]);
    // Hug the text wherever it lands. A `FlowBox` sizes a child to its
    // content anyway; a plain `Box` stretches one to the full column, which
    // is how the offer became a slab beside pills.
    button.set_halign(gtk::Align::Start);
    button.set_valign(gtk::Align::Center);
    button
}

/// A finder filter chip: `negated` for an excluded token, `partial` for one
/// still being typed.
///
/// A label rather than a button — the finder's chips are a readout of the
/// query, and the keyboard edits the query itself.
pub fn filter_chip(face: &str, spoken: &str, negated: bool, partial: bool) -> gtk::Label {
    let label = gtk::Label::new(Some(face));
    label.add_css_class("postio-chip-base");
    label.add_css_class("postio-chip");
    if negated {
        label.add_css_class("negated");
    }
    if partial {
        label.add_css_class("partial");
    }
    label.update_property(&[gtk::accessible::Property::Label(spoken)]);
    label.set_halign(gtk::Align::Start);
    label.set_valign(gtk::Align::Center);
    label
}
