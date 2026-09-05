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
    let deadline =
        std::time::Instant::now() + postio_test_support::scaled(std::time::Duration::from_secs(5));
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
///
/// One sample. Callers want [`settled`], not this.
fn pixels(window: &Window, widget: &impl IsA<gtk::Widget>) -> Option<(Vec<u8>, usize, usize)> {
    let window = window.clone().upcast::<gtk::Window>();
    if !frames(&window, 2) {
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

/// A widget's pixels once they have stopped changing.
///
/// This is the whole fix for #90, and it needs both halves.
///
/// **A minimum number of frames**, because a CSS state change reaches the
/// pixels through the frame clock and nothing sooner. Focus is set
/// synchronously; the ring is drawn later.
///
/// **Then repeated sampling until two consecutive renders agree**, because a
/// fixed frame budget is a guess that works until the machine is loaded. This
/// is what turned 796/796/796/0/0 into one answer.
///
/// Stability is the *precondition*, never the assertion — the caller still
/// compares two settled renders and decides. Waiting for "the pixels differ
/// from before" would be waiting for the thing under test, which is how an
/// await-for-condition test quietly becomes one that cannot fail.
///
/// It settles on whatever is true, including "nothing was drawn". A missing
/// ring makes this return the unfocused image and the caller's assertion
/// fail, which is the direction a wrong answer here must fail in.
fn settled(window: &Window, widget: &impl IsA<gtk::Widget>) -> Option<(Vec<u8>, usize, usize)> {
    // Enough frames that a style revalidation queued by `grab_focus` has
    // certainly run before the first sample is taken.
    if !frames(&window.clone().upcast::<gtk::Window>(), 6) {
        return None;
    }
    let mut previous: Option<Vec<u8>> = None;
    for _ in 0..30 {
        let (bytes, width, height) = pixels(window, widget)?;
        if previous.as_deref() == Some(bytes.as_slice()) {
            return Some((bytes, width, height));
        }
        previous = Some(bytes);
    }
    // Never settled. Returning the last sample rather than `None` on purpose:
    // `None` means "the compositor is not painting" and makes the caller skip,
    // and a widget that genuinely never stops changing is a finding, not a
    // reason to pass quietly.
    let (bytes, width, height) = pixels(window, widget)?;
    Some((bytes, width, height))
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
    // `as_chunks::<4>()` rather than `chunks_exact(4)`: one RGBA pixel is
    // four bytes and saying so in the type is what clippy asks for here.
    before
        .as_chunks::<4>()
        .0
        .iter()
        .zip(after.as_chunks::<4>().0.iter())
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
            snoozed: 0,
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

pub fn taking_focus_changes_what_is_drawn() {
    let state_dir = std::env::temp_dir().join(format!("postio-focus-{}", std::process::id()));
    std::fs::create_dir_all(&state_dir).unwrap();

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
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

    let rows = collect(window.upcast_ref::<gtk::Widget>(), "postio-folder");
    assert!(
        !rows.is_empty(),
        "no `.postio-folder` widgets, so the rule under test has nothing to \
         apply to and this test could not fail"
    );
    let row = rows[0].clone();
    assert!(!focused(&row), "nothing is focused yet");

    let Some((before, width, height)) = settled(&window, &row) else {
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
    assert!(focused(&row), "the row did not take the keyboard");

    let Some((after, _, _)) = settled(&window, &row) else {
        eprintln!("skipping: the compositor stopped painting mid-test");
        return;
    };

    let differing = changed(&before, &after);
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
    // above, and a ring that never goes away is worse than no ring at all —
    // it would point at the wrong row for the rest of the session.
    //
    // Dropping the window's focus rather than grabbing it somewhere else,
    // because "somewhere else" has to be a widget that will actually take it
    // and that is a second thing to get wrong.
    gtk::prelude::GtkWindowExt::set_focus(
        &window.clone().upcast::<gtk::Window>(),
        None::<&gtk::Widget>,
    );
    assert!(!focused(&row), "the row kept the keyboard");

    let Some((released, _, _)) = settled(&window, &row) else {
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

    // ── A second surface, drawn a completely different way ───────────────
    //
    // The folder row above is `outline` on the focused widget. A compose
    // field is `box-shadow: inset` on its *parent*, matched by
    // `:focus-within` rather than `:focus`. A test that only knew how to look
    // for an outline would pass on the sidebar and prove nothing here, which
    // is exactly the gap #90 asked to be closed — so this checks that the
    // harness measures *drawn pixels* and not one CSS property.
    let composer = window.composer();
    composer.open(postio_model::Draft::new(
        postio_model::AccountId::UNASSIGNED,
    ));

    let field_rows = collect(window.upcast_ref::<gtk::Widget>(), "postio-compose-row");
    assert!(
        !field_rows.is_empty(),
        "no `.postio-compose-row` widgets, so `:focus-within` has nothing to \
         apply to and this half could not fail"
    );
    let field_row = field_rows[0].clone();

    // Opening the composer puts the keyboard in `To` already, so take it away
    // first — otherwise "before" is already the focused state and the
    // comparison is between one thing and itself.
    gtk::prelude::GtkWindowExt::set_focus(
        &window.clone().upcast::<gtk::Window>(),
        None::<&gtk::Widget>,
    );
    let Some((unfocused, field_width, field_height)) = settled(&window, &field_row) else {
        eprintln!("skipping: the compositor stopped painting mid-test");
        return;
    };

    assert!(
        composer.test_focus_field(postio_gtk::composer::Field::To),
        "the To field did not take the keyboard"
    );
    let Some((focused_pixels, _, _)) = settled(&window, &field_row) else {
        eprintln!("skipping: the compositor stopped painting mid-test");
        return;
    };

    let field_differing = changed(&unfocused, &focused_pixels);
    assert!(
        field_differing >= VISIBLE_PIXELS,
        "putting the keyboard in the To field changed {field_differing} pixels \
         of {}, so there is nothing to tell a keyboard-only user which field \
         they are typing into. The rule is `.postio-compose-row:focus-within` \
         in shell.css.",
        field_width * field_height
    );
}
