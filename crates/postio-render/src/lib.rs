//! The reading renderer: message bodies drawn in-process, with no network
//! crate and no C anywhere on the path message content takes (spec 006
//! FR-001, FR-023a).
//!
//! The engine is Blitz (spec 006 research R0, decided by evaluation). The
//! crate is being built task by task; until it is, it holds the evaluation's
//! harness in `examples/` and the dependency graph the checks prove.

pub mod fonts;
pub mod render;
pub mod resources;
mod snapshot;
mod text_index;
pub mod theme;
pub mod thread;

pub use kurbo::{Point, Rect};
pub use render::{BASE_URL, FOLD_ATTRIBUTE, Raster, rasterize, render};
pub use resources::Resources;
pub use theme::{Presentation, Rgb, Theme};
pub use thread::{DEFAULT_RENDER_DEADLINE, Renderer};

/// One message's key in a composed conversation: what its container's
/// `data-postio-message` carries, and what its `cid:` references resolve in.
pub type Scope = String;

/// What the UI thread asks the render thread for (data-model.md).
#[derive(Clone, Debug)]
pub struct RenderRequest {
    /// Monotonic per reader; a result for a stale generation is dropped.
    pub generation: u64,
    /// The composed HTML.
    pub document: String,
    /// The message's plain-text alternative, drawn if the render falls
    /// back (FR-023).
    pub plain_text: String,
    /// The input cap the sanitizer found the body over, if any: such a
    /// request is drawn as `plain_text` and never reaches the engine.
    pub over_cap: Option<postio_body::Cap>,
    /// Everything the document may load; nothing else is reachable.
    pub resources: std::sync::Arc<Resources>,
    /// The laid-out width and the two scales, never folded together (R11).
    pub viewport: Viewport,
    /// The theme every message is classified and repaired against (R10).
    pub theme: Theme,
    /// The messages the user asked to darken (FR-013a).
    pub darkened: Vec<Scope>,
    /// The `<details>` folds the user has toggled from how the document
    /// sets them, by the stable id `postio-ui` stamps as
    /// `data-postio-fold`. A toggle, not a state: the latest thread message
    /// starts open and a quote starts closed, and either may be flipped.
    pub toggled_folds: Vec<String>,
    /// The messages shown in Reader view (R16).
    pub reader_view: Vec<Scope>,
}

/// The surface a document is laid out for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    /// The width in logical (CSS) pixels.
    pub width: f64,
    /// The surface's fractional scale: 1.0, 1.25, 2.0.
    pub hidpi_scale: f64,
    /// The user's zoom step, 1.0 being 100% (R11).
    pub zoom: f64,
}

/// The one thing the widget reads: immutable, and `Send + Sync`, so it
/// crosses from the render thread without the engine (R7).
#[derive(Debug)]
pub struct RenderedDocument {
    /// Copied from the request.
    pub generation: u64,
    /// The laid-out size in CSS pixels at this zoom.
    pub size: kurbo::Size,
    /// The device scale the display list was recorded at.
    pub scale: f64,
    /// The display list, recorded once and rasterised per tile.
    pub display_list: anyrender::Scene,
    /// The whole document at a quarter of its scale, at most 16 MiB: what
    /// is drawn where a tile is not ready yet, so no frame is blank.
    pub low_res: Raster,
    /// The copy, find and accessibility text, with its geometry.
    pub text: TextIndex,
    /// Every link, in document order.
    pub links: Vec<LinkBox>,
    /// Every message's box and how it is presented, in document order.
    pub messages: Vec<MessageBox>,
    /// Every fold and whether it is open.
    pub folds: Vec<FoldBox>,
    /// What the render cost, counted (Principle V).
    pub counts: RenderCounts,
    /// Whether the sender's markup was drawn, or the plain-text fallback.
    pub outcome: Outcome,
}

static_assertions::assert_impl_all!(RenderedDocument: Send, Sync);

