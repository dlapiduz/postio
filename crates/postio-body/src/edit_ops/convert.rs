//! A typed Markdown sequence becoming its block or mark (spec 002 FR-068). Which characters are a
//! trigger is `postio_ui::editor::markdown`'s question; this module knows no Markdown.

use super::Mark;
use crate::document::{HeadingLevel, Href, Language, ListKind, Task};

/// The `trigger_len` characters before the caret are the sequence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conversion {
    pub trigger_len: u32,
    pub kind: ConversionKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConversionKind {
    Heading(HeadingLevel),
    List(ListKind),
    Task(Task),
    Quote,
    /// A line alone: `---`, `***` or `___`.
    Rule,
    /// The marker under a line of text: the trigger is the break and the marker.
    Setext(HeadingLevel),
    CodeBlock(Option<Language>),
    /// The trigger opens and closes with `marker_len` characters around the text the marks cover.
    Wrap {
        marks: Vec<Mark>,
        marker_len: u32,
    },
    /// `label` is the start and length of the link's text within the trigger.
    Link {
        href: Href,
        label: (u32, u32),
    },
}
