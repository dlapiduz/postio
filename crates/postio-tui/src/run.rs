//! The binary's life: open the store, enter the terminal, loop, leave. Everything that decides is
//! `crate::app::update`; this carries inputs to it and does the effects it asks for. The terminal
//! opens the store itself, so one Postio at a time has it.

use std::io;
use std::process::ExitCode;
use std::sync::Arc;

use crossterm::event::{Event as TerminalEvent, EventStream};
use futures_util::StreamExt;
use postio_client::Client;
use postio_client::protocol::ClientKind;
use postio_host::Host;
use postio_model::ListScope;
use postio_model::listing::MailStore;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::app::{App, Effect, Input, update};
use crate::caps::{Background, Colour};
use crate::input::Keys;
use crate::term::{Mode, Session, Stdout};
use crate::theme::Theme;

/// Upkeep (body index, header repair, disk reclaim) starts this long after the store opens, so
/// the first pages have the runtime to themselves.
const IDLE_PASSES_AFTER_OPENING: std::time::Duration = std::time::Duration::from_millis(750);

/// Says on stderr what it waits on (a keyring prompt can hold it for half a minute).
/// `Err` is the line to print before exiting, such as the sentence saying another Postio has the
/// store.
pub fn open(
    config_path: Option<&std::path::Path>,
    secrets: Arc<dyn postio_account::secret::SecretStore>,
) -> Result<Host, String> {
    let say_so = saying(|line: &str| eprintln!("{line}"));
    Host::open(config_path, secrets, &say_so).map_err(|refusal| refused(&refusal))
}

fn refused(refusal: &postio_session::Refusal) -> String {
    use postio_session::{Remedy, Unreadable};
    match refusal.remedy {
        // Trying again meets the same file: name the way forward instead.
        Remedy::StartOver {
            because: Unreadable::AnotherBuild,
            ..
        } => format!(
            "postio-tui: {refusal}\npostio-tui: `postio-store reset` sets it aside and \
             starts a fresh store, keeping your accounts and config.toml"
        ),
        // The same key (#1598): nothing in it decrypts, so no account comes across.
        Remedy::StartOver {
            because: Unreadable::AnotherKey,
            ..
        } => format!(
            "postio-tui: {refusal}\npostio-tui: `postio-store reset` sets it aside, not \
             deleted, and starts a fresh store; add your accounts again in Postio"
        ),
        Remedy::TryAgain => format!("postio-tui: {refusal}"),
    }
}

/// Switches Focus mode on as `[focus]` says, after the store opens and before the first sync, so
/// the filing pass is in every engine before its first pass.
pub fn engage_focus(
    host: &Host,
    config: &postio_config::Config,
    config_path: Option<&std::path::Path>,
) -> postio_host::FocusHandle {
    host.enable_focus(
        postio_host::FocusSetup::from_config(config.focus.clone(), config_path)
            .with_remote_images_path(postio_ui::allowlist::RemoteImageAllowList::path()),
    )
}

/// `None` when the file cannot be watched: edits then wait for a restart. Dropping the watcher
/// stops it.
pub fn follow_focus_config(
    host: &Arc<Host>,
    path: &std::path::Path,
) -> Option<postio_config::watch::ConfigWatcher> {
    let mut service = postio_core::ConfigService::load(path);
    let host = Arc::downgrade(host);
    postio_config::watch::ConfigWatcher::new(path, move |checked| {
        let update = service.apply(checked);
        if update.changed.focus
            && let Some(host) = host.upgrade()
        {
            host.enable_focus(
                postio_host::FocusSetup::from_config(
                    service.config().focus.clone(),
                    Some(service.path()),
                )
                .with_remote_images_path(postio_ui::allowlist::RemoteImageAllowList::path()),
            );
        }
    })
    .inspect_err(|error| tracing::warn!(%error, "config will not be watched; edits need a restart"))
    .ok()
}

/// Skips a wait that reads the same as the last: the keyring and the store share one sentence.
fn saying(write: impl Fn(&str)) -> impl Fn(postio_ui::list_state::Waiting) {
    let last = std::cell::RefCell::new(String::new());
    move |waiting| {
        let (title, _) = postio_ui::list_state::describe_wait(waiting);
        let line = format!("{title}…");
        if *last.borrow() != line {
            write(&line);
            *last.borrow_mut() = line;
        }
    }
}

