//! Seeded, fed windows for the tools that photograph or drive Postio.
//!
//! `examples/shot.rs` and the storyboard runner both need a real `Window`
//! over a real, migrated, seeded store, fed through `feed_the_window` as the
//! application feeds it -- and a handful of panels that nothing in the
//! application reaches without a server or a person, which are fed by hand.
//! That setup lives here so the two share it rather than each keeping a
//! copy (specs/008-storyboards, R11). The argument parsing, the output and
//! the pictures stay with the tools.
//!
//! Behind the `demo` feature, which turns on `postio-storage/test-support`:
//! the seeds and the in-memory store are test-support code and must not
//! reach a normal build.
//!
//! # Seeds and presets
//!
//! A [`Seed`] is a condition of the *store* that no step can produce, such as
//! thirty threads or an unsent draft. A [`Preset`] is a condition of the
//! *window* with no command to reach it, such as the account form. Anything a
//! command can reach is a step, not a preset.
//!
//! # The clock
//!
//! Seeded timestamps read `postio_ui::clock::now`, so a frozen clock makes a
//! seed deterministic. Nothing here calls `Utc::now`.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::glib;
use postio_core::ConnectionState;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_model::ids::{AccountId, MailboxId};
use postio_session::Wiring;
use postio_storage::repository::MailboxRepository;
use postio_storage::seed::SeedReport;

use crate::{Wired, feed_the_window};

/// The instant a seed is stamped with: the interface's clock, in UTC.
fn clock_utc() -> chrono::DateTime<chrono::Utc> {
    postio_ui::clock::now().to_utc()
}

/// The conversation lengths of the `thirty-threads` seed: thirty-six threads,
/// mostly single messages, some exchanges, one of five.
const THIRTY_THREADS: &[usize] = &[
    1, 2, 1, 1, 3, 1, 1, 2, 1, 1, 1, 4, 1, 2, 1, 1, 1, 3, 1, 1, 2, 1, 1, 1, 5, 1, 1, 2, 1, 1, 1, 3,
    1, 1, 2, 1,
];

/// The one conversation of the `long-thread` seed: seven messages, the last
/// two unread.
const LONG_THREAD: &[usize] = &[7];

/// A condition of the store, named neutrally so more than one application can
/// answer to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seed {
    /// The corpus' mixed handful in one account: the default.
    Small,
    /// An account with its folders and no mail.
    Empty,
    /// A second account beside the first, with its own folder tree.
    TwoAccounts,
    /// The first-run orientation, which the store otherwise says is seen.
    FirstRun,
    /// A message queued to send and not yet sent.
    Outbox,
    /// A draft saved and never sent or queued.
    DraftLeftOver,
    /// One conversation of seven messages, the last two unread.
    LongThread,
    /// Thirty-six conversations in the Inbox.
    ThirtyThreads,
    /// A backfill in flight, with the size of the account's mail beside it.
    Backfilling,
}

impl Seed {
    /// Every seed, in the order the research table lists them.
    pub const ALL: [Seed; 9] = [
        Seed::Small,
        Seed::Empty,
        Seed::FirstRun,
        Seed::TwoAccounts,
        Seed::Outbox,
        Seed::DraftLeftOver,
        Seed::LongThread,
        Seed::ThirtyThreads,
        Seed::Backfilling,
    ];

    /// The name a storyboard uses.
    pub fn id(self) -> &'static str {
        match self {
            Seed::Small => "small",
            Seed::Empty => "empty",
            Seed::TwoAccounts => "two-accounts",
            Seed::FirstRun => "first-run",
            Seed::Outbox => "outbox",
            Seed::DraftLeftOver => "draft-left-over",
            Seed::LongThread => "long-thread",
            Seed::ThirtyThreads => "thirty-threads",
            Seed::Backfilling => "backfilling",
        }
    }

    /// The seed a storyboard's name stands for.
    pub fn from_id(id: &str) -> Option<Seed> {
        Seed::ALL.into_iter().find(|seed| seed.id() == id)
    }

    /// Whether this seed decides what mail the store holds, rather than
    /// adding to it. Exactly one base applies to a store.
    fn is_base(self) -> bool {
        matches!(
            self,
            Seed::Small | Seed::Empty | Seed::LongThread | Seed::ThirtyThreads
        )
    }
}

/// Which seeds a window is built from.
///
/// At most one is a *base* (`small`, `empty`, `long-thread`,
/// `thirty-threads`) and the rest are added on top of it, which is how
/// `shot demo accounts outbox` has always combined. With no base, `small`.
#[derive(Clone, Debug, Default)]
pub struct DemoOptions {
    seeds: Vec<Seed>,
}

impl DemoOptions {
    /// The default store: [`Seed::Small`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a seed.
    pub fn with(mut self, seed: Seed) -> Self {
        self.seeds.push(seed);
        self
    }

    /// Whether `seed` was asked for.
    pub fn has(&self, seed: Seed) -> bool {
        self.seeds.contains(&seed)
    }

    /// The seed that decides the store's mail.
    pub fn base(&self) -> Seed {
        self.seeds
            .iter()
            .copied()
            .find(|seed| seed.is_base())
            .unwrap_or(Seed::Small)
    }
}

