//! The reading renderer: message bodies drawn in-process, with no network crate
//! and no C on the path message content takes.

pub mod fonts;
mod present;
pub mod render;
pub mod resources;
mod snapshot;
mod text_index;
pub mod theme;
pub mod thread;
pub mod tile;

pub use kurbo::{Point, Rect};
pub use render::{BASE_URL, FOLD_ATTRIBUTE, Raster, rasterize, render};
pub use resources::Resources;
pub use theme::{Presentation, Rgb, Theme};
pub use thread::{DEFAULT_RENDER_DEADLINE, NOTICE_CLASS, Renderer};

pub type Scope = String;

#[derive(Clone, Debug)]
pub struct RenderRequest {
    /// Monotonic per reader; a result for a stale generation is dropped.
    pub generation: u64,
    pub document: String,
    pub plain_text: String,
    /// The plain text composed as the reader composes a plain-text body, so it takes
    /// the same column and face. `None` draws `plain_text` by the renderer's own minimal path.
    pub fallback: Option<Fallback>,
    /// The input cap the sanitizer found the body over, if any: such a request is drawn as
    /// `plain_text`.
    pub over_cap: Option<postio_body::Cap>,
    pub resources: std::sync::Arc<Resources>,
    pub viewport: Viewport,
    pub theme: Theme,
    pub darkened: Vec<Scope>,
    /// Folds toggled from how the document sets them, by the id `postio-ui` stamps
    /// as `data-postio-fold`: the latest thread message starts open, a quote closed.
    pub toggled_folds: Vec<String>,
    pub reader_view: Vec<Scope>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fallback {
    pub document: String,
    /// False when the surface says it itself (Focus's render-mode line): a swap is
    /// never silent, and said twice it is noise.
    pub notice: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub width: f64,
    pub hidpi_scale: f64,
    pub zoom: f64,
}

/// Immutable and `Send + Sync`, so it crosses from the render thread without the engine.
#[derive(Debug)]
pub struct RenderedDocument {
    pub generation: u64,
    pub size: kurbo::Size,
    pub scale: f64,
    pub display_list: anyrender::Scene,
    /// At most 16 MiB: drawn where a tile is not ready, so no frame is blank.
    pub low_res: Raster,
    pub text: TextIndex,
    pub links: Vec<LinkBox>,
    pub messages: Vec<MessageBox>,
    pub folds: Vec<FoldBox>,
    /// The id as the document spells it, and its top in CSS pixels.
    pub anchors: Vec<(String, f64)>,
    pub counts: RenderCounts,
    pub outcome: Outcome,
    pub needs_reader_view: Vec<Scope>,
    /// 1.0 when it fitted or nothing was on paper; never below [`render::PAPER_FIT_FLOOR`].
    pub fit: f64,
    _live: Live,
}

static_assertions::assert_impl_all!(RenderedDocument: Send, Sync);

static LIVE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[derive(Debug)]
struct Live(());

impl Live {
    fn new() -> Live {
        LIVE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Live(())
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Test-facing: viewing more messages must not leave more snapshots behind.
#[doc(hidden)]
pub fn live_documents() -> usize {
    LIVE.load(std::sync::atomic::Ordering::Relaxed)
}

impl RenderedDocument {
    pub fn link_at(&self, point: Point) -> Option<&LinkBox> {
        self.links.iter().find(|link| link.rect.contains(point))
    }

    pub fn fold_at(&self, point: Point) -> Option<&FoldBox> {
        self.folds
            .iter()
            .find(|fold| fold.summary_rect.contains(point))
    }
}

impl LinkTarget {
    pub fn describe(&self) -> String {
        match self {
            LinkTarget::External(url) => url.to_string(),
            LinkTarget::Verb { verb, .. } => verb.name().to_owned(),
            LinkTarget::Fragment { id, .. } => format!("#{id}"),
        }
    }
}

impl Verb {
    pub fn name(self) -> &'static str {
        match self {
            Verb::Allow => "allow",
            Verb::Reply => "reply",
            Verb::Forward => "forward",
            Verb::Continue => "continue",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextIndex {
    /// Tabs between cells, newlines at rows and blocks, `alt` included.
    pub text: String,
    /// Sorted by `range.start`, non-overlapping; every visible glyph is in
    /// exactly one.
    pub clusters: Vec<Cluster>,
}

/// A sentence kept by its words, and where it sat in the text it was read from.
/// The offset cannot address the index (laid-out, whitespace collapsed, closed
/// folds out); it only says which occurrence it was when the words repeat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Excerpt<'a> {
    pub text: &'a str,
    pub offset: usize,
    pub source_len: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cluster {
    pub range: std::ops::Range<usize>,
    pub rect: Rect,
    pub scope: Scope,
    pub color: Rgb,
    pub painted_ground: Rgb,
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LinkBox {
    pub rect: Rect,
    pub target: LinkTarget,
}

/// What a link does. `javascript:` and every other scheme are not links.
#[derive(Clone, Debug, PartialEq)]
pub enum LinkTarget {
    External(url::Url),
    Verb {
        scope: Scope,
        verb: Verb,
    },
    Fragment {
        /// The message the `id` is in: fragments never cross messages.
        scope: Scope,
        id: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    Allow,
    Reply,
    Forward,
    Continue,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MessageBox {
    pub scope: Scope,
    pub rect: Rect,
    pub presentation: Presentation,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FoldBox {
    pub id: String,
    pub summary_rect: Rect,
    pub open: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderCounts {
    /// Renders this request made: always 1.
    pub renders: u32,
    /// Style passes: 1, or 2 when colours were repaired.
    pub style_passes: u32,
    pub nodes: u32,
    pub text_clusters: u32,
    pub repaired_runs: u32,
    pub resources_resolved: u32,
    pub resources_unresolved: u32,
    pub images_placeholdered: u32,
    pub display_list_commands: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Rendered,
    FellBack(FallbackReason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FallbackReason {
    Panicked,
    Deadline,
    OverCap(postio_body::Cap),
    Undecodable,
}

impl FallbackReason {
    pub fn why(self) -> &'static str {
        match self {
            FallbackReason::Deadline => "this message took too long to lay out",
            FallbackReason::Panicked => "this message could not be laid out",
            FallbackReason::OverCap(_) => "this message is too large to lay out",
            FallbackReason::Undecodable => "this message could not be read",
        }
    }

    pub fn notice(self) -> String {
        format!("Shown as plain text: {}.", self.why())
    }
}
