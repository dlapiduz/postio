//! The binary's life: connect, enter the terminal, loop, leave.
//!
//! Everything that decides is [`crate::app::update`]; this only carries
//! inputs to it and does the effects it asks for -- the terminal, the daemon,
//! a timer. Nothing here is tested with a real terminal: what is tested is
//! what it hands `update`, and what `update` hands back.

use std::io;
use std::process::ExitCode;

use crossterm::event::{Event as TerminalEvent, EventStream};
use futures_util::StreamExt;
use postio_client::Client;
use postio_client::protocol::ClientKind;
use postio_client::socket::{ConnectError, Endpoint, connect_or_start, daemon_path};
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

/// The sentence a frontend says when it cannot reach the daemon.
pub fn refusal(error: &ConnectError) -> String {
    format!("postio-tui: {error}")
}

/// Connect to the daemon at `endpoint`, starting it if nothing answers.
pub fn connect(endpoint: &Endpoint) -> Result<Client, String> {
    let mut say_so = || eprintln!("Opening your mailbox…");
    connect_or_start(endpoint, ClientKind::Tui, &daemon_path(), &mut say_so)
        .map_err(|error| refusal(&error))
}

/// The whole program.
pub fn run() -> ExitCode {
    let config = postio_config::paths::config_path()
        .ok()
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

    // What the daemon aims this frontend's commands with. The app mirrors its
    // selection into it before each command; the client snapshots it.
    let state = postio_core::SharedState::default();

    let endpoint = match Endpoint::from_env() {
        Ok(endpoint) => endpoint,
        Err(error) => {
            eprintln!("{}", refusal(&error));
            return ExitCode::FAILURE;
        }
    };
    let client = match connect(&endpoint) {
        Ok(client) => client.with_state(state.clone()),
        Err(sentence) => {
            eprintln!("{sentence}");
            return ExitCode::FAILURE;
        }
    };

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

async fn main_loop(
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
    // the draft in Drafts (US3 scenario 5). Bounded, so a daemon that has
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
/// spawned on a task of its own could reach the daemon after the one made
/// after it; the daemon keeps order among what arrives, not what was meant.
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
async fn resume(
    client: &Client,
    message: postio_model::MessageId,
) -> Option<Box<postio_model::Draft>> {
    let draft = client.draft_behind(message).await.ok().flatten()?;
    let draft = if draft.state == postio_model::DraftState::Editing {
        draft
    } else {
        client.cancel_send(draft.id).await.ok().flatten()?
    };
    Some(Box::new(draft))
}

/// Where the loop's work reports back: inputs for `update`, and draft
/// writes for `write_drafts`.
struct Senders {
    inputs: async_channel::Sender<Input>,
    drafts: async_channel::Sender<Effect>,
    /// The pinned saved searches' names, for the sidebar.
    saved: Vec<crate::sidebar::Saved>,
}

/// What the loop does after performing a batch of effects.
enum Flow {
    /// Wait for the next input.
    Go,
    /// Leave.
    Quit,
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
    senders: &Senders,
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
                Ok(envelope) => Input::Host(envelope.event),
                // The daemon went away: nothing on screen can be trusted to
                // change any more, so leave rather than show a frozen mailbox.
                Err(_) => return Err(io::Error::other("Postio's background service went away.")),
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
    senders: &Senders,
    effects: Vec<Effect>,
    hits: &mut crate::view::hit::Hits,
) -> io::Result<Flow> {
    let Senders {
        inputs,
        drafts,
        saved,
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
                    let _ = inputs
                        .send(Input::Resumed(resume(&client, message).await))
                        .await;
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
                    let answer = client
                        .body(message)
                        .await
                        .map_err(|error| error.message().to_owned());
                    let _ = inputs.send(Input::Body { message, answer }).await;
                });
            }
            Effect::Open(scope) => {
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
    use postio_client::protocol::{BuildId, Frame, Refusal, read_frame, write_frame};

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

    #[test]
    fn a_daemon_of_another_build_is_said_naming_both_versions() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = Endpoint::at(dir.path());
        let listener = std::os::unix::net::UnixListener::bind(endpoint.socket()).unwrap();
        let daemon = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                listener.set_nonblocking(true).unwrap();
                let listener = tokio::net::UnixListener::from_std(listener).unwrap();
                let (mut stream, _) = listener.accept().await.unwrap();
                read_frame(&mut stream).await.unwrap();
                let refusal = Frame::Refused(Refusal::VersionMismatch {
                    host: BuildId("0.2.9+old".into()),
                    client: BuildId::current(),
                });
                write_frame(&mut stream, &refusal).await.unwrap();
            });
        });
        let sentence = connect(&endpoint).unwrap_err();
        assert!(sentence.starts_with("postio-tui: "), "{sentence}");
        assert!(sentence.contains("0.2.9+old"), "{sentence}");
        assert!(sentence.contains(&BuildId::current().0), "{sentence}");
        daemon.join().unwrap();
    }
}
