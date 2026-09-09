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

/// The conversation chrome, appended only to a conversation document.
const THREAD_CSS: &str = include_str!("../../data/thread.css");

/// The scheme a `Show` link uses, intercepted by the frontend rather than
/// followed.
///
/// Its own scheme rather than a fragment or a query: the reader hands every
/// navigation that leaves the pane to the system browser, so a consent verb
/// has to be distinguishable from a link the sender wrote *before* that
/// happens. The sanitizer never emits this scheme from a sender's markup, so
/// a message cannot forge one.
pub const ALLOW_SCHEME: &str = "postio-allow";

/// The scheme a per-message `Reply` uses.
///
/// A verb of its own rather than a parameter on one scheme, so the frontend
/// can tell them apart before deciding what to do — and so an unrecognised
/// verb is refused rather than mapped to the nearest thing.
pub const REPLY_SCHEME: &str = "postio-reply";

/// The scheme a per-message `Forward` uses. See [`REPLY_SCHEME`].
pub const FORWARD_SCHEME: &str = "postio-forward";

/// The element id a message carries, so a pane can scroll to it.
///
/// One function rather than two format strings, because the id and the
/// fragment that navigates to it have to agree and nothing would say so if
/// they stopped: a `#m-3` aimed at a document with no `m-3` scrolls nowhere,
/// silently, and looks exactly like a message that happens to be on screen
/// already (#1386).
///
/// Takes the scope raw and escapes it here, for the same reason: an escape
/// applied on one side and not the other is the same silent mismatch wearing
/// a different hat.
pub fn message_anchor(scope: &str) -> String {
    format!("m-{}", escape(scope))
}

