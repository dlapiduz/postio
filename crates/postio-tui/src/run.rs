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

    let saved: Vec<String> = config
        .filters
        .iter()
        .filter(|(_, filter)| filter.pinned)
        .map(|(key, filter)| filter.name.clone().unwrap_or_else(|| key.clone()))
        .collect();
    let outcome = runtime.block_on(main_loop(client, keys, theme, state, saved));
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
    saved: Vec<String>,
) -> io::Result<()> {
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    let size = terminal.size()?;
    let mut app = App::new((size.width, size.height), keys).with_state(state);

    let (inputs, arriving) = async_channel::unbounded::<Input>();
    let mut terminal_events = EventStream::new();
    let host_events = client.events();

    let contents = sidebar_contents(&client, saved.clone()).await;
    let _ = update(&mut app, Input::Sidebar(contents));
    if let Some(scope) = first_scope(&client).await {
        let total = client.list_count(scope).await.unwrap_or(0);
        let effects = update(&mut app, Input::Opened { scope, total });
        if perform(
            &client,
            &mut app,
            &mut terminal,
            &theme,
            &inputs,
            &saved,
            effects,
        )? {
            return Ok(());
        }
    }
    draw(&mut terminal, &app, &theme)?;

    loop {
        let input = tokio::select! {
            event = terminal_events.next() => match event {
                Some(Ok(TerminalEvent::Key(key))) => Input::Key(key),
                Some(Ok(TerminalEvent::Resize(width, height))) => Input::Resize(width, height),
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
        let effects = update(&mut app, input);
        if perform(
            &client,
            &mut app,
            &mut terminal,
            &theme,
            &inputs,
            &saved,
            effects,
        )? {
            return Ok(());
        }
    }
}

/// What the sidebar holds: every account, its folders, and the counts its
/// views draw.
async fn sidebar_contents(client: &Client, saved: Vec<String>) -> crate::sidebar::Contents {
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

type Screen = Terminal<CrosstermBackend<io::Stdout>>;

/// Do what `update` asked; `true` to leave.
fn perform(
    client: &Client,
    app: &mut App,
    terminal: &mut Screen,
    theme: &Theme,
    inputs: &async_channel::Sender<Input>,
    saved: &[String],
    effects: Vec<Effect>,
) -> io::Result<bool> {
    let mut redraw = false;
    for effect in effects {
        match effect {
            Effect::Quit => return Ok(true),
            Effect::Redraw => redraw = true,
            Effect::Open(scope) => {
                let client = client.clone();
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let total = client.list_count(scope).await.unwrap_or(0);
                    let _ = inputs.send(Input::Opened { scope, total }).await;
                });
            }
            Effect::RefreshSidebar => {
                let client = client.clone();
                let inputs = inputs.clone();
                let saved = saved.to_vec();
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
        draw(terminal, app, theme)?;
    }
    Ok(false)
}

fn draw(terminal: &mut Screen, app: &App, theme: &Theme) -> io::Result<()> {
    terminal.draw(|frame| crate::view::draw(frame, app, theme, chrono::Local::now()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use postio_client::protocol::{BuildId, Frame, Refusal, read_frame, write_frame};

    use super::*;

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
