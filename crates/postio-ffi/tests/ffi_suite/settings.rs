//! The settings surface, as a frontend reads and writes it.
//!
//! The clause under test is ADR 0029's load-bearing one: **Swift never parses
//! or writes TOML.** Every assertion here is about what survives a round trip
//! through the boundary, because a second writer of `config.toml` with its own
//! idea of key order and comment survival is the failure this design exists to
//! prevent — and it is a failure nobody would see until their file had already
//! been rewritten.

use postio_ffi::{
    AppearanceFfi, DensityFfi, GroupFfi, ThemeFfi, row_metrics, settings_appearance,
    settings_composing, settings_group_label, settings_handoff_label, settings_handoff_target,
    settings_humanize_interval, settings_load, settings_patch_appearance, settings_patch_composing,
    settings_path, settings_save, settings_sections, settings_status,
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
fn the_nav_lists_all_eight_sections_by_human_name_under_two_headings() {
    let sections = settings_sections();
    let labels: Vec<&str> = sections.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Accounts",
            "Filters",
            "Composing",
            "Appearance",
            "Keyboard",
            "Sync & storage",
            "Privacy",
            "Config file",
        ],
        "the nav order and names are the ones the GTK window already shows"
    );

    // The grouping is the nav's shape, and a frontend that guessed it would
    // put Composing under APPLICATION on one platform and MAIL on the other.
    let mail: Vec<&str> = sections
        .iter()
        .filter(|s| s.group == GroupFfi::Mail)
        .map(|s| s.label.as_str())
        .collect();
    assert_eq!(mail, ["Accounts", "Filters", "Composing"]);
    assert_eq!(settings_group_label(GroupFfi::Application), "APPLICATION");
}

#[test]
fn a_pane_names_the_table_it_writes_and_the_two_that_own_none_say_so() {
    // The footer under every structured pane reads `[ui] in config.toml`, so
    // the table is the pane's, not a string the frontend keeps beside it.
    let by_key = |key: &str| {
        settings_sections()
            .into_iter()
            .find(|s| s.key == key)
            .expect("the section exists")
    };
    assert_eq!(by_key("ui").table.as_deref(), Some("[ui]"));
    assert_eq!(by_key("sync").table.as_deref(), Some("[sync]"));
    // Privacy is not a `config.toml` table at all (#871), and Config file is
    // every table there is rather than one.
    assert_eq!(by_key("privacy").table, None);
}

