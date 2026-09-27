//! The reading renderer: message bodies drawn in-process, with no network
//! crate and no C anywhere on the path message content takes (spec 006
//! FR-001, FR-023a).
//!
//! The engine is Blitz (spec 006 research R0, decided by evaluation). The
//! crate is being built task by task; until it is, it holds the evaluation's
//! harness in `examples/` and the dependency graph the checks prove.

pub mod theme;

// Placeholder uses until the renderer is written, one per dependency.
#[allow(unused_imports)]
use {
    anyrender::PaintScene as _, anyrender_vello_cpu::VelloCpuImageRenderer as _,
    blitz_dom::BaseDocument as _, blitz_html::HtmlDocument as _, blitz_paint::paint_scene as _,
    blitz_traits::shell::Viewport as _, fontdb::Database as _, image::ImageFormat as _,
    parley::FontContext as _, usvg::Tree as _,
};
