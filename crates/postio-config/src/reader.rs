//! `[reader]` — how the reading pane draws a message (spec 006).

use serde::{Deserialize, Deserializer, Serialize};

use crate::{ConfigError, Extras, Result};

/// Every zoom step, in percent (spec 006 FR-021): a value off the list
/// clamps to the nearest one, because the file is edited by hand.
pub const ZOOM_STEPS: [u16; 13] = [50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300];

/// The step nearest `percent`; below the first or above the last, the end.
pub fn nearest_zoom(percent: u16) -> u16 {
    *ZOOM_STEPS
        .iter()
        .min_by_key(|step| step.abs_diff(percent))
        .expect("the steps are not empty")
}

/// The `[reader]` section.
///
/// ```toml
/// [reader]
/// zoom = 100   # percent: 50 67 75 80 90 100 110 125 150 175 200 250 300
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReaderConfig {
    /// How large messages are drawn, in percent; one of [`ZOOM_STEPS`].
    #[serde(default = "hundred", deserialize_with = "stepped")]
    pub zoom: u16,
    /// Keys this version of Postio does not know, preserved verbatim.
    #[serde(flatten)]
    pub extra: Extras,
}

fn hundred() -> u16 {
    100
}

fn stepped<'de, D: Deserializer<'de>>(deserializer: D) -> std::result::Result<u16, D::Error> {
    let value = i64::deserialize(deserializer)?;
    Ok(nearest_zoom(value.clamp(0, i64::from(u16::MAX)) as u16))
}

impl Default for ReaderConfig {
    fn default() -> Self {
        ReaderConfig {
            zoom: hundred(),
            extra: Extras::new(),
        }
    }
}

/// Rewrites `text`'s `[reader]` table to match `reader`, leaving every
/// other section and its comments untouched -- `patch_ui`'s tradeoff.
pub fn patch_reader(text: &str, reader: &ReaderConfig) -> Result<String> {
    let mut doc = text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|err| ConfigError::parse(None, &err))?;
    doc.as_table_mut().remove("reader");
    let fragment = toml::to_string(&ReaderOnly { reader })
        .map_err(|err| ConfigError::Serialize(err.to_string()))?;
    let fragment_doc = fragment
        .parse::<toml_edit::DocumentMut>()
        .map_err(|err| ConfigError::parse(None, &err))?;
    if let Some(item) = fragment_doc.as_table().get("reader") {
        doc.as_table_mut().insert("reader", item.clone());
    }
    Ok(doc.to_string())
}

#[derive(Serialize)]
struct ReaderOnly<'a> {
    reader: &'a ReaderConfig,
}

#[cfg(test)]
mod tests {
    use crate::Config;

    use super::*;

    fn parse(text: &str) -> Config {
        toml::from_str(text).expect("a config")
    }

    #[test]
    fn zoom_defaults_to_one_hundred() {
        assert_eq!(parse("").reader.zoom, 100);
        assert_eq!(parse("[reader]\n").reader.zoom, 100);
    }

    #[test]
    fn a_zoom_off_the_steps_loads_as_the_nearest_step() {
        assert_eq!(parse("[reader]\nzoom = 112\n").reader.zoom, 110);
        assert_eq!(parse("[reader]\nzoom = 5\n").reader.zoom, 50);
        assert_eq!(parse("[reader]\nzoom = 900\n").reader.zoom, 300);
    }

    #[test]
    fn zoom_round_trips_and_patching_keeps_the_rest() {
        let text = "# mine\n[ui]\ndensity = \"compact\"\n";
        let patched = patch_reader(
            text,
            &ReaderConfig {
                zoom: 150,
                ..ReaderConfig::default()
            },
        )
        .expect("patched");
        assert!(patched.contains("# mine"), "{patched}");
        let read = parse(&patched);
        assert_eq!(read.reader.zoom, 150);
        assert_eq!(read.ui.density, crate::ui::Density::Compact);
    }
}
