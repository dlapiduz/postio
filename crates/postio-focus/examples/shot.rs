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
//! | `05` | A new message: a recipient chip, suggestions for "Grac", a subject and a body |
//! | `06` | Reply to all on the Harbor thread: its recipients, its label and the folded quote |
//! | `15` | The three archived, and the undo toast |
//! | `16` | The empty inbox: a store with nothing in it, a digest rule, a morning's filtering |
//! | `17`, `18`, `19` | Screen 01 under the first sync's, the offline and the sign-in error's banner |
//! | `20` | The key map, over screen 01 |
//! | `04` | The to-do about the API draft, opened over screen 01 |
//! | `07`, `08`, `09` | The command bar over screen 01: plain English, `in:Rec`, and `arch` |
//! | `10` | The folders popover over screen 01 |
//! | `26` | The row menu (T199), right-clicked on a row below screen 01's selection |
//! | `27` | A newsletter that paints its own page, opened: on paper (handoff screens 03, 04) |
//! | `28` | Office mail in black text, opened: in app colours (handoff screens 11, 13) |
//! | `29` | The same office mail switched to the original with `O`: on paper (handoff screen 12) |
//! | `30` | A long newsletter with no page of its own, opened: in app colours (T218) |
//! | `31` | The same newsletter past its render deadline: the plain-text fallback (T218) |
//! | `32` | A new message with an attachment and "Remind if no reply" chosen (T221's states) |
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

use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::glib;
use postio_core::CommandId;
use postio_focus::demo::{self, compose_demo, demo_vault, empty_demo, treatment_demo};
use postio_focus::window::FocusWindow;
use postio_model::AccountId;
use postio_widgets::list_model::WindowedModel;

/// The screens this tool can render, and what each is.
const SCREENS: &[(&str, &str)] = &[
    ("01", "the inbox, light, three selected"),
    ("02", "the inbox, dark, three selected"),
    ("03", "the has-action filter"),
    ("05", "a new message, with recipient suggestions"),
    ("06", "reply to all, with the thread's recipients and label"),
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
    ("21", "the Filtered view"),
    ("22", "the digest window's summary"),
    ("23", "the email from a summary's reference"),
    ("24", "\"Digest this sender\" over the inbox"),
    ("25", "the capture sheet: a task from a to-do, into a vault"),
    (
        "26",
        "the row menu, right-clicked on a row outside the selection",
    ),
    ("27", "a newsletter opened, on paper"),
    ("28", "office mail opened, in app colours"),
    (
        "29",
        "office mail opened, switched to the original on paper",
    ),
    ("30", "a long newsletter opened, in app colours"),
    ("31", "the same newsletter fallen back to its plain text"),
    (
        "32",
        "a new message with an attachment and a reminder chosen",
    ),
];

/// How long to wait for the store's rows to reach the screen.
const PATIENCE: Duration = Duration::from_secs(20);

/// The references' size.
const SIZE: (i32, i32) = (1440, 900);

