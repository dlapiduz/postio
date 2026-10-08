//! The key map at the boundary (specs/009-focus-macos T103, for the Mac's
//! T106).
//!
//! `?` is the controller's (`postio_focus`, ADR 0045): it opens the key map
//! over whatever is up and puts it on the stack, and `?` or Back closes it.
//! The sheet's content is `postio_ui::keymap_sheet`'s -- the groups GTK's
//! dialog draws, the keys of the keymap in force, the words -- for this
//! platform, so the Mac teaches what the Mac answers and names the file the
//! Mac reads.

use crate::session::Session;

/// The key map, as a sheet draws it (screen 20).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct KeyMapSheetFfi {
    /// "Keys".
    pub title: String,
    /// The line beside the title.
    pub subtitle: String,
    /// The keys that close it, as the keymap spells them: `?`, `Escape`.
    pub close_keys: Vec<String>,
    /// What follows them: "close".
    pub close_word: String,
    /// What joins them: "or".
    pub close_or: String,
    /// The groups, in the key map's order, each with a row.
    pub groups: Vec<KeyMapGroupFfi>,
    /// Which groups go in which column: indices into `groups`, a group
    /// kept whole.
    pub columns: Vec<Vec<u32>>,
    /// "Rebind anything in ~/Library/Application Support/Postio/config.toml
    /// under `[keys]`".
    pub rebind_footer: String,
    /// "The mouse works everywhere: every key has a visible button."
    pub mouse_footer: String,
}

/// One group of the key map: "Move and select", "Act (row or selection)".
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct KeyMapGroupFfi {
    /// Its heading.
    pub title: String,
    /// Its rows, in the registry's order.
    pub rows: Vec<KeyMapRowFfi>,
}

/// One row of the key map.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct KeyMapRowFfi {
    /// The registry command, as `invoke` names it.
    pub command: String,
    /// What the registry calls it.
    pub title: String,
    /// Every key the keymap in force binds it to, the default first, as
    /// `[keys]` spells them. Empty for a command with no key.
    pub keys: Vec<String>,
}

/// The key map for `keymap`, on this platform: Focus's groups, less what
/// the platform does not offer, with the words for this platform's file.
pub(crate) fn sheet(keymap: &postio_core::Keymap) -> KeyMapSheetFfi {
    use postio_ui::keymap_sheet;
    let platform = postio_config::paths::Platform::host();
    let map = keymap_sheet::key_map_on(keymap, crate::FRONTEND, platform);
    let sizes: Vec<usize> = map.iter().map(|(_, rows)| rows.len()).collect();
    KeyMapSheetFfi {
        title: keymap_sheet::TITLE.to_owned(),
        subtitle: keymap_sheet::subtitle(platform).to_owned(),
        close_keys: keymap_sheet::CLOSE_COMMANDS
            .into_iter()
            .filter_map(|command| postio_ui::hints::key(keymap, command))
            .collect(),
        close_word: keymap_sheet::CLOSE_WORD.to_owned(),
        close_or: keymap_sheet::CLOSE_OR.to_owned(),
        columns: keymap_sheet::pack_columns(&sizes, keymap_sheet::COLUMNS)
            .into_iter()
            .map(|column| {
                column
                    .into_iter()
                    .map(|index| u32::try_from(index).unwrap_or(u32::MAX))
                    .collect()
            })
            .collect(),
        groups: map
            .into_iter()
            .map(|(group, rows)| KeyMapGroupFfi {
                title: group.title().to_owned(),
                rows: rows
                    .into_iter()
                    .map(|row| KeyMapRowFfi {
                        command: row.action.to_string(),
                        title: row.title.to_owned(),
                        keys: row.keys,
                    })
                    .collect(),
            })
            .collect(),
        rebind_footer: keymap_sheet::rebind_footer(platform).to_owned(),
        mouse_footer: keymap_sheet::MOUSE_FOOTER.to_owned(),
    }
}

#[uniffi::export]
impl Session {
    /// The key map under the keys in force, read now: what the sheet draws
    /// when `FocusOpenKeyMap` did not carry it, and what it draws again on
    /// `KeymapChanged` while it is up.
    pub fn focus_key_map(&self) -> KeyMapSheetFfi {
        sheet(&self.focus_driver().keymap())
    }
}
