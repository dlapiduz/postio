use postio_ffi::{
    AppearanceFfi, DensityFfi, GroupFfi, ThemeFfi, row_metrics, settings_add_filter,
    settings_appearance, settings_composing, settings_filters, settings_group_label,
    settings_handoff_label, settings_handoff_target, settings_humanize_interval, settings_load,
    settings_patch_appearance, settings_patch_composing, settings_patch_filter,
    settings_patch_syncing, settings_path, settings_remove_filter, settings_save,
    settings_sections, settings_status, settings_syncing,
};

const SAMPLE: &str = "\
# hand-written, and it should stay that way
[sync]
idle = true

[ui]
density = \"compact\"
theme = \"dark\"
some_future_key = 42

[saved_searches.urgent]
query = \"is:unread\"
";

#[test]
fn patching_appearance_leaves_everything_outside_the_ui_table_verbatim() {
    // Patch functions, not a reserialize of `Config`: reserializing reorders every key and drops
    // every comment in the file.
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
        patched.contains("[saved_searches.urgent]\nquery = \"is:unread\""),
        "filters moved:\n{patched}"
    );
    assert!(
        patched.contains("density = \"airy\""),
        "the edit did not land:\n{patched}"
    );
}

#[test]
fn patching_appearance_keeps_a_key_this_version_of_postio_does_not_know() {
    // `UiConfig::extra` (unknown keys) cannot cross as typed fields, so the patch must keep it or a
    // hand-added key is lost by opening the pane.
    let appearance = settings_appearance(SAMPLE.to_string()).expect("the sample parses");
    let patched = settings_patch_appearance(SAMPLE.to_string(), appearance).expect("it patches");

    assert!(
        patched.contains("some_future_key = 42"),
        "an unknown key in [ui] was dropped by a round trip that changed nothing:\n{patched}"
    );
}

#[test]
fn the_validity_line_names_the_line_a_broken_file_broke_on() {
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
    // A pane whose table will not parse disables its controls (ADR 0029): a default form would be
    // saved over the file being fixed.
    assert!(settings_appearance("[ui]\ndensity = = \n".to_string()).is_none());
}

#[test]
fn the_nav_is_focus_s_filtering_in_and_appearance_out() {
    // Focus honours Filtering but not Appearance: it follows the system's scheme and ignores
    // `[ui]`.
    let sections = settings_sections();
    let labels: Vec<&str> = sections.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Accounts",
            "Contacts",
            "Filtering",
            "Saved searches",
            "Composing",
            "Keyboard",
            "Sync & storage",
            "Privacy",
            "Agents",
            "Config file",
        ],
        "the nav order and names are the shared table's"
    );

    // A frontend that guessed the grouping would put Composing under different headings per
    // platform.
    let mail: Vec<&str> = sections
        .iter()
        .filter(|s| s.group == GroupFfi::Mail)
        .map(|s| s.label.as_str())
        .collect();
    assert_eq!(
        mail,
        [
            "Accounts",
            "Contacts",
            "Filtering",
            "Saved searches",
            "Composing"
        ]
    );
    assert_eq!(settings_group_label(GroupFfi::Application), "APPLICATION");
}

const FILTERING: &str = "# mine\n[focus]\nfiltering = true\n\n[focus.filter]\n\
    never = [\"ada@example.com\"]\n\
    stop_markers = [{ sender = \"news@ledger.example\", kind = \"question\" }]\n\n\
    [ui]\ndensity = \"compact\"\n";

