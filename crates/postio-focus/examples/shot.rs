//! Render a Focus screen to a PNG, over a seeded demo store.
//!
//! ```sh
//! cargo run -p postio-focus --example shot -- /tmp/01.png 01
//! cargo run -p postio-focus --example shot -- /tmp/01-dark.png 01 dark
//! cargo run -p postio-focus --example shot -- /tmp/03.png 03 1600x1000
//! ```
//!
//! The screens are the numbered references in the design folder
//! (`specs/007-postio-focus/screens.md` records every comparison):
//!
//! | Screen | What it shows |
//! |---|---|
//! | `01` | The inbox in light: three rows selected, the cursor on a marked row, the bulk bar |
//! | `02` | The same, in dark |
//! | `03` | The has-action filter on |
//! | `15` | The three archived, and the undo toast |
//! | `16` | The empty inbox: a store with nothing in it, a digest rule, a morning's filtering |
//! | `17`, `18`, `19` | Screen 01 under the first sync's, the offline and the sign-in error's banner |
//! | `20` | The key map, over screen 01 |
//! | `04` | The to-do about the API draft, opened over screen 01 |
//! | `07`, `08`, `09` | The command bar over screen 01: plain English, `in:Rec`, and `arch` |
//! | `10` | The folders popover over screen 01 |
//!
//! `light` or `dark` overrides a screen's own scheme, and `WxH` its size
//! (1440x900, the references', by default). A screen that is not built yet,
//! or a window that never became drawable, writes no file, prints `NO IMAGE
//! WAS WRITTEN`, and exits non-zero (#809): a file that exists is a picture
//! that was taken.
//!
//! # The store
//!
//! The storage seed (`postio_storage::seed::seed_small`) gives the account,
//! its folder tree and six weeks of corpus mail; today's rows are filed on top
//! of it the way sync files mail, labelled, threaded, and with markers
//! written to the markers table. The markers go straight to the table rather
//! than through the host: nothing in the host writes one yet (the detector
//! pass is milestone 1's no-op), and the rows read them the same way either
//! way. Today is 16:09 local, the time the references were drawn at, so the
//! times on the rows are theirs, and the inbox last synced then. The config
//! has one digest rule, weekly on Saturday at 16:00. Screens 17 to 19 are
//! said through the host's event sink, as the engine says them: the store
//! has no server behind it.
//!
//! Every name is invented and every address is on a reserved domain.
//! Nothing touches the network: the store is in memory and sync never
//! starts. The cargo runner sends a binary named `shot` to the private
//! headless compositor (`scripts/headless-runner.sh`), whose monitor is
//! 1280x800; mutter maximizes a window that nearly fills it, so the
//! references' size wants a larger one of its own:
//!
//! ```sh
//! POSTIO_TEST_DISPLAY=focus-shot POSTIO_TEST_GEOMETRY=1920x1200 \
//!     cargo run -p postio-focus --example shot -- /tmp/01.png 01
//! ```

use std::collections::HashMap;
use std::time::{Duration, Instant};

use adw::prelude::*;
use chrono::{DateTime, Local, TimeZone, Utc};
use gtk::glib;
use postio_core::CommandId;
use postio_focus::window::FocusWindow;
use postio_model::listing::MarkerKind;
use postio_model::{
    AccountId, Attachment, EmailAddress, Flag, FlagSet, Label, MailboxId, MailboxRole, Message,
    MessageId, RfcMessageId,
};
use postio_storage::repository::{
    ContactRepository, LabelRepository, Marker, MarkerRepository, MarkerSource, MessageRepository,
    ThreadingRepository,
};
use postio_storage::{BlobStore, Store};

/// The screens this tool can render, and what each is.
const SCREENS: &[(&str, &str)] = &[
    ("01", "the inbox, light, three selected"),
    ("02", "the inbox, dark, three selected"),
    ("03", "the has-action filter"),
    ("15", "the undo toast after archiving three"),
    ("16", "the empty inbox"),
    ("17", "the first sync's banner"),
    ("18", "the offline banner"),
    ("19", "the sign-in error's banner"),
    ("20", "the key map over the inbox"),
    ("04", "a message opened over the inbox"),
    ("07", "the command bar: plain English, lowered to chips"),
    ("08", "the command bar: in:Rec, a folder listed"),
    ("09", "the command bar: a word, its commands and places"),
    ("10", "the folders popover"),
    ("11", "the snooze picker at the row"),
    ("12", "the remind picker at the row"),
    ("13", "the label picker at the row"),
    ("14", "the move picker at the row"),
];

/// How long to wait for the store's rows to reach the screen.
const PATIENCE: Duration = Duration::from_secs(20);

/// The references' size.
const SIZE: (i32, i32) = (1440, 900);

/// The rows screen 01 selects, by position in [`TODAY`], and where its
/// cursor rests: the references' choice.
const PICKED: &[u32] = &[2, 4, 7];
const CURSOR: u32 = 1;

