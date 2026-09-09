//! The conversation rail: a column that says where you are in a thread.
//!
//! `Design/screens/28-conversation-rail-full-window.png` is the drawing and
//! `Design/conversation-rail-brief.md` §3 the specification. A 150px column on
//! the trailing edge of the conversation pane, a flex-none sibling of the
//! scroller rather than anything floating over it, with its own scroll for a
//! long thread. It carries a heading, one row per message, and a footer that
//! states the position.
//!
//! # What is not here
//!
//! Which row is marked. That rule is arithmetic over geometry and lives in
//! [`postio_ui::reader::rail`], where #1359 and #1372 prove it in
//! milliseconds — which matters, because this repository's test display lays
//! nothing out and a rule asserted against a rendered pane would be asserting
//! against zeroes. This module draws the answer; it does not compute it.
//!
//! # Why a `ListBox`
//!
//! FR-046 asks for a list with the current row marked, and a `ListBox` is one
//! to GTK's accessibility layer without being told. Arrow-key movement,
//! single selection and the announced role all come with it, so the parts of
//! the brief's accessibility section that would otherwise be hand-rolled —
//! and hand-rolled wrong — are the toolkit's.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use postio_ui::reader::rail::Row;

type ActivatedHandler = Box<dyn Fn(usize)>;

/// How wide the rail is when it is drawn in full.
///
/// From the brief's collapse ladder: 150px at 1240px and above. The reading
/// measure never gives up width to fund this (FR-043) — the rail is drawn
/// from the pane's surplus or not at all, which is what the ladder's third
/// step means by unmounting rather than shrinking the body.
pub const FULL_WIDTH: i32 = 150;

/// How wide the rail is when the window is too tight for a name.
///
/// The middle step of the ladder: numbers and initials only. A name shortened
/// to four characters is not a name, so the column drops it rather than
/// truncating it into noise.
pub const NARROW_WIDTH: i32 = 118;

/// The rail, drawn as a column.
pub struct RailColumn {
    root: gtk::Box,
    list: gtk::ListBox,
    position: gtk::Label,
    length: gtk::Label,
    rows: RefCell<Vec<Row>>,
    handlers: Rc<RefCell<Vec<ActivatedHandler>>>,
    hide: gtk::Button,
}

impl RailColumn {
    /// An empty rail. Nothing is drawn until
    /// [`set_thread`](Self::set_thread) says what the conversation is.
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("postio-rail");
        root.set_size_request(FULL_WIDTH, -1);
        root.set_hexpand(false);

        let heading = gtk::Label::new(Some("In this thread"));
        heading.add_css_class("postio-rail-heading");
        heading.set_xalign(0.0);
        root.append(&heading);

        let list = gtk::ListBox::new();
        list.add_css_class("postio-rail-rows");
        list.set_selection_mode(gtk::SelectionMode::Single);
        // The heading names the list for a screen reader, so the rail is not
        // announced as an unlabelled list of four things.
        list.update_property(&[gtk::accessible::Property::Label("In this thread")]);

        // Its own scroll, so a forty-message thread does not push the footer
        // off the bottom of the pane.
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroller.set_vexpand(true);
        scroller.set_child(Some(&list));
        root.append(&scroller);

        let footer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        footer.add_css_class("postio-rail-footer");

        let keys = gtk::Label::new(Some("J/K jumps"));
        keys.add_css_class("postio-rail-hint");
        keys.set_xalign(0.0);
        footer.append(&keys);

        let position = gtk::Label::new(None);
        position.add_css_class("postio-rail-hint");
        position.set_xalign(0.0);
        footer.append(&position);

        let length = gtk::Label::new(None);
        length.add_css_class("postio-rail-hint");
        length.set_xalign(0.0);
        footer.append(&length);

        let hide = gtk::Button::with_label("hide rail");
        hide.add_css_class("postio-rail-hide");
        hide.set_halign(gtk::Align::Start);
        hide.set_tooltip_text(Some("Hide the conversation rail"));
        footer.append(&hide);