/// A window condition with no command to reach it, hand-fed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    /// The settings window, on its first pane or on `pane`.
    Settings(Option<postio_gtk::settings::Section>),
    /// Three accounts with the weight of their mail beside each.
    AccountWeights,
    /// The account detail view; `tested` shows a connection test's answer,
    /// `signature` the signature editor.
    AccountDetail { tested: bool, signature: bool },
    /// The account detail view's Mailboxes group.
    AccountMailboxes,
    /// The add-account dialogue, on one of its steps.
    AddAccount(AddAccountStep),
    /// The composer, over a reply.
    Compose,
    /// The screen a store that will not open puts up.
    Locked,
    /// Canvas 2b's left column.
    SearchPanels,
}

/// The steps the add-account dialogue can be put on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddAccountStep {
    /// Choosing how to sign in.
    Route,
    /// Waiting for the browser.
    Browser,
    /// Choosing how much mail to keep.
    SyncWindow,
}

impl AddAccountStep {
    fn word(self) -> &'static str {
        match self {
            AddAccountStep::Route => "route",
            AddAccountStep::Browser => "browser",
            AddAccountStep::SyncWindow => "syncwindow",
        }
    }
}

impl Preset {
    /// The name a storyboard uses.
    pub fn id(self) -> String {
        use postio_gtk::settings::Section;
        match self {
            Preset::Settings(None) => "settings".to_owned(),
            Preset::Settings(Some(section)) => {
                let pane = match section {
                    Section::Filters => "filters",
                    Section::Composing => "composing",
                    Section::Appearance => "appearance",
                    Section::Keyboard => "keyboard",
                    Section::Sync => "storage",
                    Section::Privacy => "privacy",
                    Section::ConfigFile => "configfile",
                    Section::Accounts => "accounts",
                };
                format!("settings/{pane}")
            }
            Preset::AccountWeights => "settings/account-weights".to_owned(),
            Preset::AccountDetail {
                tested: false,
                signature: false,
            } => "settings/account-form".to_owned(),
            Preset::AccountDetail { tested: true, .. } => "settings/account-tested".to_owned(),
            Preset::AccountDetail {
                signature: true, ..
            } => "settings/signature-editor".to_owned(),
            Preset::AccountMailboxes => "settings/account-mailboxes".to_owned(),
            Preset::AddAccount(step) => format!("add-account/{}", step.word()),
            Preset::Compose => "compose".to_owned(),
            Preset::Locked => "locked".to_owned(),
            Preset::SearchPanels => "search-panels".to_owned(),
        }
    }

    /// The preset a storyboard's name stands for.
    pub fn from_id(id: &str) -> Option<Preset> {
        use postio_gtk::settings::Section;
        let pane = |section| Some(Preset::Settings(Some(section)));
        match id {
            "settings" => Some(Preset::Settings(None)),
            "settings/accounts" => pane(Section::Accounts),
            "settings/filters" => pane(Section::Filters),
            "settings/composing" => pane(Section::Composing),
            "settings/appearance" => pane(Section::Appearance),
            "settings/keyboard" => pane(Section::Keyboard),
            "settings/storage" => pane(Section::Sync),
            "settings/privacy" => pane(Section::Privacy),
            "settings/configfile" => pane(Section::ConfigFile),
            "settings/account-weights" => Some(Preset::AccountWeights),
            "settings/account-form" => Some(Preset::AccountDetail {
                tested: false,
                signature: false,
            }),
            "settings/account-tested" => Some(Preset::AccountDetail {
                tested: true,
                signature: false,
            }),
            "settings/signature-editor" => Some(Preset::AccountDetail {
                tested: false,
                signature: true,
            }),
            "settings/account-mailboxes" => Some(Preset::AccountMailboxes),
            "add-account/route" => Some(Preset::AddAccount(AddAccountStep::Route)),
            "add-account/browser" => Some(Preset::AddAccount(AddAccountStep::Browser)),
            "add-account/syncwindow" => Some(Preset::AddAccount(AddAccountStep::SyncWindow)),
            "compose" => Some(Preset::Compose),
            "locked" => Some(Preset::Locked),
            "search-panels" => Some(Preset::SearchPanels),
            _ => None,
        }
    }

    /// Puts the window in this condition. `wired` is what [`populate`]
    /// returned, when it ran: the search panels reuse its search view
    /// instead of attaching a second one (#831).
    pub fn apply(self, window: &Window, wired: Option<&'static Wired>) {
        match self {
            Preset::Settings(pane) => show_settings(window, pane),
            Preset::AccountWeights => show_account_weights(window),
            Preset::AccountDetail { tested, signature } => {
                show_account_detail(window, tested, signature)
            }
            Preset::AccountMailboxes => show_account_mailboxes(window),
            Preset::AddAccount(step) => show_add_account(window, step.word()),
            Preset::Compose => show_composer(window),
            Preset::Locked => show_locked(window),
            Preset::SearchPanels => show_search_panels(window, wired.and_then(|w| w.search)),
        }
    }
}

/// The screen a store that will not open puts up instead of the mail
/// (#404). Rendered from the same words `SecretError::Locked` writes, so
/// what this shows is what a person with a locked keyring sees.
pub fn show_locked(window: &Window) {
    let screen = postio_gtk::unavailable::Unavailable::new();
    screen.set_reason(
        "the login keyring is locked, so Postio cannot read the password \
         for ada@example.com. Unlock it in your keyring application — on \
         GNOME that is Passwords and Keys — and try again.",
    );
    window.set_content(Some(&postio_gtk::widgets::under_window_chrome(&screen)));
}