/// What a row's second line says, when it has one.
enum Ask {
    /// An invitation, starting this many days from today at this hour, for
    /// this many minutes.
    Invite { days: i64, hour: i64, minutes: i64 },
    /// A question, quoting this sentence.
    Question(&'static str),
    /// A to-do, due this many days from today, quoting this sentence.
    Todo { days: i64, sentence: &'static str },
}

/// One of today's conversations.
struct Row {
    name: &'static str,
    address: &'static str,
    subject: &'static str,
    preview: &'static str,
    /// Minutes before 16:09 today.
    minutes: i64,
    unread: bool,
    labels: &'static [&'static str],
    attachment: bool,
    /// How many messages the conversation holds.
    messages: usize,
    ask: Option<Ask>,
}

/// Today's inbox, top to bottom, in the references' shape.
const TODAY: &[Row] = &[
    Row {
        name: "Hollis Varga",
        address: "hollis@example.com",
        subject: "Invitation: Harbor design review",
        preview: "Tuesday 10:00\u{2013}10:45, Room 3B. Agenda: navigation, empty states, the export flow.",
        minutes: 7,
        unread: true,
        labels: &["Harbor"],
        attachment: false,
        messages: 1,
        ask: Some(Ask::Invite {
            days: 3,
            hour: 10,
            minutes: 45,
        }),
    },
    Row {
        name: "Marisol Quint",
        address: "marisol@example.com",
        subject: "Re: Atlas Q3 budget, final numbers",
        preview: "The final Q3 numbers are in the attached sheet. Can you approve these by Friday so finance can close the quarter?",
        minutes: 18,
        unread: true,
        labels: &["Atlas"],
        attachment: true,
        messages: 3,
        ask: Some(Ask::Question(
            "Can you approve these by Friday so finance can close the quarter?",
        )),
    },
    Row {
        name: "Tobias Wren",
        address: "tobias@example.net",
        subject: "Atlas staffing plan for Q4",
        preview: "Sharing the draft before Monday's sync. Nothing needed yet, just a heads up that the platform numbers moved.",
        minutes: 29,
        unread: true,
        labels: &["Atlas"],
        attachment: false,
        messages: 1,
        ask: None,
    },
    Row {
        name: "Juno Castellane",
        address: "juno@example.org",
        subject: "Harbor API draft v3",
        preview: "Uploaded v3 with the pagination changes. Please leave comments by Wednesday; I'd like to freeze it Thursday.",
        minutes: 47,
        unread: true,
        labels: &["Harbor"],
        attachment: false,
        messages: 6,
        ask: Some(Ask::Todo {
            days: 4,
            sentence: "Please leave comments by Wednesday",
        }),
    },
    Row {
        name: "Idris Mallory",
        address: "idris@example.com",
        subject: "Re: Harbor SOW",
        preview: "Legal is still reviewing section 4. Should have it back to you early next week.",
        minutes: 71,
        unread: false,
        labels: &["Harbor"],
        attachment: false,
        messages: 2,
        ask: None,
    },
    Row {
        name: "Pim Aldana",
        address: "pim@example.net",
        subject: "Cabinet order: please sign",
        preview: "Attached the final order for the uppers and the pantry unit. Once you sign I can place it with the supplier.",
        minutes: 98,
        unread: false,
        labels: &["Kitchen reno", "Home"],
        attachment: true,
        messages: 4,
        ask: None,
    },
    Row {
        name: "Solveig Brandt",
        address: "solveig@example.org",
        subject: "Contractor invoices for September",
        preview: "All three are in the shared folder. Two of them are already approved on my side.",
        minutes: 119,
        unread: false,
        labels: &["Atlas"],
        attachment: false,
        messages: 1,
        ask: None,
    },
    Row {
        name: "Caspian Holt",
        address: "caspian@example.com",
        subject: "Intro: Harbor data vendor",
        preview: "A colleague suggested I reach out. We help teams move analytics workloads off the warehouse.",
        minutes: 142,
        unread: true,
        labels: &["Harbor"],
        attachment: false,
        messages: 1,
        ask: None,
    },
    Row {
        name: "Renata Obi",
        address: "renata@example.net",
        subject: "Re: dinner Saturday?",
        preview: "We're in! Should we bring anything? The kids can bring the card game they got last week.",
        minutes: 159,
        unread: true,
        labels: &["Friends"],
        attachment: false,
        messages: 2,
        ask: Some(Ask::Question("Should we bring anything?")),
    },
    Row {
        name: "Northfield Elementary",
        address: "office@northfield.example",
        subject: "Field-trip permission form",
        preview: "Please sign and return the attached form by Monday. The trip to the science museum is next week.",
        minutes: 194,
        unread: true,
        labels: &["Kids"],
        attachment: true,
        messages: 1,
        ask: Some(Ask::Todo {
            days: 2,
            sentence: "Please sign and return the attached form by Monday.",
        }),
    },
    Row {
        name: "Marisol Quint",
        address: "marisol@example.com",
        subject: "Atlas headcount numbers",
        preview: "Quick one: do you have the Q4 headcount numbers from the planning doc? No rush, next week is fine.",
        minutes: 229,
        unread: true,
        labels: &["Atlas"],
        attachment: false,
        messages: 1,
        ask: Some(Ask::Question(
            "do you have the Q4 headcount numbers from the planning doc?",
        )),
    },
    Row {
        name: "Anselm Kade",
        address: "anselm@example.org",
        subject: "Re: talk proposal, local-first mail",
        preview: "Thanks for sending this over. The committee meets next week; I'll let you know either way.",
        minutes: 261,
        unread: false,
        labels: &["Talks"],
        attachment: false,
        messages: 2,
        ask: None,
    },
    Row {
        name: "Oak Hill HOA",
        address: "board@oakhill.example",
        subject: "October meeting agenda",
        preview: "Items: pool closing, fall cleanup day, the parking proposal. Minutes from September attached.",
        minutes: 279,
        unread: false,
        labels: &["Home"],
        attachment: true,
        messages: 1,
        ask: None,
    },
    Row {
        name: "Tobias Wren",
        address: "tobias@example.net",
        subject: "Re: 1:1 notes",
        preview: "Added my notes under yours. The main thing for me is the on-call rotation for November.",
        minutes: 307,
        unread: false,
        labels: &["Atlas"],
        attachment: false,
        messages: 3,
        ask: None,
    },
    Row {
        name: "Hollis Varga",
        address: "hollis@example.com",
        subject: "Re: export flow edge cases",
        preview: "I think we can drop the CSV option if the markdown export covers tables. I'd rather ship fewer formats.",
        minutes: 329,
        unread: false,
        labels: &["Harbor"],
        attachment: false,
        messages: 5,
        ask: None,
    },
    Row {
        name: "Mae Sorensen",
        address: "mae@example.net",
        subject: "Photos from the hike",
        preview: "Uploaded the good ones. The waterfall came out well, see the second album.",
        minutes: 357,
        unread: false,
        labels: &["Friends"],
        attachment: true,
        messages: 1,
        ask: None,
    },
    Row {
        name: "Pim Aldana",
        address: "pim@example.net",
        subject: "Schedule for next week",
        preview: "Crew arrives Monday 8 am for the demo of the old uppers. Please clear the counters by Sunday night.",
        minutes: 374,
        unread: true,
        labels: &["Kitchen reno", "Home"],
        attachment: false,
        messages: 1,
        ask: Some(Ask::Todo {
            days: 1,
            sentence: "Please clear the counters by Sunday night.",
        }),
    },
    Row {
        name: "Rhea Okonjo",
        address: "rhea@example.org",
        subject: "Coffee next week",
        preview: "I'm in town Tuesday through Thursday. Any morning work for you?",
        minutes: 408,
        unread: true,
        labels: &["Friends"],
        attachment: false,
        messages: 1,
        ask: Some(Ask::Question("Any morning work for you?")),
    },
];

/// The demo's config.toml: filtering on, and one digest rule.
const CONFIG: &str = "[focus]\nfiltering = true\n\n[[focus.digests]]\nname = \"Newsletters\"\n\
queries = [\"from:news@example.com\"]\ncadence = \"weekly\"\nday = \"saturday\"\nat = \"16:00\"\n\n\
[filters.waiting]\nquery = \"from:juno\"\npinned = true\norder = 1\nname = \"Waiting on reply\"\n\n\
[filters.atlas]\nquery = \"subject:atlas\"\npinned = true\norder = 2\nname = \"Atlas\"\n\n\
[filters.receipts]\nquery = \"in:Receipts\"\npinned = true\norder = 3\nname = \"Receipts this month\"\n\n\
[filters.school]\nquery = \"from:northfield\"\npinned = true\norder = 4\nname = \"From school\"\n";

/// What the command line asked for.
struct Request {
    path: String,
    screen: String,
    dark: bool,
    size: (i32, i32),
}

const USAGE: &str = "usage: shot <png> <screen> [light|dark] [WxH]";

fn parse(args: &[String]) -> Result<Request, String> {
    let mut words = args.iter();
    let path = words.next().ok_or(USAGE)?.clone();
    let screen = words.next().ok_or(USAGE)?.clone();
    let mut dark = screen == "02";
    let mut size = SIZE;
    for word in words {
        match word.as_str() {
            "dark" => dark = true,
            "light" => dark = false,
            other => {
                size = other
                    .split_once('x')
                    .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
                    .ok_or_else(|| format!("not a word shot knows: {other}\n{USAGE}"))?;
            }
        }
    }
    Ok(Request {
        path,
        screen,
        dark,
        size,
    })
}

/// Render the screen `args` names. `Ok` says what was written; `Err` says
/// why nothing was, and ends in `NO IMAGE WAS WRITTEN`.
pub fn run(args: &[String]) -> Result<String, String> {
    let path = args.first().cloned().unwrap_or_default();
    render(args).map_err(|error| format!("{error}\nNO IMAGE WAS WRITTEN to {path}"))
}

fn render(args: &[String]) -> Result<String, String> {
    let request = parse(args)?;
    if !SCREENS.iter().any(|(name, _)| *name == request.screen) {
        let known: Vec<String> = SCREENS
            .iter()
            .map(|(name, what)| format!("  {name}  {what}"))
            .collect();
        return Err(format!(
            "no screen {:?} is built yet; these are:\n{}",
            request.screen,
            known.join("\n")
        ));
    }
    adw::init().map_err(|error| format!("no display: {error}"))?;
    // The picture is of where things settle, not of a transition caught
    // part-way: libadwaita fades a focus ring and slides a toast in, and a
    // capture taken the moment the state is reached would show neither.
    let settings = gtk::Settings::default().ok_or("no GTK settings")?;
    let animated = settings.is_gtk_enable_animations();
    settings.set_gtk_enable_animations(false);
    let manager = adw::StyleManager::default();
    manager.set_color_scheme(if request.dark {
        adw::ColorScheme::ForceDark
    } else {
        adw::ColorScheme::ForceLight
    });

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|error| format!("no runtime: {error}"))?;
    let blobs_dir = tempfile::tempdir().map_err(|error| format!("no scratch: {error}"))?;
    let (database, account) = if request.screen == "16" {
        runtime.block_on(empty_demo())
    } else {
        runtime.block_on(demo())
    };
    let blobs = BlobStore::open(
        blobs_dir.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .map_err(|error| format!("no blob store: {error}"))?;
    let sink = std::rc::Rc::new(std::cell::RefCell::new(None));
    let host = postio_host::Host::start(database, blobs, {
        let sink = std::rc::Rc::clone(&sink);
        move |wiring| {
            sink.replace(Some(wiring.events.clone()));
            wiring
        }
    })
    .map_err(|error| format!("the host did not start: {error}"))?;
    let sink = sink.take().ok_or("the host kept its events to itself")?;
    let config = postio_config::Config::from_toml_str(CONFIG)
        .map_err(|error| format!("the demo's config: {error}"))?;

    let window = FocusWindow::new(None);
    window.set_default_size(request.size.0, request.size.1);
    window.present();
    let session = postio_focus::startup::adopt(&window, host, &config);

    let outcome = stage(&window, &request.screen, &sink, account).and_then(|()| {
        postio_widgets::capture::png(&window, std::path::Path::new(&request.path))
            .map_err(|error| error.to_string())
    });
    window.destroy();
    session.stop();
    drop(session);
    manager.set_color_scheme(adw::ColorScheme::Default);
    settings.set_gtk_enable_animations(animated);
    let written = outcome?;
    let mut said = format!("{}x{} -> {}", written.width, written.height, request.path);
    if (written.width, written.height) != request.size {
        said.push_str(&format!(
            " (asked for {}x{}: mutter sizes a window that nearly fills its monitor; \
             give it a larger one, POSTIO_TEST_GEOMETRY=1920x1200 with its own \
             POSTIO_TEST_DISPLAY)",
            request.size.0, request.size.1
        ));
    }
    Ok(said)
}

/// Turn the main loop until `done`, or give up after [`PATIENCE`].
fn settle_until(done: impl Fn() -> bool) -> bool {
    let context = glib::MainContext::default();
    let heartbeat =
        glib::timeout_add_local(Duration::from_millis(10), || glib::ControlFlow::Continue);
    let started = Instant::now();
    let reached = loop {
        if done() {
            break true;
        }
        if started.elapsed() > PATIENCE {
            break false;
        }
        context.iteration(true);
    };
    heartbeat.remove();
    reached
}

/// Whether the strip's counts have been read: its counts label says
/// something.
fn counted(window: &FocusWindow) -> bool {
    let Some(chrome) = window.chrome() else {
        return false;
    };
    let mut stack = vec![chrome.strip().clone().upcast::<gtk::Widget>()];
    while let Some(widget) = stack.pop() {
        if widget.has_css_class("focus-counts") {
            return widget
                .first_child()
                .and_then(|child| child.downcast::<gtk::Label>().ok())
                .is_some_and(|label| !label.text().is_empty());
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            stack.push(next);
        }
    }
    false
}

/// Every row on screen drawn with its content, and the strip's counts in.
fn settled(window: &FocusWindow) -> bool {
    let Some(pane) = window.pane() else {
        return false;
    };
    let rows = pane.rows_on_screen();
    counted(window) && !rows.is_empty() && rows.iter().all(|row| !row.drawn().texts.is_empty())
}

/// Put the window in the state `screen` shows.
fn stage(
    window: &FocusWindow,
    screen: &str,
    sink: &postio_core::bridge::EventSink,
    account: AccountId,
) -> Result<(), String> {
    if screen == "16" {
        if !settle_until(|| counted(window) && empty_shown(window)) {
            return Err("the empty inbox never showed".into());
        }
        return Ok(());
    }
    if !settle_until(|| settled(window)) {
        return Err("the demo store's inbox never reached the screen".into());
    }
    let pane = window.pane().ok_or("no inbox")?;
    let pick_three = || {
        for &row in PICKED {
            pane.cursor().set_selected(row);
            window.act(CommandId::ToggleSelection);
        }
        pane.cursor().set_selected(CURSOR);
    };
    let tell = |state| {
        sink.emit(postio_core::Event::ConnectionChanged { account, state });
    };
    let banner = |starts: &str| {
        settle_until(|| {
            window
                .banner_showing()
                .is_some_and(|(title, ..)| title.starts_with(starts))
                && settled(window)
        })
    };
    match screen {
        "01" | "02" => pick_three(),
        "03" => {
            let before = pane.cursor().n_items();
            window.act(CommandId::ToggleHasAction);
            if !settle_until(|| {
                window
                    .pane()
                    .is_some_and(|pane| pane.cursor().n_items() < before)
                    && settled(window)
            }) {
                return Err("the has-action filter never narrowed the list".into());
            }
            if let Some(pane) = window.pane() {
                pane.cursor().set_selected(CURSOR);
            }
        }
        "15" => {
            pick_three();
            window.act(CommandId::Archive);
            if !settle_until(|| window.toast_showing().is_some() && settled(window)) {
                return Err("archiving the three never raised the undo toast".into());
            }
        }
        "17" => {
            tell(postio_core::ConnectionState::Online);
            sink.emit(postio_core::Event::SyncProgress {
                account,
                done: 12_408,
                total: 18_204,
            });
            pick_three();
            if !banner("First sync") {
                return Err("the first sync's banner never showed".into());
            }
        }
        "18" => {
            tell(postio_core::ConnectionState::Offline);
            pick_three();
            if !banner("You're offline") {
                return Err("the offline banner never showed".into());
            }
        }
        "19" => {
            tell(postio_core::ConnectionState::Failing {
                reason: postio_core::FailureReason::Auth,
            });
            pick_three();
            if !banner("Can't sign in") {
                return Err("the sign-in banner never showed".into());
            }
        }
        "04" => {
            pick_three();
            pane.cursor().set_selected(OPENED as u32);
            window.act(CommandId::OpenMessage);
            let Some(reading) = window.reading() else {
                return Err("Enter opened nothing".into());
            };
            if !settle_until(|| {
                reading
                    .dialog()
                    .child()
                    .is_some_and(|content| content.is_mapped() && content.width() > 0)
                    && reading.body_text().contains("freeze it Thursday")
                    && !reading.reader().view().highlight_rects().is_empty()
                    && reading.reader().view().tiles_settled()
            }) {
                return Err("the opened message never drew with its sentence lit".into());
            }
        }
        "07" | "08" | "09" => {
            pick_three();
            window.act(CommandId::Search);
            let bar = window.bar().ok_or("no command bar")?;
            if !settle_until(|| bar.places_known()) {
                return Err("the bar never read its places".into());
            }
            let (typed, done): (&str, Box<dyn Fn() -> bool>) = match screen {
                "07" => (
                    "the invoice Marisol sent last month",
                    Box::new(|| !bar.chips().is_empty()),
                ),
                "08" => ("in:Rec", Box::new(|| !bar.result_subjects().is_empty())),
                _ => ("arch", Box::new(|| !bar.texts().is_empty())),
            };
            bar.set_text(typed);
            if !settle_until(&done) {
                return Err(format!("the bar never answered {typed:?}"));
            }
            // What arrives after the first answer: the search's rows.
            let started = Instant::now();
            settle_until(|| started.elapsed() > Duration::from_millis(400));
        }
        "10" => {
            pick_three();
            window.act(CommandId::GoToFolders);
            let places = window.places().ok_or("no folders popover")?;
            if !settle_until(|| places.names().len() > 3) {
                return Err("the popover never listed the places".into());
            }
        }
        "11" | "12" | "13" | "14" => {
            pick_three();
            let (command, wanted) = match screen {
                "11" => (CommandId::Snooze, "Later today"),
                "12" => (CommandId::RemindIfNoReply, "Tomorrow"),
                "13" => (CommandId::AddLabel, "Harbor"),
                _ => (CommandId::Move, "Receipts"),
            };
            window.act(command);
            let Some(picker) = window.open_picker() else {
                return Err(format!("{command} opened no picker"));
            };
            if !settle_until(|| {
                picker.is_shown() && picker.texts().iter().any(|line| line == wanted)
            }) {
                return Err(format!("the picker never listed {wanted:?}"));
            }
        }
        "20" => {
            pick_three();
            window.act(CommandId::CheatSheet);
            // The dialog is mapped before it is drawn: libadwaita's sheet
            // opens at 80% and transparent, and maps its content when that
            // settles. What is waited for is the key map's own content.
            if !settle_until(|| {
                window
                    .key_map()
                    .and_then(|dialog| dialog.child())
                    .is_some_and(|content| content.is_mapped() && content.width() > 0)
            }) {
                return Err("the key map never opened".into());
            }
        }
        _ => unreachable!("checked against SCREENS"),
    }
    // Once more, so what the last change queued is drawn.
    settle_until(|| settled(window));
    Ok(())
}

/// Whether the empty inbox is what the window shows.
fn empty_shown(window: &FocusWindow) -> bool {
    let mut stack = vec![window.clone().upcast::<gtk::Widget>()];
    while let Some(widget) = stack.pop() {
        if widget.has_css_class("focus-empty") {
            return widget.is_mapped();
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            stack.push(next);
        }
    }
    false
}

/// 16:09 today, local: when the references were drawn.
fn today() -> DateTime<Utc> {
    Local::now()
        .date_naive()
        .and_hms_opt(16, 9, 0)
        .and_then(|at| Local.from_local_datetime(&at).single())
        .map(|at| at.with_timezone(&Utc))
        .unwrap_or_else(Utc::now)
}

/// Say the inbox last synced at `at`, as a completed pass would.
async fn synced(connection: &postio_storage::Connection, inbox: MailboxId, at: DateTime<Utc>) {
    let mailboxes = postio_storage::repository::MailboxRepository::new(connection);
    if let Ok(Some(mut mailbox)) = mailboxes.get(inbox).await {
        mailbox.last_synced_at = Some(at);
        mailboxes
            .update(&mailbox)
            .await
            .expect("the inbox's sync time");
    }
}

/// The empty inbox's store (screen 16): an account and its folders, no mail
/// in the inbox, and a morning's worth of mail filed away.
pub async fn empty_demo() -> (Store, AccountId) {
    let database = postio_storage::test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, inbox) = postio_storage::test_support::account_with_inbox(&connection).await;
    for folder in ["Archive", "Sent", "Drafts", "Trash"] {
        postio_storage::test_support::mailbox(&connection, &account, folder).await;
    }
    let filtered = postio_storage::test_support::mailbox(&connection, &account, "Filtered").await;
    let morning = today() - chrono::Duration::hours(7);
    for step in 0..186_i64 {
        let at = morning + chrono::Duration::minutes(step * 2);
        let mut message = Message::new(account.id, filtered.id, at);
        message.flags = [Flag::Seen].into_iter().collect();
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("a filtered message");
        postio_storage::repository::FilterDecisionRepository::new(&connection)
            .record(&postio_storage::repository::FilterDecision {
                message: message.id,
                reason: postio_storage::repository::FilterReason::Promotion,
                source: None,
                layer: postio_storage::repository::FilterLayer::Header,
                decided_at: at,
            })
            .await
            .expect("a decision");
    }
    synced(&connection, inbox, today()).await;
    drop(connection);
    (database, account.id)
}

