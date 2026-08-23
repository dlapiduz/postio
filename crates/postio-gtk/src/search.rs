//! The `/` query bar: chips over an editable query.
//!
//! Canvas 2b: `/` opens a bar over the list, parsed operators become chips, and
//! Backspace pops a chip whole rather than nibbling a character off it. Search
//! is primary navigation, not a dialog — so there is no animation and nothing
//! to dismiss before typing.
//!
//! # Where the chips live
//!
//! The entry holds the *whole* query, and the chips are a parse of it drawn
//! alongside. They are a reading of what is typed, not a second store that could
//! disagree with it — which is why [`postio_search::ParsedQuery`] hands out
//! [`Span`](postio_search::query::Span)s into the input, and why
//! [`ParsedQuery::remove_token`] returns *the string to put back in the entry*.
//!
//! The alternative — lifting completed operators out of the entry into
//! standalone chips — is a nicer picture and a worse editor: the caret can no
//! longer move through the query, and every edit becomes a merge between two
//! representations. This way the entry is the truth and the chips follow it.
//!
//! # Two halves
//!
//! [`chips`] and [`backspace`] are pure functions over a parsed query, tested
//! with no display. [`SearchBar`] is the widget around them.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use postio_search::ParsedQuery;
use postio_search::query::{Field, TokenKind};

/// One chip: an operator the parser recognized in the query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip {
    /// Position of the token in [`ParsedQuery::tokens`], for popping it.
    pub index: usize,
    /// The exact source text, so what the chip says is what is in the entry.
    pub label: String,
    /// The operator it belongs to.
    pub field: Field,
    /// Whether it was negated with a leading `-`.
    pub negated: bool,
    /// Whether the operator has a value yet. A half-typed `from:` is still
    /// worth drawing — it tells the user the parser understood the keyword.
    pub complete: bool,
}

/// The chips to draw for a query, in the order they were typed.
///
/// Free text is not a chip: it stays plain, because it is the part the user is
/// usually still editing.
pub fn chips(parsed: &ParsedQuery) -> Vec<Chip> {
    parsed
        .tokens()
        .iter()
        .enumerate()
        .filter_map(|(index, token)| {
            let field = token.field()?;
            Some(Chip {
                index,
                label: token.raw.clone(),
                field,
                negated: token.negated(),
                complete: matches!(token.kind, TokenKind::Filter(_)),
            })
        })
        .collect()
}

/// What to call when the query changes or is submitted.
type QueryHandler = Box<dyn Fn(&ParsedQuery)>;

/// What Backspace should do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Backspace {
    /// Delete one character, as usual.
    Ordinary,
    /// Take the whole chip out.
    PopChip {
        /// The token that went.
        index: usize,
        /// What the entry should now hold.
        query: String,
        /// Where the caret should now sit, in bytes.
        caret: usize,
    },
}

/// Decides what Backspace does with the caret at `caret` bytes into the query.
///
/// A chip pops when the caret is inside it or against its right edge — which is
/// where the caret is after typing one. Against its *left* edge the caret is
/// before the chip, not in it, so Backspace deletes what precedes as usual;
/// otherwise there would be no way to remove the space in front of a chip.
///
/// Free text is never popped whole. `subject:report` is one idea and deleting
/// it in one keystroke is a convenience; a word the user typed is a word, and
/// swallowing it would be a surprise.
pub fn backspace(parsed: &ParsedQuery, caret: usize) -> Backspace {
    let Some((index, token)) = parsed
        .tokens()
        .iter()
        .enumerate()
        .find(|(_, token)| token.span.contains(caret))
    else {
        return Backspace::Ordinary;
    };

    if !token.is_operator() || caret <= token.span.start {
        return Backspace::Ordinary;
    }

    // Where the join lands after `remove_token` trims the whitespace around the
    // hole it leaves.
    let caret = parsed.input()[..token.span.start].trim_end().len();
    Backspace::PopChip {
        index,
        query: parsed.remove_token(index),
        caret,
    }
}

// ---------------------------------------------------------------------------
// The widget
// ---------------------------------------------------------------------------

mod imp {
    use std::cell::RefCell;

    use super::*;

    pub struct SearchBar {
        pub entry: gtk::Entry,
        pub chips: gtk::Box,
        pub changed: RefCell<Vec<QueryHandler>>,
        pub activated: RefCell<Vec<QueryHandler>>,
        pub dismissed: RefCell<Vec<Box<dyn Fn()>>>,
        pub parsed: RefCell<ParsedQuery>,
    }

