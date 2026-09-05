//! `[compose]` — where a signature goes when there is a quote under it.
//!
//! ```toml
//! [compose]
//! signature_on_reply = "above_quote"    # above_quote | below_quote
//! signature_on_forward = "above_quote"
//! ```
//!
//! # Why this is a setting and not a house style
//!
//! Both conventions are in wide use and neither is wrong. Bottom-posting puts
//! the signature under everything, which is what the Usenet convention and
//! most mailing lists expect. Top-posting puts it under what you just wrote
//! and above the quoted message, which is what almost every graphical client
//! now does and what most correspondents will expect to see. Postio holds no
//! opinion; it holds the user's.
//!
//! New mail has no quote, so placement cannot mean anything there — both
//! settings agree, and the signature goes at the end. That is why there is no
//! `signature_on_new`: a key whose value can never change the output is a key
//! that only invites a bug report.

use serde::{Deserialize, Serialize};

use crate::Extras;

/// Where a signature sits relative to quoted text.
///
/// Mirrors `postio_body::Placement`, spelled here because this crate is the
/// *schema* and must not depend on the body crate to describe a setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignaturePlacement {
    /// Under what was written and above the quote — what a top-posting reply
    /// looks like, and the default because it is what most mail looks like.
    #[default]
    AboveQuote,
    /// Under everything, quote included.
    BelowQuote,
}

impl SignaturePlacement {
    /// The spelling `config.toml` uses.
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

/// The `[compose]` section.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ComposeConfig {
    /// Where the signature goes on a reply.
    #[serde(default)]
    pub signature_on_reply: SignaturePlacement,
    /// Where the signature goes on a forward.
    ///
    /// Separate from the reply setting because the two are different acts: a
    /// reply answers a fragment, a forward hands the whole message on, and
    /// people who bottom-post replies often still top-post forwards.
    #[serde(default)]
    pub signature_on_forward: SignaturePlacement,
    /// Keys this version of Postio does not know, preserved verbatim.
    #[serde(flatten)]
    pub extra: Extras,
}

/// Rewrites `text`'s `[compose]` table to match `compose`, leaving every
/// other section — and any comment attached to one — untouched.
///
/// The Composing pane's write path (#1179), and the same bargain
/// [`crate::patch_ui`] and [`crate::patch_filters`] already make for the
/// same reason: `Config::to_toml_string` reserializes the whole file and
/// would reorder every key and drop every comment in it, not only in the
/// table the pane owns. `[compose]` itself is regenerated whole, so a
/// comment attached to `[compose]`'s own header does not survive — two
/// settings chosen from a segmented control are not the kind of TOML
/// anyone hand-annotates.
pub fn patch_compose(text: &str, compose: &ComposeConfig) -> crate::Result<String> {
    let mut doc = text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|err| crate::ConfigError::parse(None, &err))?;
    doc.as_table_mut().remove("compose");

    let fragment = toml::to_string(&ComposeOnly { compose })
        .map_err(|err| crate::ConfigError::Serialize(err.to_string()))?;
    let fragment_doc = fragment
        .parse::<toml_edit::DocumentMut>()
        .map_err(|err| crate::ConfigError::parse(None, &err))?;
    if let Some(item) = fragment_doc.as_table().get("compose") {
        doc.as_table_mut().insert("compose", item.clone());
    }
    Ok(doc.to_string())
}

/// Serializes as just a `[compose]` table — [`patch_compose`]'s bridge from
/// `toml`'s serde output to a fragment `toml_edit` can splice in.
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