#[test]
fn an_interval_reads_as_the_unit_it_was_written_in() {
    assert_eq!(settings_humanize_interval(300), "5 min");
    assert_eq!(settings_humanize_interval(90), "90s");
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

// -- what the list reads (#1215) --------------------------------------------

#[test]
fn the_session_reports_the_appearance_it_was_opened_with() {
    // The pane writes `[ui]`; something has to read it, or the settings
    // window is a form over a file this application ignores.
    let session = postio_ffi::Session::open(
        postio_ffi::SessionOptions::in_memory()
            .with_config_for_test("[ui]\ndensity = \"compact\"\ntheme = \"light\"\n"),
    )
    .expect("a session with a [ui] table");

    let appearance = session.appearance();
    assert_eq!(appearance.density, DensityFfi::Compact);
    assert_eq!(appearance.theme, ThemeFfi::Light);
    session.shutdown();
}

#[test]
fn a_session_with_no_ui_table_reports_the_built_in_defaults() {
    // Plain `in_memory()`. It used to need an explicit empty document, because
    // an absent one fell through to `Config::load()` and this asserted whatever
    // density the person running it happened to prefer -- it failed on exactly
    // that, and #1219 came from here. An in-memory session now ignores the
    // installed file by construction, and `session_config_isolation.rs` is what
    // holds that.
    let session = postio_ffi::Session::open(postio_ffi::SessionOptions::in_memory())
        .expect("an in-memory session");
    assert_eq!(session.appearance().density, DensityFfi::Airy);
    session.shutdown();
}

#[test]
fn a_denser_row_is_shorter_and_the_tightest_one_drops_the_snippet() {
    // What density *is*, rather than how either frontend draws it. The
    // snippet is the line that costs the most vertical space and answers the
    // triage question least, so compact is the setting that drops it — and a
    // frontend that kept it would be denser in name only.
    let airy = row_metrics(DensityFfi::Airy);
    let snug = row_metrics(DensityFfi::Comfortable);
    let compact = row_metrics(DensityFfi::Compact);

    assert!(airy.pad_y > snug.pad_y && snug.pad_y > compact.pad_y);
    assert!(airy.avatar > snug.avatar && snug.avatar > compact.avatar);
    assert!(airy.snippet && snug.snippet);
    assert!(!compact.snippet, "compact keeps the line it exists to drop");
}

#[test]
fn the_row_hints_follow_the_users_own_bindings() {
    // A row that taught the wrong key would be worse than one that taught
    // none, so this reads the session's keymap rather than the registry's
    // defaults.
    let session = postio_ffi::Session::open(
        postio_ffi::SessionOptions::in_memory().with_config_for_test("[keys]\narchive = \"x\"\n"),
    )
    .expect("a session with a rebinding");

    let hints = session.row_hints();
    let archive = hints
        .iter()
        .find(|hint| hint.label == "archive")
        .expect("the row hints at archive");
    assert_eq!(archive.key, "x");
    assert!(
        hints.iter().any(|hint| hint.label == "reply"),
        "canvas 1b hints at two verbs, not one"
    );
    session.shutdown();
}

#[test]
fn a_rows_actions_are_registry_commands_the_keyboard_also_runs() {
    // The point of crossing them rather than listing them in Swift: the mouse
    // and the keyboard have to run the same verb for the same glyph, and a
    // hover action with its own implementation would be a fourth way to
    // archive that undo did not know about.
    let actions = postio_ffi::row_actions();
    let commands: Vec<&str> = actions.iter().map(|a| a.command.as_str()).collect();
    assert_eq!(commands, ["archive", "flag", "delete"]);

    let session =
        postio_ffi::Session::open(postio_ffi::SessionOptions::in_memory().with_config_for_test(""))
            .expect("a session");
    for action in &actions {
        assert!(
            session.binding_for(action.command.clone()).is_some(),
            "{} is offered to the mouse but bound to no key",
            action.command
        );
        assert!(!action.title.is_empty());
    }
    session.shutdown();
}

// --- the Composing pane, and which editor a draft goes to (#1288) ----------

/// A file with a `[compose]` table and things around it worth preserving.
const COMPOSING: &str = "\
# hand-written, and it should stay that way
[sync]
idle = true

[compose]
signature_on_reply = \"below_quote\"
editor = \"Some Editor\"
a_key_this_build_does_not_know = true

[ui]
density = \"compact\"
";

#[test]
fn the_composing_pane_reads_the_compose_table() {
    let composing = settings_composing(COMPOSING.to_owned()).expect("the sample parses");

    assert_eq!(
        composing.signature_on_reply,
        postio_ffi::SignaturePlacementFfi::BelowQuote
    );
    assert_eq!(
        composing.signature_on_forward,
        postio_ffi::SignaturePlacementFfi::AboveQuote,
        "a key that is not in the file is its default, not an error"
    );
    assert_eq!(composing.editor, "Some Editor");
}

#[test]
fn choosing_an_editor_leaves_every_other_section_and_the_unknown_key_alone() {
    // The clause this whole module is about, applied to the pane #1288 adds:
    // a form that wrote the file itself would reorder it and drop both the
    // comment and the key it does not understand.
    let mut composing = settings_composing(COMPOSING.to_owned()).expect("parses");
    composing.editor = "Another Editor".to_owned();

    let written = settings_patch_composing(COMPOSING.to_owned(), composing).expect("patch");

    assert!(written.contains("editor = \"Another Editor\""));
    assert!(
        written.contains("# hand-written, and it should stay that way"),
        "the comment survived: {written}"
    );
    assert!(
        written.contains("a_key_this_build_does_not_know = true"),
        "a key this build does not know survived: {written}"
    );
    assert!(written.contains("density = \"compact\""), "{written}");
}

#[test]
fn an_editor_typed_with_a_space_on_the_end_is_stored_without_one() {
    // The difference is invisible in a text field and fatal to the lookup:
    // no platform finds an application called "Some Editor ".
    let mut composing = settings_composing(COMPOSING.to_owned()).expect("parses");
    composing.editor = "  Some Editor  ".to_owned();

    let written = settings_patch_composing(COMPOSING.to_owned(), composing).expect("patch");

    assert!(written.contains("editor = \"Some Editor\""), "{written}");
}

#[test]
fn clearing_the_editor_hands_the_choice_back_to_the_platform() {
    let mut composing = settings_composing(COMPOSING.to_owned()).expect("parses");
    composing.editor = String::new();
    let written = settings_patch_composing(COMPOSING.to_owned(), composing).expect("patch");

    let read_back = settings_composing(written).expect("parses");
    assert_eq!(read_back.editor, "");
    assert_eq!(
        settings_handoff_target(read_back.editor, false),
        postio_ffi::HandoffTargetFfi::PlatformDefault
    );
}

#[test]
fn an_editor_that_is_not_an_application_says_it_needs_a_terminal() {
    // The case the setting exists for, and the one that would otherwise be a
    // button doing nothing: somebody types the name of the editor they use.
    let target = settings_handoff_target("vim".to_owned(), false);

    let postio_ffi::HandoffTargetFfi::NeedsTerminal { name, advice } = target else {
        panic!("expected a terminal program, got {target:?}");
    };
    assert_eq!(name, "vim");
    assert!(advice.contains("vim"), "{advice}");
    assert!(advice.contains("terminal"), "{advice}");
}

#[test]
fn the_button_names_the_editor_when_there_is_one_to_name() {
    // `Open in $EDITOR` is canvas 26's label and it names an environment
    // variable an application launched from Finder does not have (#1288).
    assert_eq!(
        settings_handoff_label("Some Editor".to_owned()),
        "Open in Some Editor"
    );
    assert_eq!(settings_handoff_label(String::new()), "Edit elsewhere");
}
