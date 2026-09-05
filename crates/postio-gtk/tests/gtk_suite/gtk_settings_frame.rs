//! The navigation model the settings window is built on (#1179).
//!
//! The old panel appended every pane into one column and showed them all at
//! once, and its sidebar scrolled a `TextView` to a TOML header rather than
//! switching anything. The maintainer's report was that the result read as
//! floating cards with no window frame, and that nothing said whether they
//! were tabs, panes or a dialog.
//!
//! So this asserts the model itself, not the styling: **exactly one pane is
//! on screen**, the sidebar's selection is what chooses it, the frame around
//! it does not move, and the footer says which table the pane you are
//! looking at writes to. Skips without a display. Nothing here touches the
//! network.

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::settings::{Group, Section, SettingsPanel};
use postio_gtk::{fonts, style};

pub fn exactly_one_pane_is_ever_on_screen() {
    let Some((window, panel)) = panel() else {
        return;
    };

    for section in Section::ALL {
        panel.show_section(section);
        pump();

        let showing: Vec<String> = panes(&panel)
            .into_iter()
            .filter(|(_, pane)| pane.is_visible() && pane.is_mapped())
            .map(|(name, _)| name)
            .collect();
        assert_eq!(
            showing,
            vec![section.label().to_owned()],
            "{} should be the only pane drawn",
            section.label()
        );
    }

    window.destroy();
}

pub fn the_sidebar_selection_is_what_chooses_the_pane() {
    let Some((window, panel)) = panel() else {
        return;
    };

    let row = nav_row(&panel, Section::Sync);
    row.activate();
    pump();

    assert_eq!(panel.current_section(), Section::Sync);
    assert_eq!(pane_title(&panel), "Sync & storage");

    // And the other way: moving the pane moves the selection, so the two can
    // never disagree about where you are.
    panel.show_section(Section::Privacy);
    pump();
    assert!(
        nav_row(&panel, Section::Privacy).is_selected(),
        "showing a pane must select its row"
    );
    assert!(!nav_row(&panel, Section::Sync).is_selected());

    window.destroy();
}

pub fn the_frame_is_identical_on_every_pane() {
    let Some((window, panel)) = panel() else {
        return;
    };

    // The header bar, the sidebar and the footer are the *same widgets* on
    // all eight panes — not rebuilt per pane, which is what let the old
    // panel's chrome drift from section to section.
    let header = panel.header_bar();
    let mut footers = Vec::new();
    for section in Section::ALL {
        panel.show_section(section);
        pump();
        assert_eq!(
            panel.header_bar(),
            header,
            "the header bar must not be rebuilt for {}",
            section.label()
        );
        assert_eq!(
            pane_title(&panel),
            section.label(),
            "the pane repeats its own name"
        );
        footers.push(panel.footer_target_text());
    }

    assert!(
        footers.iter().all(|line| !line.is_empty()),
        "the footer says what is being written on every pane: {footers:?}"
    );

    window.destroy();
}

pub fn the_footer_names_the_table_the_pane_writes() {
    let Some((window, panel)) = panel() else {
        return;
    };

    panel.show_section(Section::Appearance);
    pump();
    assert_eq!(
        panel.footer_target_text(),
        "[ui] in config.toml",
        "Appearance owns [ui] and the strip has to say so"
    );

    panel.show_section(Section::Keyboard);
    pump();
    assert_eq!(panel.footer_target_text(), "[keys] in config.toml");

    // Privacy owns no table at all — its state lives outside config.toml —
    // so the strip names the file rather than inventing a `[privacy]`.
    panel.show_section(Section::Privacy);
    pump();
    assert!(
        !panel.footer_target_text().contains('['),
        "Privacy has no table to name: {:?}",
        panel.footer_target_text()
    );

    window.destroy();
}

pub fn the_sidebar_groups_its_sections_under_two_headings() {
    let Some((window, panel)) = panel() else {
        return;
    };
    pump();

    let headings: Vec<String> = collect(
        panel.upcast_ref::<gtk::Widget>(),
        "postio-settings-nav-heading",
    )
    .into_iter()
    .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
    .map(|label| label.text().to_string())
    .collect();

    assert_eq!(
        headings,
        Group::ALL
            .iter()
            .map(|g| g.label().to_owned())
            .collect::<Vec<_>>(),
        "both headings, in order, and drawn by the list rather than as rows \
         the keyboard would stop on"
    );

    window.destroy();
}

pub fn finding_a_setting_narrows_the_sidebar_to_the_panes_that_have_it() {
    let Some((panel_window, panel)) = panel() else {
        return;
    };
    pump();

    let search = search_entry(&panel);
    search.set_text("dark");
    pump();

    assert!(
        nav_row(&panel, Section::Appearance).is_child_visible(),
        "'dark' is Appearance, even though the word is not in its name"
    );
    assert!(
        !nav_row(&panel, Section::Keyboard).is_child_visible(),
        "and Keyboard is not"
    );

    search.set_text("");
    pump();
    assert!(
        Section::ALL
            .iter()
            .all(|section| nav_row(&panel, *section).is_child_visible()),
        "clearing the field brings every section back"
    );

    panel_window.destroy();
}

/// A realized panel, or `None` with no display.
fn panel() -> Option<(gtk::Window, SettingsPanel)> {
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
    window.set_default_size(900, 620);
    window.set_child(Some(&panel));
    window.present();
    pump();
    Some((window, panel))
}

fn pump() {
    let context = glib::MainContext::default();
    while context.iteration(false) {}
}

/// Every pane in the stack, by the name it was added under.
fn panes(panel: &SettingsPanel) -> Vec<(String, gtk::Widget)> {
    let stack = collect(panel.upcast_ref::<gtk::Widget>(), "")
        .into_iter()
        .find_map(|widget| widget.downcast::<gtk::Stack>().ok())
        .expect("the pane stack");
    let pages = stack.pages();
    (0..pages.n_items())
        .filter_map(|index| pages.item(index))
        .filter_map(|item| item.downcast::<gtk::StackPage>().ok())
        .map(|page| {
            (
                page.name().map(|name| name.to_string()).unwrap_or_default(),
                page.child(),
            )
        })
        .collect()
}

fn pane_title(panel: &SettingsPanel) -> String {
    collect(
        panel.upcast_ref::<gtk::Widget>(),
        "postio-settings-pane-title",
    )
    .into_iter()
    .find_map(|widget| widget.downcast::<gtk::Label>().ok())
    .expect("the pane title")
    .text()
    .to_string()
}

fn nav_row(panel: &SettingsPanel, section: Section) -> gtk::ListBoxRow {
    let index = Section::ALL
        .iter()
        .position(|candidate| *candidate == section)
        .expect("a known section");
    collect(panel.upcast_ref::<gtk::Widget>(), "postio-settings-nav-row")
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::ListBoxRow>().ok())
        .nth(index)
        .unwrap_or_else(|| panic!("no sidebar row for {}", section.label()))
}

fn search_entry(panel: &SettingsPanel) -> gtk::SearchEntry {
    collect(panel.header_bar().upcast_ref::<gtk::Widget>(), "")
        .into_iter()
        .find_map(|widget| widget.downcast::<gtk::SearchEntry>().ok())
        .expect("the find-a-setting field")
}

/// Every widget in the tree carrying `class` (or, when `class` is empty,
/// every widget), depth first. Copied from `gtk_settings_filters.rs` rather
/// than shared, matching that file's own reason: no dependency between the
/// two.
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