/// The runtime the store reads in this tool are driven on.
///
/// `main` returns `glib::ExitCode` and hands the thread to GTK, so it cannot
/// be `async`. What needs a runtime is the setup: seeding a store and feeding
/// the window. `block_on` polls the future on *this* thread, where GTK lives,
/// and `multi_thread` because `postio_session::blocking::now` -- how a
/// synchronous GTK callback reads the store -- reaches for `block_in_place`.
pub fn on_runtime<T>(future: impl std::future::Future<Output = T>) -> T {
    use std::sync::OnceLock;
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("a runtime for the shot")
        })
        .block_on(future)
}

/// Writes what `options` asks for into `database`: the base mail, the
/// overlays, and the settings that make the window an ordinary day's.
///
/// No window and no display: this is the half of [`populate`] that is only a
/// store, which is what lets a test count what a seed holds.
pub async fn seed_store(database: &postio_storage::Store, options: &DemoOptions) -> SeedReport {
    let report = match options.base() {
        Seed::Empty => postio_storage::seed::seed_conversations(database, 11, &[]).await,
        Seed::ThirtyThreads => {
            postio_storage::seed::seed_conversations(database, 11, THIRTY_THREADS).await
        }
        Seed::LongThread => {
            postio_storage::seed::seed_conversations(database, 11, LONG_THREAD).await
        }
        _ => postio_storage::seed::seed_small_with_bodies(database, 11).await,
    };
    let account = report.account.id;
    stamp_as_just_synced(database, &report).await;
    // Every shot is a first run otherwise -- the store is made here and
    // thrown away -- so the first-run orientation would sit across the top
    // of the compose shot, the settings shot and every other one. `demo
    // orientation` is how you ask to see it; the rest of the tool goes on
    // rendering the application as somebody uses it on any other day.
    if !options.has(Seed::FirstRun) {
        let connection = database.connect().await.expect("a connection");
        postio_storage::repository::SettingsRepository::new(&connection)
            .set("orientation_seen", "shot")
            .await
            .expect("the orientation is not what this shot is about");
    }
    // A real second account, in the store, rather than a pair of names handed
    // to the sidebar: the per-account sections are drawn from the folders the
    // feed reads, so a faked strip would draw headers over an empty tree and
    // could not fail when the wiring broke (#185).
    if options.has(Seed::TwoAccounts) {
        let second =
            postio_storage::seed::seed_extra_account(database, "Home", "home@example.net", 12)
                .await;
        stamp_as_just_synced(database, &second).await;
    }

    // A message on its way out, for the one row that is absent unless
    // something is (spec 003 FR-012).
    //
    // Queued through `DraftRepository` rather than staged: `queue_send` is
    // what `Composer::send` calls, it writes `send_state` and the operation
    // together, and the sidebar's Outbox row and its badge are read back out
    // of that column by `draft_counts`. Handing the sidebar a row here would
    // be the #596 trap this file warns about twice already -- a picture that
    // cannot fail when the path from the store to the pane is broken.
    //
    // Nothing drains it: a shot renders a window rather than running a
    // client, so the message stays where the picture wants it.
    if options.has(Seed::Outbox) {
        let connection = database.connect().await.expect("a connection");
        let drafts = postio_storage::repository::DraftRepository::new(&connection);
        let mut draft = postio_model::Draft::new(account);
        draft.subject = "Re: maildir index rebuild is O(n²)".to_owned();
        draft.to = vec![postio_model::EmailAddress::new(
            Some("Lena Tomlin"),
            "lena@example.com",
        )];
        draft.body.text = Some("Confirmed on 0.4.1 — sending the trace now.".to_owned());
        drafts.save(&mut draft).await.expect("the draft saves");
        drafts
            .queue_send(&mut draft, clock_utc())
            .await
            .expect("the send queues");
    }

    // An unsent draft already in the store at launch: saved and never queued,
    // so it is a draft someone walked away from rather than a message on its
    // way out. The sidebar's Drafts row and its count are read back out of the
    // store, as the Outbox's are.
    if options.has(Seed::DraftLeftOver) {
        let connection = database.connect().await.expect("a connection");
        let drafts = postio_storage::repository::DraftRepository::new(&connection);
        let mut draft = postio_model::Draft::new(account);
        draft.subject = "Notes for Thursday".to_owned();
        draft.to = vec![postio_model::EmailAddress::new(
            Some("Nadia Okafor"),
            "nadia@example.org",
        )];
        draft.body.text = Some("Agenda so far: the index rebuild, then the release.".to_owned());
        drafts.save(&mut draft).await.expect("the draft saves");
    }
    report
}

