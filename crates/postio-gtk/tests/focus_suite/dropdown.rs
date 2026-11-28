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

pub fn the_words_state_draws_hits_pills_and_show_all() {
    crate::gtk_case(async {
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

pub fn slash_opens_the_dropdown_and_a_word_finds_top_hits() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        fixture.file_from_ada("Harbor budget", "Numbers.", 5).await;
        fixture
            .file(("Tomas Ruiz", "tomas@example.com"), "Lunch", "Noon?", 10)
            .await;
        let (window, _client) = fixture.open().await;
        support::rows_shown(&window, 2).await;
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
        let field = bar.input().downcast::<gtk::Entry>().expect("an entry");
        assert_eq!(
            field.placeholder_text().as_deref(),
            Some(postio_ui::search_view::PLACEHOLDER)
        );
        assert!(
            !bar.dims_the_list(),
            "the inbox is dimmed behind the dropdown"
        );
        assert!(
            crate::settle_until(async || field.width() > 700).await,
            "the field did not grow: {} px",
            field.width()
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

/// A view can land before the bar has been laid out; the ghost is drawn once the field has its
/// width (#2151).
pub fn a_ghost_drawn_before_the_bar_is_laid_out_shows_once_it_is() {
    crate::gtk_case(async {
        let bar = postio_gtk::bar::Bar::new(postio_core::Keymap::defaults());
        let window = gtk::Window::new();
        window.set_default_size(1000, 600);
        window.set_child(Some(bar.widget()));
        bar.open_on("at", None);
        bar.show_dropdown(&DropdownView {
            state: DropdownState::Prefix,
            ghost: Some("las".to_owned()),
            ghost_hint: Some(Hint {
                key: "Tab".to_owned(),
                label: "completes".to_owned(),
            }),
            sections: vec![],
            highlight: None,
            ..words_view()
        });
        crate::settle();
        window.present();
        assert!(
            crate::settle_until(async || bar.ghost().as_deref() == Some("las")).await,
            "the ghost was lost to a field with no width yet: {:?}",
            bar.ghost_at()
        );
        window.destroy();
    });
}

pub fn a_prefix_draws_its_ghost_after_the_caret_and_tab_takes_it() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        fixture
            .file_from_ada("Atlas budget", "Numbers for atlas.", 5)
            .await;
        fixture.index().await;
        let (window, _client) = fixture.open().await;
        support::rows_shown(&window, 1).await;
        support::press(&window, "slash", gtk::gdk::ModifierType::empty());
        let bar = window.bar().expect("/ opened the bar");
        bar.set_text("a");
        crate::settle();
        bar.set_text("at");
        assert!(
            crate::settle_until(async || bar.ghost().as_deref() == Some("las")).await,
            "no ghost after \"at\": {:?} with {:?}",
            bar.ghost(),
            bar.dropdown().texts()
        );
        let after_the_caret = || {
            bar.ghost_at().is_some_and(|(ghost, typed_end)| {
                ghost >= typed_end - 1.0 && ghost <= typed_end + 8.0
            })
        };
        assert!(
            crate::settle_until(async || after_the_caret()).await,
            "the ghost is not after the caret: {:?}",
            bar.ghost_at()
        );
        assert!(
            bar.texts().iter().any(|text| text == "completes"),
            "no \"Tab completes\": {:?}",
            bar.texts()
        );
        assert!(
            support::deliver(&window, "Tab"),
            "Tab reached the bar: {}",
            support::focus_path(&window)
        );
        assert!(
            crate::settle_until(async || bar.typed().trim_end() == "atlas").await,
            "Tab did not take the ghost: {:?}",
            bar.typed()
        );
        assert!(
            crate::settle_until(async || bar.ghost().is_none()).await,
            "the ghost stayed: {:?}",
            bar.ghost()
        );
    });
}

pub fn escape_gives_the_keyboard_back_to_the_cursor_row() {
    crate::gtk_case(async {
        let (_fixture, window) = support::three_in_the_inbox().await;
        support::keys(&window, &["j"]);
        support::press(&window, "slash", gtk::gdk::ModifierType::empty());
        let bar = window.bar().expect("/ opened the bar");
        assert!(
            crate::settle_until(async || !bar.dropdown().titles().is_empty()).await,
            "the dropdown never drew"
        );
        support::press(&window, "Escape", gtk::gdk::ModifierType::empty());
        assert!(!bar.is_open(), "Escape left the bar up");
        let pane = window.pane().expect("the inbox");
        let on_the_cursor_row = || {
            let focus = gtk::prelude::GtkWindowExt::focus(&window);
            let rows = pane.rows_on_screen();
            rows.get(1)
                .and_then(|row| row.parent())
                .is_some_and(|item| Some(item) == focus)
        };
        assert_eq!(
            pane.cursor().selected(),
            1,
            "j put the cursor on the second row"
        );
        assert!(
            crate::settle_until(async || on_the_cursor_row()).await,
            "the keyboard is not on the cursor's row: {}",
            support::focus_path(&window)
        );
    });
}