/// The row screen 04 opens: the to-do about the API draft, by its place in
/// [`TODAY`].
const OPENED: usize = 3;

/// Its body: the marked sentence, a list, and the quoted history folded
/// under it, as screen 04 draws them.
const OPENED_BODY: &str = "Hi all,\n\n\
Uploaded v3 of the Harbor API draft with the pagination changes. The main differences from v2:\n\n\
- Cursor pagination on every list endpoint, replacing page and offset.\n\
- Rate-limit headers are documented for every response.\n\
- /exports moved under /v1/accounts/{id}, as suggested.\n\n\
Please leave comments by Wednesday; I'd like to freeze it Thursday so the client work can start.\n\n\
The rendered PDF and the raw OpenAPI file are attached.\n\n\
Juno\n\n\
On Monday, Hollis Varga wrote:\n\
> Thanks for v2. Two things before the next round:\n\
> the exports path, and the rate limits.\n\
> Everything else reads well to me.\n";

/// Its attachments: the rendered draft and its source.
fn opened_parts() -> Vec<Attachment> {
    [
        ("Harbor-API-v3.pdf", "application/pdf", 212_000),
        ("harbor-openapi.yaml", "application/yaml", 38_000),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (name, mime, size))| {
        let mut part = Attachment::new(MessageId::UNASSIGNED, mime, size);
        part.filename = Some(name.to_owned());
        // After the text part, as a synced message's parts are numbered.
        part.part_id = Some((index + 2).to_string());
        part
    })
    .collect()
}

