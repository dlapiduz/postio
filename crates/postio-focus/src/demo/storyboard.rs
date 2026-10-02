//! Focus's storyboard runner: play a storyboard against a real window over a
//! seeded store, and record what happened at every step
//! (specs/008-storyboards, contracts/runner.md).
//!
//! The same promises as Classic's runner, kept here for Focus:
//!
//! * **The real window.** `demo::start` runs the host and adopts the window
//!   over it as the application starts, so the real verbs act and the events
//!   they raise reach the toast. Nothing is dialled: the store is in memory
//!   and sync never starts.
//! * **Commands are pressed.** A `command` step looks up the binding Focus
//!   has for it in the context the window is in at that moment, through the
//!   same fallback layers the resolver uses and only for commands Focus
//!   offers, and presses it. A command the context does not bind fails the
//!   step as unbound; it is never dispatched by name.
//! * **Keys go along the focus chain** (research R3), so a key swallowed by
//!   a dialog or lost on a removed widget is reported, not assumed delivered.
//! * **What is checked is what is on screen**: `FocusWindow::observe`.
//!
//! The clock is frozen at a fixed instant, animations are off, the window has
//! a fixed size, and frames come from one renderer (research R5). The
//! process-level half is the example binary's hermetic re-exec.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::glib;
use postio_config::paths::Platform;
use postio_core::{CommandId, ConnectionState, Event, Frontend};
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
use postio_widgets::storyboard::{deliver, outline, settle};

use super::{Seed, Started};
use crate::window::FocusWindow;

/// What Focus does not observe, so a check on it is not applicable rather
/// than a pass (contracts/observation.md § Focus): Back is a cascade, not a
/// stack; the list is windowed over the store, so a row's index is not its
/// place on screen; the open message shows one message, not a conversation.
pub const UNOBSERVED: &[&str] = &["back_depth", "rows.first_visible", "reading.focused"];

/// The instant the clock is frozen at: 16:09 on the day after the seed's
/// anchor (2026-06-01), the time the references were drawn at, so the demo
/// inbox's "today" rows read as the design draws them.
const FROZEN_AT: &str = "2026-06-02T16:09:00Z";

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
    /// Axes the storyboard asked for that this runner does not have,
    /// recorded in the run so the page can say so.
    pub ignored_axes: Vec<String>,
    /// The review key of the tree this runs on.
    pub tree_key: String,
    /// The commit, for information.
    pub commit: String,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            out: None,
            frames: true,
            delivery: Delivery::Chain,
            stride: 2,
            variant: BTreeMap::new(),
            ignored_axes: Vec::new(),
            tree_key: String::new(),
            commit: String::new(),
        }
    }
}

/// The variants `board` asks Focus for, and the axes it asked for that
/// Focus does not have.
pub fn variants_for(board: &Storyboard) -> (Vec<BTreeMap<String, String>>, Vec<String>) {
    postio_storyboard::apply::variants(board, &runner_info())
}

/// What Focus's runner can do, for applicability and `runner list`.
pub fn runner_info() -> RunnerInfo {
    let axes = [
        ("scheme", &["light", "dark"][..]),
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
        app: App::Focus,
        seeds: Seed::ALL.iter().map(|seed| seed.id().to_owned()).collect(),
        presets: Vec::new(),
        axes,
    }
}

/// Seeds a store, starts the host over it and adopts a Focus window, the way
/// the application starts. The returned window is presented and waited on
/// until its first rows are drawn.
async fn acting(seed: Seed, size: (i32, i32)) -> Result<Started, String> {
    let (database, account) = match seed {
        Seed::Small => super::demo().await,
        Seed::Empty => super::empty_demo().await,
        Seed::LongNewsletter => {
            let (database, account) = super::demo().await;
            super::treatment_demo(&database, account, "30").await;
            (database, account)
        }
    };
    let config = postio_config::Config::from_toml_str(super::CONFIG)
        .map_err(|error| format!("the demo's config: {error}"))?;
    let started = super::start(database, account, &config, size)?;
    started.sink.emit(Event::ConnectionChanged {
        account,
        state: ConnectionState::Online,
    });
    wait_for_first_page(&started.window, seed);
    Ok(started)
}