        root.append(&footer);

        let handlers: Rc<RefCell<Vec<ActivatedHandler>>> = Rc::new(RefCell::new(Vec::new()));
        list.connect_row_activated({
            let handlers = Rc::clone(&handlers);
            move |_, row| {
                let index = row.index();
                if index < 0 {
                    return;
                }
                for handler in handlers.borrow().iter() {
                    handler(index as usize);
                }
            }
        });

        Self {
            root,
            list,
            position,
            length,
            rows: RefCell::new(Vec::new()),
            handlers,
            hide,
        }
    }

    /// The widget to put in a pane.
    pub fn widget(&self) -> &gtk::Widget {
        self.root.upcast_ref()
    }

    /// Draw a row per message.
    ///
    /// Takes rows already built by [`postio_ui::reader::rail::rows`], so the
    /// rail is built from the thread model and every message has a row the
    /// moment the conversation is known (FR-040) — not when its body arrives,
    /// which is the wait the rail exists to let you skip.
    pub fn set_thread(&self, rows: &[Row]) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        for row in rows {
            self.list.append(&self.draw(row, rows.len()));
        }
        self.rows.replace(rows.to_vec());
        // A thread arriving while the window is narrow must not undo the
        // step the ladder already took.
        self.set_narrow(self.root.has_css_class("postio-rail-narrow"));
        self.redraw_footer();
    }

    /// One row: number, sender, and a length where there is one worth saying.
    fn draw(&self, row: &Row, total: usize) -> gtk::ListBoxRow {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        line.add_css_class("postio-rail-row");

        let number = gtk::Label::new(Some(&row.position.to_string()));
        number.add_css_class("postio-rail-number");
        number.set_width_chars(2);
        number.set_xalign(1.0);
        line.append(&number);

        let sender = gtk::Label::new(Some(&row.sender));
        sender.add_css_class("postio-rail-sender");
        sender.set_ellipsize(gtk::pango::EllipsizeMode::End);
        sender.set_hexpand(true);
        sender.set_xalign(0.0);
        line.append(&sender);

        if let Some(lines) = row.length {
            let length = gtk::Label::new(Some(&lines.to_string()));
            length.add_css_class("postio-rail-length");
            length.set_xalign(1.0);
            line.append(&length);
        }

        let holder = gtk::ListBoxRow::new();
        holder.set_child(Some(&line));
        // The visible row is deliberately terse, so the accessible name
        // carries what the eye gets from position and typography. The brief:
        // *"Message 3 of 6, Tessa Vaughn, 84 lines"*.
        holder.update_property(&[gtk::accessible::Property::Label(&announce(row, total))]);
        holder
    }

    /// Take the middle step of the ladder, or come back off it.
    ///
    /// One component, two of its three presentations: the same rows, the same
    /// activation, a narrower box and no senders (FR-044). Rebuilding the rail
    /// as a different widget here is what the brief rules out, and it is also
    /// how the marked row would get lost on every resize.
    pub fn set_narrow(&self, narrow: bool) {
        let width = if narrow { NARROW_WIDTH } else { FULL_WIDTH };
        self.root.set_size_request(width, -1);
        if narrow {
            self.root.add_css_class("postio-rail-narrow");
        } else {
            self.root.remove_css_class("postio-rail-narrow");
        }
        for text in of_class(self.widget(), "postio-rail-sender") {
            text.set_visible(!narrow);
        }
    }

    /// Mark the row the rule chose.
    ///
    /// `None` unmarks: nothing visible is a real state while the pane settles,
    /// and the rail says so by marking nothing rather than by guessing.
    pub fn set_marked(&self, index: Option<usize>) {
        match index.and_then(|index| self.list.row_at_index(index as i32)) {
            Some(row) => self.list.select_row(Some(&row)),
            None => self.list.select_row(gtk::ListBoxRow::NONE),
        }
        self.redraw_footer();
    }

    /// Where the mark is, counted as a person counts.
    pub fn marked_position(&self) -> Option<usize> {
        self.list
            .selected_row()
            .map(|row| row.index() as usize + 1)
            .filter(|position| *position > 0)
    }

    /// Whether the row at `index` is drawn as marked.
    ///
    /// Asks the widget rather than a field: "there is a marked row" and "a row
    /// is drawn marked" are different claims, and only the second is what a
    /// person sees.
    pub fn row_is_marked(&self, index: usize) -> bool {
        self.list
            .row_at_index(index as i32)
            .is_some_and(|row| row.is_selected())
    }

    /// Activate the row at `index`, as a click would.
    pub fn activate_row(&self, index: usize) {
        if let Some(row) = self.list.row_at_index(index as i32) {
            row.emit_activate();
        }
    }

    /// Called when the footer's `hide rail` is pressed.
    ///
    /// Screen 28 draws this control, and it is how the rail can be put away
    /// at all: the brief's `⇧R` is not bound, because `R` already reaches
    /// `Refresh` on every message surface and taking it away inside the
    /// conversation is not this issue's call to make.
    pub fn connect_hide(&self, handler: impl Fn() + 'static) {
        self.hide.connect_clicked(move |_| handler());
    }

    /// Called when a row is activated, with the message it names.
    ///
    /// The index, not a scroll: what to do about it is
    /// [`postio_ui::reader::rail::Rail::activate`]'s decision, so that a rail
    /// click and `J` cannot take different routes to the same mark (#1372).
    pub fn connect_activated(&self, handler: impl Fn(usize) + 'static) {
        self.handlers.borrow_mut().push(Box::new(handler));
    }

    fn redraw_footer(&self) {
        let rows = self.rows.borrow();
        let total = rows.len();
        match self.marked_position() {
            Some(position) => {
                self.position
                    .set_text(&format!("{position} of {total} in view"));
                let lines = rows.get(position - 1).and_then(|row| row.length);
                match lines {
                    Some(lines) => self.length.set_text(&format!("{lines} lines here")),
                    None => self.length.set_text(""),
                }
            }
            None => {
                self.position.set_text(&format!("{total} messages"));
                self.length.set_text("");
            }
        }
    }
}