/// The document's text in reading order, the one serialisation copy, find
/// and the screen reader share (R7).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextIndex {
    /// Tabs between cells, newlines at rows and blocks, `alt` included.
    pub text: String,
    /// Sorted by `range.start`, non-overlapping; every visible glyph is in
    /// exactly one.
    pub clusters: Vec<Cluster>,
}

/// A run of text drawn in one colour on one ground.
#[derive(Clone, Debug, PartialEq)]
pub struct Cluster {
    /// Chars into [`TextIndex::text`].
    pub range: std::ops::Range<usize>,
    /// Where it is drawn, in CSS pixels.
    pub rect: Rect,
    /// The message it belongs to.
    pub scope: Scope,
    /// The colour it is drawn in, after repair.
    pub color: Rgb,
    /// The colour painted behind it (R10's ancestor walk).
    pub painted_ground: Rgb,
    /// Which laid-out line it is on, counted through the document.
    pub line: u32,
}

/// A link's box and where it goes.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkBox {
    /// Where it is drawn, in CSS pixels.
    pub rect: Rect,
    /// What activating it does.
    pub target: LinkTarget,
}

/// What a link does. `javascript:` and every other scheme are not links.
#[derive(Clone, Debug, PartialEq)]
pub enum LinkTarget {
    /// `http`, `https` or `mailto`, opened outside the reader.
    External(url::Url),
    /// One of the reader's own verbs, for the message that offers it.
    Verb {
        /// The message offering it.
        scope: Scope,
        /// Which verb.
        verb: Verb,
    },
    /// An `id` inside a message.
    Fragment {
        /// The message the `id` is in: fragments never cross messages.
        scope: Scope,
        /// The sender's `id`, before scoping.
        id: String,
    },
}

/// A verb a message offers inside the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    /// Show this sender's remote images.
    Allow,
    /// Reply to this message.
    Reply,
    /// Forward this message.
    Forward,
    /// Resume the composer on this draft.
    Continue,
}

/// One message's container.
#[derive(Clone, Debug, PartialEq)]
pub struct MessageBox {
    /// The message.
    pub scope: Scope,
    /// Its container's box, in CSS pixels.
    pub rect: Rect,
    /// How the theme rule presented it (R10).
    pub presentation: Presentation,
}

/// A `<details>` fold.
#[derive(Clone, Debug, PartialEq)]
pub struct FoldBox {
    /// The stable fold id `postio-ui` stamps.
    pub id: String,
    /// The summary's box, which toggles it.
    pub summary_rect: Rect,
    /// Whether it is drawn open.
    pub open: bool,
}

/// What one render cost, counted rather than timed (Principle V).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderCounts {
    /// Renders this request made: always 1.
    pub renders: u32,
    /// Style passes: 1, or 2 when colours were repaired.
    pub style_passes: u32,
    /// DOM nodes laid out.
    pub nodes: u32,
    /// Clusters in the text index.
    pub text_clusters: u32,
    /// Runs whose colour was repaired to the floor.
    pub repaired_runs: u32,
    /// Resources found in the request's table.
    pub resources_resolved: u32,
    /// Lookups the table could not answer: drawn as nothing, never fetched.
    pub resources_unresolved: u32,
    /// Images drawn as a sized placeholder.
    pub images_placeholdered: u32,
    /// Commands in the display list.
    pub display_list_commands: u32,
}

/// Whether the sender's markup was drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// It was.
    Rendered,
    /// It was not, and the plain-text alternative is drawn instead.
    FellBack(FallbackReason),
}

/// Why a render fell back to plain text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FallbackReason {
    /// The engine panicked; the document was dropped.
    Panicked,
    /// It did not finish within the deadline (FR-023).
    Deadline,
    /// The sanitizer found the body over this input cap.
    OverCap(postio_body::Cap),
    /// Nothing in the message could be decoded as a body.
    Undecodable,
}
