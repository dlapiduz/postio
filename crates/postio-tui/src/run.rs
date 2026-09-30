//! The binary's life: open the store, enter the terminal, loop, leave.
//!
//! Everything that decides is [`crate::app::update`]; this only carries
//! inputs to it and does the effects it asks for -- the terminal, the host,
//! a timer. Nothing here is tested with a real terminal: what is tested is
//! what it hands `update`, and what `update` hands back.
//!
//! The terminal opens the store itself, in this process, as the desktop app
//! does: one Postio at a time has it, and whichever starts second is told to
//! close the other (specs/005-tui-frontend). Everything the terminal reads or
//! writes still goes through a [`Client`] of a [`Host`] in this process.

use std::io;
use std::process::ExitCode;
use std::sync::Arc;

use crossterm::event::{Event as TerminalEvent, EventStream};
use futures_util::StreamExt;
use postio_client::Client;
use postio_client::protocol::ClientKind;
use postio_host::Host;
use postio_model::ListScope;
use postio_model::listing::{MailStore, PageRequest};
use postio_model::mailbox::MailboxRole;
use postio_ui::paging::Fetch;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::app::{App, Effect, Input, update};
use crate::caps::{Background, Colour};
use crate::input::Keys;
use crate::term::{Mode, Session, Stdout};
use crate::theme::Theme;

/// How long after the store opens its upkeep starts -- the body index, the
/// header repair, the disk reclaim: the desktop app's delay after its first
/// frame, so the first pages have the runtime to themselves.
const IDLE_PASSES_AFTER_OPENING: std::time::Duration = std::time::Duration::from_millis(750);

/// Open the store with the key `secrets` holds and start its host here,
/// saying on stderr what it waits on, one line per change: a keyring prompt
/// can hold it for half a minute.
///
/// `Err` is the line to print before exiting: among them, when the desktop
/// app or another terminal already has the store, the sentence saying to
/// close it.
pub fn open(
    config_path: Option<&std::path::Path>,
    secrets: Arc<dyn postio_account::secret::SecretStore>,
) -> Result<Host, String> {
    let say_so = saying(|line: &str| eprintln!("{line}"));
    Host::open(config_path, secrets, &say_so).map_err(|sentence| format!("postio-tui: {sentence}"))
}

/// Say each wait to `write` as a line, unless it reads the same as the one
/// before: the keyring and the store are two waits with one sentence.
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