/// The demo store: the storage seed, and today's inbox on top of it.
pub async fn demo() -> (Store, AccountId) {
    let database = postio_storage::test_support::memory().await;
    let report = postio_storage::seed::seed_small(&database, 1).await;
    let inbox = report
        .mailbox(MailboxRole::Inbox)
        .expect("the seed files an inbox")
        .id;
    let today = today();
    let connection = database.connect().await.expect("a connection");
    synced(&connection, inbox, today).await;
    let mut labels: HashMap<&str, postio_model::LabelId> = HashMap::new();
    for (index, row) in TODAY.iter().enumerate() {
        let at = today - chrono::Duration::minutes(row.minutes);
        let mut previous: Option<RfcMessageId> = None;
        let mut last = MessageId::UNASSIGNED;
        // The earlier messages of the conversation first, read, then the
        // one the row shows.
        for step in 0..row.messages {
            let later = (row.messages - 1 - step) as i64;
            let newest = later == 0;
            let when = at - chrono::Duration::minutes(40 * later);
            let mut message = message(report.account.id, inbox, row, when, newest);
            if newest && index == OPENED {
                message.attachments = opened_parts();
            }
            let id = RfcMessageId::new(format!("<demo.{index}.{step}@example.test>"));
            message.rfc_message_id = Some(id.clone());
            if let Some(parent) = previous.replace(id) {
                message.in_reply_to = Some(parent.clone());
                message.references = vec![parent];
            }
            MessageRepository::new(&connection)
                .create(&mut message)
                .await
                .expect("a message");
            // Sync records every address it sees; the bar reads names from
            // them (screen 07).
            ContactRepository::new(&connection)
                .record_message(&message)
                .await
                .expect("its correspondents");
            ThreadingRepository::new(&connection, report.account.id)
                .thread(&message)
                .await
                .expect("threaded");
            last = message.id;
        }
        let body = if index == OPENED {
            OPENED_BODY.to_owned()
        } else {
            format!("{}\n", row.preview)
        };
        MessageRepository::new(&connection)
            .set_body(
                last,
                &postio_storage::repository::StoredBody {
                    text: Some(body),
                    html: None,
                    headers: None,
                    headers_truncated: false,
                    encoding_problems: false,
                },
                postio_model::BodyState::Full,
            )
            .await
            .expect("a body");
        let repository = LabelRepository::new(&connection);
        for name in row.labels {
            let label = match labels.get(name) {
                Some(label) => *label,
                None => {
                    let mut label = Label::new(report.account.id, *name);
                    repository.create(&mut label).await.expect("a label");
                    labels.insert(name, label.id);
                    label.id
                }
            };
            repository.attach(last, label).await.expect("labelled");
        }
        if let Some(ask) = &row.ask {
            MarkerRepository::new(&connection)
                .insert(&marker(last, ask, today))
                .await
                .expect("a marker");
        }
    }
    // Screens 07 and 08: invoices from Marisol filed in three places, and
    // a folder of receipts.
    let receipts = postio_storage::test_support::mailbox(&connection, &report.account, "Receipts")
        .await
        .id;
    let archive = report
        .mailbox(MailboxRole::Archive)
        .map_or(inbox, |mailbox| mailbox.id);
    // The 19th of last month, whatever today is: "last month" is a
    // calendar month.
    let last_month = {
        use chrono::Datelike as _;
        let first = today.date_naive().with_day(1).unwrap_or(today.date_naive());
        let first = first
            .checked_sub_months(chrono::Months::new(1))
            .unwrap_or(first);
        let nineteenth = first.with_day(19).unwrap_or(first);
        today - (today.date_naive() - nineteenth)
    };
    for (mailbox, subject, preview, days) in [
        (
            inbox,
            "Invoice 2026-08, Atlas contractor hours",
            "Attached is the invoice for August.",
            0,
        ),
        (
            receipts,
            "Re: Invoice 2026-08 (corrected)",
            "Fixed the PO number, same total.",
            5,
        ),
        (
            archive,
            "Invoice for July (late)",
            "Sorry for the delay. This one covers three weeks.",
            11,
        ),
    ] {
        filed(
            &connection,
            report.account.id,
            mailbox,
            ("Marisol Quint", "marisol@example.com"),
            subject,
            preview,
            last_month + chrono::Duration::days(days),
        )
        .await;
    }
    for (subject, days) in [
        ("Your coffee order", 2),
        ("Bookshop receipt", 9),
        ("Train ticket", 16),
    ] {
        filed(
            &connection,
            report.account.id,
            receipts,
            ("Receipts", "receipts@shop.example"),
            subject,
            "Thank you for your order.",
            today - chrono::Duration::days(days),
        )
        .await;
    }
    // Screen 14: mail was last moved to Receipts.
    postio_storage::repository::SettingsRepository::new(&connection)
        .note_move(receipts)
        .await
        .expect("a recent move");
    // The command bar searches this machine's index, as sync fills it.
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("the search index");
    drop(connection);
    (database, report.account.id)
}

