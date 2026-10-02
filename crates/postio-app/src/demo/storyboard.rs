//! Classic's storyboard runner: play a storyboard against a real window over
//! a seeded store, and record what happened at every step
//! (specs/008-storyboards, contracts/runner.md).
//!
//! # What makes it a test of the app and not of a layer
//!
//! * **The real composition root.** The window is fed by `feed_the_window`
//!   over a real `Wiring`, the real command bus is wired to the real verbs,
//!   and the events those verbs raise are drained onto the panes the way
//!   `run` drains them. Without that drain an archive lands in the store and
//!   no toast ever says so.
//! * **Commands are pressed.** A `command` step looks up the binding Classic
//!   has for it in the context the window is in at that moment, through the
//!   same fallback layers the resolver uses, and presses it. A command the
//!   context does not bind fails the step as unbound; it is never dispatched
//!   by name, which would pass a broken binding.
//! * **Keys go along the focus chain** (research R3), so a key swallowed by
//!   a dialog or lost on a removed widget is reported, not assumed delivered.
//! * **What is checked is what is on screen**: `Window::observe`.
//!
//! # What it holds still
//!
//! The clock is frozen at the seed's day, animations are off, the window has
//! a fixed size, and frames come from one renderer (research R5). The
//! process-level half -- time zone, locale, fonts, `GSK_RENDERER` -- is the
//! example binary's hermetic re-exec, because it has to happen before GTK
//! starts.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::glib;
use postio_config::paths::Platform;
use postio_core::bridge::{Bridge, EventHub, EventSink};
use postio_core::state::SharedState;
use postio_core::{CommandId, ConnectionState, Event};
use postio_gtk::storyboard::{deliver, outline, settle};
use postio_gtk::window::Window;
use postio_model::ids::AccountId;
use postio_session::{Wiring, actions};
use postio_storyboard::apply::{App, Applicability, RunnerInfo, applies};
use postio_storyboard::check::{self, History};
use postio_storyboard::format::{
    Checks, EnvEvent, Input, Routing, Step, StepRef, Storyboard, Wait,
};
use postio_storyboard::run::{
    Delivered, Delivery, Frame, Played, Run, RunWriter, Settle, Status, StepOutcome, StepRun,
    status,
};
use postio_ui::keymap::{Binding, Keymap};
use postio_ui::observe::Observation;

use super::{DemoOptions, Preset, Seed};

/// What Classic does not observe, so a check on it is not applicable rather
/// than a pass (contracts/observation.md § Classic).
pub const UNOBSERVED: &[&str] = &["back_depth", "rows.first_visible", "banner.title"];

/// The instant the clock is frozen at: the seed's anchor (2026-06-01 09:00
/// UTC) plus one day, so relative dates read as the design draws them.
const FROZEN_AT: &str = "2026-06-02T09:00:00Z";

/// How to play a storyboard.
#[derive(Debug, Clone)]
pub struct Options {
    /// Where the output tree goes; `None` writes nothing.
    pub out: Option<PathBuf>,
    /// Whether frames are sampled, drawn and written.
    pub frames: bool,
    /// How input reaches the window.
    pub delivery: Delivery,
    /// Sample a frame every this many ticks (research R4: two, because one
    /// capture costs more than a frame).
    pub stride: u32,
    /// The variant, axis by axis.
    pub variant: BTreeMap<String, String>,
    /// The review key of the tree this runs on.
    pub tree_key: String,
    /// The commit, for information.
    pub commit: String,
    /// Axes the storyboard asked for that this app does not have, from
    /// `postio_storyboard::apply::variants`, recorded on the run.
    pub ignored_axes: Vec<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            out: None,
            frames: true,
            delivery: Delivery::Chain,
            stride: 2,
            variant: BTreeMap::new(),
            tree_key: String::new(),
            commit: String::new(),
            ignored_axes: Vec::new(),
        }
    }
}

/// What Classic's runner can do, for applicability and `runner list`.
pub fn runner_info() -> RunnerInfo {
    let axes = [
        ("scheme", &["light", "dark"][..]),
        ("contrast", &["normal", "high"][..]),
        ("width", &["wide", "normal", "narrow"][..]),
        ("text", &["100", "200"][..]),
    ]
    .into_iter()
    .map(|(axis, values)| {
        (
            axis.to_owned(),
            values.iter().map(|value| (*value).to_owned()).collect(),
        )
    })
    .collect();
    RunnerInfo {
        app: App::Classic,
        seeds: Seed::ALL.iter().map(|seed| seed.id().to_owned()).collect(),
        presets: preset_ids(),
        axes,
    }
}

