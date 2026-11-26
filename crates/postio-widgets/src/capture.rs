//! Renders a window to a texture, waiting on a condition rather than a frame count. GTK will not
//! snapshot a widget with a pending resize, and a compositor that stopped presenting never services
//! one, so the layout is forced here. A compositor is still required: an unmapped window has no
//! picture.

use std::path::Path;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gdk, glib, graphene};

pub const PATIENCE: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub struct Picture {
    pub texture: gdk::Texture,
    pub stalled: bool,
}

#[derive(Debug)]
pub struct Written {
    pub width: i32,
    pub height: i32,
    /// The compositor had stopped presenting this window. The picture is still the widgets' own,
    /// but a WebKit view composites through the compositor and renders black on a stalled surface.
    pub stalled: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// `mapped: false` means the window never reached the compositor; `true` means layout could not
    /// be forced either.
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

    #[error("the window has no renderer, so there is nothing to render through")]
    NoRenderer,

    #[error("cannot write {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: glib::BoolError,
    },
}

/// [`PATIENCE`] scaled by `POSTIO_TEST_PATIENCE`, the one dial every suite answers to.
fn patience() -> Duration {
    let factor: f64 = std::env::var("POSTIO_TEST_PATIENCE")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|factor: &f64| *factor > 0.0)
        .unwrap_or(1.0);
    PATIENCE.mul_f64(factor)
}

pub fn texture(widget: &impl IsA<gtk::Widget>) -> Result<Picture, Error> {
    texture_within(widget, patience())
}

/// Turns the main loop itself, so the caller need not settle the window first.
pub fn texture_within(
    widget: &impl IsA<gtk::Widget>,
    deadline: Duration,
) -> Result<Picture, Error> {
    let widget = widget.as_ref();
    let started = Instant::now();

    // A blocking iteration lets the frame clock tick; a non-blocking one returns at once when
    // nothing is pending. The heartbeat guarantees the blocking one returns.
    let context = glib::MainContext::default();
    let heartbeat =
        glib::timeout_add_local(Duration::from_millis(10), || glib::ControlFlow::Continue);
    // Waits for a presented frame, which separates "the compositor has not got to this window yet"
    // from "it is not going to".
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

/// Renders `widget` as it is this instant, without waiting for the compositor: for callers already
/// watching frames (storyboard settle sampling), who would pay the wait every time.
pub fn texture_now(widget: &impl IsA<gtk::Widget>) -> Result<gdk::Texture, Error> {
    render_now(widget.as_ref(), None::<fn(&gtk::Snapshot)>)
}

/// [`texture_now`] with `overlay` drawn over the widget's picture, in the widget's coordinates and
/// never part of it.
pub fn texture_with(
    widget: &impl IsA<gtk::Widget>,
    overlay: impl FnOnce(&gtk::Snapshot),
) -> Result<gdk::Texture, Error> {
    render_now(widget.as_ref(), Some(overlay))
}

fn render_now<F: FnOnce(&gtk::Snapshot)>(
    widget: &gtk::Widget,
    overlay: Option<F>,
) -> Result<gdk::Texture, Error> {
    let never_drawable = || Error::NeverDrawable {
        waited: Duration::ZERO,
        mapped: widget.is_mapped(),
        width: widget.width(),
        height: widget.height(),
    };
    let node = drawn(widget).ok_or_else(never_drawable)?;
    let node = match overlay {
        None => node,
        Some(overlay) => {
            let snapshot = gtk::Snapshot::new();
            snapshot.append_node(&node);
            overlay(&snapshot);
            snapshot.to_node().ok_or_else(never_drawable)?
        }
    };
    let renderer = widget
        .native()
        .and_then(|native| native.renderer())
        .ok_or(Error::NoRenderer)?;
    let bounds = graphene::Rect::new(0.0, 0.0, widget.width() as f32, widget.height() as f32);
    Ok(renderer.render_texture(&node, Some(&bounds)))
}

/// `None` for a widget with no allocation as well as one GTK will not snapshot: a zero-sized window
/// renders a blank PNG, not an error. GTK refuses to snapshot a widget with a pending resize, which
/// only the frame clock's layout phase services and a stalled compositor never runs, so `allocate`
/// re-runs the queued pass on every capture. The window's own CSS background is not in the picture.
fn drawn(widget: &gtk::Widget) -> Option<gtk::gsk::RenderNode> {
    if widget.width() <= 0 || widget.height() <= 0 {
        return None;
    }
    let snapshot = gtk::Snapshot::new();
    match laid_out(widget) {
        Some((window, child)) => {
            window.snapshot_child(&child, &snapshot);
            for popover in open_popovers(window) {
                over(window, &popover, &snapshot);
            }
        }
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

/// Every popover open over `window`, outermost first. A popover is a surface of its own, so the
/// window's child drawn alone leaves it out; it is found as a child of the widget it hangs from.
fn open_popovers(window: &gtk::Window) -> Vec<gtk::Popover> {
    let mut found = Vec::new();
    let mut stack: Vec<gtk::Widget> = GtkWindowExt::child(window).into_iter().collect();
    while let Some(widget) = stack.pop() {
        if let Some(popover) = widget.downcast_ref::<gtk::Popover>()
            && popover.is_mapped()
        {
            found.push(popover.clone());
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            stack.push(next);
        }
    }
    found
}

/// Where the compositor shows `popover` in `window`'s frame, less where each widget sits within its
/// own surface (shadow, margin); `None` without a popup surface.
pub(crate) fn popover_origin(
    window: &gtk::Window,
    popover: &gtk::Popover,
) -> Option<graphene::Point> {
    let popup = popover.surface()?.downcast::<gdk::Popup>().ok()?;
    let (window_x, window_y) = window.surface_transform();
    let (popover_x, popover_y) = popover.surface_transform();
    let x = f64::from(popup.position_x()) + popover_x - window_x;
    let y = f64::from(popup.position_y()) + popover_y - window_y;
    Some(graphene::Point::new(x as f32, y as f32))
}

fn over(window: &gtk::Window, popover: &gtk::Popover, snapshot: &gtk::Snapshot) {
    let Some(origin) = popover_origin(window, popover) else {
        return;
    };
    snapshot.save();
    snapshot.translate(&origin);
    let mut child = popover.first_child();
    while let Some(next) = child {
        child = next.next_sibling();
        popover.snapshot_child(&next, snapshot);
    }
    snapshot.restore();
}

/// A `GtkWidgetPaintable` over a native widget is empty exactly when the surface has no presented
/// frame, so it answers this question but is the wrong way to take a picture.
/// `GDK_TOPLEVEL_STATE_SUSPENDED` is not set by mutter for a window that merely stopped receiving
/// frame callbacks, and the frame clock is throttled by the same thing.
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

fn laid_out(widget: &gtk::Widget) -> Option<(&gtk::Window, gtk::Widget)> {
    let window = widget.downcast_ref::<gtk::Window>()?;
    let child = GtkWindowExt::child(window)?;
    if child.width() <= 0 || child.height() <= 0 {
        return None;
    }
    child.allocate(child.width(), child.height(), -1, None);
    Some((window, child))
}

/// Writes nothing when this fails, so the file's existence is a fact a caller can act on.
pub fn png(widget: &impl IsA<gtk::Widget>, path: &Path) -> Result<Written, Error> {
    png_within(widget, path, patience())
}

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