/// File one message from `from` about `subject` into `mailbox` at `at`.
async fn filed(
    connection: &postio_storage::Connection,
    account: AccountId,
    mailbox: MailboxId,
    from: (&str, &str),
    subject: &str,
    preview: &str,
    at: DateTime<Utc>,
) {
    let mut message = Message::new(account, mailbox, at);
    message.date = Some(at);
    message.from = vec![EmailAddress::new(Some(from.0), from.1)];
    message.subject = Some(subject.to_owned());
    message.preview = Some(preview.to_owned());
    message.flags = [Flag::Seen].into_iter().collect();
    message.rfc_message_id = Some(RfcMessageId::new(format!(
        "<filed.{}@example.test>",
        subject.to_lowercase().replace(' ', ".")
    )));
    MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("a filed message");
    ContactRepository::new(connection)
        .record_message(&message)
        .await
        .expect("its correspondents");
    ThreadingRepository::new(connection, account)
        .thread(&message)
        .await
        .expect("threaded");
}

fn message(
    account: AccountId,
    inbox: MailboxId,
    row: &Row,
    at: DateTime<Utc>,
    newest: bool,
) -> Message {
    let mut message = Message::new(account, inbox, at);
    message.date = Some(at);
    if newest {
        message.from = vec![EmailAddress::new(Some(row.name), row.address)];
        message.preview = Some(row.preview.to_owned());
    } else {
        message.from = vec![EmailAddress::new(Some("You"), "you@example.com")];
        message.preview = Some("Earlier in the conversation.".to_owned());
    }
    message.to = vec![EmailAddress::new(Some("You"), "you@example.com")];
    message.subject = Some(row.subject.to_owned());
    let mut flags = FlagSet::new();
    if !(newest && row.unread) {
        flags.insert(Flag::Seen);
    }
    message.flags = flags;
    if newest && row.attachment {
        let mut attachment = Attachment::new(MessageId::UNASSIGNED, "application/pdf", 48_000);
        attachment.filename = Some("attached.pdf".to_owned());
        message.attachments.push(attachment);
    }
    message
}

