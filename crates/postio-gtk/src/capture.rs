//! Render a window to a texture, and say so when it cannot be done.
//!
//! # Why this is in the library rather than in the tool that uses it
//!
//! There were three copies of it — `postio-app`'s `shot` example,
//! `postio-gtk`'s `surface` example, and `gtk_focus_visible`'s `pixels` —
//! and each of them made the *caller* settle the window first, by counting
//! frames, before asking for the picture. That split is the bug #809 is
//! about. A frame count is not a condition: eight frames on an idle
//! workstation is a wait, and eight frames on a surface the compositor has
//! stopped presenting is a five-second timeout followed by an empty
//! snapshot, one printed line, and no file. A session that did not check for
//! the file would report "rendered and checked" in good faith.
//!
//! So the wait belongs to the thing that knows what it is waiting for, and
//! what it waits for is stated as a condition — the widget is drawable and
//! has a picture — rather than as a number of frames or a number of
//! milliseconds. When it gives up it says what was true when it did.
//!
//! # What this path can and cannot do
//!
//! It renders whatever GTK would put on screen, through the window's own
//! renderer, so what comes out is the real thing rather than an
//! approximation. The cost is that **a compositor is still required**: GTK
//! refuses to snapshot a widget that is not mapped, and a window that was
//! realized but never presented produces no render node at all — measured,
//! not assumed, and `gtk_capture.rs` pins it. There is no offscreen path
//! that renders a widget tree with no compositor whatsoever, so the way to
//! run this where there is no desktop session is to give it a headless one;
//! `scripts/headless-runner.sh` does that for the two examples that render.

use std::path::Path;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gdk, glib, graphene};

/// How long to wait for a window to become drawable, before scaling.
///
/// Generous: this bounds "the compositor is not going to show this window",
/// which is a conclusion worth being slow about, and every millisecond of it
/// is skipped on the ordinary path where the picture is ready almost at once.
pub const PATIENCE: Duration = Duration::from_secs(5);

/// Why a window could not be turned into a picture.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// GTK will not snapshot a widget the compositor is not showing.
    ///
    /// The two fields are worth reading together: `mapped: false` means the
    /// window never reached the compositor, while `mapped: true` with a
    /// `0x0` allocation means it did and was never given a size — different
    /// faults with the same symptom.
    #[error(
        "the window never became drawable within {waited:.1?} (mapped: {mapped}, \
         allocation: {width}x{height}) — nothing is painted to a surface the \
         compositor is not showing. A blanked screen is the commonest cause on \
         a workstation; on a machine with no session, run it under \
         scripts/test-headless.sh."
    )]
    NeverDrawable {
        waited: Duration,
        mapped: bool,
        width: i32,
        height: i32,
    },

    /// A realized window has a renderer; an unrealized one does not.
    #[error("the window has no renderer, so there is nothing to render through")]
    NoRenderer,

    /// The picture exists and the file does not.
    #[error("cannot write {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: glib::BoolError,
    },
}

/// [`PATIENCE`], scaled by `POSTIO_TEST_PATIENCE`.
///
/// The same dial the suites answer to, and deliberately not a second one: a
/// machine slow enough to need longer here needs longer everywhere, and a
/// constant edited in this file would slow every run to fix one box.
fn patience() -> Duration {
    let factor: f64 = std::env::var("POSTIO_TEST_PATIENCE")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|factor: &f64| *factor > 0.0)
        .unwrap_or(1.0);
    PATIENCE.mul_f64(factor)
}

/// Render `widget` to a texture, waiting until it can be.
///
/// The wait is [`PATIENCE`], scaled; use [`texture_within`] to say otherwise.
pub fn texture(widget: &impl IsA<gtk::Widget>) -> Result<gdk::Texture, Error> {
    texture_within(widget, patience())
}

/// Render `widget` to a texture, giving up after `deadline`.
///
/// Turns the main loop itself: the caller does not have to settle the window
/// first, and a caller that tried would be guessing at the number of frames
/// this can simply watch for.
pub fn texture_within(
    widget: &impl IsA<gtk::Widget>,
    deadline: Duration,
) -> Result<gdk::Texture, Error> {
    let widget = widget.as_ref();
    let started = Instant::now();

    // A blocking iteration is what lets the frame clock tick — a
    // non-blocking one returns immediately when nothing is pending, which is
    // how a "settle" can spin through without a single frame (#90). The
    // heartbeat is what guarantees the blocking iteration returns.
    let context = glib::MainContext::default();
    let heartbeat =
        glib::timeout_add_local(Duration::from_millis(10), || glib::ControlFlow::Continue);
    let node = loop {
        if let Some(node) = drawn(widget) {
            break Some(node);
        }
        if started.elapsed() >= deadline {
            break None;
        }
        context.iteration(true);
    };
    heartbeat.remove();

    let Some(node) = node else {
        return Err(Error::NeverDrawable {
            waited: started.elapsed(),
            mapped: widget.is_mapped(),
            width: widget.width(),
            height: widget.height(),
        });
    };

    let renderer = widget
        .native()
        .and_then(|native| native.renderer())
        .ok_or(Error::NoRenderer)?;
    let bounds = graphene::Rect::new(0.0, 0.0, widget.width() as f32, widget.height() as f32);
    Ok(renderer.render_texture(&node, Some(&bounds)))
}

/// The widget's current picture, if it has one.
///
/// `None` for a widget with no allocation as well as for one GTK will not
/// snapshot: a zero-sized window produces a node that renders to nothing,
/// which is a blank PNG rather than an error, and a blank PNG is the failure
/// this whole module exists to stop being silent.
fn drawn(widget: &gtk::Widget) -> Option<gtk::gsk::RenderNode> {
    if widget.width() <= 0 || widget.height() <= 0 {
        return None;
    }
    let paintable = gtk::WidgetPaintable::new(Some(widget));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(
        &snapshot,
        f64::from(widget.width()),
        f64::from(widget.height()),
    );
    snapshot.to_node()
}

/// Render `widget` and write it to `path`.
///
/// Returns the size actually written. **Nothing is written when this
/// fails**, so the file's existence is a fact a caller can rely on — which
/// is what lets `shot` exit non-zero and mean it.
pub fn png(widget: &impl IsA<gtk::Widget>, path: &Path) -> Result<(i32, i32), Error> {
    png_within(widget, path, patience())
}

/// [`png`], giving up after `deadline`.
pub fn png_within(
    widget: &impl IsA<gtk::Widget>,
    path: &Path,
    deadline: Duration,
) -> Result<(i32, i32), Error> {
    let texture = texture_within(widget, deadline)?;
    texture.save_to_png(path).map_err(|source| Error::Write {
        path: path.display().to_string(),
        source,
    })?;
    Ok((texture.width(), texture.height()))
}
