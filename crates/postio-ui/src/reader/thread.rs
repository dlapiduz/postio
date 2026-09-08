//! A whole conversation as one document (ADR 0032, #1316).
//!
//! The reading pane builds a `WebView` per expanded message today, and
//! WebKitGTK runs a web process per *view* — so a thirty-message thread ends
//! up with thirty processes, and moving between them composites black while
//! a new one starts. Measured in `gtk_reader`: one reader is one process, two
//! readers are two, and rendering a second message into the same reader
//! reuses it.
//!
//! This composes the thread instead: one document, one view, one process,
//! whatever the thread's length.
//!
//! # Why several senders can share a document here
//!
//! Because they cannot contaminate each other. `postio_body::sanitize`
//! removes `<style>` tag-and-contents and strips every inline `style`
//! attribute, and parses to a tree rather than passing text through, so every
//! body already renders under Postio's stylesheet and nothing else. That
//! precondition was met for reasons that had nothing to do with this.
//!
//! # Expansion without script
//!
//! The reader runs with JavaScript off by construction (ADR 0003), so
//! expansion is `<details>`/`<summary>` — a disclosure widget in HTML itself,
//! keyboard-operable and announced by screen readers without a line of it.
//!
//! # Inline images name their message
//!
//! `postio-cid:` names a `Content-ID` and nothing else, which is exact while
//! one document is one message. Here it is not, so every body is sanitised
//! through [`postio_body::sanitize::sanitize_body_in`] with the message's own
//! scope, and the handler routes on it. Composing a body that was sanitised
//! unscoped would silently resolve one message's images against another's
//! parts, so [`Entry::body`] documents that it must be the scoped output.

use postio_body::sanitize::RemoteImages;

use super::document::{Sheet, contain_body, scroll_markers, wrap_document};

/// One message's place in a conversation document.
pub struct Entry<'a> {
    /// What `postio-cid:` references in [`body`](Self::body) are stamped with,
    /// and what the scheme handler routes on. Must be the same scope the body
    /// was sanitised under.
    ///
    /// Unreserved characters only — it is written into a URI without escaping,
    /// and a message id in decimal is what the frontend passes.
    pub scope: &'a str,
    /// Who it is from, as a person reads it.
    pub sender: &'a str,
    /// When, already formatted for the reader's locale.
    pub when: &'a str,
    /// The one line a collapsed message shows.
    pub preview: &'a str,
    /// Whether it starts open.
    pub expanded: bool,
    /// The message's body: already rendered *and already sanitised under
    /// [`scope`](Self::scope)*.
    pub body: &'a str,
}