#[test]
fn the_filtering_pane_says_what_gtk_s_says() {
    // The pane reads the people the last session to open installed: run alone, or a neighbour's
    // session is the one it reads.
    if postio_test_support::alone!("the_filtering_pane_says_what_gtk_s_says") {
        return;
    }
    let page = postio_ffi::settings_filtering(FILTERING.to_owned(), Some(186)).expect("it parses");
    let config = postio_config::Config::from_toml_str(FILTERING).expect("it parses");
    let shared = postio_ui::filtering::page(
        &config.focus,
        Some(186),
        &postio_core::Keymap::resolve_on(&config.keys, postio_ffi::PLATFORM),
    );
    assert!(page.on);
    assert_eq!(page.state, shared.state);
    assert_eq!(page.today, shared.today);
    assert_eq!(page.switch, postio_ui::filtering::SWITCH);
    let never: Vec<&str> = page.never.iter().map(|entry| entry.says.as_str()).collect();
    assert_eq!(
        never,
        shared
            .never
            .iter()
            .map(|entry| entry.says.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(page.never[0].undo_label, postio_ui::filtering::NEVER_UNDO);
    assert_eq!(page.stopped.len(), 1);
    assert_eq!(
        page.stopped[0].undo_label,
        postio_ui::filtering::STOPPED_UNDO
    );
    assert!(postio_ffi::settings_filtering("[focus\n".to_owned(), None).is_none());
}

#[test]
fn filtering_off_and_an_entry_taken_back_leave_the_rest_of_the_file_alone() {
    // The pane reads the people the last session to open installed: run alone, or a neighbour's
    // session is the one it reads.
    if postio_test_support::alone!(
        "filtering_off_and_an_entry_taken_back_leave_the_rest_of_the_file_alone"
    ) {
        return;
    }
    let off = postio_ffi::settings_patch_filtering(FILTERING.to_owned(), false).expect("written");
    assert!(off.starts_with("# mine\n"), "{off}");
    assert!(off.contains("density = \"compact\""), "{off}");
    assert!(
        !postio_ffi::settings_filtering(off, None)
            .expect("it parses")
            .on
    );

    let page = postio_ffi::settings_filtering(FILTERING.to_owned(), None).expect("it parses");
    let again =
        postio_ffi::settings_take_back_filter(FILTERING.to_owned(), page.never[0].undo.clone())
            .expect("written");
    let after = postio_ffi::settings_filtering(again.clone(), None).expect("it parses");
    assert!(after.never.is_empty(), "{again}");
    assert_eq!(after.stopped.len(), 1, "only the entry taken back goes");
    let back_on = postio_ffi::settings_take_back_filter(again, page.stopped[0].undo.clone())
        .expect("written");
    assert!(
        postio_ffi::settings_filtering(back_on, None)
            .expect("it parses")
            .stopped
            .is_empty()
    );
}

#[test]
fn the_after_unsubscribe_control_says_the_shared_words_and_writes_one_key() {
    let control = postio_ffi::settings_after_unsubscribe(FILTERING.to_owned()).expect("it parses");
    assert_eq!(control.heading, postio_ui::unsubscribe::SETTING_HEADING);
    assert_eq!(control.note, postio_ui::unsubscribe::SETTING_NOTE);
    assert_eq!(control.choices, ["Archive", "Delete"]);
    assert!(!control.delete, "archive is the default");

    let deleting =
        postio_ffi::settings_patch_after_unsubscribe(FILTERING.to_owned(), true).expect("written");
    assert!(deleting.starts_with("# mine\n"), "{deleting}");
    assert!(deleting.contains("density = \"compact\""), "{deleting}");
    assert!(
        postio_ffi::settings_after_unsubscribe(deleting.clone())
            .expect("it parses")
            .delete
    );
    let back = postio_ffi::settings_patch_after_unsubscribe(deleting, false).expect("written");
    assert!(
        !postio_ffi::settings_after_unsubscribe(back)
            .expect("it parses")
            .delete
    );
    assert!(postio_ffi::settings_after_unsubscribe("[focus\n".to_owned()).is_none());
}

#[test]
fn a_pane_names_the_table_it_writes_and_the_two_that_own_none_say_so() {
    let by_key = |key: &str| {
        settings_sections()
            .into_iter()
            .find(|s| s.key == key)
            .expect("the section exists")
    };
    assert_eq!(by_key("focus").table.as_deref(), Some("[focus]"));
    assert_eq!(by_key("sync").table.as_deref(), Some("[sync]"));
    assert_eq!(by_key("privacy").table, None);
}

#[test]
fn an_interval_reads_as_the_unit_it_was_written_in() {
    assert_eq!(settings_humanize_interval(300), "5 min");
    assert_eq!(settings_humanize_interval(90), "90s");
}

#[test]
fn every_appearance_field_round_trips_through_the_boundary() {
    // A handful of booleans and two enums gets wired wrong once and never noticed, since each field
    // looks right in its pane.
    let appearance = AppearanceFfi {
        density: DensityFfi::Comfortable,
        theme: ThemeFfi::Light,
        show_hover_actions: false,
        sender_avatars: false,
    };
    let patched =
        settings_patch_appearance(SAMPLE.to_string(), appearance.clone()).expect("it patches");
    let read_back = settings_appearance(patched).expect("what we wrote parses");
    assert_eq!(read_back, appearance);
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("postio-ffi-settings-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a temp dir");
    dir
}

#[test]
fn a_save_comes_back_byte_for_byte() {
    // Typing here and in $EDITOR must produce the same bytes: a save that normalised a trailing
    // newline or quote style would break that.
    let dir = temp_dir("roundtrip");
    let path = dir.join("config.toml");
    let text = "# a comment\n[ui]\ndensity = \"compact\"\n";

    settings_save(path.display().to_string(), text.to_string()).expect("it saves");
    assert_eq!(settings_load(path.display().to_string()), text);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_file_that_is_not_there_yet_loads_as_an_empty_document() {
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
    let dir = temp_dir("mkdir");
    let path = dir.join("nested").join("config.toml");

    settings_save(path.display().to_string(), "[ui]\n".to_string()).expect("it saves");
    assert_eq!(settings_load(path.display().to_string()), "[ui]\n");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_config_path_is_the_one_the_rest_of_postio_reads() {
    let path = settings_path().expect("this platform has a config path");
    assert!(path.ends_with("config.toml"), "{path}");
}

#[test]
fn the_session_reports_the_appearance_it_was_opened_with() {
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
    // In-memory sessions ignore the installed file by construction; `session_config_isolation.rs`
    // holds that (#1219).
    let session = postio_ffi::Session::open(postio_ffi::SessionOptions::in_memory())
        .expect("an in-memory session");
    assert_eq!(session.appearance().density, DensityFfi::Airy);
    session.shutdown();
}

#[test]
fn a_denser_row_is_shorter_and_the_tightest_one_drops_the_snippet() {
    // Compact drops the snippet, the costliest line in vertical space; a frontend that kept it
    // would be denser in name only.
    let airy = row_metrics(DensityFfi::Airy);
    let snug = row_metrics(DensityFfi::Comfortable);
    let compact = row_metrics(DensityFfi::Compact);

    assert!(airy.pad_y > snug.pad_y && snug.pad_y > compact.pad_y);
    assert!(airy.avatar > snug.avatar && snug.avatar > compact.avatar);
    assert!(airy.snippet && snug.snippet);
    assert!(!compact.snippet, "compact keeps the line it exists to drop");
}

#[test]
fn a_rows_actions_are_registry_commands_the_keyboard_also_runs() {
    // Mouse and keyboard must run the same verb; a hover action with its own implementation would
    // be a fourth way to archive that undo does not know.
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
    // A trailing space is invisible in a text field and fatal to the lookup.
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
        settings_handoff_target(read_back.editor, postio_ffi::FoundEditorFfi::Nothing),
        postio_ffi::HandoffTargetFfi::PlatformDefault
    );
}

#[test]
fn an_editor_that_is_not_an_application_says_it_needs_a_terminal() {
    let target = settings_handoff_target(
        "vim".to_owned(),
        postio_ffi::FoundEditorFfi::TerminalProgram,
    );

    let postio_ffi::HandoffTargetFfi::NeedsTerminal { name, advice } = target else {
        panic!("expected a terminal program, got {target:?}");
    };
    assert_eq!(name, "vim");
    assert!(advice.contains("vim"), "{advice}");
    assert!(advice.contains("terminal"), "{advice}");
}

#[test]
fn the_button_names_the_editor_when_there_is_one_to_name() {
    // `Open in $EDITOR` names an environment variable an application launched from Finder does not
    // have (#1288).
    assert_eq!(
        settings_handoff_label("Some Editor".to_owned()),
        "Open in Some Editor"
    );
    assert_eq!(settings_handoff_label(String::new()), "Edit elsewhere");
}

/// The comment sits above `[ui]`: a `patch_*` regenerates its own table, so a comment on that
/// table's header does not survive.
/// What must survive is everything the pane does not own.
const SYNCING: &str = "\
[sync]
check_for_mail = \"poll\"
poll_interval_secs = 900
notify = false
badge = false
a_key_this_build_does_not_know = 1

# hand-written, and it should stay that way
[ui]
density = \"compact\"
";

#[test]
fn the_sync_pane_reads_the_sync_table() {
    let syncing = settings_syncing(SYNCING.to_owned()).expect("the sample parses");

    assert_eq!(syncing.check_for_mail, postio_ffi::CheckForMailFfi::Poll);
    assert_eq!(syncing.poll_interval_secs, 900);
    assert!(!syncing.notify);
    assert!(!syncing.badge, "the Dock's unread count, switched off");
    assert!(
        syncing.sync_on_startup,
        "a key that is not in the file is its default, not an error"
    );
}

#[test]
fn emptying_junk_and_trash_is_off_until_chosen_and_written_back() {
    let mut syncing = settings_syncing(SYNCING.to_owned()).expect("parses");
    assert_eq!(
        (
            syncing.empty_junk_after_days,
            syncing.empty_trash_after_days
        ),
        (0, 0)
    );
    syncing.empty_trash_after_days = 30;
    let written = settings_patch_syncing(SYNCING.to_owned(), syncing).expect("patch");
    assert!(written.contains("empty_trash_after_days = 30"), "{written}");
    let again = settings_syncing(written).expect("reads back");
    assert_eq!(
        (again.empty_junk_after_days, again.empty_trash_after_days),
        (0, 30)
    );
}

#[test]
fn the_empty_junk_and_trash_choices_are_the_shared_ones() {
    let choices = postio_ffi::settings_auto_empty_choices();
    let named: Vec<(u32, &str)> = choices.iter().map(|c| (c.days, c.label.as_str())).collect();
    assert_eq!(
        named,
        [
            (0, "Never"),
            (7, "7 days"),
            (30, "30 days"),
            (90, "90 days")
        ]
    );
    assert!(postio_ffi::settings_auto_empty_words()[1].contains("Flagged"));
}

#[test]
fn changing_how_mail_is_checked_leaves_the_rest_of_the_file_alone() {
    let mut syncing = settings_syncing(SYNCING.to_owned()).expect("parses");
    syncing.check_for_mail = postio_ffi::CheckForMailFfi::Idle;

    let written = settings_patch_syncing(SYNCING.to_owned(), syncing).expect("patch");

    assert!(written.contains("check_for_mail = \"idle\""), "{written}");
    assert!(
        written.contains("# hand-written, and it should stay that way"),
        "the comment survived: {written}"
    );
    assert!(
        written.contains("a_key_this_build_does_not_know = 1"),
        "a key this build does not know survived: {written}"
    );
    assert!(written.contains("density = \"compact\""), "{written}");
}

#[test]
fn every_sync_field_survives_a_round_trip() {
    // A pane with eight controls has eight chances to reset the other seven when one `patch`
    // forgets a line.
    let mut syncing = settings_syncing(SYNCING.to_owned()).expect("parses");
    syncing.check_for_mail = postio_ffi::CheckForMailFfi::Manual;
    syncing.poll_interval_secs = 60;
    syncing.max_connections = 3;
    syncing.sync_on_startup = false;
    syncing.body_fetch = postio_ffi::BodyFetchFfi::Eager;
    syncing.attachment_fetch = postio_ffi::AttachmentFetchFfi::Never;
    syncing.initial_sync_messages = 250;
    syncing.notify = true;
    syncing.badge = true;

    let written = settings_patch_syncing(SYNCING.to_owned(), syncing.clone()).expect("patch");
    let read_back = settings_syncing(written).expect("the patched file parses");

    assert_eq!(read_back, syncing);
}

const FILTERS: &str = "\
[saved_searches.urgent]
query = \"is:unread flag:flagged\"
pinned = true
order = 1

[saved_searches.newsletters]
query = \"list:*\"
name = \"Mailing lists\"
a_key_this_build_does_not_know = true

# hand-written, and it should stay that way
[ui]
density = \"compact\"
";

#[test]
fn a_filter_shows_the_name_its_owner_gave_it_not_its_key() {
    let filters = settings_filters(FILTERS.to_owned()).expect("the sample parses");

    assert_eq!(filters.len(), 2);
    assert_eq!(filters[0].key, "urgent");
    assert_eq!(
        filters[0].name, "urgent",
        "never renamed, so the key stands in"
    );
    assert_eq!(filters[1].key, "newsletters");
    assert_eq!(filters[1].name, "Mailing lists");
}

#[test]
fn filters_come_back_in_the_order_the_sidebar_shows_them() {
    let filters = settings_filters(FILTERS.to_owned()).expect("parses");

    assert_eq!(
        filters.iter().map(|f| f.key.as_str()).collect::<Vec<_>>(),
        ["urgent", "newsletters"]
    );
}

#[test]
fn editing_one_filter_leaves_the_others_and_their_unknown_keys_alone() {
    let mut filters = settings_filters(FILTERS.to_owned()).expect("parses");
    filters[0].query = "is:unread".to_owned();
    filters[0].pinned = false;

    let written = settings_patch_filter(FILTERS.to_owned(), filters[0].clone()).expect("patch");

    assert!(written.contains("query = \"is:unread\""), "{written}");
    assert!(
        written.contains("a_key_this_build_does_not_know = true"),
        "another filter's unknown key survived: {written}"
    );
    assert!(
        written.contains("# hand-written, and it should stay that way"),
        "{written}"
    );
    let read_back = settings_filters(written).expect("the patched file parses");
    assert!(!read_back[0].pinned);
    assert_eq!(read_back[1].name, "Mailing lists", "the other is untouched");
}

#[test]
fn clearing_a_name_restores_the_key_rather_than_saving_an_empty_label() {
    // A saved empty name would draw as nothing, which is worse than the key.
    let mut filters = settings_filters(FILTERS.to_owned()).expect("parses");
    filters[1].name = "   ".to_owned();

    let written = settings_patch_filter(FILTERS.to_owned(), filters[1].clone()).expect("patch");

    let read_back = settings_filters(written).expect("parses");
    assert_eq!(read_back[1].name, "newsletters", "back to the key");
}

#[test]
fn a_filter_can_be_added_and_removed() {
    let added = settings_add_filter(
        FILTERS.to_owned(),
        "today".to_owned(),
        "is:unread".to_owned(),
    )
    .expect("add");
    let filters = settings_filters(added.clone()).expect("parses");
    assert_eq!(filters.len(), 3);
    assert!(filters.iter().any(|filter| filter.key == "today"));

    let removed = settings_remove_filter(added, "today".to_owned()).expect("remove");
    let filters = settings_filters(removed).expect("parses");
    assert_eq!(filters.len(), 2);
    assert!(!filters.iter().any(|filter| filter.key == "today"));
}

#[test]
fn two_filters_cannot_share_a_key() {
    // The key is the identity; an overwrite would replace somebody's saved search.
    let refusal = settings_add_filter(
        FILTERS.to_owned(),
        "urgent".to_owned(),
        "is:unread".to_owned(),
    )
    .expect_err("a duplicate key");

    assert!(format!("{refusal}").contains("already"), "{refusal}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_filtering_pane_lists_a_person_s_pin_by_name_and_taking_it_back_clears_the_person() {
    // The pane reads the people the last session to open installed: run alone, or a neighbour's
    // session is the one it reads.
    if postio_test_support::alone!(
        "the_filtering_pane_lists_a_person_s_pin_by_name_and_taking_it_back_clears_the_person"
    ) {
        return;
    }
    use postio_model::{ContactArrival, ContactSettings, EmailAddress, StoppedMarker};
    use postio_storage::repository::{ContactRepository, ContactSettingsRepository};

    let database = postio_storage::test_support::memory().await;
    {
        let connection = database.connect().await.expect("a connection");
        let ada = ContactRepository::new(&connection)
            .create(
                Some("Ada Lovelace"),
                &[
                    EmailAddress::new(None::<String>, "ada@work.example"),
                    EmailAddress::new(None::<String>, "ada@home.example"),
                ],
            )
            .await
            .expect("a person");
        let mut settings = ContactSettings {
            arrival: Some(ContactArrival::Inbox),
            ..ContactSettings::default()
        };
        settings.set_stop(StoppedMarker::Todo, true);
        ContactSettingsRepository::new(&connection)
            .put(ada, &settings)
            .await
            .expect("what she decided");
    }
    let session = postio_ffi::Session::open(postio_ffi::SessionOptions::in_memory_with(database))
        .expect("a session");

    let (listed, after, stopped_after) = tokio::task::spawn_blocking(move || {
        let page = postio_ffi::settings_filtering("[focus]\n".to_owned(), None).expect("it parses");
        let text = postio_ffi::settings_take_back_filter(
            "[focus]\n".to_owned(),
            page.never[0].undo.clone(),
        )
        .expect("taken back");
        let again = postio_ffi::settings_filtering(text, None).expect("it parses");
        let stopped_after = again.stopped.len();
        (page, again.never.len(), stopped_after)
    })
    .await
    .expect("the blocking calls");

    assert_eq!(
        listed.never[0].says,
        "Ada Lovelace \u{b7} ada@work.example, ada@home.example"
    );
    assert_eq!(listed.stopped[0].says, "To-dos in mail from Ada Lovelace");
    assert_eq!(
        after, 0,
        "her pin is gone, whichever address was handed back"
    );
    assert_eq!(stopped_after, 1, "and only that: her marker is still off");
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_person_s_digest_counts_as_a_digest_rule_in_the_strip() {
    use postio_model::{ContactArrival, ContactSettings, DigestSetting, EmailAddress};
    use postio_storage::repository::{ContactRepository, ContactSettingsRepository};

    let database = postio_storage::test_support::memory().await;
    {
        let connection = database.connect().await.expect("a connection");
        let ledger = ContactRepository::new(&connection)
            .person_for_email(&EmailAddress::new(None::<String>, "news@ledger.example"))
            .await
            .expect("a person");
        ContactSettingsRepository::new(&connection)
            .put(
                ledger,
                &ContactSettings {
                    arrival: Some(ContactArrival::Digest),
                    digest: Some(DigestSetting {
                        name: "Ledger".into(),
                        cadence: "weekly".into(),
                        day: Some("saturday".into()),
                        at: "16:00".into(),
                    }),
                    ..ContactSettings::default()
                },
            )
            .await
            .expect("their digest");
    }
    let session = postio_ffi::Session::open(postio_ffi::SessionOptions::in_memory_with(database))
        .expect("a session");
    let strip = tokio::task::spawn_blocking({
        let session = session.clone();
        move || session.focus_strip()
    })
    .await
    .expect("the blocking call")
    .expect("the strip");
    assert_eq!(
        strip.digest_rules,
        Some(postio_ui::focus_row::digest_rules(1)),
        "one rule, though the file has none"
    );
    session.shutdown();
}
