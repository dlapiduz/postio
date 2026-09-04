//! Render a Postio window straight out of GSK to a PNG.
//!
//! The canvas is the visual spec for this lane, and "matches the canvas" is
//! not something to check by squinting at a running app. This asks GTK for the
//! exact render node it would put on screen and writes it to a file, so a
//! change in spacing, weight or colour is something you can look at, diff and
//! attach to a review.
//!
//! ```sh
//! cargo run -p postio-app --example shot -- /tmp/plate.png             # light
//! cargo run -p postio-app --example shot -- /tmp/plate.png dark
//! cargo run -p postio-app --example shot -- /tmp/plate.png dark hc
//! cargo run -p postio-app --example shot -- /tmp/narrow.png 900x700
//! cargo run -p postio-app --example shot -- /tmp/plate.png demo
//! cargo run -p postio-app --example shot -- /tmp/settings.png demo settings
//! cargo run -p postio-app --example shot -- /tmp/rows.png settings weights
//! cargo run -p postio-app --example shot -- /tmp/compose.png demo compose
//! cargo run -p postio-app --example shot -- /tmp/popout.png demo compose detached
//! cargo run -p postio-app --example shot -- /tmp/tight.png demo compact
//! cargo run -p postio-app --example shot -- /tmp/large.png demo text2
//! cargo run -p postio-app --example shot -- /tmp/box.png demo command
//! cargo run -p postio-app --example shot -- /tmp/who.png demo contact
//! cargo run -p postio-app --example shot -- /tmp/selected.png demo selected
//! cargo run -p postio-app --example shot -- /tmp/reader.png demo open 1600x900
//! cargo run -p postio-app --example shot -- /tmp/thread.png demo thread 1600x900
//! cargo run -p postio-app --example shot -- /tmp/locked.png locked
//! ```
//!
//! `demo` fills the panes by calling `feed_the_window` over a real `Wiring` —
//! the same call `run` makes — on a migrated in-memory database with a real
//! folder tree, corpus-derived messages, flags, threading and the fixtures'
//! own bodies in a blob store. A shot of hand-written rows can only prove
//! that the *drawing* is right, and one that reads the rows itself can only
//! prove that the drawing is right about content the store produces. This one
//! goes through the wiring, so it is also about the panes actually being fed:
//! `demo open` renders a body the reader loaded out of the blob store, and a
//! break anywhere between SQLite and the pane shows up as an empty shot
//! (#596, and #70 is what it cost to learn that). `settings` opens the canvas
//! 3f panel over a sample `config.toml` written to a scratch directory.
//!
//! # Why this lives in `postio-app`
//!
//! Because of that seed. `postio-gtk` may not depend on `rusqlite`, and
//! `scripts/checks/check-crate-boundaries.py` counts a crate's own dev-dependencies —
//! an example is built from that graph — so a `shot` that reads a store cannot
//! live beside the widgets it renders. `postio-app` is the crate that already
//! knows both halves exist, which is what a shot of the real application over
//! a real store is.
//!
//! `comfortable` and `compact` render the other two row densities — a design
//! that only works at one of them is unfinished. `text2` is GNOME's
//! text-scaling setting at 200%, which is how a partially sighted user
//! actually reads this application, and the only way to see that the type
//! scale moves with them rather than ignoring them.
//!
//! It is a development tool, not part of the application: examples are not
//! built into the shipped binary. Nothing here touches the network — the
//! database it reads is created, seeded and thrown away in process.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::{gdk, glib};
use postio_app::feed_the_window;
use postio_core::ConnectionState;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::{app, capture, fonts, style, window::Window};
use postio_model::ids::{AccountId, MailboxId};
use postio_session::Wiring;
use postio_storage::repository::MailboxRepository;
use postio_storage::seed::SeedReport;

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
fn populate(
    window: &Window,
    two_accounts: bool,
    backfill: bool,
) -> Option<&'static postio_app::Wired> {
    let database = postio_storage::test_support::memory();
    let directory = tempfile::tempdir().expect("a blob directory for the shot");
    let blobs = postio_storage::BlobStore::open(
        directory.keep(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let report = postio_storage::seed::seed_small_with_bodies(&database, 11);
    let account = report.account.id;
    stamp_as_just_synced(&database, &report);
    // A real second account, in the store, rather than a pair of names handed
    // to the sidebar: the per-account sections are drawn from the folders the
    // feed reads, so a faked strip would draw headers over an empty tree and
    // could not fail when the wiring broke (#185).
    if two_accounts {
        let second =
            postio_storage::seed::seed_extra_account(&database, "Home", "home@example.net", 12);
        stamp_as_just_synced(&database, &second);
    }

    // A no-op command handler: a shot renders a window, it does not act on
    // one. The reads the panes make are polled on this runtime all the same.
    let (bridge, replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, events) = event_channel();
    let wiring = Wiring::new(database, blobs, bridge.handle(), sink, bridge.commands());

    // Leaked on purpose, all of it: the shot renders one window and exits, and
    // a `Wiring` or a `Bridge` dropped here would stop answering before the
    // first page arrived.
    let wiring: &'static Wiring = Box::leak(Box::new(wiring));
    let wired = feed_the_window(window, wiring).expect("the seeded store has an account");

    // A connection that is up and has just finished a sync, so the status
    // line reads `idle · imap` / `last sync 12s` as the canvas draws it.
    wired.feeds.apply(&postio_core::Event::ConnectionChanged {
        account,
        state: ConnectionState::Online,
    });
    // A backfill in flight, with the size of the account's mail beside it
    // (#411). Applied only for the shot that wants it: the ordinary `demo`
    // line reads `idle · imap` / `last sync 12s`, which is the canvas.
    if backfill {
        wired.feeds.apply(&postio_core::Event::BackfillProgress {
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

    let wired: &'static postio_app::Wired = Box::leak(Box::new(wired));
    Box::leak(Box::new(bridge));
    Box::leak(Box::new(replies));
    Box::leak(Box::new(events));

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
fn stamp_as_just_synced(database: &postio_storage::Database, report: &SeedReport) {
    let connection = database.connection().expect("a checked-out connection");
    let repository = MailboxRepository::new(&connection);
    let synced = chrono::Utc::now() - chrono::Duration::seconds(12);
    for mailbox in &report.mailboxes {
        let mut mailbox = mailbox.clone();
        mailbox.last_synced_at = Some(synced);
        repository.update(&mailbox).expect("stamp a seeded folder");
    }
}

/// Block until the list actually holds its first page of mail.
///
/// Every mode after `populate` reads the list back: `selected` picks rows out
/// of it, `thread` drills into the first one, `open` clicks it. The
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
fn wait_for_first_page(window: &Window) -> bool {
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
fn sample_contacts() -> Vec<postio_model::Contact> {
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
fn show_search_panels(window: &Window, existing: Option<&'static postio_gtk::search::View>) {
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
        received_at: chrono::Utc::now(),
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
fn show_settings(window: &Window) {
    let path =
        std::env::temp_dir().join(format!("postio-shot-settings-{}.toml", std::process::id()));
    std::fs::write(
        &path,
        "# edits here and in the panel are the same file\n\
         [ui]\n\
         density = \"compact\"\n\
         theme = \"system\"\n\
         show_hover_actions = true\n\
         thread_drill = true\n\n\
         [keys]\n\
         archive = \"a\"\n\
         archive_thread = \"A\"\n\
         undo = \"u\"\n",
    )
    .expect("a scratch config.toml for the shot");
    window.settings().load(&path);
    window.open_settings();
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
fn show_account_weights(window: &Window) {
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
}

/// Canvas 2a's own reply, so the composer can be held up against the drawing.
///
/// The canvas' addresses are not ours to ship: every one here is a reserved
/// domain, per CLAUDE.md.
fn show_composer(window: &Window) {
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
const SETTLE_FRAMES: u32 = 8;

/// The ceiling on that wait, so a window that never paints reports rather
/// than hangs.
const SETTLE_MS: u64 = 5000;

/// Run the main loop until `window` has painted [`SETTLE_FRAMES`] frames.
///
/// Not a spin count. `MainContext::iteration(false)` returns immediately
/// when nothing is pending, so a fixed number of them is not a wait at all
/// and the frame clock may never tick inside it — which is how this example
/// came to render an empty message list while the running application drew
/// it correctly. Counting actual frames is the thing that was meant all
/// along. The heartbeat guarantees the blocking iteration returns.
fn settle(window: &impl IsA<gtk::Widget>) {
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

/// Every literal mode word `flag` checks for below, so an argument matching
/// none of them can be caught rather than silently ignored (#599).
const KNOWN_FLAGS: &[&str] = &[
    "dark",
    "hc",
    "demo",
    "accounts",
    "backfill",
    "locked",
    "comfortable",
    "compact",
    "command",
    "folder",
    "contact",
    "search",
    "syncing",
    "settings",
    "weights",
    "compose",
    "detached",
    "selected",
    "thread",
    "open",
];

/// Every argument (after the output path) that matches none of
/// [`KNOWN_FLAGS`], no `WxH` size and no `text` scale prefix.
///
/// #599's actual cause: consecutive shots looked broken, and the working
/// hypothesis was a compositor that had stopped delivering frame callbacks
/// to the second window and later. It had not -- every `settle` still saw
/// its full run of frames -- and the real fault reproduces on the very
/// first shot, not the second: `for m in "demo" "demo thread"; do shot
/// out.png $m; done` passes `$m` unquoted, and zsh (unlike bash) does not
/// word-split that by default. "demo thread" then arrives as one argument
/// that matches no flag `flag()` checks for, nothing this tool recognizes
/// runs, and the window renders exactly the state it was in before any mode
/// flag took effect -- for a first render, the pre-populate placeholder:
/// empty sidebar, "offline · never synced". A confident, wrong picture,
/// with nothing on screen saying why.
fn unrecognized_arguments(args: &[String]) -> Vec<&str> {
    args.iter()
        .skip(1)
        .filter(|token| {
            !KNOWN_FLAGS.contains(&token.as_str())
                && token
                    .split_once('x')
                    .is_none_or(|(w, h)| w.parse::<i32>().is_err() || h.parse::<i32>().is_err())
                && !token.starts_with("text")
        })
        .map(String::as_str)
        .collect()
}

fn warn_about_unrecognized_arguments(args: &[String]) {
    for token in unrecognized_arguments(args) {
        eprintln!(
            "shot: '{token}' is not a mode this tool recognizes, and was silently \
             ignored -- the picture below is whatever the window looked like before \
             any mode flag took effect, not a picture of '{token}'. If this came from \
             a shell variable holding more than one word (say, `demo thread`), check \
             that it was word-split: zsh does not split an unquoted `$var` the way \
             bash does, so `for m in \"demo thread\"; do shot out.png $m; done` passes \
             \"demo thread\" as one argument under zsh and two under bash. See #599."
        );
    }
}

#[cfg(test)]
mod unrecognized_argument_tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        // `args[0]` is always the output path, and `unrecognized_arguments`
        // (like `flag`, `size` and the `text` scan in `main`) skips it.
        std::iter::once("out.png".to_owned())
            .chain(words.iter().map(|w| w.to_string()))
            .collect()
    }

    #[test]
    fn every_known_flag_is_recognized() {
        for name in KNOWN_FLAGS {
            assert_eq!(
                unrecognized_arguments(&args(&[name])),
                Vec::<&str>::new(),
                "{name} is in KNOWN_FLAGS but was flagged as unrecognized"
            );
        }
    }

    #[test]
    fn a_size_argument_is_recognized() {
        assert_eq!(
            unrecognized_arguments(&args(&["1400x800"])),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn a_text_scale_argument_is_recognized() {
        assert_eq!(
            unrecognized_arguments(&args(&["text150"])),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn two_words_collapsed_into_one_shell_argument_is_flagged() {
        // #599: exactly what an unquoted `$mode` set to "demo thread"
        // becomes under a shell that does not word-split it.
        assert_eq!(
            unrecognized_arguments(&args(&["demo thread"])),
            vec!["demo thread"]
        );
    }

    #[test]
    fn a_plain_typo_is_flagged() {
        assert_eq!(unrecognized_arguments(&args(&["dmeo"])), vec!["dmeo"]);
    }

    #[test]
    fn a_normally_split_pair_is_not_flagged() {
        assert_eq!(
            unrecognized_arguments(&args(&["demo", "thread", "1400x800"])),
            Vec::<&str>::new()
        );
    }
}

fn main() -> glib::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args
        .first()
        .cloned()
        .unwrap_or_else(|| "postio.png".to_string());
    warn_about_unrecognized_arguments(&args);
    let flag = |name: &str| args.iter().skip(1).any(|a| a == name);
    // A `WxH` argument forces the window size, which is how the adaptive
    // modes get rendered without a compositor in the loop.
    let size = args.iter().skip(1).find_map(|a| {
        let (w, h) = a.split_once('x')?;
        Some((w.parse::<i32>().ok()?, h.parse::<i32>().ok()?))
    });
    let scheme = if flag("dark") {
        adw::ColorScheme::ForceDark
    } else {
        adw::ColorScheme::ForceLight
    };
    let high_contrast = flag("hc");

    if adw::init().is_err() {
        eprintln!("shot: no display; rendering needs a Wayland or X11 session");
        return glib::ExitCode::FAILURE;
    }
    // Same order as `app::run`: fonts before the first widget.
    fonts::install().expect("the embedded fonts should install");
    let display = gdk::Display::default().expect("a display");
    style::install(&display);
    app::install_icons(&display);
    adw::StyleManager::default().set_color_scheme(scheme);

    // GNOME's "Large Text" works by moving `gtk-xft-dpi`, so this is what
    // a text-scaling user actually sees.
    if let Some(factor) = args.iter().skip(1).find_map(|a| a.strip_prefix("text")) {
        let factor: f64 = factor.parse().unwrap_or(2.0);
        if let Some(settings) = gtk::Settings::default() {
            let base = settings.gtk_xft_dpi();
            settings.set_gtk_xft_dpi((base as f64 * factor) as i32);
        }
    }

    let window = Window::default();
    if high_contrast {
        window.add_css_class(style::HIGH_CONTRAST_CLASS);
    }
    if let Some((width, height)) = size {
        window.set_default_size(width, height);
    }
    // `demo`'s own search view, if `demo` ran — `search` below reuses it
    // rather than attaching a second one on the same shell (#831).
    let wired: Option<&'static postio_app::Wired> = if flag("demo") {
        // A `demo` whose panes were never filled is not a slightly worse
        // picture, it is a picture of the empty state over a store with mail
        // in it -- which used to be rendered, saved, and reported as a
        // success under a warning nobody was required to read (#809).
        match populate(&window, flag("accounts"), flag("backfill")) {
            Some(wired) => Some(wired),
            None => {
                eprintln!("shot: NO IMAGE WAS WRITTEN to {path}");
                return glib::ExitCode::FAILURE;
            }
        }
    } else {
        None
    };
    // The screen a store that will not open puts up instead of the mail
    // (#404). Rendered from the same words `SecretError::Locked` writes, so
    // what this shows is what a person with a locked keyring sees.
    if flag("locked") {
        let screen = postio_gtk::unavailable::Unavailable::new();
        screen.set_reason(
            "the login keyring is locked, so Postio cannot read the password \
             for ada@example.com. Unlock it in your keyring application — on \
             GNOME that is Passwords and Keys — and try again.",
        );
        window.set_content(Some(&screen));
    }
    // The list has three row heights and a design that only works at one of
    // them is unfinished, so the shot can render any of them.
    for (name, density) in [
        ("comfortable", postio_config::Density::Comfortable),
        ("compact", postio_config::Density::Compact),
    ] {
        if flag(name) {
            window.list().set_density(density);
        }
    }
    // The one box, in the mode a prefix puts it in. `postio-cfd.1` folded
    // the palette and the query bar into this; a surface nobody can render
    // is a surface nobody checks against the canvas.
    if flag("command") {
        window.open_finder(postio_gtk::finder::Mode::Command);
    }
    if flag("folder") {
        window.open_finder(postio_gtk::finder::Mode::Mailbox);
    }
    if flag("contact") {
        window.finder().set_contacts(&sample_contacts());
        window.open_finder(postio_gtk::finder::Mode::Contact);
    }
    if flag("search") {
        window.open_finder(postio_gtk::finder::Mode::Search);
        window.finder().set_query(postio_gtk::finder::Query {
            mode: postio_gtk::finder::Mode::Search,
            text: "maildir from:lena has:attach after:aug1".into(),
        });
        // Canvas 2b's own readout. Delivered through the same pacing the
        // application uses — `flush` asks the question the debounce was
        // about to ask, and the answer comes back under its sequence number,
        // so what is rendered is what a real answer would look like.
        if let Some(live) = window.finder().live() {
            live.flush();
            live.deliver(
                live.outstanding(),
                postio_gtk::search::Outcome {
                    hits: 14,
                    capped: false,
                    elapsed: Duration::from_millis(11),
                    // `syncing` shows the corpus caveat (#352). Default is a
                    // settled account, which is where every account ends up
                    // under ADR 0016 and so is the honest default for a shot.
                    corpus_complete: !flag("syncing"),
                },
            );
        }
        show_search_panels(&window, wired.and_then(|w| w.search));
    }
    if flag("settings") {
        show_settings(&window);
    }
    if flag("weights") {
        show_account_weights(&window);
    }
    if flag("compose") {
        show_composer(&window);
    }
    window.present();

    settle(&window);

    // The pop-out, rendered as its own window rather than as a state of this
    // one — because that is what it is. A surface nobody can render is a
    // surface nobody checks against the canvas, and this one has chrome of
    // its own (`AdwWindow` draws none unless the content provides it) that a
    // widget test cannot look at.
    let target: gtk::Window = match flag("detached").then(|| window.composer()) {
        Some(composer) => {
            composer.toggle_detached();
            let host = composer
                .detached_window()
                .expect("`detached` needs `compose`: there is nothing to pop out");
            settle(&host);
            host.upcast()
        }
        None => window.clone().upcast(),
    };

    // The canvas draws its key hints on the first row, which means the list
    // has the keyboard — and a shot without them is a shot of a different
    // state. Focused here rather than in `populate` because the rows arrive
    // a frame or two later and an empty list has no row to focus.
    if flag("demo") {
        window.list().grab_focus();
        settle(&window);
    }

    // A selection is a *second* state on top of the focused row, and the two
    // have to be told apart at a glance (`postio-qhz.1`). This is the only way
    // to look at them together before there is a mailbox to select in.
    if flag("selected") {
        let list = window.list();
        list.first_row();
        list.toggle_cursor_row();
        list.extend_down();
        list.extend_down();
        // Leave the keyboard one row below the selection, so the shot shows a
        // cursor row that is *not* selected next to selected rows that are
        // not the cursor.
        list.next_row();
        settle(&window);
    }

    // The conversation pane (ADR 0015 Q4, canvas turn 8a): a thread stacked
    // in the reading pane, beside a list that is only ever the list. Driven
    // through `Window::show_conversation`, the same call landing on a thread
    // row makes, so the shot is the arrangement the application actually
    // puts up rather than one staged for the picture.
    if flag("conversation") {
        let list = window.list();
        list.first_row();
        // The demo's rows are conversations, and the first one's own rows
        // stand in for its members.
        let rows = list.model();
        let mut members = Vec::new();
        for index in 0..rows.n_items().min(6) {
            if let Some(object) = rows.item(index)
                && let Ok(item) = object.downcast::<postio_gtk::list::MessageRow>()
                && let Some(row) = item.row()
            {
                members.push(row);
            }
        }
        // A stand-in reader per expanded message, so the shot shows the
        // stack's real spacing rather than a column of bare headers. The
        // running application sets this from `reading::install`, which knows
        // how to load a body; a shot has no bridge to load one through.
        window.conversation().set_reader_factory({
            let window = window.clone();
            move |_message| {
                let reader = window.new_reader();
                reader.header().widget().set_visible(false);
                // Same reason `reading::install`'s real factory hides it
                // (#822): the entry already draws its own Reply/Reply
                // all/Forward row.
                reader.set_actions_visible(false);
                reader.render(
                    &postio_model::MessageBody {
                        text: None,
                        html: Some(
                            "<p>A message of the conversation, for a shot \
                             that wants one.</p>"
                                .to_string(),
                        ),
                    },
                    None,
                );
                reader.widget().set_size_request(-1, 120);
                reader
            }
        });
        if !members.is_empty() {
            window.show_conversation(members);
        }
        // The stack's readers load on WebKit's own clock, which the
        // frame-counting `settle` does not wait on.
        let deadline = Instant::now() + Duration::from_secs(3);
        let context = glib::MainContext::default();
        while Instant::now() < deadline {
            context.iteration(false);
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    // The reader stays empty by default -- `selected` above is the list's
    // own bulk-selection state, not the reading pane. `open` puts a message
    // there the way `e`/`Enter` on a real row would, through the same
    // `Window::show_message` the running application calls, so a shot can
    // show the reader as something other than an empty pane.
    if flag("open") {
        // A click on the top row, through the same seam a pointer reaches:
        // the reader then loads the body out of the blob store by itself, the
        // way it does in the running application. Handing it a body here --
        // which is what this did before #596 -- meant the shot could not fail
        // when the path from the store to the pane was broken, and for the
        // whole of #70 it did not.
        //
        // The envelope strip (#319) comes with it: the header is the real
        // message's, drawn from the store, so there is nothing left to stage
        // for the picture.
        window.list().click_row(0);
        // WebKit's load is async, on its own clock the frame-counting
        // `settle` above does not wait on -- wall time instead of frames.
        let deadline = Instant::now() + Duration::from_secs(2);
        let context = glib::MainContext::default();
        while Instant::now() < deadline {
            context.iteration(false);
            std::thread::sleep(Duration::from_millis(10));
        }
        // Which account it arrived in (#185). Drawn only with more than one
        // account configured, so `accounts` is what a shot uses to see it --
        // without the flag this is exactly what a single-account install
        // shows, which is nothing.
        //
        // After the *fill*, not merely after the click. The click starts a
        // store read that crosses to the runtime and comes back a turn or two
        // later, and that reply sets the account line itself -- to `None`
        // here, because the seed has one account and `named_accounts` is
        // empty. Setting this before the reply lands means the reply wins and
        // the line never appears, which is what happened when #596 turned the
        // synchronous `show_message` into a real click.
        if flag("accounts") {
            // Hue 0, because `Work` is first in the strip above and takes
            // hue 0 there. A shot that drew the same account blue in the
            // sidebar and magenta in the reader would be teaching the
            // opposite of what the per-account hue is for.
            window.reader().set_account(Some("Work"), 0);
            while context.iteration(false) {}
        }
    }

    // One last pump before the picture is taken. The modes above leave work
    // outstanding -- a page request a selection triggered, a relayout, a
    // reader still loading -- and `settle` counts frames, which a window the
    // compositor has stopped animating does not produce. Pumping the context
    // on wall time lets that work land, and without it a mode that changed
    // little enough to generate no frames rendered the state the window was
    // in *before* it was fed.
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        while context.iteration(false) {}
        std::thread::sleep(Duration::from_millis(10));
    }
    settle(&window);

    // The picture, and the wait for it, both belong to `postio_gtk::capture`
    // -- which turns the main loop until the window is actually drawable
    // rather than until a fixed number of frames has gone past, and writes no
    // file when it cannot. See its module docs for why that split matters
    // (#809).
    match capture::png(&target, std::path::Path::new(&path)) {
        Ok(written) => {
            let (width, height) = (written.width, written.height);
            println!("shot: {width}x{height} -> {path}");
            if written.stalled {
                // Said out loud because the picture is misleading in one
                // specific way, and silently handing it over is how a
                // compositor problem gets read as an application one (#809).
                eprintln!(
                    "shot: the compositor was not presenting this window -- a blanked \
                     or locked screen -- so the layout was done here. The widgets \
                     are drawn correctly, but anything composited by another \
                     process, the reader's web view above all, will be blank."
                );
            }
            glib::ExitCode::SUCCESS
        }
        Err(error) => {
            // Said in full, and on the way out with a non-zero status,
            // because what this replaced printed one line and exited
            // successfully: a session that did not go looking for the file
            // would report "rendered and checked" in good faith (#809).
            eprintln!("shot: {error}");
            eprintln!("shot: NO IMAGE WAS WRITTEN to {path}");
            glib::ExitCode::FAILURE
        }
    }
}