/// The whole conversation, as one hardened document.
///
/// Each message is a `<details>` whose `<summary>` is its header, and whose
/// body is wrapped in `.postio-body` — #323's visible edge between what Postio
/// wrote and what arrived, which matters more here than in a single-message
/// document, not less: several senders share this page.
pub fn conversation_document(entries: &[Entry<'_>], remote: RemoteImages, sheet: Sheet) -> String {
    let mut content = String::new();
    content.push_str(r#"<div class="postio-thread">"#);
    for entry in entries {
        content.push_str(&entry_html(entry));
    }
    content.push_str("</div>");
    content.push_str(&scroll_markers());
    wrap_document(&content, remote, sheet)
}

fn entry_html(entry: &Entry<'_>) -> String {
    let open = if entry.expanded { " open" } else { "" };
    let scope = escape(entry.scope);
    let sender = escape(entry.sender);
    let when = escape(entry.when);
    let preview = escape(entry.preview);
    let body = contain_body(entry.body);
    // One `format!` per line would be tidier and is not available: a
    // `concat!` format string cannot capture from the surrounding scope, and
    // a raw string cannot be line-continued -- the backslash is a character
    // in it, which is what `the_markup_is_well_formed` caught.
    format!(
        "<details class=\"postio-message\" id=\"m-{scope}\"{open}>\
         <summary class=\"postio-message-head\">\
         <span class=\"postio-from\">{sender}</span>\
         <span class=\"postio-when\">{when}</span>\
         <span class=\"postio-preview\">{preview}</span>\
         </summary>{body}</details>"
    )
}

/// Postio's own chrome text, escaped.
///
/// A sender's name and their preview are sender-controlled strings going into
/// markup — the one place in this module where that is true, since the body
/// arrives already sanitised. `<` and `&` are what turn a display name into an
/// element; the quotes matter because these also land in attributes elsewhere.
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(character),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry<'a>(scope: &'a str, sender: &'a str, body: &'a str, expanded: bool) -> Entry<'a> {
        Entry {
            scope,
            sender,
            when: "09:14",
            preview: "the first line of it",
            expanded,
            body,
        }
    }

    #[test]
    fn a_thread_is_one_document_holding_every_message() {
        let entries = [
            entry("1", "Ada Lovelace", "<p>first</p>", true),
            entry("2", "Grace Hopper", "<p>second</p>", false),
            entry("3", "Ada Lovelace", "<p>third</p>", false),
        ];
        let document = conversation_document(&entries, RemoteImages::Blocked, Sheet::Theme);

        // One document. The whole point: one document is one view is one web
        // process, whatever the thread's length.
        assert_eq!(document.matches("<!DOCTYPE html>").count(), 1);
        assert_eq!(document.matches("<html").count(), 1);

        for body in ["<p>first</p>", "<p>second</p>", "<p>third</p>"] {
            assert_eq!(
                document.matches(body).count(),
                1,
                "{body} should appear exactly once in the thread"
            );
        }
        assert_eq!(document.matches("<details").count(), 3);
        assert!(document.contains("Ada Lovelace"));
        assert!(document.contains("Grace Hopper"));
    }

    #[test]
    fn the_markup_is_well_formed_and_carries_no_stray_escapes() {
        // The entry's own markup, not the whole document: the generated
        // stylesheet legitimately contains backslashes, and this is about
        // whether a raw string swallowed a line continuation.
        let markup = entry_html(&entry("1", "Ada", "<p>hi</p>", true));
        assert!(
            !markup.contains('\\'),
            "a backslash reached the markup: {markup}"
        );
        assert!(
            !markup.contains("  "),
            "stray indentation in markup: {markup}"
        );
        assert!(markup.contains("<summary class=\"postio-message-head\">"));
        assert!(markup.contains("</summary>"));
        assert_eq!(markup.matches("</details>").count(), 1);
    }

    #[test]
    fn only_the_expanded_messages_start_open() {
        let entries = [
            entry("1", "Ada", "<p>a</p>", false),
            entry("2", "Grace", "<p>b</p>", true),
        ];
        let document = conversation_document(&entries, RemoteImages::Blocked, Sheet::Theme);
        assert_eq!(
            document.matches(" open>").count(),
            1,
            "exactly one message should start open: {document}"
        );
        assert!(document.contains(r#"id="m-2" open>"#));
        assert!(document.contains(r#"id="m-1">"#));
    }

    /// #323's edge, which matters more here than in a single-message document:
    /// several senders share this page, so each one's content needs its own
    /// visible boundary.
    #[test]
    fn every_body_keeps_its_own_bounded_surface() {
        let entries = [
            entry("1", "Ada", "<p>a</p>", true),
            entry("2", "Grace", "<p>b</p>", true),
        ];
        let document = conversation_document(&entries, RemoteImages::Blocked, Sheet::Theme);
        assert_eq!(document.matches(r#"<div class="postio-body">"#).count(), 2);
    }

    /// A display name is sender-controlled text going into markup. The body
    /// arrives sanitised; this does not.
    #[test]
    fn a_sender_cannot_write_markup_through_their_name() {
        let entries = [entry("1", "<script>alert(1)</script>", "<p>a</p>", true)];
        let document = conversation_document(&entries, RemoteImages::Blocked, Sheet::Theme);
        assert!(
            !document.contains("<script>"),
            "a display name became an element: {document}"
        );
        assert!(document.contains("&lt;script&gt;"));
    }

    /// The document is still the hardened one: same CSP, same no-script
    /// posture as a single message's.
    #[test]
    fn the_thread_document_is_as_hardened_as_a_single_messages() {
        let entries = [entry("1", "Ada", "<p>a</p>", true)];
        let document = conversation_document(&entries, RemoteImages::Blocked, Sheet::Theme);
        assert!(document.contains("Content-Security-Policy"));
        assert!(document.contains("img-src postio-cid: data:;"));
        assert!(document.contains("base-uri 'none'"));
    }
}