/// A seeded account, fed through the wiring the application uses.
///
/// **`feed_the_window`, not a stand-in for it.** This is the same call `run`
/// makes, over a real `Wiring` built on a migrated in-memory store, its own
/// blob store, and the runtime the reads are polled on. Everything between
/// SQLite and the panes is therefore in the picture, which is the whole
/// difference between a shot that can catch a wiring break and one that
/// cannot: #596 was filed because this used to hand the panes rows it had
/// read itself, so `shot ... demo open` drew a perfect reading pane through
/// the entire span of #70, when every real click left it blank.
///
/// The content is `postio_storage::seed`'s — corpus-derived messages with a
/// real folder tree, flags and threading — and `seed_small_with_bodies`
/// writes the fixtures' own bodies into the blob store, so the reader renders
/// mail rather than the "still downloading" plate.
///
/// # It dials nothing
///
/// `feed_the_window` reads the local store. `start_syncing` is the half that
/// opens a socket, and this never calls it.
///
/// Returns the `Wired` `feed_the_window` built, leaked `'static` like
/// everything else here — so a caller that also wants `search` can hand
/// `wired.search` to [`show_search_panels`] instead of it calling
/// `search::View::attach` a second time on the same shell (#831).
pub async fn populate(window: &Window, options: &DemoOptions) -> Option<&'static Wired> {
    let database = postio_storage::test_support::memory().await;
    let directory = tempfile::tempdir().expect("a blob directory for the shot");
    let blobs = postio_storage::BlobStore::open(
        directory.keep(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let report = seed_store(&database, options).await;
    let account = report.account.id;

    // A no-op command handler: a shot renders a window, it does not act on
    // one. The reads the panes make are polled on this runtime all the same.
    let (bridge, replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, events) = event_channel();
    let wiring = Wiring::new(database, blobs, bridge.handle(), sink, bridge.commands());

    // Leaked on purpose, all of it: the shot renders one window and exits, and
    // a `Wiring` or a `Bridge` dropped here would stop answering before the
    // first page arrived.
    let wiring: &'static Wiring = Box::leak(Box::new(wiring));
    let wired = feed_the_window(window, wiring)
        .await
        .expect("the seeded store has an account");

    // A connection that is up and has just finished a sync, so the status
    // line reads `idle · imap` / `last sync 12s` as the canvas draws it.
    wired.feeds.apply(&postio_core::Event::ConnectionChanged {
        account,
        state: ConnectionState::Online,
    });
    // A backfill in flight, with the size of the account's mail beside it
    // (#411). Applied only for the shot that wants it: the ordinary `demo`
    // line reads `idle · imap` / `last sync 12s`, which is the canvas.
    if options.has(Seed::Backfilling) {
        backfill_in_flight(&wired.feeds, account);
    }

    let wired: &'static Wired = Box::leak(Box::new(wired));
    Box::leak(Box::new(bridge));
    Box::leak(Box::new(replies));
    Box::leak(Box::new(events));

    // An empty store never has a first page to wait for: pump until the feed
    // has landed instead, and hand the wiring back.
    if options.base() == Seed::Empty {
        let context = glib::MainContext::default();
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            while context.iteration(false) {}
            std::thread::sleep(Duration::from_millis(10));
        }
        return Some(wired);
    }
    wait_for_first_page(window).then_some(wired)
}

/// Stamp every seeded folder as synced twelve seconds ago.
///
/// The seed has never talked to a server, and says so: `last_synced_at` is
/// `None` on every folder it writes. That is honest and it is the wrong
/// picture — the status line would read `never synced`, which is a shot of
/// the empty state rather than of the folder list the canvas draws. The old
/// hand-rolled source stamped this on the way past; now that the folders come
/// out of the store, the store is where it has to be stamped.
pub async fn stamp_as_just_synced(database: &postio_storage::Store, report: &SeedReport) {
    let connection = database.connect().await.expect("a checked-out connection");
    let repository = MailboxRepository::new(&connection);
    let synced = clock_utc() - chrono::Duration::seconds(12);
    for mailbox in &report.mailboxes {
        let mut mailbox = mailbox.clone();
        mailbox.last_synced_at = Some(synced);
        repository
            .update(&mailbox)
            .await
            .expect("stamp a seeded folder");
    }
}

