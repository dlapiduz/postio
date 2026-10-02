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
//! cargo run -p postio-app --example shot -- /tmp/account.png demo account
//! cargo run -p postio-app --example shot -- /tmp/compose.png demo compose
//! cargo run -p postio-app --example shot -- /tmp/reply.png demo reply 1600x900
//! cargo run -p postio-app --example shot -- /tmp/rich.png demo reply row5 unfold
//! cargo run -p postio-app --example shot -- /tmp/popout.png demo compose detached
//! cargo run -p postio-app --example shot -- /tmp/tight.png demo compact
//! cargo run -p postio-app --example shot -- /tmp/large.png demo text2
//! cargo run -p postio-app --example shot -- /tmp/box.png demo command
//! cargo run -p postio-app --example shot -- /tmp/who.png demo contact
//! cargo run -p postio-app --example shot -- /tmp/selected.png demo selected
//! cargo run -p postio-app --example shot -- /tmp/first-run.png demo orientation
//! cargo run -p postio-app --example shot -- /tmp/outbox.png demo outbox 1600x900
//! cargo run -p postio-app --example shot -- /tmp/reader.png demo open 1600x900
//! cargo run -p postio-app --example shot -- /tmp/thread.png demo conversation 1600x900
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

use std::collections::HashSet;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::{gdk, glib};
use postio_app::demo::{self, DemoOptions, Preset, Seed, on_runtime, settle};
use postio_gtk::{app, capture, fonts, style, window::Window};

/// Every argument (after the output path) that no mode asked about, and that
/// is not a `WxH` size or a `text` scale prefix.
///
/// `asked` is what `flag` was consulted about during this run, which is the
/// whole change: this compared against a hand-kept `KNOWN_FLAGS`, a second
/// copy of "what this tool understands" that drifted from the first in both
/// directions at once (#1376).
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
fn unrecognized_arguments<'a>(args: &'a [String], asked: &HashSet<String>) -> Vec<&'a str> {
    args.iter()
        .skip(1)
        .filter(|token| {
            !asked.contains(token.as_str())
                && token
                    .split_once('x')
                    .is_none_or(|(w, h)| w.parse::<i32>().is_err() || h.parse::<i32>().is_err())
                && !token.starts_with("text")
                // `row<n>` is read straight out of `args` rather than through
                // `flag()`, so it never lands in `asked` and would be
                // reported as a typo. Same escape as `text<n>` above, for the
                // same reason.
                && !(token.starts_with("row") && token[3..].parse::<u32>().is_ok())
        })
        .map(String::as_str)
        .collect()
}