/// Every preset id `Preset::from_id` understands.
fn preset_ids() -> Vec<String> {
    let panes = [
        "accounts",
        "filters",
        "composing",
        "appearance",
        "keyboard",
        "storage",
        "privacy",
        "configfile",
    ];
    let others = [
        "settings/account-weights",
        "settings/account-form",
        "settings/account-tested",
        "settings/signature-editor",
        "settings/account-mailboxes",
        "add-account/route",
        "add-account/browser",
        "add-account/syncwindow",
        "compose",
        "locked",
        "search-panels",
    ];
    let mut ids: Vec<String> = std::iter::once("settings".to_owned())
        .chain(panes.iter().map(|pane| format!("settings/{pane}")))
        .chain(others.iter().map(|id| (*id).to_owned()))
        .collect();
    ids.retain(|id| Preset::from_id(id).is_some());
    ids
}

/// A window over a seeded store whose verbs really act.
struct Acting {
    /// Where an `event` step's host event is emitted.
    sink: EventSink,
    /// The seeded account, for the events that name one.
    account: AccountId,
    /// What feeding the window built, for presets that need it.
    wired: &'static crate::Wired,
}

/// Seeds a store, feeds `window` from it, and wires the real verbs and the
/// event drain, the way `run` does. Everything is leaked: it lives as long
/// as the window, and dropping a bridge would stop it answering.
async fn acting(window: &Window, options: &DemoOptions) -> Option<Acting> {
    let database = postio_storage::test_support::memory().await;
    let directory = tempfile::tempdir().ok()?;
    let blobs = postio_storage::BlobStore::open(
        directory.keep(),
        &postio_storage::test_support::blob_keys(),
    )
    .ok()?;
    let report = super::seed_store(&database, options).await;
    let account = report.account.id;
    // Opening a store builds its search index; a seeded one has to be told,
    // or every search storyboard searches nothing.
    postio_session::ensure_search_index(&database).await.ok()?;

    let state = SharedState::default();
    let bus = actions::wire(
        postio_core::dispatch::DispatcherBuilder::new(),
        actions::Actions::new(database.clone(), state.clone()),
    )
    .build();
    let wired_ids: Vec<CommandId> = bus.wired().collect();
    let hub = EventHub::new();
    let bridge = Bridge::builder().build_with_events(bus, hub.sink()).ok()?;
    let wiring = Wiring::new(
        database,
        blobs,
        bridge.handle(),
        hub.sink(),
        bridge.commands(),
    );
    let wiring: &'static Wiring = Box::leak(Box::new(wiring));
    let wired = crate::feed_the_window(window, wiring).await?;
    wired.feeds.apply(&Event::ConnectionChanged {
        account,
        state: ConnectionState::Online,
    });
    crate::commands::install(
        window,
        &wired.feeds,
        state.clone(),
        wiring.commands.clone(),
        wired_ids,
    );
    let notifier = crate::notifications::Notifier::new(
        wiring.database.clone(),
        wiring.store.clone(),
        wiring.runtime.clone(),
        Default::default(),
    );
    crate::commands::drain(
        window,
        &wired.feeds,
        hub.subscribe("window"),
        notifier,
        state,
    );
    let sink = hub.sink();
    let wired: &'static crate::Wired = Box::leak(Box::new(wired));
    Box::leak(Box::new(bridge));
    Box::leak(Box::new(hub));
    if options.base() != Seed::Empty {
        super::wait_for_first_page(window);
    }
    Some(Acting {
        sink,
        account,
        wired,
    })
}

/// Turns the main loop for `duration`, so timers and frames happen.
fn pump(duration: Duration) {
    let context = glib::MainContext::default();
    let heartbeat =
        glib::timeout_add_local(Duration::from_millis(10), || glib::ControlFlow::Continue);
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        context.iteration(true);
    }
    heartbeat.remove();
}

