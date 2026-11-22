//! `[compose]`: where a signature goes, and which editor a draft hands off to. Top- and
//! bottom-posting
//! are both in wide use, so placement is the user's. New mail has no quote, so there is no
//! `signature_on_new`: a key that can never change the output only invites a bug report.

use serde::{Deserialize, Serialize};

use crate::Extras;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignaturePlacement {
    #[default]
    AboveQuote,
    BelowQuote,
}

impl SignaturePlacement {
    pub const fn as_str(self) -> &'static str {
        match self {
            SignaturePlacement::AboveQuote => "above_quote",
            SignaturePlacement::BelowQuote => "below_quote",
        }
    }
}

impl std::fmt::Display for SignaturePlacement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ComposeConfig {
    #[serde(default)]
    pub signature_on_reply: SignaturePlacement,
    /// Separate because a reply answers a fragment and a forward hands the whole message on; people
    /// who
    /// bottom-post replies often still top-post forwards.
    #[serde(default)]
    pub signature_on_forward: SignaturePlacement,
    /// Empty means the platform's own idea of what opens a text file. Not `$EDITOR`: an application
    /// launched from Finder has no shell environment (#1288). A name, not a command line.
    #[serde(default)]
    pub editor: String,
    #[serde(flatten)]
    pub extra: Extras,
}

/// The Composing pane's write path (#1179). `[compose]` is regenerated whole, so a comment on its
/// own
/// header does not survive; every other section and comment is untouched.
pub fn patch_compose(text: &str, compose: &ComposeConfig) -> crate::Result<String> {
    crate::splice::replace_section(text, "compose", &ComposeOnly { compose })
}

#[derive(Serialize)]
struct ComposeOnly<'a> {
    compose: &'a ComposeConfig,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;

    #[test]
    fn patching_compose_leaves_every_other_section_and_its_comments_alone() {
        let original = "\
# a hand-written comment nobody wants to lose
[ui]
density = \"compact\"

[compose]
signature_on_reply = \"above_quote\"
signature_on_forward = \"above_quote\"
";
        let compose = ComposeConfig {
            signature_on_reply: SignaturePlacement::BelowQuote,
            ..ComposeConfig::default()
        };

        let patched = patch_compose(original, &compose).expect("patches");

        assert!(
            patched.contains("# a hand-written comment nobody wants to lose"),
            "the comment was dropped: {patched}"
        );
        assert!(
            patched.contains("density = \"compact\""),
            "an untouched section was rewritten: {patched}"
        );
        let read = Config::from_toml_str(&patched).expect("parses back");
        assert_eq!(
            read.compose.signature_on_reply,
            SignaturePlacement::BelowQuote
        );
        assert_eq!(
            read.compose.signature_on_forward,
            SignaturePlacement::AboveQuote,
            "the setting that did not change must not have moved"
        );
    }

    #[test]
    fn patching_compose_writes_the_table_into_a_file_that_had_none() {
        let compose = ComposeConfig {
            signature_on_forward: SignaturePlacement::BelowQuote,
            ..ComposeConfig::default()
        };

        let patched = patch_compose("[ui]\ndensity = \"airy\"\n", &compose).expect("patches");

        let read = Config::from_toml_str(&patched).expect("parses back");
        assert_eq!(
            read.compose.signature_on_forward,
            SignaturePlacement::BelowQuote,
            "a first write has no table to rewrite and must make one: {patched}"
        );
    }

    #[test]
    fn the_default_is_where_most_mail_puts_it() {
        let compose = ComposeConfig::default();
        assert_eq!(compose.signature_on_reply, SignaturePlacement::AboveQuote);
        assert_eq!(compose.signature_on_forward, SignaturePlacement::AboveQuote);
    }

    #[test]
    fn each_act_is_configured_on_its_own() {
        let config = Config::from_toml_str(
            "[compose]\nsignature_on_reply = \"below_quote\"\nsignature_on_forward = \"above_quote\"\n",
        )
        .expect("valid");
        assert_eq!(
            config.compose.signature_on_reply,
            SignaturePlacement::BelowQuote
        );
        assert_eq!(
            config.compose.signature_on_forward,
            SignaturePlacement::AboveQuote
        );
    }

    #[test]
    fn an_unknown_key_in_the_section_survives_a_round_trip() {
        // People hand-edit this file, and a key written by a newer Postio must
        // not be deleted by an older one saving over it.
        let config = Config::from_toml_str(
            "[compose]\nsignature_on_reply = \"below_quote\"\ntop_post = true\n",
        )
        .expect("valid");

        assert!(config.compose.extra.contains_key("top_post"));
        let written = toml::to_string(&config).expect("serializes");
        assert!(written.contains("top_post"), "{written}");
    }
}
