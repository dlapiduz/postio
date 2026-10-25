//! The search dropdown (spec 010 T170, screens 01-05): GTK draws the
//! controller's `DropdownView`.
//!
//! The words, which rows run, and the default highlight are the
//! controller's, asserted in `postio-focus`. These cases ask what is GTK's
//! own: that a view is drawn as it says, that the arrows keep their place
//! across a redraw, and that a run or a pill says its token back. The last
//! drives the window.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use postio_focus::{
    DropdownPill, DropdownRow, DropdownRowKind, DropdownSection, DropdownState, DropdownView, Run,
    RunStyle,
};
use postio_gtk::dropdown::Dropdown;
use postio_ui::hints::Hint;

use crate::support::{self, Fixture};

fn run(text: &str) -> Run {
    Run {
        text: text.to_owned(),
        highlighted: false,
        style: RunStyle::Plain,
    }
}

fn found(text: &str) -> Run {
    Run {
        highlighted: true,
        ..run(text)
    }
}

fn row(token: u64, kind: DropdownRowKind, title: Vec<Run>, detail: Vec<Run>) -> DropdownRow {
    DropdownRow {
        token,
        kind,
        title,
        detail,
        folder: None,
        right: None,
        key: None,
        selectable: true,
        initials: None,
        highlighted: None,
    }
}

fn section(title: &str, rows: Vec<DropdownRow>, pills: Vec<DropdownPill>) -> DropdownSection {
    DropdownSection {
        title: title.to_owned(),
        note: None,
        note_key: None,
        rows,
        pills,
    }
}

fn pill(token: u64, op: Option<&str>, label: &str, count: &str) -> DropdownPill {
    DropdownPill {
        token,
        op: op.map(str::to_owned),
        label: label.to_owned(),
        count: Some(count.to_owned()),
        fresh: None,
        key: None,
    }
}

/// Screen 03: "budget" typed, two hits with their passages, Narrow to,
/// and Show all, which the controller highlights by default.
fn words_view() -> DropdownView {
    let mut first = row(
        11,
        DropdownRowKind::Hit,
        vec![run("Ada Moreno \u{b7} Q3 "), found("budget")],
        vec![run("the "), found("budget"), run(" is attached")],
    );
    first.folder = Some("in:Inbox".to_owned());
    first.right = Some("yesterday".to_owned());
    let second = row(
        12,
        DropdownRowKind::Hit,
        vec![run("Tomas Ruiz \u{b7} Travel")],
        vec![run("over "), found("budget"), run(" again")],
    );
    let mut show_all = row(
        13,
        DropdownRowKind::ShowAll,
        vec![
            run("Show all "),
            Run {
                style: RunStyle::Strong,
                ..run("48")
            },
            run(" results"),
        ],
        vec![],
    );
    show_all.key = Some("Return".to_owned());
    DropdownView {
        state: DropdownState::Words,
        ghost: None,
        ghost_hint: None,
        operator: None,
        understood: vec![],
        sections: vec![
            DropdownSection {
                note: Some("ranked by sender, recency and where the words matched".to_owned()),
                ..section("Top hits", vec![first, second], vec![])
            },
            section(
                "Narrow to",
                vec![],
                vec![
                    pill(21, Some("from:"), "Ada", "12"),
                    pill(22, Some("in:"), "Receipts", "3"),
                ],
            ),
            section("", vec![show_all], vec![]),
        ],
        highlight: Some(13),
        select: None,
        hints: vec![Hint {
            key: "Return".to_owned(),
            label: "show all".to_owned(),
        }],
        count: Some("48 matches \u{b7} 38 ms".to_owned()),
    }
}

/// A dropdown drawn from `view`, with what it says back recorded.
/// What a dropdown said back: `run` or `highlighted`, and the token.
type Said = Rc<RefCell<Vec<(&'static str, u64)>>>;

fn drawn(view: &DropdownView) -> (Rc<Dropdown>, Said) {
    let dropdown = Dropdown::new();
    let said = Rc::new(RefCell::new(Vec::new()));
    dropdown.connect_run({
        let said = Rc::clone(&said);
        move |token| said.borrow_mut().push(("run", token))
    });
    dropdown.connect_highlighted({
        let said = Rc::clone(&said);
        move |token| said.borrow_mut().push(("highlighted", token))
    });
    dropdown.show(view);
    (dropdown, said)
}

/// Screen 03 drawn: the sections in order, each hit's words with the
/// matched ones in the find highlight, the pills, and the footer's keys
/// and count.
pub fn the_words_state_draws_hits_pills_and_show_all() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (dropdown, _) = drawn(&words_view());
        assert_eq!(dropdown.titles(), ["Top hits", "Narrow to"]);
        let texts = dropdown.texts();
        assert!(
            texts
                .iter()
                .any(|text| text.contains("Ada Moreno \u{b7} Q3 budget")
                    && text.contains("the budget is attached")
                    && text.contains("in:Inbox")
                    && text.contains("yesterday")),
            "the first hit is not drawn whole: {texts:?}"
        );
        assert!(
            texts
                .iter()
                .any(|text| text.contains("Show all 48 results")),
            "no Show all: {texts:?}"
        );
        assert_eq!(dropdown.found(), ["budget", "budget", "budget"]);
        assert_eq!(dropdown.pills(), ["from: Ada 12", "in: Receipts 3"]);
        assert_eq!(
            dropdown.footer(),
            (
                vec!["Return show all".to_owned()],
                "48 matches \u{b7} 38 ms".to_owned()
            )
        );
        assert_eq!(dropdown.highlighted(), Some(13), "Show all is the default");
    });
}