pub fn run() -> ExitCode {
    let config_path = postio_config::paths::config_path().ok();
    // The journal, never stderr, which is the screen; the file is watched, so `[logging]` retunes
    // live.
    let logging = postio_session::logging::init_to(
        &config_path
            .as_deref()
            .map(postio_session::logging::config_at)
            .unwrap_or_default(),
        postio_session::logging::Destination::Journal,
    );
    let _log_watch = config_path.as_deref().and_then(|path| logging.watch(path));
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "postio-tui starting");
    let config = config_path
        .as_deref()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| postio_config::Config::from_toml_str(&text).ok())
        .unwrap_or_default();
    let (keys, problems) = Keys::new(&postio_core::Keymap::for_terminal(&config.keys));
    for problem in &problems {
        eprintln!("postio-tui: {problem}");
    }
    let colour = Colour::detect();
    let (theme, problems) = Theme::new(colour, Background::Unknown, &config.tui.colors);
    for problem in &problems {
        eprintln!("postio-tui: {problem}");
    }

    // The app mirrors its selection into it before each command; the client snapshots it.
    let state = postio_core::SharedState::default();

    // Before the alternate screen, so a store that will not open leaves a sentence on the terminal
    // and a non-zero exit.
    let host = match open(
        config_path.as_deref(),
        Arc::new(postio_account::secret::KeyringSecretStore::default()),
    ) {
        Ok(host) => host,
        Err(sentence) => {
            eprintln!("{sentence}");
            return ExitCode::FAILURE;
        }
    };
    let _focus = engage_focus(&host, &config, config_path.as_deref());
    let host = Arc::new(host);
    // While this terminal has the store, it answers agents' calls (ADR 0041).
    let _agents = postio_mcp::socket::listen_installed(host.connector(), config_path.clone())
        .inspect_err(|error| {
            tracing::warn!(%error, "agents will not be answered by this terminal: {error}");
        })
        .ok();
    let _focus_follow = config_path
        .as_deref()
        .and_then(|path| follow_focus_config(&host, path));
    host.start_syncing();
    host.start_idle_passes_after(IDLE_PASSES_AFTER_OPENING);
    let client = host.connect(ClientKind::Tui).with_state(state.clone());

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("postio-tui: could not start: {error}");
            return ExitCode::FAILURE;
        }
    };

    let mut modes = vec![Mode::Raw, Mode::AlternateScreen, Mode::BracketedPaste];
    if config.tui.mouse {
        modes.push(Mode::Mouse);
    }
    if crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false) {
        modes.push(Mode::KeyboardEnhancement);
    }
    crate::term::install_panic_hook();
    let mut session = match Session::enter(&mut Stdout, &modes) {
        Ok(session) => session,
        Err(error) => {
            eprintln!("postio-tui: this terminal would not start: {error}");
            return ExitCode::FAILURE;
        }
    };
    session.publish();

    let read = crate::config_file::Read::of(&config);
    let outcome = runtime.block_on(main_loop(
        &host,
        client,
        keys,
        theme,
        state,
        read,
        config.tui.preview,
        &mut session,
    ));
    let _ = session.leave(&mut Stdout);
    session.publish();
    // The engines first, then the clean-shutdown mark: the store is closed
    // before this process ends, as the desktop app closes it.
    drop(runtime);
    host.stop();
    drop(host);
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("postio-tui: {error}");
            ExitCode::FAILURE
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn main_loop(
    host: &Host,
    client: Client,
    keys: Keys,
    theme: Theme,
    state: postio_core::SharedState,
    config: crate::config_file::Read,
    preview: postio_config::Preview,
    session: &mut Session,
) -> io::Result<()> {
    let enhanced_keys = session.has(Mode::KeyboardEnhancement);
    let mut terminal = Terminal::new(backend(io::stdout()))?;
    let size = terminal.size()?;
    let mut app = App::new((size.width, size.height), keys)
        .with_state(state)
        .with_allowlist(postio_ui::allowlist::RemoteImageAllowList::load())
        .with_downloads(downloads())
        .with_preview(preview)
        .with_enhanced_keys(enhanced_keys)
        .with_mouse(session.has(Mode::Mouse));

    let (inputs, arriving) = async_channel::unbounded::<Input>();
    let (drafts, draft_jobs) = async_channel::unbounded::<Effect>();
    let writer = tokio::spawn(write_drafts(client.clone(), draft_jobs, inputs.clone()));
    let senders = Senders {
        inputs,
        drafts,
        config,
        host,
        attention: std::sync::Mutex::new(postio_ui::notify::Attention::default()),
    };
    let outcome = drive(
        &client,
        &mut app,
        &mut terminal,
        &theme,
        &arriving,
        &senders,
        session,
    )
    .await;
    // Saves what was written before leaving, so quitting mid-sentence leaves the draft. Bounded, so
    // a stalled store cannot hold the terminal.
    drop(senders);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(3), writer).await;
    outcome
}

