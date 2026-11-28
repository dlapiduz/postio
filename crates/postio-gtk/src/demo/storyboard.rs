//! Focus's storyboard runner (specs/008-storyboards): plays a storyboard against a real window over
//! a seeded store and records every step.
//! A command is pressed through the binding the window's context has for it, never dispatched by
//! name; keys go along the focus chain.
//! A preset is a window condition no key reaches (a Settings section, a locked keyring), applied by
//! hand once the window is up.

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
use postio_ui::settings::Section;

/// What Focus does not observe, so a check on it is not applicable rather than a pass.
pub const UNOBSERVED: &[&str] = &["back_depth", "rows.first_visible", "reading.focused"];

/// 16:09 on the day after the storage seed's anchor, the time the references were drawn at.
pub const FROZEN_AT: &str = "2026-06-02T16:09:00Z";

#[derive(Debug, Clone)]
pub struct Options {
    pub out: Option<PathBuf>,
    pub frames: bool,
    pub delivery: Delivery,
    /// Sample a frame every this many ticks: two, because one capture costs more than a frame.
    pub stride: u32,
    pub variant: BTreeMap<String, String>,
    pub ignored_axes: Vec<String>,
    pub tree_key: String,
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

/// With `every`, each variant asked for; otherwise the one asked for by default, which keeps an app
/// default wherever allowed, so `inbox-dark` is still filmed dark.
pub fn variants_to_play(
    board: &Storyboard,
    every: bool,
) -> (Vec<BTreeMap<String, String>>, Vec<String>) {
    if every {
        postio_storyboard::apply::variants(board, &runner_info())
    } else {
        let (variant, ignored) = postio_storyboard::apply::default_variant(board, &runner_info());
        (vec![variant], ignored)
    }
}

/// The runner's fontconfig must have them beside Postio's faces: else regular text falls back to
/// the monospace `sans-serif` match.
pub fn chrome_faces(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending: Vec<PathBuf> = roots.to_vec();
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if let Some(name) = path.file_name().and_then(|n| n.to_str())
                && (name.starts_with("AdwaitaSans") || name.starts_with("AdwaitaMono"))
                && name.ends_with(".ttf")
            {
                found.push(path);
            }
        }
    }
    found.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    found
}

