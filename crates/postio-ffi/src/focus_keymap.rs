//! The key map sheet for this platform: `postio_ui::keymap_sheet`'s groups and the keys of the
//! keymap in force, so the Mac teaches what the Mac answers. `?` is the controller's (ADR 0045).

use crate::session::Session;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct KeyMapSheetFfi {
    pub title: String,
    pub subtitle: String,
    /// The keys that close it, as the keymap spells them: `?`, `Escape`.
    pub close_keys: Vec<String>,
    /// What follows them: "close".
    pub close_word: String,
    /// What joins them: "or".
    pub close_or: String,
    pub groups: Vec<KeyMapGroupFfi>,
    /// Which groups go in which column: indices into `groups`, a group
    /// kept whole.
    pub columns: Vec<Vec<u32>>,
    /// "Rebind anything in ~/Library/Application Support/Postio/config.toml
    /// under `[keys]`".
    pub rebind_footer: String,
    /// "The mouse works everywhere: every key has a visible button."
    pub mouse_footer: String,
    /// "More below", under the columns while rows lie past the panel's
    /// edge.
    pub more_below: String,
}

/// One group of the key map: "Move and select", "Act (row or selection)".
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct KeyMapGroupFfi {
    pub title: String,
    pub rows: Vec<KeyMapRowFfi>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct KeyMapRowFfi {
    pub command: String,
    pub title: String,
    /// Every key the keymap in force binds it to, the default first, as
    /// `[keys]` spells them. Empty for a command with no key.
    pub keys: Vec<String>,
}

/// The key map for `keymap`, on this platform: less what the platform does not offer.
pub(crate) fn sheet(keymap: &postio_core::Keymap) -> KeyMapSheetFfi {
    use postio_ui::keymap_sheet;
    let platform = crate::PLATFORM;
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
        more_below: keymap_sheet::MORE_BELOW.to_owned(),
    }
}

#[uniffi::export]
impl Session {
    /// The key map under the keys in force, read now: what the sheet draws when `FocusOpenKeyMap`
    /// did not carry it.
    pub fn focus_key_map(&self) -> KeyMapSheetFfi {
        sheet(&self.focus_driver().keymap())
    }
}