/// Applies what of the variant a window can take: scheme, contrast, size,
/// text scale. Returns the requested axes this runner does not have.
fn apply_variant(window: &Window, variant: &BTreeMap<String, String>) -> Vec<String> {
    let scheme = match variant.get("scheme").map(String::as_str) {
        Some("dark") => adw::ColorScheme::ForceDark,
        _ => adw::ColorScheme::ForceLight,
    };
    adw::StyleManager::default().set_color_scheme(scheme);
    if variant.get("contrast").map(String::as_str) == Some("high") {
        window.add_css_class(postio_gtk::style::HIGH_CONTRAST_CLASS);
    }
    let (width, height) = match variant.get("width").map(String::as_str) {
        Some("wide") => (1600, 900),
        Some("narrow") => (900, 700),
        _ => (1280, 800),
    };
    window.set_default_size(width, height);
    if let Some(scale) = variant.get("text").and_then(|t| t.parse::<f64>().ok())
        && let Some(settings) = gtk::Settings::default()
    {
        let base = settings.gtk_xft_dpi();
        settings.set_gtk_xft_dpi((f64::from(base) * scale / 100.0) as i32);
    }
    let known = runner_info().axes;
    variant
        .keys()
        .filter(|axis| !known.contains_key(*axis))
        .cloned()
        .collect()
}

/// The step's checks and prose for Classic -- an override's, if it has one
/// -- and the reason it is skipped, if it is.
fn for_classic<'a>(
    board: &'a Storyboard,
    step: &'a Step,
    number: usize,
) -> (&'a Checks, Option<&'a str>, Option<String>) {
    let overrides = board.overrides.get(&App::Classic);
    let by_id = step
        .id
        .as_ref()
        .and_then(|id| overrides.and_then(|o| o.get(id)));
    let by_index = overrides.and_then(|o| o.get(&number.to_string()));
    let over = by_id.or(by_index);
    let checks = over.and_then(|o| o.check.as_ref()).unwrap_or(&step.check);
    let expect = over
        .and_then(|o| o.expect.as_deref())
        .or(step.expect.as_deref());
    let skip = over
        .and_then(|o| o.skip.as_ref())
        .map(|skip| skip.reason.clone());
    (checks, expect, skip)
}

/// The step as written, for a reader of the run.
fn describe(input: &Input) -> String {
    match input {
        Input::Command(id) => format!("command {id}"),
        Input::Key(chord) => format!("key {chord}"),
        Input::Type(text) => format!("type {text:?}"),
        Input::Wait(Wait::Ms(ms)) => format!("wait {ms} ms"),
        Input::Wait(Wait::Until(_)) => "wait until".to_owned(),
        Input::Event(event) => format!("event {}", event_name(*event)),
    }
}

fn event_name(event: EnvEvent) -> String {
    serde_json::to_value(event)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{event:?}"))
}

fn context_name(window: &Window) -> String {
    format!("{:?}", window.key_context()).to_lowercase()
}

/// Presses every chord of `binding`, in order.
fn press(window: &Window, binding: &Binding, delivery: Delivery) -> StepOutcome {
    for chord in binding.chords() {
        match delivery {
            Delivery::Direct => {
                let Ok((key, modifiers)) = deliver::chord_to_gdk(chord) else {
                    return StepOutcome::Dropped;
                };
                window.handle_key(key, modifiers);
            }
            Delivery::Chain | Delivery::Real => match deliver::press(window.upcast_ref(), chord) {
                Ok(deliver::Delivery::Delivered { .. }) => {}
                Ok(deliver::Delivery::Dropped) | Err(_) => return StepOutcome::Dropped,
            },
        }
        deliver::drain();
    }
    StepOutcome::Delivered
}