pub fn runner_info() -> RunnerInfo {
    // Focus has no contrast or density axis: it follows the system's scheme and contrast.
    let axes = [
        ("scheme", &["light", "dark"][..]),
        ("width", &["normal", "wide", "narrow"][..]),
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
        presets: Preset::ALL
            .iter()
            .map(|preset| preset.id().to_owned())
            .collect(),
        axes,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Settings(Section),
    AccountForm,
    SignatureEditor,
    AccountWeights,
    AddAccountRoute,
    AddAccountBrowser,
    AddAccountSyncWindow,
    SyncBackfill,
    FilteringOff,
    FilteringLists,
    Locked,
    AgentsShared,
    Vault,
    NativeComposer,
}

impl Preset {
    pub const ALL: [Preset; 22] = [
        Preset::Settings(Section::Accounts),
        Preset::Settings(Section::Filtering),
        Preset::Settings(Section::Filters),
        Preset::Settings(Section::Composing),
        Preset::Settings(Section::Keyboard),
        Preset::Settings(Section::Sync),
        Preset::Settings(Section::Privacy),
        Preset::Settings(Section::ConfigFile),
        Preset::Settings(Section::Agents),
        Preset::AccountForm,
        Preset::SignatureEditor,
        Preset::AccountWeights,
        Preset::AddAccountRoute,
        Preset::AddAccountBrowser,
        Preset::AddAccountSyncWindow,
        Preset::SyncBackfill,
        Preset::FilteringOff,
        Preset::FilteringLists,
        Preset::Locked,
        Preset::AgentsShared,
        Preset::Vault,
        Preset::NativeComposer,
    ];

    /// Settings' Appearance section is not a preset: Focus does not show it.
    pub fn id(self) -> &'static str {
        match self {
            Preset::Settings(Section::Filtering) => "settings/filtering",
            Preset::Settings(Section::Filters) => "settings/saved-searches",
            Preset::Settings(Section::Composing) => "settings/composing",
            Preset::Settings(Section::Keyboard) => "settings/keyboard",
            Preset::Settings(Section::Sync) => "settings/storage",
            Preset::Settings(Section::Privacy) => "settings/privacy",
            Preset::Settings(Section::ConfigFile) => "settings/configfile",
            Preset::Settings(Section::Agents) => "settings/agents",
            Preset::Settings(_) => "settings",
            Preset::AccountForm => "settings/account-form",
            Preset::SignatureEditor => "settings/signature-editor",
            Preset::AccountWeights => "settings/account-weights",
            Preset::AddAccountRoute => "add-account/route",
            Preset::AddAccountBrowser => "add-account/browser",
            Preset::AddAccountSyncWindow => "add-account/syncwindow",
            Preset::SyncBackfill => "settings/storage-backfill",
            Preset::FilteringOff => "settings/filtering-off",
            Preset::FilteringLists => "settings/filtering-lists",
            Preset::Locked => "locked",
            Preset::AgentsShared => "settings/agents-shared",
            Preset::Vault => "vault",
            Preset::NativeComposer => "native-composer",
        }
    }

    pub fn from_id(id: &str) -> Option<Preset> {
        Preset::ALL.into_iter().find(|preset| preset.id() == id)
    }

    fn apply(self, started: &Started) -> Result<(), String> {
        let window = &started.window;
        match self {
            Preset::Vault | Preset::NativeComposer => Ok(()),
            Preset::Settings(Section::Sync) | Preset::SyncBackfill => {
                let panel = open_settings(window, Section::Sync)?;
                show_mail_weight(&panel, started.account);
                pump(Duration::from_millis(800));
                // Sync & storage opens at its top for a person; the folders are a second view,
                // scrolled to.
                if self == Preset::SyncBackfill {
                    panel.reveal_backfill();
                    pump(Duration::from_millis(200));
                }
                Ok(())
            }
            Preset::Settings(Section::Accounts) => {
                let panel = open_settings(window, Section::Accounts)?;
                show_mail_weight(&panel, started.account);
                pump(Duration::from_millis(300));
                Ok(())
            }
            Preset::Settings(Section::Agents) => {
                let panel = open_settings(window, Section::Agents)?;
                // Set here so the frame does not wait on the pane's own read of a missing log.
                panel.set_agent_activity(postio_ui::agent_access::activity(
                    &[],
                    &Default::default(),
                    frozen_now(),
                ));
                pump(Duration::from_millis(200));
                Ok(())
            }
            Preset::AgentsShared => {
                let panel = open_settings(window, Section::Agents)?;
                let text = panel.text();
                panel.set_text(&format!("{text}\n{AGENTS_SHARED}"));
                panel.set_agent_entry(Some("/usr/bin/postio-mcp"));
                let calls =
                    postio_ui::agent_access::parse_log(AGENT_LOG, postio_ui::agent_access::RECENT);
                panel.set_agent_activity(postio_ui::agent_access::activity(
                    &calls,
                    &agent_names(),
                    frozen_now(),
                ));
                pump(Duration::from_millis(300));
                Ok(())
            }
            Preset::Settings(section) => open_settings(window, section).map(drop),
            Preset::FilteringOff | Preset::FilteringLists => {
                let panel = open_settings(window, Section::Filtering)?;
                let text = panel.text();
                let text = if self == Preset::FilteringOff {
                    postio_config::focus_edit::set_filtering(&text, false)
                        .map_err(|error| format!("filtering off: {error}"))?
                        .unwrap_or(text)
                } else {
                    format!("{text}\n{FILTER_LISTS}")
                };
                panel.set_text(&text);
                pump(Duration::from_millis(300));
                Ok(())
            }
            Preset::AccountForm | Preset::SignatureEditor => {
                let panel = open_settings(window, Section::Accounts)?;
                if !pump_until(Duration::from_secs(5), || {
                    panel.accounts_list().row_at_index(0).is_some()
                }) {
                    return Err("the account's row never landed in Settings".to_owned());
                }
                panel.open_account_detail(started.account);
                if self == Preset::SignatureEditor {
                    panel.open_signature_editor(None);
                }
                pump(Duration::from_millis(300));
                Ok(())
            }
            Preset::AccountWeights => {
                let panel = open_settings(window, Section::Accounts)?;
                show_account_weights(&panel);
                pump(Duration::from_millis(300));
                Ok(())
            }
            Preset::AddAccountRoute | Preset::AddAccountBrowser | Preset::AddAccountSyncWindow => {
                show_add_account(window, self)
            }
            Preset::Locked => {
                // The store is encrypted under a key the keyring holds: a locked keyring is mail
                // that cannot be opened, not a password that cannot be read.
                let refusal = postio_account::secret::SecretError::Locked {
                    keyring: "login".to_owned(),
                    account: postio_session::STORE_KEY_ENTRY.to_owned(),
                };
                window.show_unavailable(&postio_session::key_refusal(&refusal).sentence, || {});
                pump(Duration::from_millis(200));
                Ok(())
            }
        }
    }
}