/// Block until the list actually holds its first page of mail.
///
/// Every mode after `populate` reads the list back: `selected` picks rows out
/// of it, `conversation` opens the first one, `open` clicks it. The
/// hand-rolled source this replaced answered out of a `Vec` and was ready the
/// instant it was installed; a real `Wiring` crosses to the runtime and
/// answers on a later turn of the main loop, so without this a mode found an
/// empty list and drew the offline plate over a store with mail in it.
///
/// `peek`, not `n_items`: the count arrives with the page, but a row is only
/// resident once its page has been delivered, and a mode reading `item(0)`
/// while it was still a placeholder gets nothing back.
///
/// Pumped with `iteration(false)` and a sleep rather than blocking on
/// `iteration(true)`: this runs before the window is presented, and blocking
/// the main context here starves the frame clock every later `settle` counts
/// on — which surfaces as a blank render, or as "no frame after 5000ms",
/// rather than as a wait. The same shape the wiring tests use.
pub fn wait_for_first_page(window: &Window) -> bool {
    let list = window.list();
    let ready = || list.model().n_items() > 0 && list.model().peek(0).is_some();
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_millis(SETTLE_MS);
    while Instant::now() < deadline {
        while context.iteration(false) {}
        if ready() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    eprintln!(
        "shot: the seeded store's first page never arrived, so the panes are \
         empty. Nothing rendered below this would be a picture of anything."
    );
    false
}

/// Correspondents for the `@` mode, in the canvas' own cast.
///
/// Every address is a reserved domain, per CLAUDE.md.
pub fn sample_contacts() -> Vec<postio_model::Contact> {
    let person = |name: &str, address: &str, seen: u32| {
        let mut contact =
            postio_model::Contact::new(postio_model::EmailAddress::new(Some(name), address));
        contact.times_seen = seen;
        contact
    };
    vec![
        person("Lena Tomlin", "lena@example.com", 412),
        person("Nadia Okafor", "nadia@example.org", 96),
        person("Diogo Ferreira", "diogo@example.org", 54),
        person("Sara Abadi", "sara@example.com", 31),
        person("buildbot", "buildbot@example.net", 1204),
    ]
}

/// Canvas 2b's left column, over the artboard's own numbers.
///
/// `existing` is the view `feed_the_window` already installed, when there is
/// one — `demo search` has one, since `postio_app::search::install` is the
/// one call a running Postio makes and `populate` already ran it. Attaching
/// a second one on the same shell for the same demo is exactly #831: two
/// previews stacked in `shell.reader()`, and since #831 `register_reader_occupant`
/// panics on it rather than drawing it. Falling back to `View::attach` only
/// when there is no wiring behind the window keeps `shot out.png search`
/// (no `demo`) working — the one case that has nothing to reuse.
pub fn show_search_panels(window: &Window, existing: Option<&'static postio_gtk::search::View>) {
    use postio_search::facets::{Facets, Refinement, Scope, ScopeCount};

    let view = existing.unwrap_or_else(|| {
        Box::leak(Box::new(postio_gtk::search::View::attach(
            &window.shell(),
            &window.finder(),
        )))
    });
    let count = |scope, hits| ScopeCount { scope, hits };
    let refinement = |token: &str, hits| Refinement {
        token: token.to_owned(),
        hits,
    };
    view.set_facets(
        &Facets {
            scopes: vec![
                count(Scope::AllMail, 14),
                count(Scope::Inbox, 6),
                count(Scope::Lists, 8),
            ],
            refinements: vec![
                refinement("is:unread", 9),
                refinement("larger:1M", 5),
                refinement("is:flagged", 2),
                refinement("in:lkml", 8),
            ],
        },
        14,
    );
    view.set_searching(true);

    // Canvas 2b's own focused result, snippet and all. The markers are what
    // FTS5 puts around a match, so this is the shape a real hit arrives in.
    let marked = |text: &str| {
        text.replace('[', &postio_search::highlight::MATCH_START.to_string())
            .replace(']', &postio_search::highlight::MATCH_END.to_string())
    };
    view.set_focused(Some(&postio_search::SearchHit {
        message_id: postio_model::ids::MessageId::new(1),
        thread_id: Some(postio_model::ids::ThreadId::new(1)),
        mailbox_id: MailboxId::new(1),
        subject: Some("Re: maildir index rebuild is O(n²)".to_owned()),
        from: Some(postio_model::EmailAddress::new(
            Some("Lena Tomlin"),
            "lena@example.com",
        )),
        received_at: clock_utc(),
        snippet: marked(
            "…the rebuild walks every message once per folder rather than once per \
             store, so a 40k-message [maildir] takes about nine minutes on a cold \
             cache. The patch keys the header cache on the [maildir] filename…",
        ),
        score: -3.5,
    }));

    // And the body itself, so the match tint the reader stylesheet paints is
    // something that can be looked at rather than only asserted on.
    view.preview().set_body(
        postio_model::ids::MessageId::new(1),
        &postio_model::MessageBody {
            text: Some(
                "Confirmed on 0.4.1 — the rebuild walks every message once per folder \
                 rather than once per store, so a 40k-message maildir takes about nine \
                 minutes on a cold cache.\n\n\
                 The patch moves the header cache to a single pass and keys it on the \
                 maildir filename instead of Message-ID.\n"
                    .to_owned(),
            ),
            html: None,
        },
        Some("lena@example.com"),
    );
}

/// Canvas 3f's own sample file, so the shot can be held up against the
/// drawing.
pub fn show_settings(window: &Window, pane: Option<postio_gtk::settings::Section>) {
    // One fixed path, because the privacy pane prints it and a storyboard
    // films that pane: a path with the process id in it made every run a
    // different frame. Written beside it and renamed into place, so two
    // processes at once each read a whole file -- the same bytes either way.
    let path = std::env::temp_dir().join("postio-demo-settings.toml");
    let scratch =
        std::env::temp_dir().join(format!("postio-demo-settings-{}.toml", std::process::id()));
    std::fs::write(
        &scratch,
        "# edits here and in the window are the same file\n\
         [ui]\n\
         density = \"compact\"\n\
         theme = \"system\"\n\
         show_hover_actions = true\n\
         sender_avatars = false\n\
         [sync]\n\
         check_for_mail = \"idle\"\n\
         poll_interval_secs = 300\n\
         attachment_fetch = \"on_open\"\n\
         notify = true\n\
         [compose]\n\
         signature_on_reply = \"above_quote\"\n\
         [keys]\n\
         archive = \"a\"\n\
         archive_thread = \"A\"\n\
         undo = \"u\"\n",
    )
    .expect("a scratch config.toml for the shot");
    std::fs::rename(&scratch, &path).expect("the config.toml moved into place");
    window.settings().load(&path);
    window.open_settings();
    // Every pane wears the same frame, so a shot of one says nothing about
    // the others — checking the rebuild against `22-settings-panes.png`
    // means rendering each of them. `Accounts` is where the window opens,
    // so no argument draws that one.
    if let Some(pane) = pane {
        window.settings().show_section(pane);
    }
}

/// The account detail view's Mailboxes group (#966), open on an account
/// whose server has the shape that started all this: its own `Sent Messages`
/// beside a `Sent` another client made, an Archive the user has pointed by
/// hand, and a Junk folder the server no longer lists.
///
/// Hand-fed rather than seeded, for `show_account_weights`' reason: the three
/// states a row can be in -- automatic, chosen, and pointing at a folder that
/// has gone -- do not occur together in any one real account, and looking at
/// them side by side is the whole point of rendering this.
pub fn show_account_mailboxes(window: &Window) {
    use postio_gtk::settings::AccountMailboxes;
    use postio_model::MailboxRole;

    let mut account = postio_model::Account::new(
        "Ada Lovelace",
        postio_model::EmailAddress::new(Some("Ada Lovelace"), "ada@example.com"),
    );
    account.id = AccountId::new(1);
    account.enabled = true;

    let panel = window.settings();
    panel.set_accounts(vec![account]);
    panel.set_account_mailboxes(vec![(
        AccountId::new(1),
        AccountMailboxes {
            folders: vec![
                "INBOX".to_owned(),
                "Archive".to_owned(),
                "Deleted Messages".to_owned(),
                "Drafts".to_owned(),
                "Sent".to_owned(),
                "Sent Messages".to_owned(),
            ],
            chosen: vec![
                (MailboxRole::Archive, "Archive".to_owned()),
                (MailboxRole::Junk, "Posta indesiderata".to_owned()),
            ],
            resolved: vec![
                (MailboxRole::Sent, "Sent".to_owned()),
                (MailboxRole::Archive, "Archive".to_owned()),
                (MailboxRole::Drafts, "Drafts".to_owned()),
                (MailboxRole::Trash, "Deleted Messages".to_owned()),
            ],
            // The awkward account this shot exists to draw: a role the server
            // will not make a folder for, so the picker has to say why rather
            // than just "no folder" (spec 003, FR-031).
            refused: vec![(MailboxRole::Junk, "Permission denied".to_owned())],
        },
    )]);
    window.open_settings();
    panel.open_account_detail(AccountId::new(1));
}

/// Three account rows, to look at what #411 put under the names.
///
/// **A layout check, not a wiring check.** `demo settings` draws one row and
/// `demo accounts settings` two, both through `feed_the_window` -- those are
/// the shots that can fail when nothing feeds them (#596). This one
/// hand-feeds three, because "does the second line still read at three rows,
/// one of them long enough to ellipsize" is a question about spacing that
/// only three rows can answer, and because the seed cannot produce the three
/// states side by side.
///
/// The three are the states that look different: payloads not being fetched,
/// payloads being fetched, and totals still being counted.
/// The account detail view (#880), on an account that has signatures.
///
/// Its own flag because the view is reached by activating a row, so no
/// existing mode renders it — and #979's signature picker is *hidden* for an
/// account with none, which is correct and also means the ordinary demo
/// store cannot show it. Two signatures here, so the row is on screen and
/// can be looked at.
/// `tested` also shows what a connection test found (#980) -- set by hand
/// rather than by pressing the button, and that is the point: pressing it
/// would dial a real server, and a shot must not. What there is to look at is
/// the row's *shape*, which has to read as the server having said no rather
/// than as Postio being broken, in dark and high contrast too.
pub fn show_account_detail(window: &Window, tested: bool, signature: bool) {
    let mut account = postio_model::Account::new(
        "Ada Lovelace",
        postio_model::EmailAddress::new(Some("Ada Lovelace"), "ada@example.com"),
    );
    account.id = AccountId::new(1);
    account.enabled = true;
    account.incoming.host = "imap.example.com".to_owned();
    account.incoming.port = 993;
    account.outgoing.host = "smtp.example.com".to_owned();
    account.outgoing.port = 587;
    let mut work = postio_model::Signature::new("Work", "-- \nAda, Analytical Engines");
    work.id = postio_model::ids::SignatureId::new(1);
    let mut brief = postio_model::Signature::new("Brief", "-- \nAda");
    brief.id = postio_model::ids::SignatureId::new(2);
    account.default_signature_id = Some(work.id);
    account.signatures = vec![work, brief];

    let panel = window.settings();
    panel.set_accounts(vec![account]);
    // Open, never toggle: `settings account` asks for the account form in
    // the settings window, and a toggle after `show_settings` has already
    // opened it closes the very window the shot is about — which then
    // cannot be drawn at all (#1179).
    window.open_settings();
    panel.open_account_detail(AccountId::new(1));
    if signature {
        // The editor, on the signature the account already has (#1086).
        panel.open_signature_editor(Some(postio_model::ids::SignatureId::new(1)));
    }
    if tested {
        panel.set_connection_status(postio_gtk::settings::ConnectionStatus::Answered {
            incoming: Ok(()),
            outgoing: Err("could not reach smtp.example.com:587: connection refused".to_owned()),
        });
    }
}

pub fn show_account_weights(window: &Window) {
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
        account.id = AccountId::new(id);
        account.enabled = true;
        account
    };

    let panel = window.settings();
    panel.set_accounts(vec![
        account(1, "Ada Lovelace", "ada@example.com"),
        account(2, "Grace Hopper", "grace@example.com"),
        account(3, "A rather long display name", "someone@example.invalid"),
    ]);
    panel.set_mail_weights(
        &[
            (
                AccountId::new(1),
                footprint(12_884_901_888, 11_811_160_064, 933_232_640, true),
            ),
            (
                AccountId::new(2),
                footprint(1_503_238_553, 1_400_000_000, 933_232_640, true),
            ),
            (
                AccountId::new(3),
                footprint(12_884_901_888, 11_811_160_064, 933_232_640, false),
            ),
        ],
        false,
    );
    window.open_settings();
}