/// Delivers one step's input: what happened to it, and what was delivered.
/// `Err` is a step this runner cannot play at all, which makes the run
/// unavailable rather than failed.
fn deliver_input(
    window: &Window,
    input: &Input,
    acting: &Acting,
    delivery: Delivery,
) -> Result<(StepOutcome, Delivered), String> {
    let mut delivered = Delivered {
        step: describe(input),
        chord: None,
        context: None,
    };
    let outcome = match input {
        Input::Command(id) => {
            CommandId::from_str(id).map_err(|_| format!("unknown command `{id}`"))?;
            let context = window.key_context();
            delivered.context = Some(context_name(window));
            let (keymap, _) = Keymap::from_commands(&window.keymap_in_force());
            let binding = context
                .chain()
                .iter()
                .find_map(|layer| keymap.binding_for(*layer, id))
                .cloned();
            match binding {
                None => StepOutcome::Unbound {
                    context: context_name(window),
                },
                Some(binding) => {
                    delivered.chord = Some(binding.to_string());
                    press(window, &binding, delivery)
                }
            }
        }
        Input::Key(chord) => {
            let expanded = postio_config::keys::expand_mod(chord, Platform::Freedesktop);
            let binding: Binding = expanded
                .parse()
                .map_err(|error| format!("`{chord}` is not a chord: {error:?}"))?;
            delivered.chord = Some(binding.to_string());
            delivered.context = Some(context_name(window));
            press(window, &binding, delivery)
        }
        Input::Type(text) => {
            // The composer's body is a web view over an editable document,
            // not a `GtkText`; it takes text through the editor itself.
            let composer_body = |_widget: &gtk::Widget, typed: &str| -> bool {
                if !window.has_composer() {
                    return false;
                }
                let composer = window.composer();
                if composer.focused_field() != Some(postio_gtk::composer::Field::Body) {
                    return false;
                }
                let escaped = serde_json::to_string(typed).unwrap_or_default();
                composer.test_body_eval(&format!(
                    "document.execCommand('insertText', false, {escaped})"
                ));
                true
            };
            match deliver::type_text(window.upcast_ref(), text, Some(&composer_body)) {
                deliver::TypeOutcome::Typed => StepOutcome::Delivered,
                deliver::TypeOutcome::NothingToTypeInto => StepOutcome::NothingToTypeInto,
            }
        }
        Input::Wait(Wait::Ms(ms)) => {
            pump(Duration::from_millis(*ms));
            StepOutcome::Delivered
        }
        // Waited for by the caller, which evaluates the checks.
        Input::Wait(Wait::Until(_)) => StepOutcome::Delivered,
        Input::Event(event) => {
            let account = acting.account;
            let emitted = match event {
                EnvEvent::MailboxesChanged => acting.sink.emit(Event::MailboxesChanged { account }),
                EnvEvent::ConnectionLost => acting.sink.emit(Event::ConnectionChanged {
                    account,
                    state: ConnectionState::Offline,
                }),
                EnvEvent::ConnectionRestored => acting.sink.emit(Event::ConnectionChanged {
                    account,
                    state: ConnectionState::Online,
                }),
                EnvEvent::BackfillProgress => acting.sink.emit(Event::BackfillProgress {
                    account,
                    done: 12_400,
                    total: 81_744,
                    footprint: None,
                }),
                other => {
                    return Err(format!(
                        "event {} is not supported by Classic's runner yet",
                        event_name(*other)
                    ));
                }
            };
            if !emitted {
                return Err("the event hub has stopped".to_owned());
            }
            StepOutcome::Delivered
        }
    };
    Ok((outcome, delivered))
}

/// The 1-based step a `same_as` names.
fn step_number(board: &Storyboard, reference: &StepRef) -> Option<usize> {
    match reference {
        StepRef::Index(n) => usize::try_from(*n).ok(),
        StepRef::Id(id) => board
            .steps
            .iter()
            .position(|step| step.id.as_deref() == Some(id))
            .map(|i| i + 1),
    }
}

