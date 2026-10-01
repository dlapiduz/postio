//! How a message body is shown: in the app's own colours, or as sent, on
//! paper (the message dialog redesign, T205-T214 in
//! `specs/007-postio-focus/tasks.md`).
//!
//! The rule is that the sender's colours never meet the app's surface, so
//! every body gets exactly one of the two, in light mode as in dark. Which one
//! decides the reading column's width too, which is why the decision is a
//! value of its own rather than a flag inside the renderer.

/// One body's treatment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Treatment {
    /// The sender's colours, backgrounds and fonts removed; the app's ink on
    /// the dialog's surface, in a 480px column. Plain text always, and HTML
    /// that does not paint its own page.
    #[default]
    AppColours,
    /// Rendered as sent on a white sheet, dimmed a little in dark mode and
    /// never inverted, in a column up to 640px. HTML that paints its own page.
    Paper,
}
