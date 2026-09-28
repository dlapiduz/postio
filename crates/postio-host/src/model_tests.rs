//! The person's own model in Focus's body stage (spec 007 T153, T152):
//! US12 scenario 6 and SC-016, through the host as `postio-focus` drives it.
//!
//! Every case runs the model client against `postio_ai::fake`: nothing here
//! opens a connection, loopback included.

use std::sync::Arc;

use postio_ai::fake::{FakeRuntime, Reply};
use postio_config::FocusConfig;
use postio_storage::repository::MarkerSource;

use crate::tests::{
    APPROVE, World, egress, eventually, letter_from_tove, marker_on, saturday_noon,
};

/// `[focus]` naming a model on this computer, with `needs_action` as given.
fn with_model(needs_action: bool) -> FocusConfig {
    postio_config::Config::from_toml_str(&format!(
        "[focus.model]\nendpoint = \"http://127.0.0.1:11434/v1\"\nmodel = \"a-small-model\"\n\
         needs_action = {needs_action}\n"
    ))
    .expect("a config")
    .focus
}

/// Focus on over `world` with `config`, its model reached through
/// `runtime`, and the catch-up done.
fn focus_with(world: &World, config: FocusConfig, runtime: &FakeRuntime) {
    let focus = world.host().enable_focus(
        crate::FocusSetup::default()
            .with_config(config)
            .with_model_transport(Arc::new(runtime.clone())),
    );
    eventually(world, || focus.caught_up().then_some(()));
}

fn the_question(world: &World) -> postio_model::MessageId {
    letter_from_tove(
        world,
        saturday_noon(),
        &format!("Hi,\n\n{APPROVE}\n\nThanks,\nTove"),
        |_| {},
    )
}

#[test]
fn with_no_model_section_nothing_connects_and_the_detector_marks() {
    // SC-016: no `[focus.model]`, and the marker still comes -- from the
    // built-in detector -- with zero connection attempts to any runtime.
    let world = World::new();
    let question = the_question(&world);
    let runtime = FakeRuntime::always(Reply::content(
        r#"{"kind":"todo","quote":"Thanks","due":""}"#,
    ));

    focus_with(&world, FocusConfig::default(), &runtime);

    let marker = marker_on(&world, question).expect("the detector's marker");
    assert_eq!(marker.source, MarkerSource::Detector);
    assert_eq!(runtime.connections(), 0, "no connection was attempted");
    assert!(egress(&world).is_empty(), "{:?}", egress(&world));
}

#[test]
fn with_a_model_connected_its_marker_is_written_and_its_call_logged() {
    // US12 scenario 6, first half: the model answers in the detector's
    // place, the marker quotes the message's own sentence, and the call is
    // in the egress log under `model`.
    let world = World::new();
    let question = the_question(&world);
    let runtime = FakeRuntime::always(Reply::content(format!(
        r#"{{"kind":"todo","quote":"{APPROVE}","due":"2026-10-02"}}"#
    )));

    focus_with(&world, with_model(true), &runtime);

    let marker = marker_on(&world, question).expect("the model's marker");
    assert_eq!(marker.source, MarkerSource::Model);
    assert_eq!(marker.kind, postio_model::listing::MarkerKind::Todo);
    assert_eq!(marker.excerpt.as_deref(), Some(APPROVE));
    assert!(marker.due_at.is_some());
    assert_eq!(runtime.connections(), 1);
    let logged = eventually(&world, || {
        let logged = egress(&world);
        (!logged.is_empty()).then_some(logged)
    });
    assert_eq!(logged, ["model"]);
}

#[test]
fn with_the_model_not_running_the_detector_answers() {
    // US12 scenario 6, second half: nothing waits for the model.
    let world = World::new();
    let question = the_question(&world);
    let runtime = FakeRuntime::refusing();

    focus_with(&world, with_model(true), &runtime);

    let marker = marker_on(&world, question).expect("the detector's marker");
    assert_eq!(marker.source, MarkerSource::Detector);
    assert_eq!(marker.excerpt.as_deref(), Some(APPROVE));
}

#[test]
fn a_model_whose_needs_action_switch_is_off_is_not_asked() {
    // FR-166: each feature has its own switch.
    let world = World::new();
    let question = the_question(&world);
    let runtime = FakeRuntime::always(Reply::content(r#"{"kind":"none","quote":"","due":""}"#));

    focus_with(&world, with_model(false), &runtime);

    let marker = marker_on(&world, question).expect("the detector's marker");
    assert_eq!(marker.source, MarkerSource::Detector);
    assert_eq!(runtime.connections(), 0);
}