/// Turns the main loop until the inbox's rows are drawn (or, for the empty
/// seed, until a moment has passed), so step 0 is a window with mail in it.
fn wait_for_first_page(window: &FocusWindow, seed: Seed) {
    let deadline = Instant::now() + Duration::from_secs(20);
    let context = glib::MainContext::default();
    let heartbeat =
        glib::timeout_add_local(Duration::from_millis(10), || glib::ControlFlow::Continue);
    while Instant::now() < deadline {
        let drawn = window.pane().is_some_and(|pane| {
            let rows = pane.rows_on_screen();
            !rows.is_empty() && rows.iter().all(|row| !row.drawn().texts.is_empty())
        });
        if drawn || (seed == Seed::Empty && window.observe().rows.count == Some(0)) {
            break;
        }
        context.iteration(true);
    }
    heartbeat.remove();
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

/// Applies what of the variant a window can take: scheme, size, text scale.
/// Returns the requested axes this runner does not have.
fn apply_variant(variant: &BTreeMap<String, String>) -> ((i32, i32), Vec<String>) {
    let scheme = match variant.get("scheme").map(String::as_str) {
        Some("dark") => adw::ColorScheme::ForceDark,
        _ => adw::ColorScheme::ForceLight,
    };
    adw::StyleManager::default().set_color_scheme(scheme);
    let size = match variant.get("width").map(String::as_str) {
        Some("wide") => (1600, 900),
        Some("narrow") => (900, 700),
        _ => (1280, 800),
    };
    if let Some(scale) = variant.get("text").and_then(|t| t.parse::<f64>().ok())
        && let Some(settings) = gtk::Settings::default()
    {
        let base = settings.gtk_xft_dpi();
        settings.set_gtk_xft_dpi((f64::from(base) * scale / 100.0) as i32);
    }
    let known = runner_info().axes;
    let ignored = variant
        .keys()
        .filter(|axis| !known.contains_key(*axis))
        .cloned()
        .collect();
    (size, ignored)
}

/// The step's checks and prose for Focus -- an override's, if it has one
/// -- and the reason it is skipped, if it is.
fn for_focus<'a>(
    board: &'a Storyboard,
    step: &'a Step,
    number: usize,
) -> (&'a Checks, Option<&'a str>, Option<String>) {
    let overrides = board.overrides.get(&App::Focus);
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

fn context_name(window: &FocusWindow) -> String {
    format!("{:?}", window.key_context()).to_lowercase()
}

/// Presses every chord of `binding`, in order.
fn press(window: &FocusWindow, binding: &Binding, delivery: Delivery) -> StepOutcome {
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
    window: &FocusWindow,
    input: &Input,
    acting: &Started,
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
            // Only what Focus offers is bound, as the resolver it presses
            // keys through binds it: `*` flags in Classic and is nothing here.
            let (keymap, _) = Keymap::from_commands_for(&window.keymap(), Frontend::Focus);
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
                let Some(composer) = window.composer() else {
                    return false;
                };
                if composer.focused_field() != Some(postio_widgets::composer::Field::Body) {
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
                        "event {} is not supported by Focus's runner yet",
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
    window: &FocusWindow,
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
    window: &FocusWindow,
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

/// Plays `board` on Focus and returns the run, writing it to the output tree
/// when `options.out` is set.
///
/// Awaited on the GTK main thread after `adw::init`, inside a multi-threaded
/// tokio runtime -- the arrangement `shot` and the focus suite already make.
pub async fn run(board: &Storyboard, options: &Options) -> Run {
    let seed = board.seed.clone().unwrap_or_else(|| "small".to_owned());
    let mut played = Run {
        storyboard: Played {
            name: board.name.clone(),
            hash: std::fs::read_to_string(&board.path)
                .map(|text| postio_storyboard::key::key(&[&text]))
                .unwrap_or_default(),
        },
        app: App::Focus,
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
        .and_then(|out| RunWriter::new(out, App::Focus, &board.name, &options.variant).ok());
    let finish = |played: Run| {
        if let Some(writer) = &writer {
            let _ = writer.write(&played);
        }
        played
    };

    if let Applicability::NotApplicable(reasons) = applies(board, App::Focus, &runner_info()) {
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
            reason: format!("Focus cannot build the seed `{seed}`"),
        };
        return finish(played);
    };

    postio_ui::clock::freeze(
        chrono::DateTime::parse_from_rfc3339(FROZEN_AT)
            .expect("a fixed instant")
            .with_timezone(&chrono::Local),
    );
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_enable_animations(false);
    }
    let (size, ignored) = apply_variant(&options.variant);
    played.ignored_axes = ignored;
    played
        .ignored_axes
        .extend(options.ignored_axes.iter().cloned());

    let started = match acting(base, size).await {
        Ok(started) => started,
        Err(message) => {
            played.status = Status::Error { message };
            postio_ui::clock::thaw();
            return finish(played);
        }
    };
    let window = started.window.clone();
    deliver::drain();
    let end = |started: Started| {
        adw::StyleManager::default().set_color_scheme(adw::ColorScheme::Default);
        started.finish();
        postio_ui::clock::thaw();
    };
    if board.preset.is_some() {
        played.status = Status::NotApplicable {
            reason: format!(
                "Focus has no preset `{}`",
                board.preset.as_deref().unwrap_or_default()
            ),
        };
        end(started);
        return finish(played);
    }

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
        let (checks, expect, skip) = for_focus(board, step, number);
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
            match deliver_input(&window, &step.input, &started, options.delivery) {
                Ok((outcome, delivered)) => (outcome, Some(delivered)),
                Err(reason) => {
                    played.status = Status::Unavailable { reason };
                    end(started);
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
    end(started);
    finish(played)
}

/// Commands the generated pass never presses, because they reach outside the
/// window: the network (FR-013), the desktop, or a file chooser that would
/// sit waiting for a person. Reported as skipped, with the reason, so the
/// list is visible and argued with rather than silently shrinking coverage.
/// The same list Classic's pass keeps (`postio-app`'s `demo::storyboard`):
/// what leaves the machine does so from either app.
pub const NEVER_PRESSED: &[(&str, &str)] = &[
    ("refresh", "syncs, which dials the server"),
    ("retry_send", "sends, which dials the server"),
    ("show_images", "fetches remote images"),
    ("always_show_images", "fetches remote images"),
    ("unsubscribe", "follows an unsubscribe link off the machine"),
    ("edit_config", "opens an external editor"),
    ("edit_externally", "opens an external editor"),
    ("open_part", "hands a part to another application"),
    (
        "open_part_externally",
        "hands a part to another application",
    ),
    ("save_part", "opens a file chooser"),
    ("save_all_parts", "opens a file chooser"),
    ("attach_file", "opens a file chooser"),
    ("insert_image", "opens a file chooser"),
    ("add_account", "may open a browser to sign in"),
    ("update_credential", "may open a browser to sign in"),
];

/// How each context is reached from a fresh window: the commands that put
/// the keyboard there, in Focus's own words. Focus has no panes to cycle
/// between: its contexts are the list, the surfaces a key opens over it --
/// the search bar, Filtered, the digest window, the open message, the
/// composer -- and nothing else. A context the setup does not land in is
/// reported, not pressed in.
pub const CONTEXTS: &[(&str, &[&str])] = &[
    // Focus opens with no cursor row, so a `j` puts it on the first.
    ("list", &["next_message"]),
    ("search", &["search"]),
    ("filtered", &["go_to_filtered"]),
    // The digest row is the second row of the small seed's inbox.
    ("digest", &["next_message", "next_message", "open_message"]),
    // The first row is a message with an invitation card.
    ("reader", &["next_message", "open_message"]),
    ("composer", &["compose"]),
];

/// One context's coverage, or why it could not be reached.
#[derive(Debug, Clone)]
pub struct ContextCoverage {
    /// The context.
    pub context: String,
    /// What each bound command came to.
    pub presses: Vec<postio_storyboard::coverage::Press>,
    /// Commands not pressed, and why.
    pub skipped: Vec<(String, String)>,
    /// Set when the setup did not reach this context.
    pub unreachable: Option<String>,
}

impl ContextCoverage {
    /// As JSON, for `coverage.json`.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "context": self.context,
            "unreachable": self.unreachable,
            "skipped": self.skipped.iter()
                .map(|(command, why)| serde_json::json!({ "command": command, "why": why }))
                .collect::<Vec<_>>(),
            "presses": self.presses,
        })
    }
}

/// The generated pass (spec US6): every command bound in every context,
/// each from a fresh window in that context's starting state, judged on
/// whether anything a person can see changed.
pub async fn every_command(gap_list: &std::path::Path) -> Result<Vec<ContextCoverage>, String> {
    use postio_storyboard::coverage::{Effect, Press, judge, load_gaps};
    let gaps = load_gaps(gap_list)?;
    let gaps = gaps.as_slice();

    // One context, to run a gap list's entries again without the other
    // five minutes: `POSTIO_STORYBOARD_CONTEXT=composer`.
    let only = std::env::var("POSTIO_STORYBOARD_CONTEXT").ok();
    let mut all = Vec::new();
    for (context, setup) in CONTEXTS {
        if only.as_deref().is_some_and(|only| only != *context) {
            continue;
        }
        let mut coverage = ContextCoverage {
            context: (*context).to_owned(),
            presses: Vec::new(),
            skipped: Vec::new(),
            unreachable: None,
        };
        // The commands bound here, found from a window set up for it.
        let Some(started) = fresh(setup).await else {
            coverage.unreachable = Some("the seeded store fed no window".to_owned());
            all.push(coverage);
            continue;
        };
        let reached = context_name(&started.window);
        if reached != *context {
            coverage.unreachable =
                Some(format!("setup {setup:?} left the keyboard in `{reached}`"));
            started.finish();
            all.push(coverage);
            continue;
        }
        let (keymap, _) = Keymap::from_commands_for(&started.window.keymap(), Frontend::Focus);
        let here = started.window.key_context();
        let mut commands: Vec<String> = keymap
            .entries()
            .filter(|(layer, _, _)| here.chain().contains(layer))
            .map(|(_, _, command)| command.to_owned())
            .collect();
        commands.sort();
        commands.dedup();
        started.finish();

        for command in commands {
            if let Some((_, why)) = NEVER_PRESSED.iter().find(|(id, _)| *id == command) {
                coverage.skipped.push((command, (*why).to_owned()));
                continue;
            }
            let Some(started) = fresh(setup).await else {
                continue;
            };
            let window = started.window.clone();
            let settings = settle::Settings {
                stride: 2,
                ..settle::Settings::default()
            };
            let before_frame = settle::settle(window.upcast_ref(), &settings);
            let before = window.observe();
            // Typing wins: with the keyboard in a text field, a command bound
            // to a bare key is a letter, not a command, and that is right.
            let (keymap, _) = Keymap::from_commands_for(&window.keymap(), Frontend::Focus);
            let bare = window
                .key_context()
                .chain()
                .iter()
                .find_map(|layer| keymap.binding_for(*layer, &command))
                .and_then(|binding| binding.chords().first().cloned())
                .is_some_and(|chord| {
                    let shown = chord.to_string();
                    shown.chars().count() == 1
                        || shown.starts_with("shift+") && shown.chars().count() == 7
                });
            if before.keyboard.typing && bare {
                coverage.presses.push(Press {
                    command: command.clone(),
                    context: (*context).to_owned(),
                    effect: Effect::Typing,
                });
                started.finish();
                continue;
            }
            let outcome = deliver_input(
                &window,
                &Input::Command(command.clone()),
                &started,
                Delivery::Chain,
            );
            let press = match outcome {
                Ok((StepOutcome::Delivered, _)) => {
                    let after_frame = settle::settle(window.upcast_ref(), &settings);
                    let after = window.observe();
                    judge(
                        &command,
                        context,
                        (&before, &before_frame.hash),
                        (&after, &after_frame.hash),
                        gaps,
                    )
                }
                Ok((StepOutcome::Dropped, _)) => Press {
                    command: command.clone(),
                    context: (*context).to_owned(),
                    effect: Effect::Dropped,
                },
                _ => Press {
                    command: command.clone(),
                    context: (*context).to_owned(),
                    effect: Effect::Unbound,
                },
            };
            coverage.presses.push(press);
            started.finish();
        }
        all.push(coverage);
    }
    postio_ui::clock::thaw();
    adw::StyleManager::default().set_color_scheme(adw::ColorScheme::Default);
    Ok(all)
}

/// A fresh, seeded, acting window, with `setup` pressed.
async fn fresh(setup: &[&str]) -> Option<Started> {
    postio_ui::clock::freeze(
        chrono::DateTime::parse_from_rfc3339(FROZEN_AT)
            .expect("a fixed instant")
            .with_timezone(&chrono::Local),
    );
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_enable_animations(false);
    }
    let (size, _) = apply_variant(&BTreeMap::new());
    let started = acting(Seed::Small, size).await.ok()?;
    deliver::drain();
    for command in setup {
        let _ = deliver_input(
            &started.window,
            &Input::Command((*command).to_owned()),
            &started,
            Delivery::Chain,
        );
        pump(Duration::from_millis(200));
    }
    Some(started)
}