/// Put `text` on the clipboard of the terminal's own machine, with OSC 52:
/// it works over SSH, where no clipboard tool on this side would help.
fn copy_to_clipboard(text: &str) {
    use std::io::Write;
    let sequence = format!("\x1b]52;c;{}\x07", base64(text.as_bytes()));
    let mut out = io::stdout();
    let _ = out.write_all(sequence.as_bytes());
    let _ = out.flush();
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let triple = chunk.iter().enumerate().fold(0u32, |acc, (index, byte)| {
            acc | u32::from(*byte) << (16 - 8 * index)
        });
        for index in 0..4 {
            if index <= chunk.len() {
                out.push(char::from(
                    ALPHABET[(triple >> (18 - 6 * index)) as usize & 63],
                ));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The most inputs handled before a frame is painted, so a flood of them
/// cannot keep the screen from moving at all.
const BATCH: usize = 64;

fn terminal_input(event: TerminalEvent, hits: &crate::view::hit::Hits) -> Option<Input> {
    match event {
        TerminalEvent::Key(key) => Some(Input::Key(key)),
        TerminalEvent::Resize(width, height) => Some(Input::Resize(width, height)),
        TerminalEvent::Paste(pasted) => Some(Input::Paste(pasted)),
        TerminalEvent::Mouse(mouse) => pointer(&mouse, hits).map(Input::Pointer),
        _ => None,
    }
}

fn next_ready(
    host: &async_channel::Receiver<postio_core::EventEnvelope>,
    arriving: &async_channel::Receiver<Input>,
) -> Option<Input> {
    host.try_recv()
        .ok()
        .map(|envelope| Input::Host(envelope.event))
        .or_else(|| arriving.try_recv().ok())
}

pub(crate) fn pointer(
    mouse: &crossterm::event::MouseEvent,
    hits: &crate::view::hit::Hits,
) -> Option<crate::app::Pointer> {
    use crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};
    let hit = hits.at(mouse.column, mouse.row)?;
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => Some(crate::app::Pointer::Click {
            hit,
            ctrl: mouse.modifiers.contains(KeyModifiers::CONTROL),
            shift: mouse.modifiers.contains(KeyModifiers::SHIFT),
        }),
        MouseEventKind::ScrollDown => Some(crate::app::Pointer::Wheel { hit, down: true }),
        MouseEventKind::ScrollUp => Some(crate::app::Pointer::Wheel { hit, down: false }),
        _ => None,
    }
}

/// Draft writes, sent one at a time in order: a save spawned on its own task could reach the host
/// after a later one.
async fn write_drafts(
    client: Client,
    jobs: async_channel::Receiver<Effect>,
    inputs: async_channel::Sender<Input>,
) {
    while let Ok(job) = jobs.recv().await {
        match job {
            Effect::SaveDraft {
                generation,
                composition,
                draft,
            } => {
                let saved = client
                    .save_draft(generation, *draft)
                    .await
                    .map_err(|error| error.message().to_owned());
                let _ = inputs
                    .send(Input::DraftSaved {
                        generation,
                        composition,
                        saved,
                    })
                    .await;
            }
            Effect::DiscardDraft { generation, known } => {
                let _ = client.discard_draft(generation, known).await;
            }
            Effect::QueueSend {
                generation,
                draft,
                at,
            } => {
                let queued = client
                    .queue_send(generation, *draft, at)
                    .await
                    .map(|queued| queued.draft)
                    .map_err(|error| error.message().to_owned());
                let _ = inputs.send(Input::Queued { at, queued }).await;
            }
            _ => {}
        }
    }
}

async fn resume(client: &Client, found: Option<postio_model::Draft>) -> Input {
    let Some(draft) = found else {
        return Input::Resumed {
            found: None,
            failure: None,
        };
    };
    let draft = if matches!(
        draft.state,
        postio_model::DraftState::Editing | postio_model::DraftState::Failed
    ) {
        Some(draft)
    } else {
        client.cancel_send(draft.id).await.ok().flatten()
    };
    // A failed send names what went wrong where the person returns to act on it (#1487).
    let failure = match &draft {
        Some(draft) if draft.state == postio_model::DraftState::Failed => {
            client.send_failure(draft.id).await.ok().flatten()
        }
        _ => None,
    };
    Input::Resumed {
        found: draft.map(Box::new),
        failure,
    }
}

struct Senders<'a> {
    inputs: async_channel::Sender<Input>,
    drafts: async_channel::Sender<Effect>,
    config: crate::config_file::Read,
    host: &'a Host,
    /// What the person is looking at, so mail arriving in the folder
    /// already on screen is not announced.
    attention: std::sync::Mutex<postio_ui::notify::Attention>,
}

fn tell(
    senders: &Senders<'_>,
    mailbox: postio_model::MailboxId,
    messages: Vec<postio_model::MessageId>,
) {
    let attention = *senders.attention.lock().expect("never poisoned");
    let deciding = senders.host.notification(mailbox, messages, attention);
    let inputs = senders.inputs.clone();
    tokio::spawn(async move {
        if let Some(notification) = deciding.await {
            let _ = inputs.send(Input::Notified(notification)).await;
        }
    });
}

enum Flow {
    Go,
    Quit,
    EditConfig(Option<postio_ui::settings::Section>),
    EditSignature {
        account: postio_model::AccountId,
        signature: Option<postio_model::SignatureId>,
        name: String,
        text: String,
    },
    Edit {
        generation: u64,
        markdown: String,
    },
}

