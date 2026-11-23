use serde::{Deserialize, Serialize};

use crate::Extras;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SignaturesConfig {
    #[serde(default)]
    pub gnupg_keyring: bool,
    #[serde(flatten)]
    pub extra: Extras,
}

/// `None` when the file already says so.
pub fn set_gnupg_keyring(text: &str, on: bool) -> crate::Result<Option<String>> {
    let mut doc = crate::splice::parse(text)?;
    let says = doc
        .get("signatures")
        .and_then(|signatures| signatures.get("gnupg_keyring"))
        .and_then(toml_edit::Item::as_bool)
        .unwrap_or(false);
    if says == on {
        return Ok(None);
    }
    let signatures = doc
        .as_table_mut()
        .entry("signatures")
        .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .ok_or_else(|| {
            crate::ConfigError::Serialize(
                "[signatures] is not a table, so Postio will not write to it; fix that line and try again".to_owned(),
            )
        })?;
    signatures.insert("gnupg_keyring", toml_edit::value(on));
    Ok(Some(doc.to_string()))
}