/// The backfill the `backfilling` seed shows in flight (#411), with the size
/// of the account's mail beside it. Shared by `shot` and the storyboard
/// runner, which wire the window differently and must not drift apart.
pub(crate) fn backfill_in_flight(feeds: &postio_gtk::feed::Feeds, account: AccountId) {
    feeds.apply(&postio_core::Event::BackfillProgress {
        account,
        done: 12_400,
        total: 81_744,
        footprint: Some(postio_core::event::MailFootprint {
            total_bytes: 1_503_238_553,
            attachment_bytes: 1_400_000_000,
            local_bytes: 933_232_640,
            complete: true,
        }),
    });
}

/// Canvas 2a's own reply, so the composer can be held up against the drawing.
///
/// The canvas' addresses are not ours to ship: every one here is a reserved
/// domain, per CLAUDE.md.
/// The add-account dialogue, on whichever of its three steps was asked for.
///
/// **The widget, driven directly — not the flow.** `Onboarding` is a form
/// and a set of states (`postio_gtk::onboarding`'s own module doc): it does
/// not probe, connect or write, and the composition root supplies all
/// three. So a shot can put it in any of its states without dialling
/// anything, which is the only way this screen is renderable at all — it
/// otherwise needs a provider, a browser and a person.
///
/// It is a layout check, not a wiring check, and says so for the reason
/// #596 recorded: a picture drawn from state the tool wrote itself cannot
/// fail when the wiring is broken. What it *can* catch is the thing the
/// browser step is for — whether a person waiting on their browser can see
/// what they are about to approve.
pub fn show_add_account(window: &Window, step: &str) {
    use postio_gtk::onboarding::{BrowserSignIn, Onboarding, Server, Settings, Status};

    let screen = Onboarding::new();
    screen.set_address("lena.tomlin@example.com");
    let settings = Settings {
        imap: Server {
            host: "outlook.office365.com".to_owned(),
            port: 993,
            security: postio_model::TransportSecurity::Tls,
        },
        smtp: Server {
            host: "smtp.office365.com".to_owned(),
            port: 587,
            security: postio_model::TransportSecurity::StartTls,
        },
        login: "lena.tomlin@example.com".to_owned(),
        source: "Microsoft 365".to_owned(),
        oauth_sign_in: true,
        ..Settings::default()
    };
    screen.set_status(Status::Found(settings));

    match step {
        "browser" => {
            screen.set_browser_sign_in(BrowserSignIn {
                provider: "Microsoft 365".to_owned(),
                scopes: vec![
                    "https://outlook.office.com/IMAP.AccessAsUser.All".to_owned(),
                    "https://outlook.office.com/SMTP.Send".to_owned(),
                    "offline_access".to_owned(),
                ],
                redirect_uri: "http://127.0.0.1:41337/".to_owned(),
                authorize_url: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize"
                    .to_owned(),
            });
            screen.set_status(Status::WaitingForBrowser);
        }
        "syncwindow" => screen.set_status(Status::SyncWindow),
        _ => {}
    }

    let dialog = adw::Dialog::builder()
        .title("Add account")
        .content_width(460)
        .content_height(520)
        .child(&screen)
        .build();
    dialog.present(Some(window));
}

