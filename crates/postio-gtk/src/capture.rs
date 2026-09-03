//! Render a window to a texture, and say so when it cannot be done.
//!
//! # Why this is in the library rather than in the tool that uses it
//!
//! There were three copies of it — `postio-app`'s `shot` example,
//! `postio-gtk`'s `surface` example, and `gtk_focus_visible`'s `pixels` —
//! and the two that write PNGs made the *caller* settle the window first, by
//! counting eight frames, before asking for the picture.
//!
//! A frame count is not a condition. Eight frames on an idle workstation is a
//! wait; eight frames on a surface the compositor has stopped presenting is a
//! five-second timeout, an empty snapshot, one printed line and no file —
//! and nothing distinguished that from success except going to look for the
//! file afterwards. So the wait belongs to the thing that knows what it is
//! waiting for, and what it waits for is a condition: the window has a
//! picture.
//!
//! (`gtk_focus_visible` keeps its own frame counting on purpose. It compares
//! two samples across a state change, so the frames *are* the thing it is
//! measuring.)
//!
//! # What stops a window having a picture, measured
//!
//! #809 reported that `shot` wrote nothing here and blamed the frame
//! callback. The frame callback is the trigger, but not the mechanism, and
//! the difference decides whether it can be fixed. Measured on this
//! workstation with the screen blanked — an `AdwApplicationWindow`, mapped,
//! presented, and allocated 600x400 throughout:
//!
//! ```text
//! after present                 child 600x400   picture: yes
//! after queue_resize + pump     child 600x400   picture: NO
//! after request_phase(LAYOUT)   child 600x400   picture: NO
//! after request_phase(PAINT)    child 600x400   picture: NO
//! after allocating the child    child 600x400   picture: yes
//! ```
//!
//! GTK refuses to snapshot a widget with a pending resize — `Trying to
//! snapshot AdwDialogHost without a current allocation`, on stderr, once per
//! attempt. The pending resize is serviced in the frame clock's layout
//! phase, and a compositor that has stopped presenting never runs one, so
//! **any** invalidation after the last presented frame leaves the window
//! permanently unrenderable. The window's own width and height still read
//! back as the old allocation, which is why this looks like nothing is
//! wrong.
//!
//! Asking the frame clock for the phase directly does not help; it is
//! throttled by exactly the thing that has stopped. Doing the layout
//! ourselves does, and is repeatable.
//!
//! # What this path cannot do
//!
//! A compositor is still required. GTK will not snapshot an unmapped widget
//! at all, so a window that was realized but never presented has no picture
//! and no way to get one — measured too, and pinned by `gtk_capture.rs`.
//! That rules out the shape #809 hoped for, of rendering a widget tree on a
//! machine with no seat: the way to run this where there is no desktop
//! session is to give it a headless one, which is what
//! `scripts/headless-runner.sh` now does for the two examples that render.

use std::path::Path;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gdk, glib, graphene};

/// How long to wait for a window to become drawable, before scaling.
///
/// Generous: this bounds "the compositor is not going to show this window",
/// which is a conclusion worth being slow about, and none of it is paid on
/// the ordinary path, where the picture is ready on the first look.
pub const PATIENCE: Duration = Duration::from_secs(5);

/// A picture of a window, and what it cost to get one.
#[derive(Debug)]
pub struct Picture {
    /// What the window draws.
    pub texture: gdk::Texture,
    /// See [`Written::stalled`].
    pub stalled: bool,
}

/// What a written picture turned out to be.
#[derive(Debug)]
pub struct Written {
    pub width: i32,
    pub height: i32,
    /// Whether the compositor had stopped presenting this window.
    ///
    /// The picture is still the widgets' own, and is worth looking at. What
    /// it may not contain is anything drawn by a *different* process: the
    /// reader's WebKit view composites through the same compositor, and on a
    /// stalled surface it renders as a black rectangle. That reads exactly
    /// like a broken reader, so a caller that does not say this out loud is
    /// handing someone a picture that lies.
    pub stalled: bool,
}

/// Why a window could not be turned into a picture.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// GTK will not snapshot a widget the compositor is not showing.
    ///
    /// `mapped: false` means the window never reached the compositor at all,
    /// which nothing here can rescue. `mapped: true` means it did, and the
    /// layout could not be forced either.
    #[error(
        "the window never became drawable within {waited:.1?} (mapped: {mapped}, \
         allocation: {width}x{height}) — nothing is painted to a surface the \
         compositor is not showing. A blanked or locked screen is the commonest \
         cause on a workstation; on a machine with no session, run it under \
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
/// The same dial every suite here answers to, and deliberately not a second
/// one: a machine slow enough to need longer here needs longer everywhere,
/// and a constant edited in this file would slow every run to fix one box.
fn patience() -> Duration {
    let factor: f64 = std::env::var("POSTIO_TEST_PATIENCE")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|factor: &f64| *factor > 0.0)
        .unwrap_or(1.0);
    PATIENCE.mul_f64(factor)
}

/// Render `widget` to a picture, waiting until it can be.
///
/// The wait is [`PATIENCE`], scaled; use [`texture_within`] to say otherwise.
pub fn texture(widget: &impl IsA<gtk::Widget>) -> Result<Picture, Error> {
    texture_within(widget, patience())
}

