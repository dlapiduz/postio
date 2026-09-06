//! The settings surface, as a frontend reads and writes it.
//!
//! The clause under test is ADR 0029's load-bearing one: **Swift never parses
//! or writes TOML.** Every assertion here is about what survives a round trip
//! through the boundary, because a second writer of `config.toml` with its own
//! idea of key order and comment survival is the failure this design exists to
//! prevent — and it is a failure nobody would see until their file had already
//! been rewritten.

use postio_ffi::{
    AppearanceFfi, DensityFfi, ThemeFfi, settings_appearance, settings_load,
    settings_patch_appearance, settings_path, settings_save, settings_sections, settings_status,
};

/// A file with things in it that a naive form would destroy: a comment, a key
/// this version does not know, and tables either side of `[ui]`.
const SAMPLE: &str = "\
# hand-written, and it should stay that way
[sync]
idle = true

[ui]
density = \"compact\"
theme = \"dark\"
some_future_key = 42

[filters.urgent]
query = \"is:unread\"
";

#[test]
fn patching_appearance_leaves_everything_outside_the_ui_table_verbatim() {
    // The whole reason the patch functions exist rather than a serialize of
    // the whole `Config`: reserializing reorders every key and drops every
    // comment in the file, not only in the table the pane owns.
    let mut appearance = settings_appearance(SAMPLE.to_string()).expect("the sample parses");
    appearance.density = DensityFfi::Airy;
    let patched = settings_patch_appearance(SAMPLE.to_string(), appearance).expect("it patches");

    assert!(
        patched.contains("# hand-written, and it should stay that way"),
        "the comment did not survive:\n{patched}"
    );
    assert!(
        patched.contains("[sync]\nidle = true"),
        "sync moved:\n{patched}"
    );
    assert!(
        patched.contains("[filters.urgent]\nquery = \"is:unread\""),
        "filters moved:\n{patched}"
    );
    assert!(
        patched.contains("density = \"airy\""),
        "the edit did not land:\n{patched}"
    );
}

#[test]
fn patching_appearance_keeps_a_key_this_version_of_postio_does_not_know() {
    // `UiConfig::extra` is a `toml::Table` of unknown keys, and it is exactly
    // what cannot cross a boundary as five typed fields. If Swift sent an
    // `AppearanceFfi` and the Rust side wrote only those five, a user who had
    // hand-added a key -- or who opened a newer file with an older build --
    // would lose it by opening the pane and touching nothing that owns it.
    let appearance = settings_appearance(SAMPLE.to_string()).expect("the sample parses");
    let patched = settings_patch_appearance(SAMPLE.to_string(), appearance).expect("it patches");

    assert!(
        patched.contains("some_future_key = 42"),
        "an unknown key in [ui] was dropped by a round trip that changed nothing:\n{patched}"
    );
}

#[test]
fn the_validity_line_names_the_line_a_broken_file_broke_on() {
    // Canvas 3f's footer replaces a dialog's buttons, so "invalid" alone is a
    // dead end -- 3d's rule is that every state names something and gives a
    // way forward.
    let status = settings_status("[ui]\ndensity = = \n".to_string());
    assert!(!status.valid);
    assert_eq!(status.line, Some(2), "the footer pointed at the wrong line");
    assert!(!status.message.is_empty());
}

#[test]
fn a_valid_file_says_so_and_says_how_long_it_took() {
    let status = settings_status(SAMPLE.to_string());
    assert!(
        status.valid,
        "the sample should be valid: {}",
        status.message
    );
    assert!(
        status.status_line.contains("parsed in"),
        "canvas 3f shows the parse timing: {}",
        status.status_line
    );
}

#[test]
fn appearance_is_unreadable_from_a_file_that_will_not_parse() {
    // ADR 0029's Failing state: a pane whose table will not parse disables its
    // controls rather than showing values it had to guess. Answering
    // `AppearanceFfi::default()` here would draw a form full of plausible
    // settings that are not the user's, and saving it would erase the file
    // they were trying to fix.
    assert!(settings_appearance("[ui]\ndensity = = \n".to_string()).is_none());
}

#[test]
fn the_nav_lists_all_six_sections_by_human_name() {
    let sections = settings_sections();
    let titles: Vec<&str> = sections.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(
        titles,
        [
            "Appearance",
            "Keyboard",
            "Accounts",
            "Sync",
            "Filters",
            "Privacy"
        ],
        "the nav order is canvas 3f's, and the names are the human ones"
    );
}

#[test]
fn every_appearance_field_round_trips_through_the_boundary() {
    // Five booleans and two enums is exactly the shape that gets wired up
    // wrong once and never noticed, because each field looks right in the
    // pane that set it.
    let appearance = AppearanceFfi {
        density: DensityFfi::Comfortable,
        theme: ThemeFfi::Light,
        show_hover_actions: false,
        show_key_hints: false,
        sender_avatars: false,
    };
    let patched =
        settings_patch_appearance(SAMPLE.to_string(), appearance.clone()).expect("it patches");
    let read_back = settings_appearance(patched).expect("what we wrote parses");
    assert_eq!(read_back, appearance);
}

// -- the file itself --------------------------------------------------------

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("postio-ffi-settings-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a temp dir");
    dir
}

#[test]
fn a_save_comes_back_byte_for_byte() {
    // Canvas 3f: typing here and typing in $EDITOR produce the same bytes on
    // disk. A save that normalised anything -- a trailing newline, a quote
    // style -- would make that false the first time someone used both.
    let dir = temp_dir("roundtrip");
    let path = dir.join("config.toml");
    let text = "# a comment\n[ui]\ndensity = \"compact\"\n";

    settings_save(path.display().to_string(), text.to_string()).expect("it saves");
    assert_eq!(settings_load(path.display().to_string()), text);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_file_that_is_not_there_yet_loads_as_an_empty_document() {
    // ADR 0029's Empty state: a first run has no `config.toml`, and the pane
    // shows defaults over an empty document rather than an error. The file is
    // created on the first save.
    let dir = temp_dir("absent");
    let missing = dir.join("nothing-here.toml");
    assert_eq!(settings_load(missing.display().to_string()), "");
    assert!(
        settings_appearance(String::new()).is_some(),
        "defaults are readable"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn saving_creates_the_directory_the_config_belongs_in() {
    // A first run on a machine with no `~/.config/postio` at all.
    let dir = temp_dir("mkdir");
    let path = dir.join("nested").join("config.toml");

    settings_save(path.display().to_string(), "[ui]\n".to_string()).expect("it saves");
    assert_eq!(settings_load(path.display().to_string()), "[ui]\n");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_config_path_is_the_one_the_rest_of_postio_reads() {
    // Not a second opinion about where settings live: `postio_config::paths`
    // already resolves this per platform, and a frontend that guessed would
    // edit a file nothing loads.
    let path = settings_path().expect("this platform has a config path");
    assert!(path.ends_with("config.toml"), "{path}");
}