fn warn_about_unrecognized_arguments(args: &[String], asked: &HashSet<String>) {
    for token in unrecognized_arguments(args, asked) {
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

    /// What `flag` was asked about during the run.
    fn asked(names: &[&str]) -> HashSet<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    #[test]
    fn a_row_number_is_an_argument_and_not_a_typo() {
        // It is read out of `args` rather than asked for through `flag()`, so
        // without an escape the guard reports the one argument that chose the
        // message as the thing it did not understand.
        assert_eq!(
            unrecognized_arguments(
                &args(&["demo", "reply", "row5"]),
                &asked(&["demo", "reply"])
            ),
            Vec::<&str>::new()
        );
        // And still catches a real typo that merely starts the same way.
        assert_eq!(
            unrecognized_arguments(&args(&["demo", "rowdy"]), &asked(&["demo"])),
            vec!["rowdy"]
        );
    }

    #[test]
    fn a_mode_the_run_asked_about_is_recognized() {
        assert_eq!(
            unrecognized_arguments(&args(&["demo"]), &asked(&["demo"])),
            Vec::<&str>::new()
        );
    }

    /// The `thread` case exactly: passed, accepted, and inert.
    ///
    /// It sat in `KNOWN_FLAGS` and nothing queried it, so the usage this
    /// example documents in its own header -- `shot out.png demo thread` --
    /// was accepted, drew the *default* picture, and said nothing. The
    /// opposite happened too: `conversation`, which the code did query, was
    /// missing from the list and so was reported as "silently ignored" when it
    /// had in fact run. Two failures that look like each other's opposite,
    /// from one list that had to be kept in step by hand (#1376).
    #[test]
    fn a_mode_nothing_asked_about_is_flagged() {
        assert_eq!(
            unrecognized_arguments(&args(&["thread"]), &asked(&["conversation"])),
            vec!["thread"]
        );
    }

    #[test]
    fn a_size_argument_is_recognized() {
        assert_eq!(
            unrecognized_arguments(&args(&["1400x800"]), &asked(&[])),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn a_text_scale_argument_is_recognized() {
        assert_eq!(
            unrecognized_arguments(&args(&["text150"]), &asked(&[])),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn two_words_collapsed_into_one_shell_argument_is_flagged() {
        // #599: exactly what an unquoted `$mode` set to "demo conversation"
        // becomes under a shell that does not word-split it.
        assert_eq!(
            unrecognized_arguments(
                &args(&["demo conversation"]),
                &asked(&["demo", "conversation"])
            ),
            vec!["demo conversation"]
        );
    }

    #[test]
    fn a_plain_typo_is_flagged() {
        assert_eq!(
            unrecognized_arguments(&args(&["dmeo"]), &asked(&["demo"])),
            vec!["dmeo"]
        );
    }

    #[test]
    fn a_normally_split_pair_is_not_flagged() {
        assert_eq!(
            unrecognized_arguments(
                &args(&["demo", "conversation", "1400x800"]),
                &asked(&["demo", "conversation"])
            ),
            Vec::<&str>::new()
        );
    }
}

/// What to say when the size a `WxH` argument asked for and the size the
/// compositor actually gave the window disagree (#933).
///
/// `set_default_size` is a hint: a toplevel larger than the monitor is
/// clamped, and the height is clamped further by whatever the session
/// reserves. `capture::png` reports the size it actually got, so this is the
/// one place that can compare it against what was asked for and say so --
/// silently handing over a smaller picture under a size that looks honoured
/// is the same shape as #599, an argument that looks applied and was not.
///
/// `None` when nothing was requested, or when the compositor gave back
/// exactly what was asked for -- the ordinary case, which must stay silent.
fn size_mismatch(requested: Option<(i32, i32)>, got: (i32, i32)) -> Option<String> {
    let requested = requested?;
    if requested == got {
        return None;
    }
    let (want_w, want_h) = requested;
    let (got_w, got_h) = got;
    Some(format!(
        "asked for {want_w}x{want_h} but the compositor gave {got_w}x{got_h} -- \
         the picture below is at the size it actually got, not the size named \
         on the command line"
    ))
}

#[cfg(test)]
mod size_mismatch_tests {
    use super::*;

    #[test]
    fn nothing_requested_is_silent() {
        assert_eq!(size_mismatch(None, (1280, 800)), None);
    }

    #[test]
    fn the_size_asked_for_is_silent() {
        assert_eq!(size_mismatch(Some((1280, 800)), (1280, 800)), None);
    }

    #[test]
    fn a_clamped_size_is_reported() {
        let message = size_mismatch(Some((1600, 900)), (1280, 800))
            .expect("a clamped size should be reported");
        assert!(message.contains("1600x900"), "{message}");
        assert!(message.contains("1280x800"), "{message}");
    }
}

fn main() -> glib::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args
        .first()
        .cloned()
        .unwrap_or_else(|| "postio.png".to_string());
    // Every mode word this run asks about. `flag` is the only way a mode is
    // consumed, so what it was asked *is* what the tool understands -- a fact
    // about the run rather than a list somebody has to remember to update.
    let asked: std::cell::RefCell<HashSet<String>> = std::cell::RefCell::new(HashSet::new());
    let flag = |name: &str| {
        asked.borrow_mut().insert(name.to_owned());
        args.iter().skip(1).any(|a| a == name)
    };
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
        // The words become seeds; the library does the rest.
        let mut options = DemoOptions::new();
        for (word, seed) in [
            ("accounts", Seed::TwoAccounts),
            ("backfill", Seed::Backfilling),
            ("orientation", Seed::FirstRun),
            ("outbox", Seed::Outbox),
        ] {
            if flag(word) {
                options = options.with(seed);
            }
        }
        match on_runtime(demo::populate(&window, &options)) {
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
        Preset::Locked.apply(&window, wired);
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
        window.finder().set_contacts(&demo::sample_contacts());
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
                    // `unreachable` shows ADR 0005 Q10's caveat (#812), so a
                    // shot can be taken of the state a reviewer would
                    // otherwise have to unplug a server to see. Empty is the
                    // ordinary case: every account answering.
                    unreachable: match flag("unreachable") {
                        true => vec!["Work".to_owned()],
                        false => Vec::new(),
                    },
                },
            );
        }
        Preset::SearchPanels.apply(&window, wired);
    }
    if flag("settings") {
        // Which pane, from the mode words. `flag` is `main`'s own closure
        // over the arguments, so the choice is made here and handed over.
        let pane = [
            ("filters", postio_gtk::settings::Section::Filters),
            ("composing", postio_gtk::settings::Section::Composing),
            ("appearance", postio_gtk::settings::Section::Appearance),
            ("keyboard", postio_gtk::settings::Section::Keyboard),
            ("storage", postio_gtk::settings::Section::Sync),
            ("privacy", postio_gtk::settings::Section::Privacy),
            ("configfile", postio_gtk::settings::Section::ConfigFile),
        ]
        .into_iter()
        .find(|(word, _)| flag(word))
        .map(|(_, section)| section);
        Preset::Settings(pane).apply(&window, wired);
    }
    if flag("weights") {
        Preset::AccountWeights.apply(&window, wired);
    }
    if flag("account") {
        Preset::AccountDetail {
            tested: flag("tested"),
            signature: flag("signature"),
        }
        .apply(&window, wired);
    }
    if flag("addaccount") {
        let step = if flag("browser") {
            demo::AddAccountStep::Browser
        } else if flag("syncwindow") {
            demo::AddAccountStep::SyncWindow
        } else {
            demo::AddAccountStep::Route
        };
        Preset::AddAccount(step).apply(&window, wired);
    }
    if flag("mailboxes") {
        Preset::AccountMailboxes.apply(&window, wired);
    }
    if flag("compose") {
        Preset::Compose.apply(&window, wired);
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
        // Settings is its own window now (#1179), so a shot of the main one
        // would show the workspace it is sitting in front of. Same shape as
        // the detached composer directly above, and for the same reason:
        // a surface nobody can render is a surface nobody checks against
        // the drawing, and this one has chrome of its own.
        None if flag("settings") => {
            let host = window
                .settings_window()
                .expect("`settings` opens a window; there is nothing to shoot without one");
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

    // The first-run keyboard orientation (ADR 0012 Q4). Only ever on screen
    // for one moment of one run, so without a flag it is a surface nobody
    // can look at -- and a strip along the top of the mail column is exactly
    // the kind of thing that has to be looked at in dark and at the narrow
    // breakpoint before it is called done.
    if flag("orientation") {
        // Nothing forces it on: `populate` left the store saying it has
        // never been seen, and the application's own wiring shows it once
        // the seeded sync lands. A shot that reached in and set the widget
        // visible would render even if nothing in the app ever did (#596).
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
                Some(reader)
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
    if flag("outbox") {
        // Open it the way the sidebar's own handler does, by role. The list
        // then pages `ListScope::Outbox` out of the store for itself, so what
        // the picture shows is the predicate over `send_state` rather than a
        // row handed to the pane.
        window.open_view(postio_model::mailbox::MailboxRole::Outbox);
        while glib::MainContext::default().iteration(false) {}
        settle(&window);
    }

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

    if flag("reply") {
        // The composer as a *reply* actually produces it, which `compose`
        // cannot show and was never meant to: `show_composer` hands the
        // composer a body with `> ` typed into it, so what it renders is a
        // draft the tool wrote rather than a quote the code built. That is
        // the #596 trap this file warns about, and the quote is exactly where
        // it bites -- ADR 0033 changed what a reply carries, and a picture of
        // a hand-written body could not have shown it either way.
        //
        // So: click a real row, let the reader load it out of the blob store,
        // then press the key. Everything between the store and the editor's
        // WebView is in the picture -- `quote_of`, the sanitiser, the styles,
        // `postio-ui`'s editor document, the folded `<details>`.
        // Which row: `row<n>` picks one, because the interesting reply is to
        // a *rich* message and the top of the list is plain text. A quote of
        // plain text cannot show what ADR 0033 changed.
        let row = args
            .iter()
            .find_map(|arg| arg.strip_prefix("row"))
            .and_then(|n| n.parse::<u32>().ok())
            .unwrap_or(0);
        window.list().click_row(row);
        let context = glib::MainContext::default();
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            context.iteration(false);
            std::thread::sleep(Duration::from_millis(10));
        }

        window.handle_key(gtk::gdk::Key::e, gtk::gdk::ModifierType::empty());

        // WebKit loads the quote on its own clock, which the frame-counting
        // `settle` does not wait on. Wall time, as `open` does for the reader.
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            context.iteration(false);
            std::thread::sleep(Duration::from_millis(10));
        }

        // `open` unfolds the quote, which is the half a folded shot cannot
        // show: whether what survived the sanitiser actually *looks* like the
        // message being answered. Through the element's own `open` property,
        // because `<details>` is script-free by design and there is no
        // gesture to send it from here.
        if flag("unfold") {
            window.composer().test_body_eval(
                "(() => { const d = document.querySelector('details.postio-quote'); \
                   if (d) d.open = true; return 'opened'; })()",
            );
            let deadline = Instant::now() + Duration::from_secs(1);
            while Instant::now() < deadline {
                context.iteration(false);
                std::thread::sleep(Duration::from_millis(10));
            }
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

    // #1030: the facts block a transactional message gets above its body
    // copy. The corpus fixture is handed straight to the reader rather than
    // clicked out of the list, because the demo seed re-dates the corpus and
    // there is no way to name a row from here.
    //
    // That makes this a *design* shot and nothing more -- it is for looking
    // at spacing, weight and the two columns. It deliberately does not prove
    // the block reaches the pane from a real message, which is what #596 says
    // a shot handed its own body cannot do; `gtk_reader.rs` proves that,
    // through `render`, on the same fixture.
    if flag("shipping") {
        let fixture = postio_model::test_corpus::load("transactional-shipping-notice");
        let parsed = postio_model::mime::parse(fixture.bytes());
        // The reader that is *on screen*, which in a conversation is the
        // focused message's and not the single-message one behind it --
        // rendering into `window.reader()` paints a hidden widget, and the
        // picture comes back showing whatever the demo had already drawn.
        // The conversation has one reader for the whole thread now (#1426),
        // and it is not reachable from here -- the pane fills it from the
        // store. So this draws into the single-message reader, which is what
        // the shot wants anyway: one message, rendered, for the camera.
        let reader = window.reader();
        reader.render(&parsed.body, Some("orders@shop.example.test"));
        let deadline = Instant::now() + Duration::from_secs(2);
        let context = glib::MainContext::default();
        while Instant::now() < deadline {
            context.iteration(false);
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    // Now, not before the modes run. The question is whether anything *asked*
    // about each word the caller passed, and that is only answerable once
    // every `flag` call has happened -- which is the whole reason this stopped
    // being a hand-kept list. It comes before the picture so the warning is
    // read alongside it rather than scrolled off above.
    warn_about_unrecognized_arguments(&args, &asked.borrow());

    // The picture, and the wait for it, both belong to `postio_gtk::capture`
    // -- which turns the main loop until the window is actually drawable
    // rather than until a fixed number of frames has gone past, and writes no
    // file when it cannot. See its module docs for why that split matters
    // (#809).
    match capture::png(&target, std::path::Path::new(&path)) {
        Ok(written) => {
            let (width, height) = (written.width, written.height);
            println!("shot: {width}x{height} -> {path}");
            if let Some(message) = size_mismatch(size, (width, height)) {
                eprintln!("shot: {message}");
            }
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
