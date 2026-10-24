//! The settings panel's Agents pane (ADR 0010 Q6, #1832): which accounts an
//! agent may read, taking that back, and what agents did lately.
//!
//! Revoking edits the panel's own buffer through `postio_config`'s patchers,
//! and the panel's debounced write takes it to disk, as for every other
//! pane. Skips without a display. Nothing here touches the network.

use std::cell::Cell;
use std::rc::Rc;

use crate::support_compose::{fonts, style};
use gtk::gdk;
use gtk::prelude::*;
use postio_ui::agent_access::{
    Activity, CallRow, NOTHING_SHARED, REVOKE_ALL, REVOKE_NOTE, REVOKE_ONE,
};
use postio_ui::settings::Section;
use postio_widgets::settings::SettingsPanel;

const SHARED: &str = "[ui]\ntheme = \"dark\"\n\n\
    [mcp.accounts.\"ada@example.com\"]\nmailboxes = [\"INBOX\"]\n\n\
    [mcp.accounts.\"grace@example.com\"]\nmailboxes = [\"*\"]\nactions = [\"archive\"]\n";

pub fn the_pane_lists_what_is_shared_and_revoking_takes_one_back() {
    let Some((window, panel)) = agents_pane(SHARED) else {
        return;
    };

    assert_eq!(
        shared(&panel),
        [
            "ada@example.com INBOX \u{b7} Read and draft only Revoke",
            "grace@example.com All folders \u{b7} Also archive Revoke",
        ]
    );
    assert!(texts(&panel, "postio-settings-agents-note").contains(&REVOKE_NOTE.to_owned()));

    let revoke = buttons(&panel, "postio-settings-agents-revoke");
    assert_eq!(revoke[0].label().as_deref(), Some(REVOKE_ONE));
    revoke[0].emit_clicked();
    pump();

    let text = panel.text();
    assert!(!text.contains("ada@example.com"), "{text}");
    assert!(
        text.contains("grace@example.com"),
        "the other account stays: {text}"
    );
    assert!(
        text.contains("theme = \"dark\""),
        "the rest of the file stays: {text}"
    );
    assert_eq!(shared(&panel).len(), 1, "the pane follows the file");

    window.destroy();
}

pub fn revoke_all_leaves_nothing_shared_and_says_so() {
    let Some((window, panel)) = agents_pane(SHARED) else {
        return;
    };

    let all = buttons(&panel, "postio-settings-agents-revoke-all");
    assert_eq!(all[0].label().as_deref(), Some(REVOKE_ALL));
    all[0].emit_clicked();
    pump();

    assert!(!panel.text().contains("[mcp"), "{}", panel.text());
    assert!(shared(&panel).is_empty());
    let empty = texts(&panel, "postio-settings-agents-empty");
    assert_eq!(empty, [NOTHING_SHARED]);
    assert!(
        buttons(&panel, "postio-settings-agents-revoke-all")
            .iter()
            .all(|button| !button.is_visible()),
        "nothing left to revoke"
    );

    window.destroy();
}

pub fn opening_the_pane_asks_for_the_activity_and_draws_it() {
    let Some((window, panel)) = panel_with_text(SHARED) else {
        return;
    };
    let asked = Rc::new(Cell::new(0));
    panel.connect_agents_opened({
        let asked = asked.clone();
        move || asked.set(asked.get() + 1)
    });
    panel.show_section(Section::Agents);
    pump();
    assert_eq!(asked.get(), 1, "the log is read when the pane is looked at");

    panel.set_agent_activity(Activity {
        summary: "2 calls \u{b7} last 5 min ago".to_owned(),
        calls: vec![
            CallRow {
                when: "5 min ago".to_owned(),
                line: "Archived no messages \u{b7} refused".to_owned(),
                refused: true,
            },
            CallRow {
                when: "1 h ago".to_owned(),
                line: "Read 1 message \u{201c}Receipt\u{201d} in INBOX".to_owned(),
                refused: false,
            },
        ],
    });
    panel.set_agent_entry(Some("/usr/libexec/postio-mcp"));
    pump();

    assert_eq!(
        texts(&panel, "postio-settings-agents-summary"),
        ["2 calls \u{b7} last 5 min ago"]
    );
    assert_eq!(
        texts(&panel, "postio-settings-agents-call"),
        [
            "Archived no messages \u{b7} refused",
            "Read 1 message \u{201c}Receipt\u{201d} in INBOX",
        ]
    );
    assert_eq!(
        texts(&panel, "postio-settings-agents-entry"),
        ["/usr/libexec/postio-mcp"]
    );

    window.destroy();
}

pub fn a_file_that_will_not_parse_revokes_nothing() {
    let Some((window, panel)) = agents_pane(SHARED) else {
        return;
    };
    panel.set_text("[mcp");
    pump();
    let pane = collect(panel.upcast_ref(), "postio-settings-agents");
    assert!(
        pane.iter().all(|widget| !widget.is_sensitive()),
        "a broken file disables the pane rather than show a guess"
    );
    assert_eq!(panel.text(), "[mcp");

    window.destroy();
}

fn agents_pane(text: &str) -> Option<(gtk::Window, SettingsPanel)> {
    let (window, panel) = panel_with_text(text)?;
    panel.show_section(Section::Agents);
    pump();
    Some((window, panel))
}

fn panel_with_text(text: &str) -> Option<(gtk::Window, SettingsPanel)> {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return None;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let panel = SettingsPanel::new();
    let window = gtk::Window::new();
    style::track(&window);
    window.set_child(Some(&panel));
    window.present();
    panel.set_text(text);
    pump();
    Some((window, panel))
}

fn pump() {
    let context = glib::MainContext::default();
    while context.iteration(false) {}
}

/// Each shared row's words as a person reads them: the address, what an
/// agent may do, and the button.
fn shared(panel: &SettingsPanel) -> Vec<String> {
    collect(panel.upcast_ref(), "postio-settings-agents-row")
        .iter()
        .map(|row| {
            collect(row, "")
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
                .map(|label| label.text().to_string())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

fn texts(panel: &SettingsPanel, class: &str) -> Vec<String> {
    collect(panel.upcast_ref(), class)
        .into_iter()
        .filter(|widget| widget.is_visible())
        .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
        .map(|label| label.text().to_string())
        .collect()
}

fn buttons(panel: &SettingsPanel, class: &str) -> Vec<gtk::Button> {
    collect(panel.upcast_ref(), class)
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
        .collect()
}

/// Every widget in the tree carrying `class` (or, when `class` is empty,
/// every widget), depth first.
fn collect(widget: &gtk::Widget, class: &str) -> Vec<gtk::Widget> {
    let mut found = Vec::new();
    if class.is_empty() || widget.has_css_class(class) {
        found.push(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        found.extend(collect(&current, class));
        child = current.next_sibling();
    }
    found
}