/// The rows screen 01 selects, by position in [`TODAY`], and where its
/// cursor rests: the references' choice.
const PICKED: &[u32] = &[3, 5, 8];
const CURSOR: u32 = 2;

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

    let (database, account) = if request.screen == "16" {
        demo::on_runtime(empty_demo())
    } else {
        demo::on_runtime(demo::demo())
    };
    if matches!(request.screen.as_str(), "05" | "06" | "32") {
        demo::on_runtime(compose_demo(&database, account));
    }
    if matches!(request.screen.as_str(), "27" | "28" | "29" | "30" | "31") {
        demo::on_runtime(treatment_demo(&database, account, &request.screen));
    }
    // Screen 25 captures into a vault: a throwaway one, with projects.
    let vault = (request.screen == "25")
        .then(demo_vault)
        .transpose()
        .map_err(|error| format!("no vault: {error}"))?;
    let text = match &vault {
        Some(vault) => format!(
            "{}\n[focus.vault]\npath = \"{}\"\nprojects = \"Projects\"\n",
            demo::CONFIG,
            vault.path().display()
        ),
        None => demo::CONFIG.to_owned(),
    };
    let config = postio_config::Config::from_toml_str(&text)
        .map_err(|error| format!("the demo's config: {error}"))?;
    let started = demo::start(database, account, &config, request.size)?;
    let (window, sink) = (&started.window, started.sink.clone());

    let outcome = stage(window, &request.screen, &sink, account).and_then(|()| {
        postio_widgets::capture::png(window, std::path::Path::new(&request.path))
            .map_err(|error| error.to_string())
    });
    started.finish();
    manager.set_color_scheme(adw::ColorScheme::Default);
    settings.set_gtk_enable_animations(animated);
    drop(vault);
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
            // OPENED counts the conversations filed; the digest's row sits
            // above them in the list.
            pane.cursor().set_selected(demo::OPENED as u32 + 1);
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
        "27" | "28" | "29" | "30" | "31" => {
            pick_three();
            pane.cursor().set_selected(demo::OPENED as u32 + 1);
            window.act(CommandId::OpenMessage);
            let Some(mut reading) = window.reading() else {
                return Err("Enter opened nothing".into());
            };
            if screen == "31" {
                // Opened again with every render held on its thread past
                // the deadline: what shows is the fallback, as a layout
                // that took too long leaves it. The dialog is built by the
                // first open, and reused by the next.
                settle_until(|| reading.reader().view().document().is_some());
                reading.close();
                settle_until(|| !reading.dialog().is_mapped());
                reading.reader().view().hold_renders();
                window.act(CommandId::OpenMessage);
                reading = window.reading().ok_or("Enter opened nothing")?;
            }
            let drawn = |words: &str| {
                settle_until(|| {
                    reading
                        .dialog()
                        .child()
                        .is_some_and(|content| content.is_mapped() && content.width() > 0)
                        && reading.body_text().contains(words)
                        && reading.reader().treated().is_some()
                        && reading.reader().view().tiles_settled()
                })
            };
            let words = match screen {
                "27" => "dividing perennials",
                "30" => "Tip 1: docs",
                "31" => "Release notes, part 1",
                _ => "temporary routes",
            };
            if !drawn(words) {
                return Err("the opened message never drew under a treatment".into());
            }
            if screen == "31"
                && !settle_until(|| {
                    reading.reader().view().document().is_some_and(|document| {
                        matches!(document.outcome, postio_render::Outcome::FellBack(_))
                    })
                })
            {
                return Err("the held render never fell back".into());
            }
            if screen == "29" {
                let reader = reading.reader();
                reader.switch_treatment();
                if !settle_until(|| {
                    reader.treatment() == postio_body::treatment::Treatment::Paper
                        && reader.view().tiles_settled()
                }) {
                    return Err("O never put the office mail on paper".into());
                }
            }
            let settled = Instant::now();
            settle_until(|| settled.elapsed() > Duration::from_millis(300));
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
        "26" => {
            pick_three();
            let row = pane
                .rows_on_screen()
                .into_iter()
                .nth(7)
                .ok_or("too few rows on screen")?;
            let position = row
                .item()
                .and_then(|item| pane.feed().list().position_of(item.id()))
                .ok_or("the row has no place")?;
            let at = row
                .compute_point(pane.view(), &gtk::graphene::Point::new(420.0, 20.0))
                .ok_or("the row is not in the list")?;
            window.open_row_menu(
                position,
                gtk::gdk::Rectangle::new(at.x() as i32, at.y() as i32, 1, 1),
            );
            let menu = window.row_menu().ok_or("no row menu")?;
            if !settle_until(|| menu.is_open() && menu.widget().width() > 0) {
                return Err("the row menu never showed".into());
            }
        }
        "22" | "23" => {
            // The digest row: this week's newsletters, with its summary.
            let Some(position) = (0..pane.cursor().n_items()).find(|position| {
                pane.cursor()
                    .item(*position)
                    .and_downcast::<postio_focus::list::RowObject>()
                    .and_then(|row| row.item())
                    .is_some_and(|item| matches!(item, postio_focus::list::FocusRow::Digest(_)))
            }) else {
                return Err("no digest row to open".into());
            };
            pane.cursor().set_selected(position);
            window.act(CommandId::OpenMessage);
            if !settle_until(|| {
                window.digest().is_some_and(|digest| {
                    digest
                        .dialog()
                        .child()
                        .is_some_and(|content| content.is_mapped() && content.width() > 0)
                })
            }) {
                return Err("Enter never opened the digest".into());
            }
            let digest = window.digest().expect("checked");
            if !settle_until(|| digest.showing() == postio_focus::digest::DigestPage::Summary) {
                return Err(format!(
                    "the digest never opened on its summary: {:?}",
                    digest.showing()
                ));
            }
            if screen == "23" {
                digest.activate();
                if !settle_until(|| digest.showing() == postio_focus::digest::DigestPage::Email) {
                    return Err("Enter never opened the reference's email".into());
                }
                if !settle_until(|| !digest.reader().view().highlight_rects().is_empty()) {
                    return Err("the cited passage was never highlighted".into());
                }
            }
        }
        "24" => {
            // The Oak Hill row, whose sender the reference digests.
            let Some(position) = (0..pane.cursor().n_items()).find(|position| {
                pane.cursor()
                    .item(*position)
                    .and_downcast::<postio_focus::list::RowObject>()
                    .and_then(|row| row.item())
                    .and_then(|item| {
                        item.as_conversation()
                            .and_then(|row| row.summary.representative.from.clone())
                    })
                    .is_some_and(|from| from.address == "board@oakhill.example")
            }) else {
                return Err("no Oak Hill row to digest".into());
            };
            pane.cursor().set_selected(position);
            window.act(CommandId::DigestRule);
            if !settle_until(|| {
                // Drawn, not only built: the sheet maps its content once
                // it has opened.
                window.rule_dialog().is_some_and(|dialog| {
                    dialog
                        .dialog()
                        .child()
                        .is_some_and(|content| content.is_mapped() && content.width() > 0)
                        && dialog
                            .texts()
                            .iter()
                            .any(|text| text.starts_with("Would have caught"))
                })
            }) {
                return Err("the rule dialog never previewed".into());
            }
        }
        "21" => {
            window.act(CommandId::GoToFiltered);
            if !settle_until(|| {
                window
                    .filtered()
                    .is_some_and(|view| view.subjects().len() == demo::FILTERED.len())
            }) {
                return Err("Filtered never listed the demo's filtered mail".into());
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
        "05" => {
            pick_three();
            window.act(CommandId::Compose);
            let composer = composing(window)?;
            composer.test_set_to("Ada Moreno <ada@example.com>, ");
            composer.test_set_subject("Q4 headcount numbers");
            composer.test_set_body(
                "Hi Ada, here are the Q4 headcount numbers. Two roles move to the \
                 platform team; everything else stays flat.",
            );
            // Four characters: the one completion rule opens at four (C23).
            composer.test_set_to("Grac");
            if !settle_until(|| composer.test_recipient_popover_visible()) {
                return Err("the recipient suggestions never opened".into());
            }
            // The editing surface paints in its own process: give it the
            // frames to draw what was typed.
            let typed = Instant::now();
            settle_until(|| typed.elapsed() > Duration::from_millis(500));
        }
        "06" => {
            pick_three();
            pane.cursor().set_selected(demo::HARBOR);
            window.act(CommandId::ReplyAll);
            let composer = composing(window)?;
            if !settle_until(|| {
                window.compose_dialog().is_some_and(|dialog| {
                    shown_with_class(&dialog, "focus-compose-labels")
                        && composer.test_body_eval(
                            "document.querySelector('details.postio-quote') ? 'y' : 'n'",
                        ) == "y"
                })
            }) {
                return Err("the reply's labels and quote never showed".into());
            }
            // The editing surface paints in its own process, and restyles
            // to the dialog's palette once it is on screen: give it the
            // frames to draw the quote.
            let shown = Instant::now();
            settle_until(|| shown.elapsed() > Duration::from_millis(500));
        }
        "32" => {
            pick_three();
            window.act(CommandId::Compose);
            let composer = composing(window)?;
            composer.test_set_to("Ada Moreno <ada@example.com>, ");
            composer.test_set_subject("Q4 headcount numbers");
            composer.test_set_body("Hi Ada, the numbers are attached. Two roles move.");
            // A file of its own, kept for the life of the shot.
            let folder = tempfile::tempdir().map_err(|error| error.to_string())?;
            let sheet = folder.path().join("Q4-headcount.pdf");
            std::fs::write(&sheet, vec![b'x'; 182_000]).map_err(|error| error.to_string())?;
            composer.test_attach_path(&sheet);
            std::mem::forget(folder);
            if !settle_until(|| composer.test_attachment_count() == 1) {
                return Err("the attachment never landed".into());
            }
            let due = chrono::Local::now() + chrono::Duration::days(3);
            composer.set_remind_at(Some(due.with_timezone(&chrono::Utc)));
            let typed = Instant::now();
            settle_until(|| typed.elapsed() > Duration::from_millis(500));
        }
        "25" => {
            pick_three();
            pane.cursor().set_selected(demo::HARBOR);
            window.act(CommandId::CaptureTask);
            let sheet = window.capture().ok_or("t opened no capture sheet")?;
            // The project list open, as the reference draws it.
            if !settle_until(|| {
                sheet.is_shown()
                    && sheet
                        .texts()
                        .iter()
                        .any(|text| text.starts_with("Project \u{b7} suggested"))
            }) {
                return Err("the capture sheet never read the vault".into());
            }
            sheet.run(CommandId::CaptureChangeProject);
            let opened = Instant::now();
            settle_until(|| opened.elapsed() > Duration::from_millis(300));
        }
        _ => unreachable!("checked against SCREENS"),
    }
    // Once more, so what the last change queued is drawn.
    settle_until(|| settled(window));
    Ok(())
}

/// The composer, once its dialog has opened over the window and drawn.
fn composing(window: &FocusWindow) -> Result<postio_widgets::composer::Composer, String> {
    if !settle_until(|| {
        window
            .compose_dialog()
            .and_then(|dialog| dialog.child())
            .is_some_and(|content| content.is_mapped() && content.width() > 0)
    }) {
        return Err("the compose dialog never opened".into());
    }
    window.composer().ok_or_else(|| "no composer".into())
}

/// Whether a widget under `root` wearing `class` is on screen.
fn shown_with_class(root: &impl IsA<gtk::Widget>, class: &str) -> bool {
    let mut stack = vec![root.clone().upcast::<gtk::Widget>()];
    while let Some(widget) = stack.pop() {
        if widget.has_css_class(class) && widget.is_mapped() {
            return true;
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            stack.push(next);
        }
    }
    false
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
