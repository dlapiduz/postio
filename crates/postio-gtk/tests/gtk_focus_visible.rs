//! Focus you can actually see, measured in pixels.
//!
//! `docs/PRODUCT.md` §20 and #36 both ask that focus always be visible, and
//! until now nothing checked it. Asserting a widget *has* focus proves
//! nothing about whether a keyboard-only user can see where they are: the
//! focus rules live in CSS, and a stylesheet edit can delete the ring with
//! every accessibility test still green. The only honest test renders the
//! thing twice and compares what is drawn.
//!
//! # Why the first attempt at this was withdrawn
//!
//! It was written while working #36 and failed about four runs in five (#90).
//! The cause is stated in `gtk_row.rs`'s own `frames` helper and is worth
//! repeating here because it will catch the next person too:
//!
//! > `pump` is not a wait: a non-blocking iteration returns immediately when
//! > nothing is pending, so it can spin through without the frame clock
//! > ticking once.
//!
//! Focus changes a widget's CSS state, and CSS state reaches the pixels
//! through a frame. A test that pumps and then snapshots is sampling
//! whichever side of that frame it happened to land on — which is also why
//! the withdrawn version appeared to work with a deliberately loud override
//! and not with the real stylesheet, and why two spellings of the *same
//! colour* disagreed with each other. That was never a finding about CSS. It
//! was noise.
//!
//! So everything here waits on real frames, and the assertions are on counts
//! of changed pixels rather than on byte equality, so a failure says how much
//! changed rather than only that something did.
//!
//! One test function: GTK is initialised once per process. Skips without a
//! display. Nothing here touches the network.

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::ids::{AccountId, MailboxId};
use postio_model::mailbox::{Mailbox, MailboxCounts, MailboxRole};

/// How different two samples of one channel must be to count as a change.
///
/// Not zero: the renderer is free to differ by a bit-or-two on unchanged
/// pixels between passes, and a threshold of zero would make every comparison
/// report the whole surface as changed and pass for the wrong reason.
const CHANNEL_DELTA: i16 = 8;

/// How many pixels must change before a ring counts as drawn.
///
/// The real rule is `outline: 2px solid var(--postio-accent)` with
/// `outline-offset: -2px` on a row about 200x36, which is on the order of a
/// thousand pixels. A hundred is far enough below that to survive a different
/// row size or a scale factor, and far enough above zero that renderer noise
/// under `CHANNEL_DELTA` cannot reach it.
const VISIBLE_PIXELS: usize = 100;

