//! `[ui]` — appearance and interaction preferences.

use serde::{Deserialize, Serialize};

use crate::{ConfigError, Extras, Result};

/// Message-list row height. The PLATE design is airy (40px rows); the other two
/// tighten the same row anatomy rather than changing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Density {
    /// 40px rows — the default, matching the chosen design direction.
    #[default]
    Airy,
    /// Middle setting.
    Comfortable,
    /// Tightest rows, most messages on screen.
    Compact,
}

/// Light/dark preference. `System` follows the desktop's color scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    /// Follow the GNOME light/dark setting.
    #[default]
    System,
    /// Always light.
    Light,
    /// Always dark.
    Dark,
}

/// The `[ui]` section.
///
/// ```toml
/// [ui]
/// density = "airy"          # airy | comfortable | compact
/// theme = "system"          # system | light | dark
/// show_hover_actions = true # mouse parity: reveal row actions on hover
/// show_key_hints = true     # the focused row's own keyboard hints
/// sender_avatars = true     # initials chip per row, from canvas 1b
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiConfig {
    /// Message-list row height.
    #[serde(default)]
    pub density: Density,
    /// Light/dark preference.
    #[serde(default)]
    pub theme: Theme,
    /// Show per-row actions when the pointer is over a row.
    #[serde(default = "crate::yes")]
    pub show_hover_actions: bool,
    /// Show the focused row's key hints (`e reply`, `a archive`).
    /// Off leaves every binding in force -- this only stops the row from
    /// naming them, for someone who already knows the keyboard (#422).
    #[serde(default = "crate::yes")]
    pub show_key_hints: bool,
    /// Show each row's sender-initials chip, per canvas 1b's row anatomy.
    #[serde(default = "crate::yes")]
    pub sender_avatars: bool,
    /// Keys this version of Postio does not know, preserved verbatim.
    #[serde(flatten)]
    pub extra: Extras,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            density: Density::default(),
            theme: Theme::default(),
            show_hover_actions: true,
            show_key_hints: true,
            sender_avatars: true,
            extra: Extras::new(),
        }
    }
}

/// Rewrites `text`'s `[ui]` table to match `ui`, leaving every *other*
/// section — and any comment attached to one — untouched. The structured
/// Appearance pane's write path (#873): the same `patch_filters` (#869)
/// already established for exactly this reason — `Config::to_toml_string`
/// reserializes the whole file and would drop a hand-written comment
/// anywhere in it, not only in `[ui]`.
///
/// `[ui]` itself is regenerated whole on every call, the same tradeoff
/// `patch_filters` makes for a changed entry: a comment attached directly to
/// `[ui]` — its own header, or one of its own keys — does not survive,
/// because there is no per-field diff here, only "this table, freshly
/// written". Six settings a person picks from a dropdown or flips a switch
/// on are not the kind of TOML anyone hand-annotates the way a whole section
/// might be, so that is the deliberate half of the promise, not an
/// oversight.
pub fn patch_ui(text: &str, ui: &UiConfig) -> Result<String> {
    let mut doc = text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|err| ConfigError::parse(None, &err))?;
    doc.as_table_mut().remove("ui");

    let fragment =
        toml::to_string(&UiOnly { ui }).map_err(|err| ConfigError::Serialize(err.to_string()))?;
    let fragment_doc = fragment
        .parse::<toml_edit::DocumentMut>()
        .map_err(|err| ConfigError::parse(None, &err))?;
    if let Some(item) = fragment_doc.as_table().get("ui") {
        doc.as_table_mut().insert("ui", item.clone());
    }
    Ok(doc.to_string())
}

/// Serializes as just a `[ui]` table, with no other section — [`patch_ui`]'s
/// bridge from `toml`'s serde-derived output to a fragment `toml_edit` can
/// splice in.
#[derive(Serialize)]
struct UiOnly<'a> {
    ui: &'a UiConfig,
}

#[cfg(test)]
mod tests {
    use crate::Config;

    use super::*;

    #[test]
    fn sender_avatars_round_trips_through_the_config_file() {
        let mut config = Config::default();
        config.ui.sender_avatars = false;

        let written = toml::to_string(&config).expect("serializes");
        let read = Config::from_toml_str(&written).expect("parses back");

        assert!(
            !read.ui.sender_avatars,
            "an explicit false must survive the round trip: {written}"
        );
    }

    #[test]
    fn sender_avatars_defaults_to_true_when_absent_from_an_existing_file() {
        // Back-compat: a config.toml written before this field existed has
        // no `sender_avatars` key at all, and must still parse as "on" --
        // the same default a brand new file gets.
        let config = Config::from_toml_str("[ui]\ntheme = \"dark\"\n").expect("parses");
        assert!(config.ui.sender_avatars);
    }

    // -- Acceptance: the Appearance pane patches [ui] only (#873) -----------

    #[test]
    fn patch_ui_rewrites_only_the_ui_table_leaving_everything_else_verbatim() {
        let original = "\
# a hand-written comment nobody wants to lose
[filters.old]
query = \"is:unread\"
pinned = true

[ui]
theme = \"system\" # inline comment, also not to be lost
density = \"airy\"
";
        let mut config = Config::from_toml_str(original).expect("parses");
        config.ui.theme = super::Theme::Dark;
        config.ui.density = super::Density::Compact;

        let patched = patch_ui(original, &config.ui).expect("patches");

        assert!(
            patched.contains("# a hand-written comment nobody wants to lose"),
            "a comment outside [ui] must survive verbatim: {patched}"
        );
        assert!(
            patched.contains("[filters.old]") && patched.contains("query = \"is:unread\""),
            "an unrelated section must survive untouched: {patched}"
        );

        let reparsed = Config::from_toml_str(&patched).expect("still parses");
        assert_eq!(reparsed.ui.theme, super::Theme::Dark);
        assert_eq!(reparsed.ui.density, super::Density::Compact);
        assert_eq!(reparsed.filters["old"].query, "is:unread");
    }

    #[test]
    fn patch_ui_adds_the_table_when_it_did_not_exist_before() {
        let original = "[sync]\ncheck_for_mail = \"poll\"\n";
        let config = Config::from_toml_str(original).expect("parses");

        let patched = patch_ui(original, &config.ui).expect("patches");
        assert!(patched.contains("[ui]"));
        assert!(patched.contains("check_for_mail = \"poll\""));
    }
}