fn marker(message: MessageId, ask: &Ask, today: DateTime<Utc>) -> Marker {
    let midnight = today - chrono::Duration::minutes(16 * 60 + 9);
    let blank = Marker {
        message,
        kind: MarkerKind::Question,
        source: MarkerSource::Detector,
        span: None,
        excerpt: None,
        starts_at: None,
        ends_at: None,
        due_at: None,
        invite: None,
        invite_state: None,
        answer: None,
        dismissed_at: None,
    };
    match ask {
        Ask::Invite {
            days,
            hour,
            minutes,
        } => {
            let starts = midnight + chrono::Duration::days(*days) + chrono::Duration::hours(*hour);
            Marker {
                kind: MarkerKind::Invite,
                source: MarkerSource::Calendar,
                starts_at: Some(starts),
                ends_at: Some(starts + chrono::Duration::minutes(*minutes)),
                ..blank
            }
        }
        Ask::Question(sentence) => Marker {
            span: Some((0, sentence.len() as u32)),
            excerpt: Some((*sentence).to_owned()),
            ..blank
        },
        Ask::Todo { days, sentence } => Marker {
            kind: MarkerKind::Todo,
            span: Some((0, sentence.len() as u32)),
            excerpt: Some((*sentence).to_owned()),
            due_at: Some(midnight + chrono::Duration::days(*days) + chrono::Duration::hours(17)),
            ..blank
        },
    }
}

