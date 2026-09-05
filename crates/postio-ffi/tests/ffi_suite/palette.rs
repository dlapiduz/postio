//! The palette and the cheat sheet, which are one list read two ways.
//!
//! #658 puts them in one issue on purpose: the palette filters the registry
//! by what was typed and by which surface has focus, and the cheat sheet
//! prints the same rows with their bindings. Built separately, two places
//! would decide what "available here" means and they would disagree.
//!
//! What these guard is that Swift is handed a *ranking* rather than a list to
//! rank. The matcher is `postio_ui::palette`'s, and two matchers mean the same
//! query offers different things on each platform.

use postio_ffi::{Session, SessionOptions, UiContext};

fn session() -> std::sync::Arc<Session> {
    Session::open(SessionOptions::in_memory()).expect("an in-memory session")
}

#[test]
fn an_empty_query_lists_what_the_surface_can_run() {
    let session = session();
    let listed = session.palette_entries("", UiContext::List);
    assert!(!listed.is_empty(), "the palette offered nothing at all");
    assert!(
        listed.iter().any(|entry| entry.id == "archive"),
        "the list can archive, so the palette must offer it"
    );
    session.shutdown();
}

#[test]
fn a_command_the_focused_surface_cannot_run_is_not_offered() {
    let session = session();
    // The rule that matters more than ranking: offering a command the surface
    // will ignore is worse than omitting it, because the user presses Return,
    // nothing happens, and that reads as a broken application rather than an
    // unavailable command.
    let in_list = session.palette_entries("", UiContext::List);
    assert!(
        !in_list.iter().any(|entry| entry.id == "send"),
        "the message list offered to send a draft"
    );
    let in_composer = session.palette_entries("", UiContext::Composer);
    assert!(
        in_composer.iter().any(|entry| entry.id == "send"),
        "the composer did not offer to send, so the filter is inverted"
    );
    session.shutdown();
}

#[test]
fn the_ranking_is_the_shared_matcher_s() {
    let session = session();
    // `cp` finds "Command palette" ahead of anything that merely contains
    // both letters, because a match at a word start is worth far more than
    // one mid-word. That is `postio_ui::palette`'s scoring, and asserting it
    // *here* is what stops Swift from quietly writing its own.
    let found = session.palette_entries("cp", UiContext::List);
    assert_eq!(
        found.first().map(|entry| entry.id.as_str()),
        Some("command_palette"),
        "the shared ranking did not survive the crossing: {:?}",
        found.iter().map(|e| &e.id).collect::<Vec<_>>()
    );
    session.shutdown();
}

#[test]
fn the_matched_characters_cross_as_offsets_rather_than_markup() {
    let session = session();
    // #568 changed the matcher to return ranges rather than pre-escaped Pango
    // precisely so a second frontend could highlight them its own way. Swift
    // builds an `AttributedString` from the same numbers GTK turns into `<b>`.
    let found = session.palette_entries("arc", UiContext::List);
    let archive = found
        .iter()
        .find(|entry| entry.id == "archive")
        .expect("archive matches `arc`");
    assert_eq!(
        archive.positions,
        vec![0, 1, 2],
        "`arc` matches the first three characters of `Archive`"
    );
    assert!(
        !archive.title.contains('<'),
        "the title arrived as markup, which is a frontend's decision"
    );
    session.shutdown();
}

#[test]
fn a_query_matching_nothing_offers_nothing() {
    let session = session();
    assert!(
        session
            .palette_entries("zzzqqq", UiContext::List)
            .is_empty()
    );
    session.shutdown();
}

#[test]
fn the_cheat_sheet_is_the_same_list_unfiltered() {
    let session = session();
    // Same source, same context filter, no query. Two lists built separately
    // is the thing #658 exists to prevent.
    let sheet = session.cheat_sheet(UiContext::List);
    let palette = session.palette_entries("", UiContext::List);
    assert_eq!(
        sheet.iter().map(|e| &e.id).collect::<Vec<_>>(),
        palette.iter().map(|e| &e.id).collect::<Vec<_>>()
    );
    assert!(
        sheet.iter().any(|entry| entry.binding.is_some()),
        "the cheat sheet printed no bindings, which is all it is for"
    );
    session.shutdown();
}

#[test]
fn the_binding_shown_is_the_one_in_force() {
    // Not the registry default. A cheat sheet that printed the default for a
    // command somebody rebound would be teaching the wrong key, which is
    // worse than teaching none.
    let session = Session::open(
        SessionOptions::in_memory().with_config_for_test("[keys]\narchive = \"ctrl+shift+e\"\n"),
    )
    .expect("a session with an override");

    let archive = session
        .cheat_sheet(UiContext::List)
        .into_iter()
        .find(|entry| entry.id == "archive")
        .expect("archive is reachable from the list");
    assert_eq!(archive.binding.as_deref(), Some("ctrl+shift+e"));
    session.shutdown();
}

#[test]
fn the_primary_modifier_reaches_the_sheet_as_this_platform_spells_it() {
    let session = session();
    // `mod+k` in the registry, and what the sheet prints has to be the key
    // somebody can actually press. Reads the same on both platforms and
    // asserts a different string on each, which is the point.
    let palette = session
        .cheat_sheet(UiContext::List)
        .into_iter()
        .find(|entry| entry.id == "command_palette")
        .expect("the palette can be opened from the list");
    let expected = if cfg!(target_os = "macos") {
        "cmd+k"
    } else {
        "ctrl+k"
    };
    assert_eq!(palette.binding.as_deref(), Some(expected));
    session.shutdown();
}