async fn drive(
    client: &Client,
    app: &mut App,
    terminal: &mut Screen,
    theme: &Theme,
    arriving: &async_channel::Receiver<Input>,
    senders: &Senders<'_>,
    session: &mut Session,
) -> io::Result<()> {
    let mut terminal_events = EventStream::new();
    let host_events = client.events();

    let contents = places_contents(
        client,
        senders
            .config
            .clone()
            .with_contacts(senders.host.wiring().focus.config()),
    )
    .await;
    let effects = update(app, Input::Places(contents));
    let mut paint = false;
    if let Flow::Quit = perform(client, senders, effects, &mut paint)? {
        return Ok(());
    }
    let mut hits = draw(terminal, app, theme)?;

    loop {
        let first = tokio::select! {
            event = terminal_events.next() => match event {
                Some(Ok(event)) => match terminal_input(event, &hits) {
                    Some(input) => input,
                    None => continue,
                },
                Some(Err(error)) => return Err(error),
                None => return Ok(()),
            },
            heard = host_events.recv() => match heard {
                Ok(envelope) => Input::Host(envelope.event),
                // The host stopped: nothing on screen can be trusted to
                // change any more, so leave rather than show a frozen mailbox.
                Err(_) => return Err(io::Error::other("Postio's store stopped answering.")),
            },
            arrived = arriving.recv() => match arrived {
                Ok(input) => input,
                Err(_) => return Ok(()),
            },
        };
        // Everything already waiting is handled before the frame is painted, so a burst of keys or
        // sync events is one frame, not one each (SSH flicker).
        let mut paint = false;
        let mut flow = Flow::Go;
        let mut next = Some(first);
        let mut handled = 0;
        while let Some(input) = next.take() {
            if let Input::Host(postio_core::Event::NewMail {
                mailbox, messages, ..
            }) = &input
            {
                tell(senders, *mailbox, messages.clone());
            }
            let effects = update(app, input);
            flow = perform(client, senders, effects, &mut paint)?;
            handled += 1;
            if !matches!(flow, Flow::Go) || handled >= BATCH {
                break;
            }
            // Asked inside this task so the stream keeps this task's waker: polled from elsewhere,
            // the next key would wake nothing.
            next = tokio::select! {
                biased;
                event = terminal_events.next() => match event {
                    Some(Ok(event)) => terminal_input(event, &hits),
                    Some(Err(error)) => return Err(error),
                    None => return Ok(()),
                },
                () = std::future::ready(()) => None,
            }
            .or_else(|| next_ready(&host_events, arriving));
        }
        // While an open message's reads are on their way, the frame is
        // painted once they land (or after a frame's wait), not once per read.
        if paint && !app.holds_paint() {
            hits = draw(terminal, app, theme)?;
        }
        match flow {
            Flow::Go => {}
            Flow::Quit => return Ok(()),
            Flow::EditConfig(section) => {
                let Some(path) = crate::config_file::path() else {
                    let _ = senders.inputs.try_send(Input::ConfigEdited(Err(
                        "There is no config.toml here".into(),
                    )));
                    continue;
                };
                let text = crate::config_file::text(&path);
                let line = section
                    .and_then(|section| postio_ui::settings::find_section(&text, section))
                    .map(|line| line + 1);
                drop(terminal_events);
                let edited = session
                    .suspended(&mut Stdout, || {
                        crate::external::edit_in_place(&path, line, &crate::external::editor())
                    })?
                    .map_err(|error| error.to_string());
                session.publish();
                terminal_events = EventStream::new();
                terminal.clear()?;
                if let Ok(config) =
                    postio_config::Config::from_toml_str(&crate::config_file::text(&path))
                {
                    app.rekey(Keys::new(&postio_core::Keymap::for_terminal(&config.keys)).0);
                }
                let _ = senders.inputs.try_send(Input::ConfigEdited(edited));
            }
            Flow::Edit {
                generation,
                markdown,
            } => {
                // The event stream reads the terminal on its own thread and would take the editor's
                // keystrokes, so it is dropped while the editor runs.
                drop(terminal_events);
                let edited = session
                    .suspended(&mut Stdout, || {
                        crate::external::edit(
                            &markdown,
                            &crate::external::directory(),
                            &crate::external::editor(),
                        )
                    })?
                    .map_err(|error| error.to_string());
                session.publish();
                terminal_events = EventStream::new();
                terminal.clear()?;
                let _ = senders
                    .inputs
                    .try_send(Input::Edited { generation, edited });
            }
            Flow::EditSignature {
                account,
                signature,
                name,
                text,
            } => {
                drop(terminal_events);
                let edited = session
                    .suspended(&mut Stdout, || {
                        crate::external::edit(
                            &text,
                            &crate::external::directory(),
                            &crate::external::editor(),
                        )
                    })?
                    .map_err(|error| error.to_string());
                session.publish();
                terminal_events = EventStream::new();
                terminal.clear()?;
                let inputs = senders.inputs.clone();
                let client = client.clone();
                tokio::spawn(async move {
                    let saved = match edited {
                        Ok(text) => client
                            .save_signature(account, signature, name, text)
                            .await
                            .map_err(|error| error.message().to_owned()),
                        Err(error) => Err(error),
                    };
                    let _ = inputs.send(Input::SignatureSaved(saved)).await;
                });
            }
        }
    }
}

