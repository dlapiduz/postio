use crate::support_compose::{fonts, style};
use crate::support_reader::collect;
use gtk::gdk;
use gtk::prelude::*;
use postio_widgets::settings::SettingsPanel;

const SAMPLE: &str = "\
# a hand-written comment nobody wants to lose
[ui]
theme = \"dark\" # inline comment, also not to be lost

[saved_searches.needs-reply]
query = \"is:unread from:team\"
pinned = true
order = 0

[saved_searches.has-attach]
query = \"has:attach\"
pinned = true
order = 1

[saved_searches.archived-search]
query = \"is:archived\"
pinned = false
";

pub fn filters_render_as_rows_and_hide_when_there_are_none() {
    let Some((window, panel)) = panel_with_text("") else {
        return;
    };

    assert!(rows(&panel).is_empty(), "no filters, no rows");
    assert!(
        !section_visible(&panel),
        "an empty filters section should not show at all"
    );
    assert!(
        empty_state_visible(&panel),
        "an empty filters section should say why, not just vanish"
    );

    panel.set_text(SAMPLE);
    pump();

    assert!(section_visible(&panel));
    assert!(!empty_state_visible(&panel));
    assert_eq!(rows(&panel).len(), 3, "one row per filter, pinned or not");

    window.destroy();
}

pub fn a_pinned_search_shows_the_key_that_runs_it() {
    let Some((window, panel)) = panel_with_text(SAMPLE) else {
        return;
    };
    let keymap = postio_core::Keymap::resolve(&postio_config::KeyBindings::default());
    panel.set_keymap(&keymap);
    panel.set_text(SAMPLE);
    pump();

    let caps = |row: &gtk::ListBoxRow| -> Vec<String> {
        collect(
            row.upcast_ref::<gtk::Widget>(),
            "postio-settings-filter-key",
        )
        .into_iter()
        .flat_map(|w| collect(&w, "postio-keyhint"))
        .filter_map(|w| w.downcast::<gtk::Label>().ok())
        .map(|l| l.text().to_string())
        .collect()
    };
    let key = |id| keymap.binding(id).expect("a default binding").to_string();
    let rows = rows(&panel);
    assert_eq!(
        caps(&rows[0]),
        vec![key(postio_core::CommandId::SavedSearch1)]
    );
    assert_eq!(
        caps(&rows[1]),
        vec![key(postio_core::CommandId::SavedSearch2)]
    );
    assert!(
        caps(&rows[2]).is_empty(),
        "an unpinned search runs from no key"
    );

    window.destroy();
}

pub fn toggling_pinned_writes_straight_to_the_buffer() {
    let Some((window, panel)) = panel_with_text(SAMPLE) else {
        return;
    };

    let archived_row = rows(&panel)
        .into_iter()
        .find(|row| query_text(row) == "is:archived")
        .expect("the unpinned row");
    let switch = pinned_switch_in(&archived_row);
    assert!(!switch.is_active(), "archived-search starts unpinned");

    switch.set_active(true);
    pump();

    assert!(
        panel.text().contains("[saved_searches.archived-search]") && {
            let text = panel.text();
            let after = text
                .split("[saved_searches.archived-search]")
                .nth(1)
                .unwrap();
            after.contains("pinned = true")
        },
        "toggling the switch must flip pinned in the buffer: {}",
        panel.text()
    );
    assert!(
        panel
            .text()
            .contains("# a hand-written comment nobody wants to lose"),
        "an edit here must not disturb an unrelated section's comment: {}",
        panel.text()
    );
    assert!(
        panel
            .text()
            .contains("theme = \"dark\" # inline comment, also not to be lost"),
        "nor its own inline comment: {}",
        panel.text()
    );

    window.destroy();
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

fn query_text(row: &gtk::ListBoxRow) -> String {
    collect(
        row.upcast_ref::<gtk::Widget>(),
        "postio-settings-filter-query",
    )
    .into_iter()
    .find_map(|w| w.downcast::<gtk::Label>().ok())
    .map(|label| label.text().to_string())
    .expect("every filter row has a query label")
}

fn pinned_switch_in(row: &gtk::ListBoxRow) -> gtk::CheckButton {
    collect(row.upcast_ref::<gtk::Widget>(), "")
        .into_iter()
        .find_map(|w| w.downcast::<gtk::CheckButton>().ok())
        .expect("every filter row has a pinned checkbox")
}

fn rows(panel: &SettingsPanel) -> Vec<gtk::ListBoxRow> {
    collect(
        panel.upcast_ref::<gtk::Widget>(),
        "postio-settings-filter-row",
    )
    .into_iter()
    .filter_map(|w| w.downcast().ok())
    .collect()
}

fn section_visible(panel: &SettingsPanel) -> bool {
    collect(panel.upcast_ref::<gtk::Widget>(), "postio-settings-filters")
        .into_iter()
        .any(|w| w.is_visible())
}

fn empty_state_visible(panel: &SettingsPanel) -> bool {
    collect(
        panel.upcast_ref::<gtk::Widget>(),
        "postio-settings-filters-empty",
    )
    .into_iter()
    .any(|w| w.is_visible())
}
