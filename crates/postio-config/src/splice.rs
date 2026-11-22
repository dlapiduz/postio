//! Every structured pane drops its table, serializes a fresh one and splices it in at the end; one
//! copy
//! fixes the missing blank line above the fresh table.

use serde::Serialize;

use crate::{ConfigError, Result};

/// `wrapper` serializes as a document with the one table `name`. A caller with nothing to write (an
/// empty map, which `toml` would spell as a bare header) removes the section itself.
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

pub(crate) fn parse(text: &str) -> Result<toml_edit::DocumentMut> {
    text.parse::<toml_edit::DocumentMut>()
        .map_err(|err| ConfigError::parse(None, &err))
}

/// For an implicit parent such as `[saved_searches]`, its first child's header.
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