async fn places_contents(
    client: &Client,
    config: crate::config_file::Read,
) -> crate::places::Places {
    let accounts = client.accounts().await.unwrap_or_default();
    let mut folders = Vec::new();
    let mut counts = Vec::new();
    for account in &accounts {
        let theirs = client.mailboxes(account.id).await.unwrap_or_default();
        let drafts = client.draft_counts(account.id).await.unwrap_or_default();
        counts.push((
            account.id,
            postio_ui::sidebar::ViewCounts {
                flagged: theirs.iter().map(|folder| folder.counts.flagged).sum(),
                snoozed: theirs.iter().map(|folder| folder.counts.snoozed).sum(),
                outbox: drafts.outbox,
                drafts: drafts.drafts,
                attention: drafts.attention,
            },
        ));
        folders.extend(theirs);
    }
    crate::places::Places {
        accounts,
        folders,
        counts,
        saved: config.saved,
        features: config.features,
    }
}

fn downloads() -> std::path::PathBuf {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    std::env::var_os("XDG_DOWNLOAD_DIR")
        .filter(|dir| !dir.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            home.as_ref()
                .map(|home| home.join("Downloads"))
                .filter(|dir| dir.is_dir())
        })
        .or(home)
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

type Screen = Terminal<CrosstermBackend<io::BufWriter<io::Stdout>>>;

const FRAME_BUFFER: usize = 1 << 16;

/// Sends a frame in one write when flushed: over SSH every write is a packet the far terminal may
/// paint before the next arrives.
fn backend<W: io::Write>(out: W) -> CrosstermBackend<io::BufWriter<W>> {
    CrosstermBackend::new(io::BufWriter::with_capacity(FRAME_BUFFER, out))
}

