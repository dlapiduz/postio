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
    // Offering a command the surface will ignore is worse than omitting it: Return does nothing and
    // reads as a broken application.
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
    // The scoring is `postio_ui::palette`'s; asserting it here stops Swift quietly writing its own.
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
    // A cheat sheet that printed the default for a rebound command would teach the wrong key.
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
    // The sheet prints the Mac's key on every host, because the boundary serves the Mac
    // (`postio_ffi::PLATFORM`).
    let palette = session
        .cheat_sheet(UiContext::List)
        .into_iter()
        .find(|entry| entry.id == "command_palette")
        .expect("the palette can be opened from the list");
    assert_eq!(palette.binding.as_deref(), Some("cmd+k"));
    session.shutdown();
}

#[test]
fn the_menu_and_the_palette_agree_about_what_is_available() {
    // The menu drew every registry command enabled whatever the surface could run (#1158); assert
    // agreement with the palette, not a list.
    let session = session();
    for context in [UiContext::List, UiContext::Composer, UiContext::Reader] {
        let offered: Vec<String> = session
            .palette_entries("", context)
            .into_iter()
            .map(|entry| entry.id)
            .collect();
        for spec in session.commands() {
            assert_eq!(
                session.is_available(&spec.id, context),
                offered.contains(&spec.id),
                "`{}` is available={} in {context:?} but the palette {} it",
                spec.id,
                session.is_available(&spec.id, context),
                if offered.contains(&spec.id) {
                    "offers"
                } else {
                    "does not offer"
                }
            );
        }
    }
    session.shutdown();
}

#[test]
fn a_composer_command_is_unavailable_from_the_list() {
    // `Send`, `Bold` and the rest are composer-only, so a list menu gets `false` and greys them.
    let session = session();
    for id in ["send", "bold", "attach_file", "save_draft", "quote_block"] {
        assert!(
            !session.is_available(id, UiContext::List),
            "`{id}` reported available from the message list"
        );
        assert!(
            session.is_available(id, UiContext::Composer),
            "`{id}` is a composer command and should be available there"
        );
    }
    session.shutdown();
}

#[test]
fn an_id_this_build_does_not_know_is_unavailable() {
    let session = session();
    assert!(!session.is_available("not_a_command", UiContext::List));
    session.shutdown();
}