/// Run the main loop until `window` has actually painted `count` frames.
///
/// The whole point of this file. See the module docs for what happens without
/// it. Returns `false` if the frames never came, which is a fact about the
/// compositor rather than about the widget — see [`pixels`].
fn frames(window: &gtk::Window, count: u32) -> bool {
    let left = std::rc::Rc::new(std::cell::Cell::new(count));
    window.add_tick_callback({
        let left = left.clone();
        move |_, _| {
            left.set(left.get().saturating_sub(1));
            if left.get() == 0 {
                gtk::glib::ControlFlow::Break
            } else {
                gtk::glib::ControlFlow::Continue
            }
        }
    });
    let context = gtk::glib::MainContext::default();
    // A frame clock with nothing else to do can sit idle in a blocking
    // iteration; the heartbeat keeps the loop turning so the callback fires.
    let heartbeat = gtk::glib::timeout_add_local(std::time::Duration::from_millis(10), || {
        gtk::glib::ControlFlow::Continue
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while left.get() > 0 && std::time::Instant::now() < deadline {
        context.iteration(true);
    }
    heartbeat.remove();
    left.get() == 0
}

/// One widget's pixels, as RGBA, or `None` if the compositor is not painting.
///
/// `None` does not mean the widget is broken. A compositor stops delivering
/// frame callbacks to a window nobody can see — a blanked screen is the
/// commonest cause on a developer's machine — and every comparison would then
/// be between two blank textures, failing for a reason that has nothing to do
/// with the code. The caller skips instead, loudly.
fn pixels(window: &Window, widget: &impl IsA<gtk::Widget>) -> Option<(Vec<u8>, usize, usize)> {
    let window = window.clone().upcast::<gtk::Window>();
    if !frames(&window, 3) {
        return None;
    }
    let widget = widget.as_ref();
    let (width, height) = (widget.width().max(1), widget.height().max(1));
    let paintable = gtk::WidgetPaintable::new(Some(widget));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, width as f64, height as f64);
    let node = snapshot.to_node()?;
    let renderer = window.native().and_then(|native| native.renderer())?;
    let bounds = gtk::graphene::Rect::new(0.0, 0.0, width as f32, height as f32);
    let texture = renderer.render_texture(&node, Some(&bounds));

    let stride = (width * 4) as usize;
    let mut bytes = vec![0u8; stride * height as usize];
    texture.download(&mut bytes, stride);
    Some((bytes, width as usize, height as usize))
}

/// How many pixels differ between two renders of the same widget.
///
/// Counted per pixel rather than per byte so the number means something a
/// person can picture: "1,412 pixels changed" is a ring, "3" is noise.
fn changed(before: &[u8], after: &[u8]) -> usize {
    assert_eq!(
        before.len(),
        after.len(),
        "the widget changed size between renders, so this is not a comparison \
         of focus styling — fix the layout before reading anything into it"
    );
    before
        .chunks_exact(4)
        .zip(after.chunks_exact(4))
        .filter(|(a, b)| {
            a.iter()
                .zip(b.iter())
                .any(|(x, y)| (i16::from(*x) - i16::from(*y)).abs() > CHANNEL_DELTA)
        })
        .count()
}

/// The folders the canvas draws, so the row under test is a real one.
fn canvas_mailboxes() -> Vec<Mailbox> {
    let account = AccountId::new(1);
    let folder = |id: i64, path: &str, role, (total, unread, flagged)| {
        let mut mailbox = Mailbox::new(account, path, Some('/'));
        mailbox.id = MailboxId::new(id);
        mailbox.role = role;
        mailbox.counts = MailboxCounts {
            total,
            unread,
            flagged,
        };
        mailbox
    };
    vec![
        folder(1, "INBOX", MailboxRole::Inbox, (940, 12, 3)),
        folder(2, "Flagged", MailboxRole::Flagged, (940, 12, 3)),
        folder(3, "Drafts", MailboxRole::Drafts, (2, 0, 0)),
        folder(4, "Sent", MailboxRole::Sent, (4021, 0, 0)),
    ]
}

/// Whether `widget` is in the CSS state `:focus` selects on.
///
/// **Not `has_focus()`**, which is the trap this file exists downstream of.
/// GTK gates `has-focus` on the toplevel being *active*, and a headless
/// window is never active — so `has_focus()` is false on a row that GTK has
/// nonetheless put in `FOCUSED` state and is drawing the ring for. Asserting
/// on it fails the test before it renders anything, and reads like the focus
/// never landed.
///
/// The state flag is the right question here anyway: what is being tested is
/// a CSS rule, and this is the flag that rule matches on.
fn focused(widget: &impl IsA<gtk::Widget>) -> bool {
    widget
        .as_ref()
        .state_flags()
        .contains(gtk::StateFlags::FOCUSED)
}

/// Every widget under `widget` carrying `class`, in draw order.
///
/// By class rather than by type, deliberately: the assertion below is about a
/// CSS rule, so the widget it runs on has to be selected the same way the
/// rule selects it. A row found by walking to "the first `GtkListBoxRow`"
/// could stop matching `.postio-folder` and the test would go on passing
/// against whatever it found instead.
fn collect(widget: &gtk::Widget, class: &str) -> Vec<gtk::Widget> {
    let mut found = Vec::new();
    if widget.has_css_class(class) {
        found.push(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        found.extend(collect(&current, class));
        child = current.next_sibling();
    }
    found
}

#[test]
fn taking_focus_changes_what_is_drawn() {
    let state_dir = std::env::temp_dir().join(format!("postio-focus-{}", std::process::id()));
    std::fs::create_dir_all(&state_dir).unwrap();

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let window = Window::default();
    window.set_default_size(1120, 700);
    window.present();

    // Real folders, so the row is the one `.postio-folder:focus` styles
    // rather than a stand-in that happens to carry the class.
    window.sidebar().set_mailboxes(&canvas_mailboxes());

    assert!(
        frames(&window.clone().upcast::<gtk::Window>(), 3),
        "the window never painted; nothing below could mean anything"
    );

    if std::env::var("LOUD").is_ok() {
        let provider = gtk::CssProvider::new();
        provider.load_from_string(".postio-folder:focus { background-color: rgb(0,255,0); outline: 6px solid rgb(255,0,0); }");
        gtk::style_context_add_provider_for_display(&display, &provider, 900);
    }
    let rows = collect(window.upcast_ref::<gtk::Widget>(), "postio-folder");
    assert!(
        !rows.is_empty(),
        "no `.postio-folder` widgets, so the rule under test has nothing to \
         apply to and this test could not fail"
    );
    let row = rows[0].clone();
    assert!(!focused(&row), "nothing is focused yet");

    let Some((before, width, height)) = pixels(&window, &row) else {
        eprintln!("skipping: the compositor is not painting this window");
        return;
    };
    assert!(
        width > 1 && height > 1,
        "the row rendered {width}x{height}, which is not a row"
    );

    // Through the sidebar's own `focus_folders`, which is what `g f` calls —
    // not `row.grab_focus()` from outside. The stylesheet uses `:focus` and
    // not `:focus-visible` precisely because this path is programmatic, and
    // its comment says so; a test that focused the row some other way could
    // pass while the way a person gets there did not.
    assert!(
        window.sidebar().focus_folders(),
        "the sidebar had no folder to land on"
    );

    let Some((after, _, _)) = pixels(&window, &row) else {
        eprintln!("skipping: the compositor stopped painting mid-test");
        return;
    };

    let differing = changed(&before, &after);
    eprintln!("DEBUG differing={differing} of {}", width * height);
    assert!(
        differing >= VISIBLE_PIXELS,
        "focusing the folder row changed {differing} pixels of {}, which is \
         not something a keyboard-only user can see. docs/PRODUCT.md §20 asks \
         that focus always be visible; the rule that draws it is \
         `.postio-folder:focus` in shell.css.",
        width * height
    );

    // …and letting go puts it back. Not a bonus assertion: without it, a rule
    // that painted the ring permanently on first focus would pass the test
    // above and be a worse bug than no ring at all.
    let elsewhere = window.shell().list();
    elsewhere.set_can_focus(true);
    elsewhere.grab_focus();
    assert!(!focused(&row), "the row kept the keyboard");

    let Some((released, _, _)) = pixels(&window, &row) else {
        eprintln!("skipping: the compositor stopped painting mid-test");
        return;
    };
    let lingering = changed(&before, &released);
    assert!(
        lingering < VISIBLE_PIXELS,
        "the row still differs from its unfocused self by {lingering} pixels \
         after the keyboard left it, so the ring is drawn on a row that no \
         longer has focus"
    );
}