fn perform(
    client: &Client,
    senders: &Senders<'_>,
    effects: Vec<Effect>,
    paint: &mut bool,
) -> io::Result<Flow> {
    let Senders {
        inputs,
        drafts,
        config,
        attention,
        ..
    } = senders;
    let mut redraw = false;
    let mut flow = Flow::Go;
    for effect in effects {
        match effect {
            Effect::Quit => return Ok(Flow::Quit),
            Effect::EditExternally {
                generation,
                markdown,
            } => {
                flow = Flow::Edit {
                    generation,
                    markdown,
                };
            }
            Effect::EditConfig(section) => flow = Flow::EditConfig(section),
            Effect::EditSignature {
                account,
                signature,
                name,
                text,
            } => {
                flow = Flow::EditSignature {
                    account,
                    signature,
                    name,
                    text,
                };
            }
            Effect::SaveSignature {
                account,
                signature,
                name,
                text,
            } => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let saved = client
                        .save_signature(account, signature, name, text)
                        .await
                        .map_err(|error| error.message().to_owned());
                    let _ = inputs.send(Input::SignatureSaved(saved)).await;
                });
            }
            Effect::DeleteSignature(signature) => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let deleted = client
                        .delete_signature(signature)
                        .await
                        .map_err(|error| error.message().to_owned());
                    let _ = inputs.send(Input::SignatureSaved(deleted)).await;
                });
            }
            Effect::Account(op) => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    // Done needs no word: every list of places hears of the change.
                    if let Err(error) = client.account(op).await {
                        let _ = inputs
                            .send(Input::Host(postio_core::Event::Error {
                                message: error.message().to_owned(),
                                account: None,
                            }))
                            .await;
                    }
                });
            }
            Effect::Redraw => redraw = true,
            Effect::ReplySource { kind, message } => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let found = client
                        .reply_source(message)
                        .await
                        .ok()
                        .flatten()
                        .map(Box::new);
                    let _ = inputs.send(Input::ReplySource { kind, found }).await;
                });
            }
            Effect::Unsubscribe(message) => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let answer = client
                        .unsubscribe(message)
                        .await
                        .map_err(|error| error.message().to_owned());
                    let _ = inputs.send(Input::Unsubscribed { message, answer }).await;
                });
            }
            Effect::ReadListMail { message, name } => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let found = client
                        .list_mail(message)
                        .await
                        .map_err(|error| error.message().to_owned());
                    let _ = inputs
                        .send(Input::ListMail {
                            message,
                            name,
                            found,
                        })
                        .await;
                });
            }
            Effect::SavePart {
                message,
                attachment,
                to,
            } => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let open = to.is_none();
                    let written = match to {
                        Some(to) => client.save_part(message, attachment, to).await,
                        None => client.open_part(message, attachment).await,
                    }
                    .map_err(|error| error.message().to_owned());
                    let _ = inputs.send(Input::PartWritten { written, open }).await;
                });
            }
            Effect::Discover(address) => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let found = client
                        .discover(address)
                        .await
                        .map_err(|error| error.message().to_owned());
                    let _ = inputs.send(Input::Discovered(found)).await;
                });
            }
            Effect::BeginOAuth(submission) => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let consent = client
                        .begin_oauth(*submission)
                        .await
                        .map_err(|error| error.message().to_owned());
                    let _ = inputs.send(Input::Consent(consent)).await;
                });
            }
            Effect::FinishOAuth(address) => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let added = client
                        .finish_oauth(address)
                        .await
                        .map_err(|error| error.message().to_owned());
                    let _ = inputs.send(Input::AccountAdded(added)).await;
                });
            }
            Effect::CancelOAuth(address) => {
                let client = client.clone();
                tokio::spawn(async move {
                    let _ = client.cancel_oauth(address).await;
                });
            }
            Effect::CopyText(text) => copy_to_clipboard(&text),
            Effect::AddAccount(submission) => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let added = client
                        .add_account(*submission)
                        .await
                        .map_err(|error| error.message().to_owned());
                    let _ = inputs.send(Input::AccountAdded(added)).await;
                });
            }
            Effect::SaveSyncWindow(window) => {
                tokio::task::spawn_blocking(move || {
                    // The account is already saved; a failed write costs the
                    // depth picked, not the account (as on the desktop).
                    if let Err(error) = postio_ui::onboarding::write_sync_window(window) {
                        tracing::warn!(%error, "could not save the sync window");
                    }
                });
            }
            Effect::OpenLink(target) => {
                // POSTIO-CONSENT: asked for only by a second click on a link
                // in the reader, the first having shown where it goes, or by
                // Enter on the browser sign-in address the first run shows
                // whole -- one deliberate activation, one open; never on
                // render.
                let opened = std::process::Command::new("xdg-open")
                    .arg(&target)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();
                if opened.is_err() {
                    // No opener (a server over SSH): the link goes to the terminal's clipboard,
                    // which
                    // reaches the person's machine (OSC 52).
                    copy_to_clipboard(&target);
                    let _ = inputs.try_send(Input::Host(postio_core::Event::Error {
                        message: format!("No opener here; {target} is on the clipboard"),
                        account: None,
                    }));
                }
            }
            Effect::Launch(path) => {
                // POSTIO-CONSENT: asked for only by the parts' Open command on
                // one part the person chose, already written to disk; the
                // opener is their own choice of application.
                let launched = std::process::Command::new("xdg-open")
                    .arg(&path)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();
                if let Err(error) = launched {
                    tracing::info!(%error, "no opener for a part");
                }
            }
            Effect::SaveAllowlist(list) => {
                if let Err(error) = list.save() {
                    tracing::warn!(%error, "could not save the remote-image allow list: {error}");
                }
            }
            Effect::SetReading(reading) => {
                if let Err(error) = crate::config_file::path()
                    .ok_or_else(|| "There is no config.toml here".to_owned())
                    .and_then(|path| crate::config_file::set_reading(&path, reading))
                {
                    tracing::warn!(%error, "could not write where messages open");
                }
            }
            Effect::ReadSource(message) => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let raw = client
                        .raw_source(message)
                        .await
                        .map_err(|error| error.message().to_owned());
                    let _ = inputs.send(Input::Source { message, raw }).await;
                });
            }
            Effect::Settle { generation, after } => {
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(after).await;
                    let _ = inputs.send(Input::Settled { generation }).await;
                });
            }
            Effect::ArmDwell {
                generation,
                message,
                after,
            } => {
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(after).await;
                    let _ = inputs
                        .send(Input::DwellDue {
                            generation,
                            message,
                        })
                        .await;
                });
            }
            Effect::ExpireNotice { generation, after } => {
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(after).await;
                    let _ = inputs.send(Input::NoticeDue { generation }).await;
                });
            }
            effect @ (Effect::SaveDraft { .. }
            | Effect::DiscardDraft { .. }
            | Effect::QueueSend { .. }) => {
                let _ = drafts.try_send(effect);
            }
            Effect::ReadClipboardImage => {
                let inputs = inputs.clone();
                tokio::task::spawn_blocking(move || {
                    let read = crate::clipboard::read(&mut crate::clipboard::system());
                    let _ = inputs.send_blocking(Input::ClipboardImage(read));
                });
            }
            Effect::InlineImage { bytes, mime_type } => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let stored = client.inline_image(bytes, mime_type).await.ok().flatten();
                    let _ = inputs.send(Input::InlineStored(stored)).await;
                });
            }
            // POSTIO-CONSENT: on a Download and attach press alone; the
            // host's handler makes the one request (spec 014 FR-031).
            Effect::AttachRemoteImage {
                generation,
                draft,
                href,
            } => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let outcome = client
                        .attach_remote_image(draft, href.clone())
                        .await
                        .unwrap_or_else(|_| {
                            Err(postio_ui::compose::RemoteRefusal::Unreachable(
                                "Postio could not ask for it".to_owned(),
                            ))
                        });
                    let _ = inputs
                        .send(Input::RemoteImage {
                            generation,
                            href,
                            outcome,
                        })
                        .await;
                });
            }
            Effect::Attach(path) => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let attached = client.attach(path.clone()).await.ok().flatten();
                    let _ = inputs.send(Input::Attached { path, attached }).await;
                });
            }
            // POSTIO-CONSENT: a local notification of mail that arrived,
            // chosen under `[sync]`'s notify rules; nothing
            // leaves this machine.
            Effect::DesktopNotify { title, body } => {
                if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some() {
                    let _ = std::process::Command::new("notify-send")
                        .args(["--app-name=Postio", "--", &title, &body])
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn();
                }
            }
            Effect::Recipients { account, prefix } => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let found = client
                        .recipients(account, prefix.clone())
                        .await
                        .unwrap_or_default();
                    let _ = inputs.send(Input::Recipients { prefix, found }).await;
                });
            }
            Effect::Resume(message) => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let found = client.draft_behind(message).await.ok().flatten();
                    let _ = inputs.send(resume(&client, found).await).await;
                });
            }
            Effect::ResumeDraft(draft) => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let found = client.draft_by_id(draft).await.ok().flatten();
                    let _ = inputs.send(resume(&client, found).await).await;
                });
            }
            Effect::ReadNewer(messages) => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    use postio_model::listing::MailStore as _;
                    let rows = client.message_rows(messages).await.unwrap_or_default();
                    let _ = inputs.send(Input::Newer(rows)).await;
                });
            }
            Effect::ReadBody(message) => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    // The reading, not just the body: the same one read, and
                    // the row it was decided from says whom it was sent to.
                    let reading = client
                        .readings(vec![message], false)
                        .await
                        .map_err(|error| error.message().to_owned())
                        .map(|mut readings| readings.pop());
                    let answer = match reading {
                        Ok(Some(reading)) => {
                            if let Some(row) = reading.row {
                                let _ = inputs
                                    .send(Input::Addressed {
                                        message,
                                        row,
                                        warning: reading.sender_warning,
                                    })
                                    .await;
                            }
                            Ok(reading.body)
                        }
                        Ok(None) => Ok(postio_client::protocol::Body::Missing),
                        Err(error) => Err(error),
                    };
                    let _ = inputs.send(Input::Body { message, answer }).await;
                });
            }
            Effect::Ask(ask) => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let answer = ask.perform(&client).await;
                    let _ = inputs.send(Input::Answer(answer)).await;
                });
            }
            Effect::ReadPrivacy => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let log = client.privacy_log().await;
                    let connections = client.egress_log(postio_ui::privacy::CONNECTION_ROWS).await;
                    match (log, connections) {
                        (Ok(log), Ok(connections)) => {
                            let _ = inputs.send(Input::Privacy { log, connections }).await;
                        }
                        (Err(error), _) | (_, Err(error)) => {
                            tracing::warn!(%error, "could not read the privacy log");
                        }
                    }
                });
            }
            Effect::RefreshPlaces => {
                let client = client.clone();
                let inputs = inputs.clone();
                // Read again: a search saved since startup is in the file,
                // not in what was read then.
                let read = crate::config_file::path()
                    .and_then(|path| crate::config_file::read_at(&path))
                    .unwrap_or_else(|| config.clone())
                    .with_contacts(senders.host.wiring().focus.config());
                tokio::spawn(async move {
                    let contents = places_contents(&client, read).await;
                    let _ = inputs.send(Input::Places(contents)).await;
                });
            }
            Effect::Send(command) => {
                let client = client.clone();
                tokio::spawn(async move {
                    if let Err(error) = client.send(command).await {
                        tracing::warn!(%error, "a command was not taken: {error}");
                    }
                });
            }
            Effect::Focus { ticket, request } => {
                let request = match postio_focus::perform_now(client, request) {
                    Ok(reply) => {
                        let _ = inputs.try_send(Input::FocusReply(ticket, reply));
                        continue;
                    }
                    Err(request) => request,
                };
                if let postio_focus::Request::OpenScope { scope, .. } = &request {
                    *attention.lock().expect("never poisoned") = postio_ui::notify::Attention {
                        showing: match scope {
                            ListScope::Mailbox(mailbox) => Some(*mailbox),
                            _ => None,
                        },
                        active: true,
                    };
                }
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let reply = postio_focus::perform(&client, request).await;
                    let _ = inputs.send(Input::FocusReply(ticket, reply)).await;
                });
            }
            Effect::FocusTimer { token, after } => {
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(after).await;
                    let _ = inputs.send(Input::FocusTimer(token)).await;
                });
            }
        }
    }
    *paint |= redraw;
    Ok(flow)
}

