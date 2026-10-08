//! Tab's walk across the panes, at the boundary.
//!
//! The order is [`postio_ui::focus::next_pane`]'s, the same table the GTK
//! window walks; the macOS app's `Pane.next()` asks this rather than
//! keeping a copy, so the two frontends cannot disagree on what Tab does.

use crate::UiContext;

/// The pane after `context`, wrapping, or `None` when `context` is not a
/// pane Tab cycles through — the composer, the palette, a settings list.
#[uniffi::export]
pub fn next_pane(context: UiContext, forward: bool) -> Option<UiContext> {
    postio_ui::focus::next_pane(context.into(), forward).map(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_boundary_walks_the_same_cycle_as_the_gtk_window() {
        assert_eq!(next_pane(UiContext::Sidebar, true), Some(UiContext::List));
        assert_eq!(next_pane(UiContext::List, true), Some(UiContext::Reader));
        assert_eq!(next_pane(UiContext::Reader, true), Some(UiContext::Sidebar));
        assert_eq!(
            next_pane(UiContext::Sidebar, false),
            Some(UiContext::Reader)
        );
        assert_eq!(next_pane(UiContext::Composer, true), None);
    }
}