/// One message's place in a conversation document.
///
/// A single message is a thread of one, expanded — the pane renders both
/// through this, so there is one set of chrome rather than two that can drift.
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
    /// Their address, shown beside the name on an open message (canvas 17).
    pub address: &'a str,
    /// When, already formatted for the reader's locale.
    pub when: &'a str,
    /// The one line a collapsed message shows.
    pub preview: &'a str,
    /// Whether it starts open.
    pub expanded: bool,
    /// Whether this is the newest message in the thread — canvas 17's
    /// `latest` badge. Always false for a thread of one, where there is
    /// nothing for it to distinguish.
    pub latest: bool,
    /// How many remote references this message had stripped, so the document
    /// can say so where the decision was made rather than once for the page.
    pub blocked: u32,
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
    // Ours, not a sender's: `sanitize` strips `<style>` tag-and-contents from
    // everything that arrives, which is exactly what makes it safe for
    // several senders to share this page. `style-src 'unsafe-inline'` is
    // already the reader's policy, so this needs no widening of it.
    content.push_str("<style>");
    content.push_str(THREAD_CSS);
    content.push_str("</style>");
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
    let sender = escape(entry.sender);
    let address = escape(entry.address);
    let when = escape(entry.when);
    let preview = escape(entry.preview);
    let latest = if entry.latest {
        r#"<span class="postio-latest">latest</span>"#.to_string()
    } else {
        String::new()
    };
    // Per message, because the decision it reports is per sender — and since
    // #1353 the document honours that: a sender the user has allowed keeps
    // their images while the rest of the thread does not.
    //
    // The link is the consent. A notice that only *reports* a decision the
    // user cannot make is not a privacy feature, it is a dead end: blocking
    // without a way to unblock is the feature missing its other half. With
    // JavaScript off, a verb inside the document is a navigation, which
    // `postio_gtk::reader::view` intercepts by scheme.
    let blocked = match entry.blocked {
        0 => String::new(),
        count => {
            let what = if count == 1 {
                "1 remote image blocked".to_owned()
            } else {
                format!("{count} remote images blocked")
            };
            format!(
                r#"<div class="postio-blocked">{what} <a class="postio-blocked-show" href="{ALLOW_SCHEME}:{}">Show</a></div>"#,
                entry.scope
            )
        }
    };
    // Spec FR-009: the header's bar is fixed to the latest message, so
    // without these there is no way to reply to an older one at all — the
    // mistake the fixed bar exists to prevent, arriving from the other side.
    //
    // Out of flow, which is how the brief's two requirements — "reserve no
    // space when idle" and "nothing shifts when they appear" — are both true
    // at once rather than contradictory. `thread.css` positions them against
    // the header row.
    //
    // Named for the message rather than the verb: an icon-only control that a
    // screen reader announces as "button" is a control that is not reachable,
    // and in a stack of six the verb alone does not say which one it means.
    let actions = format!(
        "<span class=\"postio-message-actions\">\
         <a class=\"postio-message-action\" href=\"{REPLY_SCHEME}:{scope}\" \
         aria-label=\"Reply to {sender}\" title=\"Reply to {sender}\">Reply</a>\
         <a class=\"postio-message-action\" href=\"{FORWARD_SCHEME}:{scope}\" \
         aria-label=\"Forward {sender}&#39;s message\" \
         title=\"Forward {sender}&#39;s message\">Forward</a>\
         </span>",
        // Escaped. It was not, and a scope carrying a quote closed the
        // `href` and put whatever followed it into the tag as an attribute
        // -- found by `an_anchor_cannot_break_out_of_the_attribute_it_sits_in`
        // while proving the *anchor* was safe. Not reachable today, because a
        // scope is a message's own database id and `enable_javascript_markup`
        // is off besides; a link that builds an attribute out of an
        // unescaped value is still a link waiting for the day one of those
        // stops being true.
        scope = escape(entry.scope),
        sender = sender,
    );
    let body = contain_body(entry.body);
    let anchor = message_anchor(entry.scope);
    // A normal string, not a raw one: a raw string cannot be line-continued,
    // and the backslash would be a character in the markup — which is what
    // `the_markup_is_well_formed` caught.
    format!(
        "<details class=\"postio-message\" id=\"{anchor}\"{open}>\
         <summary class=\"postio-message-head\">\
         <span class=\"postio-from\">{sender}</span>\
         <span class=\"postio-address\">{address}</span>\
         <span class=\"postio-preview\">{preview}</span>\
         {latest}\
         <span class=\"postio-when\">{when}</span>\
         {actions}\
         </summary>{blocked}{body}</details>"
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
    #[test]
    fn a_blocked_message_offers_a_way_to_show_its_images() {
        // A notice that only reports a decision the user cannot make is not a
        // privacy feature. Blocking without a way to unblock is the feature
        // missing its other half (#1353).
        let mut entry = entry("1", "Ada", "<p>hi</p>", true);
        entry.blocked = 6;
        let document = conversation_document(
            &[entry],
            postio_body::RemoteImages::Blocked,
            crate::reader::document::Sheet::Theme,
        );
        assert!(document.contains("6 remote images blocked"));
        assert!(
            document.contains(&format!("{ALLOW_SCHEME}:")),
            "the notice reports the block and offers no way to act on it"
        );
    }

    #[test]
    fn the_show_link_names_the_message_it_belongs_to() {
        // Per sender, not per thread: allowing one correspondent must not
        // carry the rest with them, so the verb has to say which message it
        // is speaking for.
        let mut first = entry("11", "Ada", "<p>one</p>", true);
        first.blocked = 1;
        let mut second = entry("22", "Grace", "<p>two</p>", true);
        second.blocked = 1;
        let document = conversation_document(
            &[first, second],
            postio_body::RemoteImages::Blocked,
            crate::reader::document::Sheet::Theme,
        );
        assert!(document.contains(&format!("{ALLOW_SCHEME}:11")));
        assert!(document.contains(&format!("{ALLOW_SCHEME}:22")));
    }

    #[test]
    fn a_message_with_nothing_held_back_offers_nothing() {
        let document = conversation_document(
            &[entry("1", "Ada", "<p>hi</p>", true)],
            postio_body::RemoteImages::Blocked,
            crate::reader::document::Sheet::Theme,
        );
        assert!(
            !document.contains(ALLOW_SCHEME),
            "a message that had nothing blocked is offering consent for nothing"
        );
    }

    use super::*;

    fn entry<'a>(scope: &'a str, sender: &'a str, body: &'a str, expanded: bool) -> Entry<'a> {
        Entry {
            scope,
            sender,
            address: "ada@example.com",
            when: "09:14",
            preview: "the first line of it",
            expanded,
            latest: false,
            blocked: 0,
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

    /// A single message is a thread of one, so the pane has one set of
    /// chrome rather than two that can drift (the maintainer asked for this
    /// directly, 2026-09-07).
    #[test]
    fn one_message_is_a_thread_of_one() {
        let mut only = entry("7", "Marketside", "<p>out for delivery</p>", true);
        only.address = "orders@marketside.example";
        let document = conversation_document(&[only], RemoteImages::Blocked, Sheet::Theme);

        assert_eq!(document.matches("<details").count(), 1);
        assert!(
            document.contains(r#"id="m-7" open>"#),
            "the one message has to be open, or a single message opens closed"
        );
        assert!(document.contains("orders@marketside.example"));
        assert!(document.contains("<p>out for delivery</p>"));
        assert!(
            // The markup, not the class name: the stylesheet names it too.
            !document.contains(r#"<span class="postio-latest">"#),
            "a thread of one has nothing for `latest` to distinguish"
        );
    }

    /// Canvas 17 states what was held back inside the message it was held
    /// back for, because the decision is per sender.
    #[test]
    fn each_message_says_what_was_held_back_for_it() {
        let mut first = entry("1", "Ada", "<p>a</p>", true);
        first.blocked = 6;
        let mut second = entry("2", "Grace", "<p>b</p>", true);
        second.blocked = 1;
        let third = entry("3", "Hedy", "<p>c</p>", true);
        let document =
            conversation_document(&[first, second, third], RemoteImages::Blocked, Sheet::Theme);

        assert!(document.contains("6 remote images blocked"));
        assert!(document.contains("1 remote image blocked"));
        assert_eq!(
            document.matches(r#"<div class="postio-blocked">"#).count(),
            2,
            "a message with nothing held back should say nothing"
        );
    }

    /// The newest message wears the badge; nothing else does.
    #[test]
    fn only_the_newest_message_is_marked_latest() {
        let first = entry("1", "Ada", "<p>a</p>", false);
        let mut second = entry("2", "Grace", "<p>b</p>", true);
        second.latest = true;
        let document = conversation_document(&[first, second], RemoteImages::Blocked, Sheet::Theme);
        assert_eq!(
            document.matches(r#"<span class="postio-latest">"#).count(),
            1
        );
    }

    /// The chrome is dressed from the token layer, never from literals: #296
    /// says a colour has one source.
    #[test]
    fn the_thread_chrome_reads_its_colours_off_the_tokens() {
        let document = conversation_document(
            &[entry("1", "Ada", "<p>a</p>", true)],
            RemoteImages::Blocked,
            Sheet::Theme,
        );
        assert!(
            document.contains(".postio-message"),
            "the thread sheet is missing"
        );
        for token in ["var(--r-hairline)", "var(--r-accent)", "var(--r-dim)"] {
            assert!(
                document.contains(token),
                "the thread chrome should dress itself from {token}"
            );
        }
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

    #[test]
    fn an_anchor_cannot_break_out_of_the_attribute_it_sits_in() {
        // The document writes this into `id="…"` and a pane navigates to it.
        // A scope carrying a quote would close the attribute and everything
        // after it would be markup the sender wrote.
        //
        // Asserting the *pair* -- that the document contains what the helper
        // produces -- would prove nothing: both sides call this function, so
        // they agree by construction and the assertion cannot fail. This is
        // the part construction does not give for free.
        //
        // Against `entry_html` rather than a whole document, deliberately.
        // `conversation_document` counts itself in `reader::cost`, whose own
        // test asserts an exact delta over a *global* counter -- so a test
        // assembling a document here races it under libtest's thread pool
        // and fails it (#1390). One message is all this needs.
        let hostile = r#"1" onmouseover="steal()"#;
        let anchor = message_anchor(hostile);
        assert!(
            !anchor.contains('"'),
            "an anchor with a bare quote in it escapes its attribute: {anchor}"
        );

        let html = entry_html(&entry(hostile, "Ada", "<p>one</p>", true));
        assert!(
            !html.contains("onmouseover=\"steal()"),
            "the sender's attribute survived into the markup: {html}"
        );
        assert!(
            html.contains(&format!("id=\"{anchor}\"")),
            "and the message still has an anchor to scroll to: {html}"
        );
    }
}