pub fn show_composer(window: &Window) {
    let account = AccountId::new(1);
    let identity = |name: &str, address: &str, default| postio_model::Identity {
        display_name: name.to_owned(),
        is_default: default,
        signature: Some(postio_model::Signature {
            id: Default::default(),
            name: String::new(),
            text: format!("{name} · postio.example.com"),
            html: None,
        }),
        ..postio_model::Identity::new(
            account,
            postio_model::EmailAddress::new(Some(name), address),
        )
    };

    // `Window::composer`, not `composer::install`: the window caches the one
    // it mounted, and installing a second means the shot renders one composer
    // while `detached` below pops out another.
    let composer = window.composer();
    composer.set_identities(vec![
        identity("Lena Tomlin", "lena@example.com", true),
        identity("Lena Tomlin", "lena@example.net", false),
    ]);

    let mut draft = postio_model::Draft::new(account);
    draft.kind = postio_model::DraftKind::Reply;
    draft.to = vec![postio_model::EmailAddress::new(
        Some("Diogo Ferreira"),
        "diogo@example.org",
    )];
    draft.subject = "Re: mbox importer review".to_owned();
    draft.body = postio_model::MessageBody {
        text: Some(
            "Looking now. The folder walker reads right, but I'd key the dedupe on \
             the maildir filename so it matches the index patch — otherwise the two \
             disagree on re-imported mail.\n\n\
             > Small diff, mostly the folder walker and a\n\
             > dedupe pass keyed on Message-ID.\n"
                .to_owned(),
        ),
        html: None,
    };
    composer.open(draft);
}