/// The arrows walk the rows that run something, skipping the titles and
/// the pills' line, and say where they rest; Return runs that row.
pub fn the_arrows_skip_what_runs_nothing_and_return_runs() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (dropdown, said) = drawn(&words_view());
        dropdown.step(-1);
        assert_eq!(
            dropdown.highlighted(),
            Some(12),
            "up from Show all is the second hit"
        );
        dropdown.step(-1);
        dropdown.step(-1);
        assert_eq!(dropdown.highlighted(), Some(11), "the first hit is the top");
        assert!(dropdown.run_highlighted());
        assert_eq!(
            *said.borrow(),
            [("highlighted", 12), ("highlighted", 11), ("run", 11)]
        );
    });
}

/// A redraw -- the passages landing -- leaves the arrows where a person
/// put them, though the view's default is elsewhere; a run's `select`
/// moves them; a row no longer drawn gives way to the default. A redraw
/// says nothing back.
pub fn a_redraw_keeps_the_highlight_unless_told() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (dropdown, said) = drawn(&words_view());
        dropdown.step(-1);
        dropdown.show(&words_view());
        assert_eq!(
            dropdown.highlighted(),
            Some(12),
            "the redraw kept the second hit"
        );

        let mut moved = words_view();
        moved.select = Some(11);
        dropdown.show(&moved);
        assert_eq!(dropdown.highlighted(), Some(11), "select moved it");

        let mut gone = words_view();
        gone.sections[0].rows.remove(0);
        dropdown.show(&gone);
        assert_eq!(dropdown.highlighted(), Some(13), "the default took it");
        assert_eq!(*said.borrow(), [("highlighted", 12)]);
    });
}

/// A pill runs its token on a click, as a row does on Return.
pub fn a_pill_runs_its_token() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (dropdown, said) = drawn(&words_view());
        assert!(dropdown.click_pill("in: Receipts"));
        assert_eq!(*said.borrow(), [("run", 22)]);
    });
}

/// `/` opens the dropdown in the bar's
/// panel: empty, it offers the cheat sheet; typing a word in a message's
/// subject lists that message under Top hits with the word found, and
/// Show all is where Return goes.
pub fn slash_opens_the_dropdown_and_a_word_finds_top_hits() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Harbor budget",
                "Numbers.",
                5,
            )
            .await;
        fixture
            .file(("Tomas Ruiz", "tomas@example.com"), "Lunch", "Noon?", 10)
            .await;
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 2).await,
            "the inbox never reached the screen"
        );
        support::press(&window, "slash", gtk::gdk::ModifierType::empty());
        let bar = window.bar().expect("/ opened the bar");
        let dropdown = bar.dropdown();
        assert!(
            crate::settle_until(async || dropdown.titles().contains(&"Search by".to_owned())).await,
            "the empty dropdown has no cheat sheet: {:?}",
            dropdown.titles()
        );
        assert!(
            dropdown.widget().is_mapped(),
            "the dropdown is not on screen"
        );

        bar.set_text("budget");
        assert!(
            crate::settle_until(async || {
                dropdown
                    .titles()
                    .first()
                    .is_some_and(|title| title == "Top hits")
                    && dropdown
                        .texts()
                        .iter()
                        .any(|text| text.contains("Harbor budget"))
            })
            .await,
            "no Top hits for budget: {:?}",
            dropdown.texts()
        );
        assert!(
            dropdown
                .found()
                .iter()
                .any(|words| words.eq_ignore_ascii_case("budget")),
            "budget is not in the find highlight: {:?}",
            dropdown.found()
        );
        let texts = dropdown.texts();
        assert!(
            !texts.iter().any(|text| text.contains("Lunch")),
            "a message without the word is listed: {texts:?}"
        );
    });
}