fn region_name(observation: &Observation) -> String {
    serde_json::to_value(observation.keyboard.region)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// Waits until `checks` hold against what the window shows, or `max` passes.
fn wait_until(
    window: &Window,
    board: &Storyboard,
    checks: &Checks,
    history: &[serde_json::Value],
    max: Duration,
) {
    let deadline = Instant::now() + max;
    let resolve = |reference: &StepRef| step_number(board, reference);
    while Instant::now() < deadline {
        let mut observations = history.to_vec();
        observations.push(serde_json::to_value(window.observe()).unwrap_or_default());
        let step = observations.len() - 1;
        let results = check::evaluate(
            checks,
            step,
            &History {
                observations: &observations,
                step_number: &resolve,
                unobserved: UNOBSERVED,
            },
        );
        if results
            .iter()
            .all(|result| result.outcome != check::Outcome::Fail)
        {
            return;
        }
        pump(Duration::from_millis(30));
    }
}

/// Lets a step finish, then records its frames: plain and outlined, plus the
/// extra frames a jump or a blank kept.
fn settle_and_capture(
    window: &Window,
    options: &Options,
    step: usize,
    settings: &settle::Settings,
    writer: Option<&RunWriter>,
) -> (Settle, Option<Frame>, Option<String>) {
    // Settled the same way whether or not frames are kept. "No frames"
    // means nothing is written, not that the step is given less time: a
    // check run that waited less than a filmed one would disagree with it
    // about anything on a timer, which is how a live search's debounce
    // made the two read #1744 differently.
    let settled = settle::settle(window.upcast_ref(), settings);
    if !options.frames {
        let verdict = match settled.verdict {
            settle::Verdict::Settled { ms } => Settle::Settled { ms },
            settle::Verdict::Jumped { .. } => Settle::Jumped { frames: vec![] },
            settle::Verdict::Blanked { .. } => Settle::Blanked { frames: vec![] },
            settle::Verdict::Unsettled { ms } => Settle::Unsettled { ms },
        };
        return (verdict, None, None);
    }
    let mut extra_names = Vec::new();
    if let Some(writer) = writer {
        for (n, texture) in settled.extra.iter().enumerate() {
            let name = RunWriter::extra(step, n + 1);
            if texture.save_to_png(writer.dir().join(&name)).is_ok() {
                extra_names.push(name);
            }
        }
    }
    let verdict = match settled.verdict {
        settle::Verdict::Settled { ms } => Settle::Settled { ms },
        settle::Verdict::Jumped { .. } => Settle::Jumped {
            frames: extra_names.clone(),
        },
        settle::Verdict::Blanked { .. } => Settle::Blanked {
            frames: extra_names.clone(),
        },
        settle::Verdict::Unsettled { ms } => Settle::Unsettled { ms },
    };
    let Some(writer) = writer else {
        return (verdict, None, None);
    };
    let plain = RunWriter::frame(step);
    let frame = settled
        .texture
        .save_to_png(writer.dir().join(&plain))
        .ok()
        .map(|()| Frame {
            path: plain,
            hash: settled.hash.clone(),
        });
    let region = region_name(&window.observe());
    let outlined_name = RunWriter::outlined(step);
    let outlined = outline::outlined(window.upcast_ref(), &region)
        .save_to_png(writer.dir().join(&outlined_name))
        .ok()
        .map(|()| outlined_name);
    (verdict, frame, outlined)
}

fn settle_settings(options: &Options, step: Option<&Step>) -> settle::Settings {
    let mut settings = settle::Settings {
        stride: options.stride.max(1),
        ..settle::Settings::default()
    };
    if let Some(custom) = step.and_then(|step| step.settle.as_ref()) {
        if let Some(ms) = custom.max_ms {
            settings.max = Duration::from_millis(ms);
        }
        if let Some(ms) = custom.watch_ms {
            settings.watch = Duration::from_millis(ms);
        }
    }
    settings
}

fn renderer() -> String {
    std::env::var("GSK_RENDERER").unwrap_or_else(|_| "default".to_owned())
}

/// Plays `board` on Classic and returns the run, writing it to the output
/// tree when `options.out` is set.
///
/// Awaited on the GTK main thread after `adw::init`, inside a multi-threaded
/// tokio runtime -- the arrangement `shot` and the app suite already make,
/// because the store's synchronous reads reach for `block_in_place`.
pub async fn run(board: &Storyboard, options: &Options) -> Run {
    let seed = board.seed.clone().unwrap_or_else(|| "small".to_owned());
    let mut played = Run {
        storyboard: Played {
            name: board.name.clone(),
            hash: std::fs::read_to_string(&board.path)
                .map(|text| postio_storyboard::key::key(&[&text]))
                .unwrap_or_default(),
        },
        app: App::Classic,
        variant: options.variant.clone(),
        ignored_axes: vec![],
        tree_key: options.tree_key.clone(),
        commit: options.commit.clone(),
        delivery: options.delivery,
        renderer: renderer(),
        stride: options.stride,
        seed: seed.clone(),
        preset: board.preset.clone(),
        steps: vec![],
        status: Status::Passed,
    };
    let writer = options
        .out
        .as_ref()
        .and_then(|out| RunWriter::new(out, App::Classic, &board.name, &options.variant).ok());
    let finish = |played: Run| {
        if let Some(writer) = &writer {
            let _ = writer.write(&played);
        }
        played
    };

    if let Applicability::NotApplicable(reasons) = applies(board, App::Classic, &runner_info()) {
        played.status = Status::NotApplicable {
            reason: reasons
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; "),
        };
        return finish(played);
    }
    let Some(base) = Seed::from_id(&seed) else {
        played.status = Status::NotApplicable {
            reason: format!("Classic cannot build the seed `{seed}`"),
        };
        return finish(played);
    };
    let demo = DemoOptions::new().with(base);

    postio_ui::clock::freeze(
        chrono::DateTime::parse_from_rfc3339(FROZEN_AT)
            .expect("a fixed instant")
            .with_timezone(&chrono::Local),
    );
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_enable_animations(false);
    }

    let window = Window::default();
    played.ignored_axes = apply_variant(&window, &options.variant);
    played
        .ignored_axes
        .extend(options.ignored_axes.iter().cloned());
    window.present();
    deliver::drain();

    let Some(acting) = acting(&window, &demo).await else {
        played.status = Status::Error {
            message: "the seeded store fed no window".to_owned(),
        };
        window.destroy();
        postio_ui::clock::thaw();
        return finish(played);
    };
    if let Some(preset) = board.preset.as_deref() {
        let Some(preset) = Preset::from_id(preset) else {
            played.status = Status::NotApplicable {
                reason: format!("Classic has no preset `{preset}`"),
            };
            window.destroy();
            postio_ui::clock::thaw();
            return finish(played);
        };
        preset.apply(&window, Some(acting.wired));
    }
    deliver::drain();

    let resolve = |reference: &StepRef| step_number(board, reference);
    let mut history: Vec<serde_json::Value> = Vec::new();

    // Step 0: the starting state, as the window gives it.
    let (settle, frame, outlined) = settle_and_capture(
        &window,
        options,
        0,
        &settle_settings(options, None),
        writer.as_ref(),
    );
    let observation = window.observe();
    history.push(serde_json::to_value(&observation).unwrap_or_default());
    played.steps.push(StepRun {
        step: 0,
        id: None,
        input: None,
        expect: None,
        outcome: StepOutcome::Delivered,
        observation,
        checks: vec![],
        settle,
        frame,
        outlined,
    });

    for (index, step) in board.steps.iter().enumerate() {
        let number = index + 1;
        let (checks, expect, skip) = for_classic(board, step, number);
        let needs_real = board.routing == Routing::Real || step.routing == Some(Routing::Real);
        let (outcome, input) = if let Some(reason) = skip {
            (StepOutcome::Skipped { reason }, None)
        } else if needs_real && options.delivery != Delivery::Real {
            (
                StepOutcome::NotCovered {
                    delivery: options.delivery,
                },
                Some(Delivered {
                    step: describe(&step.input),
                    chord: None,
                    context: None,
                }),
            )
        } else {
            match deliver_input(&window, &step.input, &acting, options.delivery) {
                Ok((outcome, delivered)) => (outcome, Some(delivered)),
                Err(reason) => {
                    played.status = Status::Unavailable { reason };
                    window.destroy();
                    postio_ui::clock::thaw();
                    return finish(played);
                }
            }
        };

        if outcome == StepOutcome::Delivered {
            let wait_for = match &step.input {
                Input::Wait(Wait::Until(until)) => Some(until),
                _ => step.settle.as_ref().and_then(|s| s.until.as_ref()),
            };
            if let Some(until) = wait_for {
                let max = step
                    .settle
                    .as_ref()
                    .and_then(|s| s.max_ms)
                    .map_or(Duration::from_secs(3), Duration::from_millis);
                wait_until(&window, board, until, &history, max);
            }
        }

        let (settle, frame, outlined) = settle_and_capture(
            &window,
            options,
            number,
            &settle_settings(options, Some(step)),
            writer.as_ref(),
        );
        let observation = window.observe();
        history.push(serde_json::to_value(&observation).unwrap_or_default());
        let results = if outcome == StepOutcome::Delivered {
            check::evaluate(
                checks,
                number,
                &History {
                    observations: &history,
                    step_number: &resolve,
                    unobserved: UNOBSERVED,
                },
            )
        } else {
            vec![]
        };
        played.steps.push(StepRun {
            step: number,
            id: step.id.clone(),
            input,
            expect: expect.map(str::to_owned),
            outcome,
            observation,
            checks: results,
            settle,
            frame,
            outlined,
        });
    }

    played.status = status(&played.steps);
    window.destroy();
    postio_ui::clock::thaw();
    finish(played)
}