/// Painted inside a synchronized update, so a terminal that keeps them shows the frame whole.
fn draw<W: io::Write>(
    terminal: &mut Terminal<CrosstermBackend<W>>,
    app: &App,
    theme: &Theme,
) -> io::Result<crate::view::hit::Hits> {
    let mut hits = crate::view::hit::Hits::default();
    crossterm::queue!(
        terminal.backend_mut(),
        crossterm::terminal::BeginSynchronizedUpdate
    )?;
    terminal.draw(|frame| hits = crate::view::draw(frame, app, theme, chrono::Local::now()))?;
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::terminal::EndSynchronizedUpdate
    )?;
    Ok(hits)
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_store_under_another_key_is_reset_not_retried() {
        let said = refused(&postio_session::Refusal {
            sentence: "The store is another installation\u{2019}s.".to_owned(),
            remedy: postio_session::Remedy::StartOver {
                store: "/stores/postio.db".into(),
                because: postio_session::Unreadable::AnotherKey,
            },
        });
        assert!(said.contains("postio-store reset"), "{said}");
        assert!(said.contains("add your accounts again"), "{said}");
        assert!(!said.contains("keeping your accounts"), "{said}");
    }

    #[derive(Clone, Default)]
    struct Writes(std::rc::Rc<std::cell::RefCell<Vec<Vec<u8>>>>);
    impl io::Write for Writes {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().push(bytes.to_vec());
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_frame_leaves_in_one_write_not_a_kilobyte_at_a_time() {
        let writes = Writes::default();
        let mut terminal = ratatui::Terminal::with_options(
            backend(writes.clone()),
            ratatui::TerminalOptions {
                viewport: ratatui::Viewport::Fixed(ratatui::layout::Rect::new(0, 0, 120, 36)),
            },
        )
        .expect("a terminal");
        let (app, _) = crate::test_support::sample::state("list", 120, 36);
        let theme = crate::test_support::plain_theme();
        draw(&mut terminal, &app, &theme).expect("drawn");
        let sent = writes.0.borrow();
        let frame = sent
            .iter()
            .find(|write| write.len() > 1024)
            .expect("the whole frame in one write");
        let text = String::from_utf8_lossy(frame);
        assert!(
            text.starts_with("\x1b[?2026h") && text.contains("Grace Oyelaran"),
            "the synchronized update begins it and the mail is in it"
        );
        assert!(
            sent.len() <= 2,
            "the frame, then its end: {} writes",
            sent.len()
        );
    }

    #[test]
    fn what_is_waiting_already_is_taken_without_waiting() {
        let (host_tx, host) = async_channel::unbounded();
        let (answers, arriving) = async_channel::unbounded();
        assert!(next_ready(&host, &arriving).is_none(), "nothing waits");
        host_tx
            .try_send(postio_core::EventEnvelope::untracked(
                postio_core::Event::SurfacedChanged,
            ))
            .unwrap();
        answers.try_send(Input::Settled { generation: 1 }).unwrap();
        let taken: Vec<_> = std::iter::from_fn(|| next_ready(&host, &arriving)).collect();
        assert_eq!(taken.len(), 2, "both, at once: {taken:?}");
        assert!(next_ready(&host, &arriving).is_none());
    }

    #[test]
    fn a_frame_is_written_whole_inside_a_synchronized_update() {
        #[derive(Clone, Default)]
        struct Tap(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);
        impl io::Write for Tap {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.0.borrow_mut().extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let tap = Tap::default();
        let backend = ratatui::backend::CrosstermBackend::new(tap.clone());
        // A fixed viewport, so the backend is never asked its size: that asks
        // the controlling terminal, which a CI runner does not have ("a
        // terminal: No such file or directory").
        let mut terminal = ratatui::Terminal::with_options(
            backend,
            ratatui::TerminalOptions {
                viewport: ratatui::Viewport::Fixed(ratatui::layout::Rect::new(0, 0, 40, 12)),
            },
        )
        .expect("a terminal");
        let app = crate::test_support::app((40, 12));
        let theme = crate::test_support::plain_theme();
        draw(&mut terminal, &app, &theme).expect("drawn");
        let written = String::from_utf8_lossy(&tap.0.borrow()).into_owned();
        let begin = written.find("\x1b[?2026h").expect("begun");
        let end = written.rfind("\x1b[?2026l").expect("ended");
        assert!(begin == 0 && end > begin, "{written:?}");
        assert!(
            written[begin..end].contains("\x1b[1;1H"),
            "the frame is between the two: {written:?}"
        );
    }
    use super::*;

    #[test]
    fn base64_is_the_standard_alphabet_padded() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(
            base64(b"https://example.com/a?b"),
            "aHR0cHM6Ly9leGFtcGxlLmNvbS9hP2I="
        );
    }
}

#[cfg(test)]
mod waits {
    use postio_ui::list_state::Waiting;

    #[test]
    fn a_wait_that_reads_the_same_as_the_last_is_not_said_again() {
        let said = std::cell::RefCell::new(Vec::new());
        let say = super::saying(|line: &str| said.borrow_mut().push(line.to_owned()));
        say(Waiting::Keyring);
        say(Waiting::Store);
        assert_eq!(*said.borrow(), ["Opening your mailbox…"]);
    }
}