    impl Default for SearchBar {
        fn default() -> Self {
            Self {
                entry: gtk::Entry::new(),
                chips: gtk::Box::new(gtk::Orientation::Horizontal, 6),
                changed: RefCell::new(Vec::new()),
                activated: RefCell::new(Vec::new()),
                dismissed: RefCell::new(Vec::new()),
                parsed: RefCell::new(ParsedQuery::default()),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SearchBar {
        const NAME: &'static str = "PostioSearchBar";
        type Type = super::SearchBar;
        type ParentType = adw::Bin;
    }

    impl ObjectImpl for SearchBar {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }
    }

    impl WidgetImpl for SearchBar {}
    impl BinImpl for SearchBar {}
}

glib::wrapper! {
    /// The `/` query bar.
    pub struct SearchBar(ObjectSubclass<imp::SearchBar>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Editable;
}

impl Default for SearchBar {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl SearchBar {
    /// An empty bar.
    pub fn new() -> Self {
        Self::default()
    }

    /// The query as typed.
    pub fn query(&self) -> String {
        self.imp().entry.text().to_string()
    }

    /// Replaces the query, as though the user had typed it.
    pub fn set_query(&self, query: &str) {
        self.imp().entry.set_text(query);
        self.imp().entry.set_position(-1);
    }

    /// The query, parsed.
    pub fn parsed(&self) -> ParsedQuery {
        self.imp().parsed.borrow().clone()
    }

    /// The chips currently drawn.
    pub fn chips(&self) -> Vec<Chip> {
        chips(&self.imp().parsed.borrow())
    }

    /// The caret, in bytes into the query.
    ///
    /// GTK counts the cursor in characters; the parser's spans are in bytes,
    /// and the two differ the moment somebody searches for a name with an
    /// accent in it.
    pub fn caret(&self) -> usize {
        let text = self.imp().entry.text();
        let chars = self.imp().entry.position();
        if chars < 0 {
            return text.len();
        }
        text.char_indices()
            .nth(chars as usize)
            .map(|(offset, _)| offset)
            .unwrap_or(text.len())
    }

    /// Clears the query and takes the keyboard. What `/` does.
    ///
    /// The query is not remembered between openings: a bar that reopens showing
    /// the last search is one the user has to clear before they can use it.
    pub fn focus_entry(&self) {
        self.imp().entry.set_text("");
        self.grab_entry_focus();
        self.refresh();
    }

    /// Puts the keyboard in the entry without disturbing what is in it.
    pub fn grab_entry_focus(&self) {
        self.imp().entry.grab_focus();
    }

    /// Called on every keystroke, with the query parsed.
    pub fn connect_changed(&self, handler: impl Fn(&ParsedQuery) + 'static) {
        self.imp().changed.borrow_mut().push(Box::new(handler));
    }

    /// Called when the user presses Enter.
    pub fn connect_activated(&self, handler: impl Fn(&ParsedQuery) + 'static) {
        self.imp().activated.borrow_mut().push(Box::new(handler));
    }

    /// Called when the user presses `Escape`.
    pub fn connect_dismissed(&self, handler: impl Fn() + 'static) {
        self.imp().dismissed.borrow_mut().push(Box::new(handler));
    }

    /// Applies the Backspace rule, and says whether it popped a chip.
    ///
    /// Public so the behaviour can be driven in a test without synthesizing a
    /// key event, which GTK4 gives no supported way to do.
    pub fn press_backspace(&self) -> bool {
        let decision = backspace(&self.imp().parsed.borrow(), self.caret());
        match decision {
            Backspace::Ordinary => false,
            Backspace::PopChip { query, caret, .. } => {
                let entry = &self.imp().entry;
                entry.set_text(&query);
                let chars = query[..caret.min(query.len())].chars().count();
                entry.set_position(chars as i32);
                true
            }
        }
    }

    fn dismiss(&self) {
        for handler in self.imp().dismissed.borrow().iter() {
            handler();
        }
    }

    fn build(&self) {
        let imp = self.imp();
        self.add_css_class("postio-searchbar");

        imp.entry.add_css_class("postio-search");
        imp.entry.set_placeholder_text(Some("Search mail…"));
        imp.entry.set_hexpand(true);
        imp.entry
            .update_property(&[gtk::accessible::Property::Label("Search query")]);

        imp.chips.add_css_class("postio-chips");

        let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
        column.append(&imp.entry);
        column.append(&imp.chips);
        self.set_child(Some(&column));

        imp.entry.connect_changed(glib::clone!(
            #[weak(rename_to = bar)]
            self,
            move |_| bar.refresh()
        ));

        imp.entry.connect_activate(glib::clone!(
            #[weak(rename_to = bar)]
            self,
            move |_| {
                let parsed = bar.imp().parsed.borrow().clone();
                for handler in bar.imp().activated.borrow().iter() {
                    handler(&parsed);
                }
            }
        ));

        // Capture, so Backspace is decided before the entry deletes a character.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = bar)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, _| match key {
                gtk::gdk::Key::BackSpace if bar.press_backspace() => glib::Propagation::Stop,
                gtk::gdk::Key::Escape => {
                    bar.dismiss();
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        ));
        imp.entry.add_controller(keys);

        self.refresh();
    }

    /// Reparses and redraws.
    ///
    /// The whole strip, every keystroke. A query is a line of text and the
    /// parser is a single pass over it, so there is nothing to gain from an
    /// incremental redraw and a stale chip to lose by it.
    fn refresh(&self) {
        let imp = self.imp();
        let parsed = postio_search::parse(&imp.entry.text(), today());
        let drawn = chips(&parsed);

        while let Some(child) = imp.chips.first_child() {
            imp.chips.remove(&child);
        }
        for chip in &drawn {
            imp.chips.append(&chip_widget(chip));
        }
        imp.chips.set_visible(!drawn.is_empty());

        *imp.parsed.borrow_mut() = parsed;
        let parsed = imp.parsed.borrow().clone();
        for handler in imp.changed.borrow().iter() {
            handler(&parsed);
        }
    }
}

/// The day relative dates resolve against.
///
/// The *local* day, not UTC: `after:yesterday` means the user's yesterday.
/// Read per keystroke rather than cached, so a session left open across
/// midnight does not go on searching against the wrong day.
fn today() -> chrono::NaiveDate {
    chrono::Local::now().date_naive()
}

fn chip_widget(chip: &Chip) -> gtk::Label {
    let label = gtk::Label::new(Some(&chip.label));
    label.add_css_class("postio-chip");
    if chip.negated {
        label.add_css_class("negated");
    }
    if !chip.complete {
        label.add_css_class("partial");
    }
    // Read as what it does, not as the shorthand it is written in.
    label.update_property(&[gtk::accessible::Property::Label(&spoken(chip))]);
    label
}

/// How a chip reads to a screen reader.
pub fn spoken(chip: &Chip) -> String {
    let field = chip.field.keyword();
    let value = chip
        .label
        .split_once(':')
        .map(|(_, value)| value)
        .unwrap_or_default();
    match (chip.negated, chip.complete) {
        (false, true) => format!("{field} {value}"),
        (true, true) => format!("not {field} {value}"),
        (false, false) => format!("{field}, no value yet"),
        (true, false) => format!("not {field}, no value yet"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed day, so a test that mentions a relative date is not a test of
    /// what day it is.
    fn parse(query: &str) -> ParsedQuery {
        postio_search::parse(
            query,
            chrono::NaiveDate::from_ymd_opt(2026, 3, 1).expect("a real date"),
        )
    }

    // -- chips ------------------------------------------------------------

    #[test]
    fn an_empty_query_has_no_chips() {
        assert!(chips(&parse("")).is_empty());
    }

    #[test]
    fn free_text_is_not_a_chip() {
        assert!(
            chips(&parse("quarterly report")).is_empty(),
            "the part the user is still editing stays plain"
        );
    }

    #[test]
    fn an_operator_becomes_a_chip_labelled_as_typed() {
        let drawn = chips(&parse("from:ada@example.com report"));

        assert_eq!(drawn.len(), 1);
        assert_eq!(drawn[0].label, "from:ada@example.com");
        assert_eq!(drawn[0].field, Field::From);
        assert!(drawn[0].complete);
        assert!(!drawn[0].negated);
    }

    #[test]
    fn a_half_typed_operator_is_still_drawn() {
        let drawn = chips(&parse("from:"));

        assert_eq!(drawn.len(), 1);
        assert!(
            !drawn[0].complete,
            "it tells the user the keyword was understood"
        );
    }

    #[test]
    fn a_negated_operator_says_so() {
        let drawn = chips(&parse("-is:unread"));

        assert_eq!(drawn.len(), 1);
        assert!(drawn[0].negated);
        assert_eq!(drawn[0].label, "-is:unread");
    }

    #[test]
    fn several_operators_keep_the_order_they_were_typed_in() {
        let drawn = chips(&parse("from:ada@example.com is:flagged has:attach"));
        let fields: Vec<Field> = drawn.iter().map(|chip| chip.field).collect();

        assert_eq!(fields, vec![Field::From, Field::Is, Field::Has]);
    }

    #[test]
    fn a_word_that_merely_contains_a_colon_is_not_an_operator() {
        assert!(
            chips(&parse("note:this")).is_empty(),
            "`note` is not an operator this build knows"
        );
    }

    // -- backspace --------------------------------------------------------

    #[test]
    fn backspace_in_free_text_is_ordinary() {
        let parsed = parse("report");

        assert_eq!(backspace(&parsed, 6), Backspace::Ordinary);
    }

    #[test]
    fn backspace_at_the_right_edge_of_a_chip_pops_it_whole() {
        let query = "from:ada@example.com report";
        let parsed = parse(query);
        let end = "from:ada@example.com".len();

        assert_eq!(
            backspace(&parsed, end),
            Backspace::PopChip {
                index: 0,
                query: "report".to_owned(),
                caret: 0
            }
        );
    }

    #[test]
    fn backspace_inside_a_chip_pops_it_whole_too() {
        let parsed = parse("is:flagged report");

        let Backspace::PopChip { query, .. } = backspace(&parsed, 4) else {
            panic!("expected a pop");
        };
        assert_eq!(query, "report", "not `is:lagged`");
    }

    #[test]
    fn backspace_at_the_left_edge_of_a_chip_is_ordinary() {
        let parsed = parse("report is:flagged");
        let start = "report ".len();

        assert_eq!(
            backspace(&parsed, start),
            Backspace::Ordinary,
            "the caret is before the chip, so there is a space to delete"
        );
    }

    #[test]
    fn popping_a_chip_from_the_middle_tidies_the_gap() {
        let query = "report is:flagged more";
        let parsed = parse(query);
        let end = "report is:flagged".len();

        assert_eq!(
            backspace(&parsed, end),
            Backspace::PopChip {
                index: 1,
                query: "report more".to_owned(),
                caret: "report".len()
            },
            "one space between the halves, and the caret where the chip was"
        );
    }

    #[test]
    fn popping_the_last_chip_leaves_what_came_before() {
        let query = "report is:flagged";
        let parsed = parse(query);

        assert_eq!(
            backspace(&parsed, query.len()),
            Backspace::PopChip {
                index: 1,
                query: "report".to_owned(),
                caret: "report".len()
            }
        );
    }

    #[test]
    fn free_text_is_never_popped_whole() {
        let parsed = parse("quarterly report");

        assert_eq!(
            backspace(&parsed, "quarterly".len()),
            Backspace::Ordinary,
            "a word the user typed is a word"
        );
    }

    #[test]
    fn a_half_typed_chip_pops_whole_as_well() {
        let parsed = parse("report from:");

        assert!(matches!(
            backspace(&parsed, "report from:".len()),
            Backspace::PopChip { .. }
        ));
    }

    #[test]
    fn backspace_past_the_end_of_the_query_is_ordinary() {
        let parsed = parse("from:ada@example.com");

        assert_eq!(backspace(&parsed, 999), Backspace::Ordinary);
    }

    #[test]
    fn chips_can_be_popped_one_after_another() {
        let mut query = "from:ada@example.com is:flagged report".to_owned();

        // The caret rests at the right edge of the last chip each time, which
        // is where it is just after typing one.
        loop {
            let parsed = parse(&query);
            let Some(last) = chips(&parsed).last().cloned() else {
                break;
            };
            let caret = parsed.tokens()[last.index].span.end;
            match backspace(&parsed, caret) {
                Backspace::PopChip { query: next, .. } => query = next,
                Backspace::Ordinary => panic!("a chip at its right edge must pop"),
            }
        }

        assert_eq!(query, "report", "the free text is what survives");
    }

    #[test]
    fn backspace_at_the_end_of_trailing_free_text_is_ordinary() {
        let query = "from:ada@example.com report";
        let parsed = parse(query);

        assert_eq!(
            backspace(&parsed, query.len()),
            Backspace::Ordinary,
            "the caret is in `report`, not in the chip before it"
        );
    }

    // -- spoken -----------------------------------------------------------

    #[test]
    fn a_chip_reads_as_what_it_does() {
        let drawn = chips(&parse("from:ada@example.com"));
        assert_eq!(spoken(&drawn[0]), "from ada@example.com");

        let drawn = chips(&parse("-is:unread"));
        assert_eq!(spoken(&drawn[0]), "not is unread");

        let drawn = chips(&parse("subject:"));
        assert_eq!(spoken(&drawn[0]), "subject, no value yet");
    }
}
