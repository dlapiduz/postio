//! The palette and the cheat sheet, one list read two ways, so "available here" is decided once
//! (#658).
//! The matcher is `postio_ui::palette`'s and Swift must not write another: ranking is a product
//! decision.
//! `PaletteEntryFfi::positions` are byte offsets rather than markup, so each frontend highlights
//! its own way (#568).

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PaletteEntryFfi {
    pub id: String,
    pub title: String,
    /// The binding in force, or `None` when the command is palette-only; from the live keymap,
    /// expanded for this platform (`cmd+k`, not `mod+k`).
    pub binding: Option<String>,
    /// **Byte** offsets into `title` that the query matched; Swift converts once, at the edge.
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

/// The grouping is `postio_ui::cheatsheet::sections`'s and Swift must not regroup.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CheatSectionFfi {
    pub title: String,
    pub rows: Vec<CheatRowFfi>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CheatRowFfi {
    /// The command it runs, as `invoke` names it — or `None` for the one
    /// box's prefixes, which are typed rather than invoked.
    pub id: Option<String>,
    pub title: String,
    /// The binding in force, or `None` when the command is palette-only —
    /// still listed, because "this exists and has no key" is an answer.
    pub binding: Option<String>,
    /// How the row reads to a screen reader: a sentence, not "Archive a".
    pub spoken: String,
}

impl From<postio_ui::cheatsheet::Section> for CheatSectionFfi {
    fn from(section: postio_ui::cheatsheet::Section) -> Self {
        CheatSectionFfi {
            title: section.title.to_owned(),
            rows: section.rows.into_iter().map(CheatRowFfi::from).collect(),
        }
    }
}

impl From<postio_ui::cheatsheet::Row> for CheatRowFfi {
    fn from(row: postio_ui::cheatsheet::Row) -> Self {
        let spoken = postio_ui::cheatsheet::spoken(&row);
        CheatRowFfi {
            id: row.id.map(|id| id.as_str().to_owned()),
            title: row.title.to_owned(),
            binding: row.binding,
            spoken,
        }
    }
}