const FILTER_LISTS: &str = "[focus.filter]\n\
never = [\"ada@example.org\", \"@harbor.example\", \"grace\"]\n\
stop_markers = [{ sender = \"news@ledger.example\", kind = \"question\" }]\n";

const AGENTS_SHARED: &str = "[mcp.accounts.\"lena.tomlin@example.com\"]\n\
mailboxes = [\"*\"]\n\
actions = [\"archive\", \"label\"]\n\
[mcp.accounts.\"ops@example.org\"]\n\
mailboxes = [\"INBOX\"]\n";

const AGENT_LOG: &str = "\
{\"at\":\"2026-06-01T16:10:00Z\",\"tool\":\"archive_messages\",\"accounts\":[2],\"mailboxes\":[],\"messages\":[],\"drafts\":[],\"labels\":[],\"outcome\":\"refused\"}
{\"at\":\"2026-06-02T08:12:00Z\",\"tool\":\"search_mail\",\"accounts\":[1],\"mailboxes\":[10],\"messages\":[101,102,103],\"drafts\":[],\"labels\":[],\"outcome\":\"answered\"}
{\"at\":\"2026-06-02T08:55:00Z\",\"tool\":\"read_message\",\"accounts\":[1],\"mailboxes\":[10],\"messages\":[101],\"drafts\":[],\"labels\":[],\"outcome\":\"answered\"}
";

fn agent_names() -> postio_ui::agent_access::Names {
    postio_ui::agent_access::Names {
        accounts: [
            (1, "lena.tomlin@example.com".to_owned()),
            (2, "ops@example.org".to_owned()),
        ]
        .into(),
        mailboxes: [(10, "INBOX".to_owned())].into(),
        messages: [(101, "Harbor lease renewal".to_owned())].into(),
    }
}

fn frozen_now() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339("2026-06-02T09:00:00Z")
        .expect("a fixed instant")
        .with_timezone(&chrono::Utc)
}

fn open_settings(
    window: &FocusWindow,
    section: Section,
) -> Result<postio_widgets::settings::SettingsPanel, String> {
    window.act(CommandId::Settings);
    let open = || window.settings().filter(|settings| settings.is_open());
    if !pump_until(Duration::from_secs(5), || {
        open().is_some_and(|settings| settings.panel().is_mapped() && settings.panel().width() > 0)
    }) {
        return Err("Settings never opened".to_owned());
    }
    let panel = open().ok_or("Settings closed")?.panel().clone();
    panel.show_section(section);
    pump(Duration::from_millis(500));
    Ok(panel)
}

/// No seed has a server to measure it.
fn show_mail_weight(
    panel: &postio_widgets::settings::SettingsPanel,
    account: postio_model::AccountId,
) {
    panel.set_mail_weights(
        &[(
            account,
            postio_core::event::MailFootprint {
                total_bytes: 12_884_901_888,
                attachment_bytes: 11_811_160_064,
                local_bytes: 933_232_640,
                complete: true,
            },
        )],
        false,
    );
}

fn show_account_weights(panel: &postio_widgets::settings::SettingsPanel) {
    let footprint = |total: u64, attachments: u64, local: u64, complete: bool| {
        postio_core::event::MailFootprint {
            total_bytes: total,
            attachment_bytes: attachments,
            local_bytes: local,
            complete,
        }
    };
    let account = |id: i64, name: &str, address: &str| {
        let mut account =
            postio_model::Account::new(name, postio_model::EmailAddress::new(Some(name), address));
        account.id = postio_model::AccountId::new(id);
        account.enabled = true;
        account
    };
    panel.set_accounts(vec![
        account(1, "Ada Moreno", "ada@example.com"),
        account(2, "Lena Park", "lena@example.org"),
        account(3, "A rather long display name", "someone@example.invalid"),
    ]);
    panel.set_mail_weights(
        &[
            (
                postio_model::AccountId::new(1),
                footprint(12_884_901_888, 11_811_160_064, 933_232_640, true),
            ),
            (
                postio_model::AccountId::new(2),
                footprint(1_503_238_553, 1_400_000_000, 103_809_024, true),
            ),
            (
                postio_model::AccountId::new(3),
                footprint(8_589_934_592, 7_516_192_768, 2_254_857_830, false),
            ),
        ],
        false,
    );
}

/// Each step needs a server or a person to reach, so the screen is told. The provider is invented,
/// on a reserved domain: providers are data.
fn show_add_account(window: &FocusWindow, preset: Preset) -> Result<(), String> {
    use postio_widgets::onboarding::{BrowserSignIn, Onboarding, Server, Settings, Status};

    window.act(CommandId::AddAccount);
    let find = || {
        let dialog = window.add_account_dialog()?;
        let mut pending: Vec<gtk::Widget> = vec![dialog.upcast()];
        while let Some(widget) = pending.pop() {
            if let Ok(screen) = widget.clone().downcast::<Onboarding>() {
                return Some(screen);
            }
            let mut child = widget.first_child();
            while let Some(next) = child {
                child = next.next_sibling();
                pending.push(next);
            }
        }
        None
    };
    let mut screen = None;
    pump_until(Duration::from_secs(5), || {
        screen = find();
        screen.is_some()
    });
    let screen = screen.ok_or("the add-account dialog never opened")?;
    screen.set_address("lena.tomlin@example.com");
    screen.set_status(Status::Found(Settings {
        imap: Server {
            host: "imap.example.com".to_owned(),
            port: 993,
            security: postio_model::TransportSecurity::Tls,
        },
        smtp: Server {
            host: "smtp.example.com".to_owned(),
            port: 587,
            security: postio_model::TransportSecurity::StartTls,
        },
        login: "lena.tomlin@example.com".to_owned(),
        source: "Example Mail".to_owned(),
        oauth_sign_in: true,
        ..Settings::default()
    }));
    match preset {
        Preset::AddAccountBrowser => {
            screen.set_browser_sign_in(BrowserSignIn {
                provider: "Example Mail".to_owned(),
                scopes: vec![
                    "https://mail.example.com/imap".to_owned(),
                    "https://mail.example.com/smtp".to_owned(),
                    "offline_access".to_owned(),
                ],
                redirect_uri: "http://127.0.0.1:41337/".to_owned(),
                authorize_url: "https://login.example.com/oauth2/authorize".to_owned(),
            });
            screen.set_status(Status::WaitingForBrowser);
        }
        Preset::AddAccountSyncWindow => screen.set_status(Status::SyncWindow),
        _ => {}
    }
    pump(Duration::from_millis(300));
    Ok(())
}

/// One path per storyboard, the same every run, so a frame of the foot strip that prints it is the
/// same picture twice.
fn config_path(name: &str) -> PathBuf {
    std::env::temp_dir()
        .join("postio-storyboard")
        .join(name)
        .join("config.toml")
}

/// `hold_body` takes one seeded body away before the window opens, for a `body_arrived` step to
/// bring back.
async fn acting(
    seed: Seed,
    size: (i32, i32),
    name: &str,
    hold_body: bool,
    vault: bool,
) -> Result<Started, String> {
    let (database, account) = super::seeded(seed).await;
    if hold_body {
        super::hold_back_body(&database, account).await?;
    }
    let vault = vault
        .then(super::demo_vault)
        .transpose()
        .map_err(|error| format!("no vault: {error}"))?;
    let config = match &vault {
        Some(vault) => format!(
            "{}\n[focus.vault]\npath = \"{}\"\nprojects = \"Projects\"\n",
            super::config(),
            vault.path().display()
        ),
        None => super::config(),
    };
    let mut started = super::start(database, account, &config, &config_path(name), size)?;
    if let Some(vault) = vault {
        started.keep(vault);
    }
    started.sink.emit(Event::ConnectionChanged {
        account,
        state: ConnectionState::Online,
    });
    if seed == Seed::Backfilling {
        started.sink.emit(Event::BackfillProgress {
            account,
            done: 12_400,
            total: 81_744,
            footprint: None,
        });
    }
    wait_for_first_page(&started.window, seed);
    if seed == Seed::Search {
        wait_for_attachment_text(&started.store).await?;
    }
    Ok(started)
}

/// Without it, a file found only by its contents is a card on one runner and missing on another
/// (#2178).
async fn wait_for_attachment_text(store: &postio_storage::Store) -> Result<(), String> {
    use postio_storage::sql::RowExt as _;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let owed: bool = {
            let reader = store.read().await.map_err(|error| error.to_string())?;
            postio_storage::sql::one(
                &reader,
                "SELECT EXISTS (SELECT 1 FROM attachment_text_owed)",
                (),
                |row| row.col(0),
            )
            .await
            .map_err(|error| error.to_string())?
        };
        if !owed {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err("the search seed's files were still unread after a minute".to_owned());
        }
        pump_until(Duration::from_millis(50), || false);
    }
}

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

