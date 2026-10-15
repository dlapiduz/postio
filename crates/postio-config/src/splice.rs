//! Putting one freshly written section into a person's file.
//!
//! Every structured pane (`patch_ui`, `patch_sync`, `patch_compose`,
//! `patch_keys`, `patch_filters`, `patch_reader`) writes the same way: drop
//! its table from the document, serialize a fresh one, splice it in at the
//! end. Six copies of that splice each forgot the same thing -- the fresh
//! table comes out of `toml` with no blank line above it, so the file read
//! `editor = ""` straight into `[sync]` -- and one copy is where that is
//! fixed once.

use serde::Serialize;

use crate::{ConfigError, Result};

/// `doc` with its `name` table replaced by `wrapper`'s, serialized: the
/// table set off from whatever is above it by one blank line, as a person
/// writes TOML and as `toml::to_string_pretty` writes the starter file.
///
/// `wrapper` serializes as a document with the one table `name` in it. A
/// caller with nothing to write (an empty map, which `toml` would spell as a
/// bare header) removes the section itself instead.
pub(crate) fn replace_section<T: Serialize>(text: &str, name: &str, wrapper: &T) -> Result<String> {
    let mut doc = parse(text)?;
    doc.as_table_mut().remove(name);
    let fragment =
        toml::to_string(wrapper).map_err(|err| ConfigError::Serialize(err.to_string()))?;
    let fragment = parse(&fragment)?;
    if let Some(item) = fragment.as_table().get(name) {
        let below_something = !doc.as_table().is_empty();
        doc.as_table_mut().insert(name, item.clone());
        if below_something
            && let Some(table) = doc.get_mut(name).and_then(|item| item.as_table_mut())
        {
            set_off(table);
        }
    }
    Ok(doc.to_string())
}

/// `text` as a document, or the parse error the panes report.
pub(crate) fn parse(text: &str) -> Result<toml_edit::DocumentMut> {
    text.parse::<toml_edit::DocumentMut>()
        .map_err(|err| ConfigError::parse(None, &err))
}

/// A blank line above the first header `table` draws: its own, or, for an
/// implicit parent such as `[saved_searches]` (which has no header of its
/// own), its first child's.
fn set_off(table: &mut toml_edit::Table) {
    if !table.is_implicit() {
        table.decor_mut().set_prefix("\n");
        return;
    }
    let first = table
        .iter()
        .filter_map(|(key, item)| {
            item.as_table()
                .map(|child| (child.position(), key.to_owned()))
        })
        .min();
    if let Some((_, key)) = first
        && let Some(child) = table.get_mut(&key).and_then(|item| item.as_table_mut())
    {
        set_off(child);
    }
}