/// Set once the shot is running in a configuration of its own.
const HERMETIC: &str = "POSTIO_FOCUS_SHOT_HERMETIC";

/// Run this binary again with an empty configuration directory, and exit as
/// it does -- unless this is that run.
///
/// GTK reads the person's own `gtk-4.0/gtk.css`, and a desktop can put its
/// whole palette there above any application's: COSMIC writes its dark
/// colours as `@define-color`s whatever the scheme, which renders a light
/// screen dark and its header strip unreadable. A picture compared with a
/// reference has to be Focus's alone. The focus suite does the same, for the
/// same reason; setting the variable here would need `unsafe`, which the
/// workspace forbids, so a child is given it.
fn hermetic(args: &[String]) -> Option<glib::ExitCode> {
    if std::env::var_os(HERMETIC).is_some() {
        return None;
    }
    let config = tempfile::tempdir().ok()?;
    let status = std::process::Command::new(std::env::current_exe().ok()?)
        .args(args)
        .env(HERMETIC, "1")
        .env("XDG_CONFIG_HOME", config.path())
        .status()
        .ok()?;
    Some(if status.success() {
        glib::ExitCode::SUCCESS
    } else {
        glib::ExitCode::FAILURE
    })
}

fn main() -> glib::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = hermetic(&args) {
        return code;
    }
    match run(&args) {
        Ok(said) => {
            println!("shot: {said}");
            glib::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("shot: {error}");
            glib::ExitCode::FAILURE
        }
    }
}