impl Default for RailColumn {
    fn default() -> Self {
        Self::new()
    }
}

/// Every label in the tree carrying `class`.
///
/// The narrow step hides senders, and a `ListBox` row's children are several
/// boxes down — so this walks rather than assuming a shape the row could grow
/// out of.
fn of_class(root: &gtk::Widget, class: &str) -> Vec<gtk::Label> {
    let mut found = Vec::new();
    let mut next = root.first_child();
    while let Some(child) = next {
        if let Some(label) = child.downcast_ref::<gtk::Label>()
            && label.has_css_class(class)
        {
            found.push(label.clone());
        }
        found.extend(of_class(&child, class));
        next = child.next_sibling();
    }
    found
}

/// What a screen reader says for one row.
fn announce(row: &Row, total: usize) -> String {
    let mut said = format!("Message {} of {}, {}", row.position, total, row.sender);
    if let Some(lines) = row.length {
        said.push_str(&format!(", {lines} lines"));
    }
    said
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_announces_more_than_it_draws() {
        // The visible row is a number, a name and sometimes a count. The
        // label has to carry the position too, because "3" beside a name
        // reads as a position on screen and as nothing at all aloud.
        let row = Row {
            position: 3,
            sender: "Tessa Vaughn".to_owned(),
            length: Some(84),
        };
        assert_eq!(announce(&row, 6), "Message 3 of 6, Tessa Vaughn, 84 lines");
    }

    #[test]
    fn a_row_with_no_length_says_nothing_about_length() {
        let row = Row {
            position: 1,
            sender: "Ada".to_owned(),
            length: None,
        };
        assert_eq!(announce(&row, 2), "Message 1 of 2, Ada");
    }
}