/// Render `widget` to a picture, giving up after `deadline`.
///
/// Turns the main loop itself, so the caller does not have to settle the
/// window first — and a caller that tried would be guessing at a number of
/// frames this can simply watch for.
pub fn texture_within(
    widget: &impl IsA<gtk::Widget>,
    deadline: Duration,
) -> Result<Picture, Error> {
    let widget = widget.as_ref();
    let started = Instant::now();

    // A blocking iteration is what lets the frame clock tick — a
    // non-blocking one returns immediately when nothing is pending, so a
    // fixed number of them is not a wait at all and no frame need happen
    // inside it (#90). The heartbeat is what guarantees the blocking
    // iteration returns.
    let context = glib::MainContext::default();
    let heartbeat =
        glib::timeout_add_local(Duration::from_millis(10), || glib::ControlFlow::Continue);
    // What is waited for is a *presented* frame, which is the one thing that
    // separates "the compositor has not got to this window yet" from "the
    // compositor is not going to". The picture itself does not depend on the
    // answer — see `drawn` — but whether to warn about it does, and that
    // distinction cannot be made without spending the wait.
    let stalled = loop {
        if presenting(widget) {
            break false;
        }
        if started.elapsed() >= deadline {
            break true;
        }
        context.iteration(true);
    };
    heartbeat.remove();

    let Some(node) = drawn(widget) else {
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
    Ok(Picture {
        texture: renderer.render_texture(&node, Some(&bounds)),
        stalled,
    })
}

/// The window's picture, laid out first.
///
/// `None` for a widget with no allocation as well as for one GTK will not
/// snapshot: a zero-sized window yields a node that renders to nothing,
/// which is a blank PNG rather than an error, and a blank PNG that reports
/// success is the failure this module exists to stop.
///
/// # The layout is done here rather than waited for
///
/// GTK refuses to snapshot a widget with a pending resize, and a pending
/// resize is serviced in the frame clock's layout phase — which a compositor
/// that has stopped presenting never runs. So the window stays permanently
/// unrenderable while reporting its old width and height, which is what
/// #809 saw and read as a missing frame.
///
/// `allocate` is the call a parent makes on its child, which is exactly the
/// relationship here, and the size is the window's own last allocation, so
/// this invents no geometry: it re-runs the pass that was queued and never
/// serviced. Asking the frame clock for the phase directly does not work —
/// it is throttled by the very thing that has stopped — and was measured not
/// to.
///
/// Done on every capture rather than only as a rescue, so there is one path
/// and the tests exercise it. The picture is therefore of the window's
/// child, which for a `GtkWindow` fills it: the window widget's own CSS
/// background is not in the node. Postio's content paints its own plate, so
/// nothing visible is lost; a bare `GtkWindow` leaning on the default
/// background would render transparent.
fn drawn(widget: &gtk::Widget) -> Option<gtk::gsk::RenderNode> {
    if widget.width() <= 0 || widget.height() <= 0 {
        return None;
    }
    let snapshot = gtk::Snapshot::new();
    match laid_out(widget) {
        Some((window, child)) => window.snapshot_child(&child, &snapshot),
        None => {
            let paintable = gtk::WidgetPaintable::new(Some(widget));
            paintable.snapshot(
                &snapshot,
                f64::from(widget.width()),
                f64::from(widget.height()),
            );
        }
    }
    snapshot.to_node()
}

/// Whether the compositor is presenting this window right now.
///
/// A `GtkWidgetPaintable` over a **native** widget answers out of the
/// surface, so it is empty exactly when the surface has no presented frame —
/// which is why it is the wrong way to take a picture and the right way to
/// ask this question.
///
/// It cannot, on its own, tell "no frame yet" from "no frame ever": that is
/// what the wait in [`texture_within`] is for. Two other spellings were tried
/// first and neither works here. `GDK_TOPLEVEL_STATE_SUSPENDED` is the
/// compositor's own word for it and would be better if it were set — mutter
/// does not set it for a window that has merely stopped receiving frame
/// callbacks, measured. Asking the frame clock for a layout or paint phase
/// does nothing, because it is throttled by the very thing that has stopped.
fn presenting(widget: &gtk::Widget) -> bool {
    let paintable = gtk::WidgetPaintable::new(Some(widget));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(
        &snapshot,
        f64::from(widget.width().max(1)),
        f64::from(widget.height().max(1)),
    );
    snapshot.to_node().is_some()
}

/// The window and its child, with any pending resize settled.
fn laid_out(widget: &gtk::Widget) -> Option<(&gtk::Window, gtk::Widget)> {
    let window = widget.downcast_ref::<gtk::Window>()?;
    let child = GtkWindowExt::child(window)?;
    if child.width() <= 0 || child.height() <= 0 {
        return None;
    }
    child.allocate(child.width(), child.height(), -1, None);
    Some((window, child))
}

/// Render `widget` and write it to `path`.
///
/// **Nothing is written when this fails**, so the file's existence is a fact
/// a caller can act on — which is what lets `shot` exit non-zero and mean it.
pub fn png(widget: &impl IsA<gtk::Widget>, path: &Path) -> Result<Written, Error> {
    png_within(widget, path, patience())
}

/// [`png`], giving up after `deadline`.
pub fn png_within(
    widget: &impl IsA<gtk::Widget>,
    path: &Path,
    deadline: Duration,
) -> Result<Written, Error> {
    let picture = texture_within(widget, deadline)?;
    picture
        .texture
        .save_to_png(path)
        .map_err(|source| Error::Write {
            path: path.display().to_string(),
            source,
        })?;
    Ok(Written {
        width: picture.texture.width(),
        height: picture.texture.height(),
        stalled: picture.stalled,
    })
}