/// How many frames to let the window paint before the shot is taken.
///
/// One to allocate, one to settle any size that depended on it, and the
/// rest for work that only starts once there is a viewport to fill — the
/// message list asks for its first page from inside its first layout, so
/// the rows are a frame or two behind the panes around them.
pub const SETTLE_FRAMES: u32 = 8;

/// The ceiling on that wait, so a window that never paints reports rather
/// than hangs.
pub const SETTLE_MS: u64 = 5000;

/// Run the main loop until `window` has painted [`SETTLE_FRAMES`] frames.
///
/// Not a spin count. `MainContext::iteration(false)` returns immediately
/// when nothing is pending, so a fixed number of them is not a wait at all
/// and the frame clock may never tick inside it — which is how this example
/// came to render an empty message list while the running application drew
/// it correctly. Counting actual frames is the thing that was meant all
/// along. The heartbeat guarantees the blocking iteration returns.
pub fn settle(window: &impl IsA<gtk::Widget>) {
    let left = Rc::new(Cell::new(SETTLE_FRAMES));
    window.as_ref().add_tick_callback(glib::clone!(
        #[strong]
        left,
        move |_, _| {
            left.set(left.get().saturating_sub(1));
            if left.get() == 0 {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        }
    ));

    let context = glib::MainContext::default();
    let heartbeat =
        glib::timeout_add_local(Duration::from_millis(10), || glib::ControlFlow::Continue);
    let deadline = Instant::now() + Duration::from_millis(SETTLE_MS);
    while left.get() > 0 && Instant::now() < deadline {
        context.iteration(true);
    }
    heartbeat.remove();
}

/// What each seed leaves in the store, counted without a window.
#[cfg(test)]
mod tests {
    use super::*;
    use postio_storage::repository::{DraftRepository, ThreadListQuery, ThreadRepository};

    async fn seeded(seeds: &[Seed]) -> (postio_storage::Store, SeedReport) {
        let database = postio_storage::test_support::memory().await;
        let options = seeds
            .iter()
            .fold(DemoOptions::new(), |options, seed| options.with(*seed));
        let report = seed_store(&database, &options).await;
        (database, report)
    }

    async fn threads(
        database: &postio_storage::Store,
        account: AccountId,
    ) -> Vec<postio_storage::repository::ThreadListRow> {
        let connection = database.connect().await.expect("a connection");
        let mut query = ThreadListQuery::account(account);
        query.limit = 500;
        ThreadRepository::new(&connection)
            .page(&query)
            .await
            .expect("the thread list")
    }

    #[tokio::test]
    async fn thirty_threads_holds_at_least_thirty_conversations() {
        let (database, report) = seeded(&[Seed::ThirtyThreads]).await;
        let rows = threads(&database, report.account.id).await;
        assert!(rows.len() >= 30, "{} threads", rows.len());
        assert!(rows.iter().any(|row| row.unread_count > 0));
        assert!(rows.iter().any(|row| row.message_count > 1));
    }

    #[tokio::test]
    async fn long_thread_is_one_conversation_of_six_or_more_with_some_unread() {
        let (database, report) = seeded(&[Seed::LongThread]).await;
        let rows = threads(&database, report.account.id).await;
        assert_eq!(rows.len(), 1, "one conversation");
        assert!(rows[0].message_count >= 6, "{}", rows[0].message_count);
        assert!(rows[0].unread_count > 0);
        assert!(rows[0].unread_count < rows[0].message_count, "some read");
    }

    #[tokio::test]
    async fn draft_left_over_is_saved_and_never_queued() {
        let (database, report) = seeded(&[Seed::DraftLeftOver]).await;
        let connection = database.connect().await.expect("a connection");
        let drafts = DraftRepository::new(&connection)
            .list_for_account(report.account.id)
            .await
            .expect("the drafts");
        assert_eq!(drafts.len(), 1);
        assert!(
            DraftRepository::new(&connection)
                .by_state(postio_model::DraftState::Queued)
                .await
                .expect("the queued sends")
                .is_empty(),
            "an unsent draft is not on its way out"
        );
    }

    #[tokio::test]
    async fn the_small_seed_has_no_leftover_draft_and_the_empty_seed_no_mail() {
        let (database, report) = seeded(&[Seed::Small]).await;
        let connection = database.connect().await.expect("a connection");
        assert!(
            DraftRepository::new(&connection)
                .list_for_account(report.account.id)
                .await
                .expect("the drafts")
                .is_empty()
        );
        assert!(report.message_count > 0);

        let (_, report) = seeded(&[Seed::Empty]).await;
        assert_eq!(report.message_count, 0);
    }

    #[test]
    fn every_seed_and_preset_name_round_trips() {
        for seed in Seed::ALL {
            assert_eq!(Seed::from_id(seed.id()), Some(seed));
        }
        for id in [
            "settings",
            "settings/keyboard",
            "settings/account-form",
            "add-account/browser",
            "compose",
            "locked",
            "search-panels",
        ] {
            assert_eq!(Preset::from_id(id).map(Preset::id).as_deref(), Some(id));
        }
    }
}

pub mod storyboard;