/// The whole program.
pub fn run() -> ExitCode {
    let config_path = postio_config::paths::config_path().ok();
    // The journal and never stderr, which is the screen this draws on; the
    // file is watched, so `[logging]` retunes a running terminal as it does
    // the desktop app.
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
    let (keys, problems) = Keys::new(&postio_core::Keymap::resolve(&config.keys));
    for problem in &problems {
        eprintln!("postio-tui: {problem}");
    }
    let colour = Colour::detect();
    let (theme, problems) = Theme::new(colour, Background::Unknown, &config.tui.colors);
    for problem in &problems {
        eprintln!("postio-tui: {problem}");
    }

    // What the host aims this frontend's commands with. The app mirrors its
    // selection into it before each command; the client snapshots it.
    let state = postio_core::SharedState::default();

    // Before the alternate screen: a store that will not open -- another
    // Postio has it, the keyring said no -- is a sentence left on the
    // terminal the person typed into, and a non-zero exit.
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

    let saved = crate::config_file::pinned(&config);
    let outcome = runtime.block_on(main_loop(
        &host,
        client,
        keys,
        theme,
        state,
        saved,
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

/// The first list: the first enabled account's inbox.
async fn first_scope(client: &Client) -> Option<ListScope> {
    let accounts = client.accounts().await.ok()?;
    let account = accounts.into_iter().find(|account| account.enabled)?;
    let folders = client.mailboxes(account.id).await.ok()?;
    let inbox = folders
        .iter()
        .find(|folder| folder.role == MailboxRole::Inbox)?;
    Some(ListScope::Mailbox(inbox.id))
}

#[allow(clippy::too_many_arguments)]
async fn main_loop(
    host: &Host,
    client: Client,
    keys: Keys,
    theme: Theme,
    state: postio_core::SharedState,
    saved: Vec<crate::sidebar::Saved>,
    preview: postio_config::Preview,
    session: &mut Session,
) -> io::Result<()> {
    let enhanced_keys = session.has(Mode::KeyboardEnhancement);
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    let size = terminal.size()?;
    let mut app = App::new((size.width, size.height), keys)
        .with_state(state)
        .with_allowlist(postio_ui::allowlist::RemoteImageAllowList::load())
        .with_downloads(downloads())
        .with_preview(preview)
        .with_enhanced_keys(enhanced_keys)
        .with_layout(crate::state::TerminalState::load())
        .with_mouse(session.has(Mode::Mouse));

    let (inputs, arriving) = async_channel::unbounded::<Input>();
    let (drafts, draft_jobs) = async_channel::unbounded::<Effect>();
    let writer = tokio::spawn(write_drafts(client.clone(), draft_jobs, inputs.clone()));
    let senders = Senders {
        inputs,
        drafts,
        saved,
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
    // What was written is saved before leaving: quitting mid-sentence leaves
    // the draft in Drafts (US3 scenario 5). Bounded, so a store that has
    // stopped answering cannot hold the terminal hostage.
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

/// Standard Base64, padded: all OSC 52 asks for, too small to be a
/// dependency.
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

/// A mouse event as the app hears it: what it landed on in the last frame.
fn pointer(
    mouse: &crossterm::event::MouseEvent,
    hits: &crate::view::hit::Hits,
) -> Option<crate::app::Pointer> {
    use crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};
    // A drag and a release are about where the button went down, not about
    // what is under the pointer now.
    match mouse.kind {
        MouseEventKind::Drag(MouseButton::Left) => {
            return Some(crate::app::Pointer::Drag {
                column: mouse.column,
            });
        }
        MouseEventKind::Up(MouseButton::Left) => return Some(crate::app::Pointer::Release),
        _ => {}
    }
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

/// Draft writes, sent one at a time in the order they were made. A save
/// spawned on a task of its own could reach the host after the one made
/// after it; the host keeps order among what arrives, not what was meant.
async fn write_drafts(
    client: Client,
    jobs: async_channel::Receiver<Effect>,
    inputs: async_channel::Sender<Input>,
) {
    while let Ok(job) = jobs.recv().await {
        match job {
            Effect::SaveDraft { generation, draft } => {
                let saved = client
                    .save_draft(generation, *draft)
                    .await
                    .map_err(|error| error.message().to_owned());
                let _ = inputs.send(Input::DraftSaved { generation, saved }).await;
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
                    .map(|_| ())
                    .map_err(|error| error.message().to_owned());
                let _ = inputs.send(Input::Queued { at, queued }).await;
            }
            _ => {}
        }
    }
}

/// The local draft behind a row, taken back from the Outbox if it was
/// queued, as the desktop does on opening one (#433).
async fn resume(client: &Client, message: postio_model::MessageId) -> Input {
    let Some(draft) = client.draft_behind(message).await.ok().flatten() else {
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
    // A failed send names what went wrong, where the person has come back to
    // do something about it -- the desktop's rule (FR-066, #1487).
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

/// Where the loop's work reports back: inputs for `update`, and draft
/// writes for `write_drafts`.
struct Senders<'a> {
    inputs: async_channel::Sender<Input>,
    drafts: async_channel::Sender<Effect>,
    /// The pinned saved searches' names, for the sidebar.
    saved: Vec<crate::sidebar::Saved>,
    /// The store's host, which decides whether new mail is worth saying.
    host: &'a Host,
    /// What the person is looking at, so mail arriving in the folder
    /// already on screen is not announced.
    attention: std::sync::Mutex<postio_ui::notify::Attention>,
}

/// Ask whether `messages` arriving in `mailbox` is worth telling the person
/// about, off the loop, and answer with [`Input::Notified`] if it is.
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

/// What the loop does after performing a batch of effects.
enum Flow {
    /// Wait for the next input.
    Go,
    /// Leave.
    Quit,
    /// Hand the terminal to the person's editor on `config.toml`, at a
    /// section, then read the file again.
    EditConfig(Option<postio_ui::settings::Section>),
    /// Hand the terminal to the editor on a signature's text, then save
    /// what it wrote.
    EditSignature {
        /// Whose.
        account: postio_model::AccountId,
        /// Which, or a new one.
        signature: Option<postio_model::SignatureId>,
        /// What it is called.
        name: String,
        /// Its text before the edit.
        text: String,
    },
    /// Hand the terminal to the external editor, then carry on.
    Edit {
        /// Which composition.
        generation: u64,
        /// The body to edit.
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

    // What is where on the screen, as last drawn: what a click lands on.
    let mut hits = crate::view::hit::Hits::default();
    let contents = sidebar_contents(client, senders.saved.clone()).await;
    let _ = update(app, Input::Sidebar(contents));
    if let Some(scope) = first_scope(client).await {
        let total = client.list_count(scope).await.unwrap_or(0);
        let effects = update(app, Input::Opened { scope, total });
        if let Flow::Quit = perform(client, app, terminal, theme, senders, effects, &mut hits)? {
            return Ok(());
        }
    }
    hits = draw(terminal, app, theme)?;

    loop {
        let input = tokio::select! {
            event = terminal_events.next() => match event {
                Some(Ok(TerminalEvent::Key(key))) => Input::Key(key),
                Some(Ok(TerminalEvent::Resize(width, height))) => Input::Resize(width, height),
                Some(Ok(TerminalEvent::Paste(pasted))) => Input::Paste(pasted),
                Some(Ok(TerminalEvent::Mouse(mouse))) => match pointer(&mouse, &hits) {
                    Some(pointer) => Input::Pointer(pointer),
                    None => continue,
                },
                Some(Ok(_)) => continue,
                Some(Err(error)) => return Err(error),
                None => return Ok(()),
            },
            heard = host_events.recv() => match heard {
                Ok(envelope) => {
                    if let postio_core::Event::NewMail { mailbox, messages, .. } = &envelope.event {
                        tell(senders, *mailbox, messages.clone());
                    }
                    Input::Host(envelope.event)
                }
                // The host stopped: nothing on screen can be trusted to
                // change any more, so leave rather than show a frozen mailbox.
                Err(_) => return Err(io::Error::other("Postio's store stopped answering.")),
            },
            arrived = arriving.recv() => match arrived {
                Ok(input) => input,
                Err(_) => return Ok(()),
            },
        };
        let effects = update(app, input);
        match perform(client, app, terminal, theme, senders, effects, &mut hits)? {
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
                // Lines count from one for an editor.
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
                // New bindings take effect now, as the desktop's do.
                if let Ok(config) =
                    postio_config::Config::from_toml_str(&crate::config_file::text(&path))
                {
                    app.rekey(Keys::new(&postio_core::Keymap::resolve(&config.keys)).0);
                }
                let _ = senders.inputs.try_send(Input::ConfigEdited(edited));
            }
            Flow::Edit {
                generation,
                markdown,
            } => {
                // The event stream reads the terminal on a thread of its own;
                // left running, it would take the editor's keystrokes. So it
                // goes, the screen is handed back to what it was, the editor
                // runs, and both come back.
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
                // Whatever the editor drew is on the real screen now.
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
                // As for a draft: the terminal's reader stops while the
                // editor has the screen.
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

/// What the sidebar holds: every account, its folders, and the counts its
/// views draw.
async fn sidebar_contents(
    client: &Client,
    saved: Vec<crate::sidebar::Saved>,
) -> crate::sidebar::Contents {
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
    crate::sidebar::Contents {
        accounts,
        folders,
        counts,
        saved,
    }
}

/// Where saved parts go: `$XDG_DOWNLOAD_DIR`, else `~/Downloads`, else the
/// home directory, else here.
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

type Screen = Terminal<CrosstermBackend<io::Stdout>>;

/// Do what `update` asked, and say what the loop does next.
fn perform(
    client: &Client,
    app: &mut App,
    terminal: &mut Screen,
    theme: &Theme,
    senders: &Senders<'_>,
    effects: Vec<Effect>,
    hits: &mut crate::view::hit::Hits,
) -> io::Result<Flow> {
    let Senders {
        inputs,
        drafts,
        saved,
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
                    // Done needs no word: every sidebar hears of the change.
                    if let Err(error) = client.account(op).await {
                        let _ = inputs
                            .send(Input::Host(postio_core::Event::Error {
                                message: error.message().to_owned(),
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
                    let _ = inputs.send(Input::Unsubscribed(answer)).await;
                });
            }
            Effect::ReadParts(message) => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let parts = client
                        .parts(message)
                        .await
                        .map_err(|error| error.message().to_owned());
                    let _ = inputs.send(Input::Parts { message, parts }).await;
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
            Effect::SaveLayout(layout) => {
                tokio::task::spawn_blocking(move || {
                    if let Err(error) = layout.save() {
                        tracing::info!(%error, "could not remember the layout");
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
                    // No opener here -- a server over SSH. The link goes to
                    // the terminal's clipboard instead, which reaches the
                    // person's own machine (research R7, OSC 52).
                    copy_to_clipboard(&target);
                    let _ = inputs.try_send(Input::Host(postio_core::Event::Error {
                        message: format!("No opener here; {target} is on the clipboard"),
                    }));
                }
            }
            Effect::Launch(path) => {
                // The system's opener, detached, its output away from the
                // terminal. Where there is none -- a server over SSH -- the
                // notice has already said where the file is.
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
            Effect::Autosave { generation, edit } => {
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(crate::app::AUTOSAVE).await;
                    let _ = inputs.send(Input::AutosaveDue { generation, edit }).await;
                });
            }
            // One writer, in order: see `write_drafts`.
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
            Effect::Attach(path) => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let attached = client.attach(path.clone()).await.ok().flatten();
                    let _ = inputs.send(Input::Attached { path, attached }).await;
                });
            }
            Effect::Facets {
                sequence,
                account,
                query,
                scope,
            } => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let parsed = postio_search::parse(&query, chrono::Local::now().date_naive());
                    let facets = client.facets(account, parsed, scope).await.ok().flatten();
                    let _ = inputs.send(Input::Facets { sequence, facets }).await;
                });
            }
            Effect::Search { sequence, search } => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let found = client
                        .search(search)
                        .await
                        .map_err(|error| error.message().to_owned());
                    let _ = inputs.send(Input::Found { sequence, found }).await;
                });
            }
            // The desktop's own notification service, where the session has
            // one; over SSH there is none, and the status line has said it.
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
            // Nothing to offer is what a store that cannot be read offers:
            // the finder says "no label matches" either way.
            Effect::ReadLabels(account) => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let labels = client.labels(account).await.unwrap_or_default();
                    let _ = inputs.send(Input::Labels(labels)).await;
                });
            }
            Effect::ReadCorrespondents(account) => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let found = client.correspondents(account).await.unwrap_or_default();
                    let _ = inputs.send(Input::Correspondents(found)).await;
                });
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
                    let _ = inputs.send(resume(&client, message).await).await;
                });
            }
            Effect::Rest(message) => {
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(crate::app::READ_REST).await;
                    let _ = inputs.send(Input::Rested(message)).await;
                });
            }
            Effect::ReadConversation(thread) => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let members = client
                        .conversation(thread)
                        .await
                        .map_err(|error| error.message().to_owned());
                    let _ = inputs.send(Input::Conversation { thread, members }).await;
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
                                let to = row.to.into_iter().chain(row.cc).collect();
                                let _ = inputs.send(Input::Addressed { message, to }).await;
                            }
                            Ok(reading.body)
                        }
                        Ok(None) => Ok(postio_client::protocol::Body::Missing),
                        Err(error) => Err(error),
                    };
                    let _ = inputs.send(Input::Body { message, answer }).await;
                });
            }
            Effect::Open(scope) => {
                // What the person is looking at, so mail arriving in the
                // folder already on screen is not announced.
                *attention.lock().expect("never poisoned") = postio_ui::notify::Attention {
                    showing: match scope {
                        ListScope::Mailbox(mailbox) => Some(mailbox),
                        _ => None,
                    },
                    active: true,
                };
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let total = client.list_count(scope).await.unwrap_or(0);
                    let _ = inputs.send(Input::Opened { scope, total }).await;
                });
            }
            Effect::SaveSearch(query) => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let saved = tokio::task::spawn_blocking(move || {
                        crate::config_file::path()
                            .ok_or_else(|| "There is no config.toml here".to_owned())
                            .and_then(|path| crate::config_file::save_search(&path, &query))
                    })
                    .await
                    .unwrap_or_else(|error| Err(error.to_string()));
                    match saved {
                        Ok(names) => {
                            let contents = sidebar_contents(&client, names).await;
                            let _ = inputs.send(Input::Sidebar(contents)).await;
                        }
                        Err(reason) => {
                            tracing::warn!(%reason, "could not save the search");
                        }
                    }
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
            Effect::EditSearch(edit) => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let edited = tokio::task::spawn_blocking(move || {
                        crate::config_file::path()
                            .ok_or_else(|| "There is no config.toml here".to_owned())
                            .and_then(|path| crate::config_file::edit_search(&path, &edit))
                    })
                    .await
                    .unwrap_or_else(|error| Err(error.to_string()));
                    match edited {
                        Ok(names) => {
                            let contents = sidebar_contents(&client, names).await;
                            let _ = inputs.send(Input::Sidebar(contents)).await;
                        }
                        Err(reason) => {
                            tracing::warn!(%reason, "could not change the saved search");
                        }
                    }
                });
            }
            Effect::RefreshSidebar => {
                let client = client.clone();
                let inputs = inputs.clone();
                // Read again: a search saved since startup is in the file,
                // not in what was read then.
                let saved = crate::config_file::path()
                    .and_then(|path| crate::config_file::pinned_at(&path))
                    .unwrap_or_else(|| saved.to_vec());
                tokio::spawn(async move {
                    let contents = sidebar_contents(&client, saved).await;
                    let _ = inputs.send(Input::Sidebar(contents)).await;
                });
            }
            Effect::Recount(scope) => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    if let Ok(total) = client.list_count(scope).await {
                        let _ = inputs.send(Input::Recounted { scope, total }).await;
                    }
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
            Effect::Fetch {
                generation,
                page,
                fetch,
            } => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let rows = match fetch {
                        Fetch::Scope(request) => client
                            .list_page(PageRequest {
                                scope: request.scope,
                                offset: request.offset,
                                limit: request.limit,
                            })
                            .await
                            .map(crate::row::page_of)
                            .map_err(|error| error.message().to_owned()),
                        Fetch::Hits { ids, .. } => client
                            .message_rows(ids)
                            .await
                            .map(|rows| postio_ui::paging::Page {
                                total: 0,
                                rows: rows.into_iter().map(crate::row::Row::from).collect(),
                            })
                            .map_err(|error| error.message().to_owned()),
                    };
                    let _ = inputs
                        .send(Input::Page {
                            generation,
                            page,
                            rows,
                        })
                        .await;
                });
            }
        }
    }
    if redraw {
        *hits = draw(terminal, app, theme)?;
    }
    Ok(flow)
}

fn draw(terminal: &mut Screen, app: &App, theme: &Theme) -> io::Result<crate::view::hit::Hits> {
    let mut hits = crate::view::hit::Hits::default();
    terminal.draw(|frame| hits = crate::view::draw(frame, app, theme, chrono::Local::now()))?;
    Ok(hits)
}

#[cfg(test)]
mod tests {
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
        // The keyring and the store are two waits with one sentence; the
        // person sees one line for them, not the same line twice.
        let said = std::cell::RefCell::new(Vec::new());
        let say = super::saying(|line: &str| said.borrow_mut().push(line.to_owned()));
        say(Waiting::Keyring);
        say(Waiting::Store);
        assert_eq!(*said.borrow(), ["Opening your mailbox…"]);
    }
}