fn pump_until(max: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + max;
    let context = glib::MainContext::default();
    let heartbeat =
        glib::timeout_add_local(Duration::from_millis(10), || glib::ControlFlow::Continue);
    let mut reached = done();
    while !reached && Instant::now() < deadline {
        context.iteration(true);
        reached = done();
    }
    heartbeat.remove();
    reached
}

fn off_thread<T: Send + 'static>(
    work: impl std::future::Future<Output = Result<T, String>> + Send + 'static,
) -> Result<T, String> {
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = send.send(super::on_runtime(work));
    });
    let mut answer = None;
    pump_until(Duration::from_secs(10), || {
        answer = receive.try_recv().ok();
        answer.is_some()
    });
    answer.unwrap_or_else(|| Err("the store did not answer in time".to_owned()))
}

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

/// The overlay scrollbar's indicator shows when a list scrolls and hides after a while, so whether
/// a frame caught it depended on step timing; frames leave it out.
fn hold_still() {
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_enable_animations(false);
    }
    thread_local! {
        static HIDDEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    if HIDDEN.get() {
        return;
    }
    if let Some(display) = gtk::gdk::Display::default() {
        let provider = gtk::CssProvider::new();
        provider.load_from_string("scrollbar.overlay-indicator { opacity: 0; }");
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_USER + 1,
        );
        HIDDEN.set(true);
    }
}

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
    // Text scale is process-wide: set it from the DPI the process started with, every run, so one
    // storyboard's 200% is not the next one's start.
    if let Some(settings) = gtk::Settings::default() {
        static BASE_DPI: std::sync::OnceLock<i32> = std::sync::OnceLock::new();
        let base = *BASE_DPI.get_or_init(|| settings.gtk_xft_dpi());
        let scale = variant
            .get("text")
            .and_then(|t| t.parse::<f64>().ok())
            .unwrap_or(100.0);
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

/// `Err` is a step this runner cannot play, which makes the run unavailable rather than failed.
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
            // The composer's body is a web view, not a `GtkText`: it takes text through the editor.
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
                EnvEvent::NewMail => {
                    let store = acting.store.clone();
                    let (mailbox, message) =
                        off_thread(async move { super::deliver_new_mail(&store, account).await })
                            .map_err(|error| format!("new mail could not be filed: {error}"))?;
                    acting.sink.emit(Event::NewMail {
                        account,
                        mailbox,
                        messages: vec![message],
                    })
                }
                EnvEvent::Reply => {
                    let store = acting.store.clone();
                    let arrival = off_thread(async move {
                        let arrival = postio_demo::deliver_harbor_reply(&store, account).await?;
                        // Through the filing pass every arrival takes: a muted thread's reply is
                        // archived there.
                        postio_host::FocusSetup::default()
                            .file_stored(&store, arrival.message, arrival.thread)
                            .await
                            .map_err(|error| error.to_string())?;
                        let connection = store.connect().await.map_err(|e| e.to_string())?;
                        let filed = postio_storage::repository::MessageRepository::new(&connection)
                            .get(arrival.message)
                            .await
                            .map_err(|e| e.to_string())?
                            .ok_or("the reply is gone")?;
                        Ok::<_, String>((arrival, filed.mailbox_id))
                    })
                    .map_err(|error| format!("the reply could not be filed: {error}"))?;
                    let (arrival, landed) = arrival;
                    // A reply the filing pass archived (a muted thread's) is no arrival, only a
                    // changed thread.
                    (landed != arrival.mailbox
                        || acting.sink.emit(Event::NewMail {
                            account,
                            mailbox: arrival.mailbox,
                            messages: vec![arrival.message],
                        }))
                        && acting.sink.emit(Event::ThreadChanged {
                            account,
                            thread: arrival.thread,
                            members_added: vec![arrival.message],
                        })
                }
                EnvEvent::BodyArrived => {
                    let store = acting.store.clone();
                    let message =
                        off_thread(async move { super::deliver_held_body(&store, account).await })
                            .map_err(|error| format!("the body could not be stored: {error}"))?;
                    acting.sink.emit(Event::BodyLoaded {
                        account,
                        message,
                        sections: postio_core::event::Sections::Text,
                    })
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

fn settle_and_capture(
    window: &FocusWindow,
    options: &Options,
    step: usize,
    settings: &settle::Settings,
    writer: Option<&RunWriter>,
) -> (Settle, Option<Frame>, Option<String>) {
    // Settled the same whether or not frames are kept: a check run that waited less would disagree
    // with a filmed one about anything on a timer (#1744).
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
    let detached = window
        .composer()
        .and_then(|composer| composer.detached_window())
        .filter(|host| host.is_visible());
    let outlined = match detached {
        Some(host) => {
            outline::outlined_with_window(window.upcast_ref(), host.upcast_ref(), &region)
        }
        None => outline::outlined(window.upcast_ref(), &region),
    }
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

/// Awaited on the GTK main thread after `adw::init`, inside a multi-threaded tokio runtime.
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
    hold_still();
    let (size, ignored) = apply_variant(&options.variant);
    played.ignored_axes = ignored;
    played
        .ignored_axes
        .extend(options.ignored_axes.iter().cloned());

    let hold_body = board
        .steps
        .iter()
        .any(|step| step.input == Input::Event(EnvEvent::BodyArrived));
    let vault = board.preset.as_deref() == Some(Preset::Vault.id());
    let native = board.preset.as_deref() == Some(Preset::NativeComposer.id());
    postio_widgets::composer::surface::test_choose(
        native.then_some(postio_widgets::composer::surface::SurfaceKind::Native),
    );
    let started = match acting(base, size, &board.name, hold_body, vault).await {
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
    if let Some(preset) = board.preset.as_deref() {
        let Some(preset) = Preset::from_id(preset) else {
            played.status = Status::NotApplicable {
                reason: format!("Focus has no preset `{preset}`"),
            };
            end(started);
            return finish(played);
        };
        if let Err(message) = preset.apply(&started) {
            played.status = Status::Error { message };
            end(started);
            return finish(played);
        }
    }
    deliver::drain();

    let resolve = |reference: &StepRef| step_number(board, reference);
    let mut history: Vec<serde_json::Value> = Vec::new();

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

/// Commands the generated pass never presses because they reach outside the window: the network,
/// the desktop, or a file chooser that would wait for a person.
pub const NEVER_PRESSED: &[(&str, &str)] = &[
    ("refresh", "syncs, which dials the server"),
    ("retry_send", "sends, which dials the server"),
    ("show_images", "fetches remote images"),
    ("always_show_images", "fetches remote images"),
    ("unsubscribe", "follows an unsubscribe link off the machine"),
    ("edit_config", "opens an external editor"),
    ("edit_externally", "opens an external editor"),
    ("attach_file", "opens a file chooser"),
    ("insert_image", "opens a file chooser"),
    ("add_account", "may open a browser to sign in"),
    ("update_credential", "may open a browser to sign in"),
];

/// Commands that put the keyboard in each context from a fresh window. A context the setup does not
/// land in is reported, not pressed in.
pub const CONTEXTS: &[(&str, &[&str])] = &[
    ("list", &[]),
    ("search", &["search"]),
    ("filtered", &["go_to_filtered"]),
    ("digest", &["next_message", "open_message"]),
    ("reader", &["open_message"]),
    ("composer", &["compose"]),
];

/// Commands that act on a thread of more than one message, which the small seed's inbox has none
/// of: each is pressed over the long thread's seed.
/// A reply is opened to the thread's first message so a newer one is behind it.
const THREAD_COMMANDS: &[(&str, &str, &[&str])] = &[
    ("list", "expand_thread", &[]),
    ("list", "collapse_thread", &["expand_thread"]),
    ("list", "thread_outline", &["expand_thread"]),
    (
        "list",
        "move_this_message",
        &["expand_thread", "next_message"],
    ),
    (
        "list",
        "delete_this_message",
        &["expand_thread", "next_message"],
    ),
    (
        "composer",
        "reply_to_latest",
        &["open_message", "first_in_thread", "reply"],
    ),
    (
        "composer",
        "show_newer",
        &["open_message", "first_in_thread", "reply"],
    ),
    ("composer", "expand_quoted", &["forward", "forward_what_2"]),
];

#[derive(Debug, Clone)]
pub struct ContextCoverage {
    pub context: String,
    pub presses: Vec<postio_storyboard::coverage::Press>,
    pub skipped: Vec<(String, String)>,
    pub unreachable: Option<String>,
}

impl ContextCoverage {
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

pub async fn every_command(gap_list: &std::path::Path) -> Result<Vec<ContextCoverage>, String> {
    use postio_storyboard::coverage::{Effect, Press, judge, load_gaps};
    let gaps = load_gaps(gap_list)?;
    let gaps = gaps.as_slice();

    // One context, to rerun a gap list's entries: `POSTIO_STORYBOARD_CONTEXT=composer`.
    let only = std::env::var("POSTIO_STORYBOARD_CONTEXT").ok();
    // Counted across every context, so the shards stay even however commands fall between contexts.
    let shard = postio_storyboard::shard::Shard::from_env();
    let mut position = 0;
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
            position += 1;
            if !shard.takes(position - 1) {
                continue;
            }
            if let Some((_, why)) = NEVER_PRESSED.iter().find(|(id, _)| *id == command) {
                coverage.skipped.push((command, (*why).to_owned()));
                continue;
            }
            let thread = THREAD_COMMANDS
                .iter()
                .find(|(here, id, _)| here == context && *id == command);
            let started = match thread {
                Some((_, _, setup)) => {
                    let started = fresh_over(Seed::LongThread, setup).await;
                    // What the setup opened (a thread read for a quote) lands a moment after the
                    // key that asked for it.
                    pump(Duration::from_millis(1500));
                    started
                }
                None => fresh(setup).await,
            };
            let Some(started) = started else {
                continue;
            };
            let window = started.window.clone();
            let settings = settle::Settings {
                stride: 2,
                ..settle::Settings::default()
            };
            let before_frame = settle::settle(window.upcast_ref(), &settings);
            let before = window.observe();
            // Typing wins: with the keyboard in a text field, a bare-key command is a letter, not a
            // command.
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

async fn fresh(setup: &[&str]) -> Option<Started> {
    fresh_over(Seed::Small, setup).await
}

async fn fresh_over(seed: Seed, setup: &[&str]) -> Option<Started> {
    postio_ui::clock::freeze(
        chrono::DateTime::parse_from_rfc3339(FROZEN_AT)
            .expect("a fixed instant")
            .with_timezone(&chrono::Local),
    );
    hold_still();
    let (size, _) = apply_variant(&BTreeMap::new());
    let started = acting(seed, size, "every-command", false, false)
        .await
        .ok()?;
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
    // The composer's body paints and listens only once its script has loaded (#1716): judged
    // earlier, any press "changes" the frame as the editor finishes drawing.
    if started.window.observe().composer.open
        && let Some(composer) = started.window.composer()
    {
        pump_until(Duration::from_secs(5), || {
            composer.test_body_eval("window.postioEditorReady === true ? 'ready' : ''") == "ready"
        });
        pump(Duration::from_millis(300));
    }
    Some(started)
}

#[cfg(test)]
mod tests {
    use super::chrome_faces;

    #[test]
    fn the_chrome_faces_are_found_under_the_system_font_roots() {
        let root = tempfile::tempdir().unwrap();
        let sans = root.path().join("adwaita-sans-fonts");
        let mono = root.path().join("adwaita-mono-fonts");
        let other = root.path().join("dejavu");
        for dir in [&sans, &mono, &other] {
            std::fs::create_dir_all(dir).unwrap();
        }
        for file in [
            sans.join("AdwaitaSans-Regular.ttf"),
            sans.join("AdwaitaSans-Italic.ttf"),
            mono.join("AdwaitaMono-Bold.ttf"),
            other.join("DejaVuSans.ttf"),
        ] {
            std::fs::write(file, b"font").unwrap();
        }
        let names: Vec<String> = chrome_faces(&[root.path().to_path_buf()])
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            [
                "AdwaitaMono-Bold.ttf",
                "AdwaitaSans-Italic.ttf",
                "AdwaitaSans-Regular.ttf"
            ]
        );
    }
}
