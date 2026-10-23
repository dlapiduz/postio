//! Settings > Contacts' Suggestions switches, from the controller's side
//! (specs/011-focus-contacts D7, handoff screen 08): the pane asks which are
//! on, a click on a switch asks the host to turn exactly that one over, and
//! the pane is redrawn from the host's answer, not from a guess.

use crate::search_support::asks;
use postio_config::paths::Platform;
use postio_focus::{Effect, FocusController, Input, Intent, NoRows, Policy, Reply, Request};
use postio_model::{ContactSuggestions, Suggestion};
use postio_ui::contacts_settings::SuggestionsView;

fn controller(platform: Platform) -> FocusController {
    let mut focus = FocusController::new(Policy::for_platform(platform));
    let _ = focus.handle(Input::Keymap(postio_core::Keymap::resolve_on(
        &postio_config::KeyBindings::default(),
        platform,
    )));
    focus
}

fn drawn(effects: &[Effect]) -> Option<SuggestionsView> {
    effects.iter().rev().find_map(|effect| match effect {
        Effect::Show(Intent::ContactsSettings(view)) => Some((**view).clone()),
        _ => None,
    })
}

/// The pane opened and the host answered with `switches`.
fn opened(focus: &mut FocusController, switches: ContactSuggestions) -> SuggestionsView {
    let effects = focus.handle_on(Input::ContactsSettingsOpened, &NoRows);
    let [(ticket, Request::ContactSuggestions)] = asks(&effects)[..] else {
        panic!("the pane asks which switches are on: {effects:?}");
    };
    let effects = focus.handle_on(
        Input::Reply(ticket, Reply::ContactSuggestions(Ok(switches))),
        &NoRows,
    );
    drawn(&effects).expect("the pane is drawn from the answer")
}

#[test]
fn the_pane_asks_which_switches_are_on_and_draws_the_answer() {
    let mut focus = controller(Platform::Apple);
    let view = opened(&mut focus, ContactSuggestions::default());
    let on: Vec<(Suggestion, bool)> = view.rows.iter().map(|row| (row.which, row.on)).collect();
    assert_eq!(
        on,
        [
            (Suggestion::SaveAutomatically, false),
            (Suggestion::RecogniseKinds, true),
            (Suggestion::ReadSignatures, true),
            (Suggestion::SuggestMerges, true),
        ]
    );
    assert!(view.footer.contains("this Mac"), "{}", view.footer);
    assert!(view.footer.contains("sync accounts"), "{}", view.footer);
}

#[test]
fn each_switch_asks_for_exactly_itself_turned_over_and_is_redrawn_from_the_answer() {
    for (index, which) in Suggestion::ALL.into_iter().enumerate() {
        let mut focus = controller(Platform::Apple);
        let start = ContactSuggestions::default();
        opened(&mut focus, start);

        let effects = focus.handle_on(Input::ContactsSuggestion(index as u32), &NoRows);
        let [(ticket, Request::ContactSuggestionSet { which: asked, on })] = asks(&effects)[..]
        else {
            panic!("{which:?}: a click asks the host: {effects:?}");
        };
        assert_eq!(asked, which, "the click on row {index} turns that row over");
        assert_eq!(on, !start.is_on(which), "{which:?} is flipped, not set");
        assert!(
            drawn(&effects).is_none(),
            "{which:?}: nothing is drawn as changed before the host says so"
        );

        let effects = focus.handle_on(
            Input::Reply(ticket, Reply::ContactSuggestions(Ok(start.with(which, on)))),
            &NoRows,
        );
        let view = drawn(&effects).expect("redrawn");
        for row in &view.rows {
            assert_eq!(
                row.on,
                start.is_on(row.which) != (row.which == which),
                "{which:?} moved {:?}",
                row.which
            );
        }
        // And a second click turns it back.
        let effects = focus.handle_on(Input::ContactsSuggestion(index as u32), &NoRows);
        let [(_, Request::ContactSuggestionSet { on: back, .. })] = asks(&effects)[..] else {
            panic!("{effects:?}");
        };
        assert_eq!(back, start.is_on(which));
    }
}

#[test]
fn a_switch_the_host_would_not_change_stays_as_it_was_and_says_why() {
    let mut focus = controller(Platform::Apple);
    opened(&mut focus, ContactSuggestions::default());
    let effects = focus.handle_on(Input::ContactsSuggestion(0), &NoRows);
    let [(ticket, _)] = asks(&effects)[..] else {
        panic!("{effects:?}");
    };
    let effects = focus.handle_on(
        Input::Reply(
            ticket,
            Reply::ContactSuggestions(Err("Could not change that setting".to_owned())),
        ),
        &NoRows,
    );
    let view = drawn(&effects).expect("redrawn with the reason");
    assert!(!view.rows[0].on, "still off");
    assert_eq!(view.error.as_deref(), Some("Could not change that setting"));
}

#[test]
fn a_click_before_the_pane_has_been_read_does_nothing_and_linux_has_no_sync_words() {
    let mut focus = controller(Platform::Apple);
    assert!(
        asks(&focus.handle_on(Input::ContactsSuggestion(1), &NoRows)).is_empty(),
        "nothing to turn over yet"
    );
    let mut linux = controller(Platform::Freedesktop);
    let view = opened(&mut linux, ContactSuggestions::default());
    assert!(view.footer.contains("this computer"), "{}", view.footer);
    assert!(!view.footer.contains("sync"), "{}", view.footer);
}
