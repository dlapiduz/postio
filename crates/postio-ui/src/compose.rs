//! What the composer says about itself.
//!
//! Two frontends write mail and both have a footer that names what will
//! leave. The words are here so they cannot differ, for the same reason the
//! reader's document assembly is shared: a claim about what Postio puts on
//! the wire is one it should be willing to make identically everywhere.

/// The MIME shape an outgoing message will have: `html + text/plain`, or
/// `text/plain, format=flowed`.
///
/// **The plain part is not optional.** Rich mail sends `text/html` *and* a
/// `text/plain` alternative, always — a recipient reading in a terminal, a
/// screen reader, or a client that refuses HTML gets the message rather than
/// an apology. Plain mail is wrapped at 72 columns and flowed (RFC 3676), so
/// it reads correctly whether the receiving client rewraps it or not.
pub fn outgoing_shape(rich: bool) -> String {
    if rich {
        "html + text/plain".to_owned()
    } else {
        "text/plain, format=flowed".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rich_always_carries_a_plain_alternative() {
        // The half that is easy to drop and expensive to notice: it is
        // invisible to the sender and it is the whole message to some
        // recipients.
        assert!(outgoing_shape(true).contains("text/plain"));
    }

    #[test]
    fn plain_says_it_is_flowed_rather_than_just_plain() {
        // `format=flowed` is the difference between a paragraph that rewraps
        // in a narrow window and one that arrives with a ragged 72-column
        // edge, and the footer is where a person can see which they chose.
        assert_eq!(outgoing_shape(false), "text/plain, format=flowed");
    }
}

/// The composer's editing bridge — the one script Postio runs in a web view.
///
/// Both frontends put a `contenteditable` document inside a web view and
/// both need the same three things from it: the paragraph separator and
/// `styleWithCSS` settings that pin the dialect [`postio_body::parse`] reads
/// back, an edit channel carrying `innerHTML`, and a reflection channel
/// saying what formatting is in force at the caret.
///
/// One copy, because a second one is a second dialect. The GTK reader's
/// `gtk_editable_dialect.rs` proves the surface emits `<p>` paragraphs and
/// element-form bold/italic; a macOS surface running a *different* script
/// would emit `<div>`s and `<span style>`s, `parse` would narrow them to
/// something else, and the two composers would disagree about what the same
/// keystrokes wrote — invisibly, because both would still round-trip
/// through a `Document`.
///
/// `window.webkit.messageHandlers` is the same API on WebKitGTK and on
/// `WKWebView`, which is why one file can serve both without a shim.
pub const EDITOR_SCRIPT: &str = include_str!("../data/editor.js");

#[cfg(test)]
mod editor_script_tests {
    use super::EDITOR_SCRIPT;

    /// `postio-gtk` still `include_str!`s its own copy, because this branch
    /// is worked from a Mac and `issue-land.sh` will not land a crate whose
    /// gates cannot run there. Until a Linux session points it here, the two
    /// files are pinned to each other: this fails the moment either is
    /// edited alone, which is the only thing that makes a temporary
    /// duplicate safe.
    #[test]
    fn the_gtk_copy_of_the_bridge_has_not_drifted_from_this_one() {
        let gtk =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../postio-gtk/data/editor.js");
        let theirs = std::fs::read_to_string(&gtk)
            .unwrap_or_else(|error| panic!("reading {}: {error}", gtk.display()));
        assert_eq!(
            theirs, EDITOR_SCRIPT,
            "the two copies of the editing bridge have diverged, which is two \
             dialects: make them equal, or finish the move and delete the GTK one"
        );
    }
}

/// The script that applies one of the composer's marks to the selection.
///
/// The mapping from a registry command to what a `contenteditable` document
/// does about it is a *decision* — `quote_block` is the interesting one,
/// because `formatBlock` does not toggle and the toggle is Postio's. Writing
/// it once here rather than once per frontend is the same argument as
/// [`EDITOR_SCRIPT`]: two hosts running different scripts for `bold` would
/// produce different markup, `postio_body::parse` would narrow it
/// differently, and the two composers would disagree about what the same
/// button did.
///
/// `None` for a command that is not one of the marks, which is how a caller
/// tells "this button does something else" from "this button is broken".
///
/// Every script ends by dispatching `input`, because `execCommand` does not
/// always raise one (bold yes, `insertUnorderedList` no) and the host's
/// record is only updated by that event. A duplicate event is absorbed as a
/// no-change; a missing one loses the edit.
pub fn mark_script(command: &str) -> Option<String> {
    let body = match command {
        "bold" => "document.execCommand('bold');",
        "italic" => "document.execCommand('italic');",
        "bullet_list" => "document.execCommand('insertUnorderedList');",
        "numbered_list" => "document.execCommand('insertOrderedList');",
        // `formatBlock` toggles nothing on its own; the toggle is ours.
        "quote_block" => {
            "if (document.queryCommandValue('formatBlock') === 'blockquote') { \
                 document.execCommand('formatBlock', false, 'p'); \
             } else { \
                 document.execCommand('formatBlock', false, 'blockquote'); \
             }"
        }
        _ => return None,
    };
    Some(format!(
        "{body} document.dispatchEvent(new Event('input'));"
    ))
}

/// The script that turns the selection into a link to `href`.
///
/// `None` when `href` is not something a message may link to. The gate is
/// the canonical subset's — `postio_body::Href` refuses anything but http,
/// https and mailto — and it is applied *here*, before the document is
/// touched, so the composer can say so rather than have the link silently
/// disappear at the next parse.
pub fn link_script(href: &str) -> Option<String> {
    postio_body::Href::parse(href)?;
    // Single quotes and backslashes escaped: the href has been through
    // `Href::parse`, which refuses control characters, but it is still
    // somebody's input going into a script literal.
    let escaped = href.replace('\\', "\\\\").replace('\'', "\\'");
    Some(format!(
        "document.execCommand('createLink', false, '{escaped}'); \
         document.dispatchEvent(new Event('input'));"
    ))
}

#[cfg(test)]
mod mark_tests {
    use super::*;

    #[test]
    fn every_mark_the_bar_offers_has_a_script() {
        // The bar is built from the registry ids; a mark with no script is a
        // button that does nothing, which is what #1271 reported.
        for command in [
            "bold",
            "italic",
            "bullet_list",
            "numbered_list",
            "quote_block",
        ] {
            assert!(
                mark_script(command).is_some(),
                "{command} has no script, so its button is decoration"
            );
        }
    }

    #[test]
    fn a_command_that_is_not_a_mark_has_none() {
        // So a caller can tell "does something else" from "broken".
        assert_eq!(mark_script("archive"), None);
        assert_eq!(mark_script("insert_link"), None, "link needs an href");
    }

    #[test]
    fn every_script_raises_the_event_the_host_records_on() {
        // `execCommand` does not always raise `input` -- bold does,
        // `insertUnorderedList` does not -- and the host's record is only
        // updated by that event. A mark that skipped it would apply on
        // screen and be lost on save.
        for command in ["bold", "bullet_list", "quote_block"] {
            let script = mark_script(command).expect("a script");
            assert!(
                script.contains("new Event('input')"),
                "{command} applies without telling the host: {script}"
            );
        }
    }

    #[test]
    fn quoting_toggles_rather_than_only_applying() {
        // `formatBlock` has no toggle of its own, so pressing Quote twice
        // would otherwise nest rather than undo.
        let script = mark_script("quote_block").expect("a script");
        assert!(script.contains("queryCommandValue"), "no toggle: {script}");
        assert!(
            script.contains("'p'"),
            "nothing to toggle back to: {script}"
        );
    }

    #[test]
    fn a_link_to_somewhere_a_message_may_point_gets_a_script() {
        assert!(link_script("https://example.com").is_some());
        assert!(link_script("mailto:ada@example.com").is_some());
    }

    #[test]
    fn a_link_the_subset_refuses_is_refused_here_rather_than_at_the_next_parse() {
        // Refused up front so the composer can say so. Left to the parse, the
        // link would be created, look right, and vanish on save.
        assert_eq!(link_script("javascript:alert(1)"), None);
        assert_eq!(link_script("file:///etc/passwd"), None);
        assert_eq!(
            link_script("example.com"),
            None,
            "relative means nothing in mail"
        );
    }

    #[test]
    fn a_quote_in_an_href_cannot_close_the_script_literal() {
        // `Href::parse` refuses control characters, not quotes.
        let script = link_script("https://example.com/a'b").expect("a script");
        assert!(script.contains("a\\'b"), "unescaped quote: {script}");
    }
}
