//! The palette and the cheat sheet, which are one list read two ways.
//!
//! The palette filters the registry by what was typed and by which surface
//! has focus; the cheat sheet prints the same rows grouped, with the bindings
//! in force. Building them separately would mean two places deciding what
//! "available here" means, and #658 is explicit that they will disagree.
//!
//! **The matcher is `postio_ui::palette`'s and Swift must not write another.**
//! The ranking is a product decision — `cp` finds "Command palette" ahead of
//! "Copy" because of how word starts are scored — and two rankings mean the
//! same query offers different things on each platform.
//!
//! What crosses instead of markup is [`PaletteEntryFfi::positions`]: byte
//! offsets into the title. #568 changed the matcher to return ranges rather
//! than pre-escaped Pango precisely so a second frontend could highlight them
//! its own way, and Swift builds an `AttributedString` from the same numbers
//! GTK turns into `<b>`.

/// One row of the palette, or one line of the cheat sheet.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PaletteEntryFfi {
    /// The command this row runs, as `invoke` names it.
    pub id: String,
    /// Its title, as the registry gives it.
    pub title: String,
    /// The binding in force, or `None` when the command is palette-only.
    ///
    /// From the live keymap, so a `[keys]` override shows here without
    /// anything else being told — and expanded for this platform, so a Mac
    /// reads `cmd+k` rather than the `mod+k` the table stores.
    pub binding: Option<String>,
    /// **Byte** offsets into `title` that the query matched.
    ///
    /// Byte, not character: the matcher works in `char_indices`, and saying
    /// so is the difference between highlighting the right glyph and
    /// splitting a multi-byte one. Swift converts once, at the edge.
    pub positions: Vec<u32>,
}

impl From<postio_ui::palette::Entry> for PaletteEntryFfi {
    fn from(entry: postio_ui::palette::Entry) -> Self {
        PaletteEntryFfi {
            id: entry.id.as_str().to_string(),
            title: entry.title.to_string(),
            binding: entry.binding,
            positions: entry.positions.into_iter().map(|at| at as u32).collect(),
        }
    }
}
