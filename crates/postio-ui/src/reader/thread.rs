//! A whole conversation as one document, so a thread is one WebView process (ADR 0032, #1316).
//! Bodies are sanitised under the message's own scope, so `postio-cid:` images resolve against the
//! right message.
//! Expansion is `<details>`, since the reader runs without script (ADR 0003).

use postio_body::sanitize::RemoteImages;

use super::document::{
    Sheet, contain_body_in, number_quote_folds, scroll_markers, senders_stylesheet, wrap_document,
};

const THREAD_CSS: &str = include_str!("../../data/thread.css");

/// Its own scheme so the frontend can tell a consent verb from a sender's link before it goes to
/// the browser; the sanitizer never emits it.
pub const ALLOW_SCHEME: &str = "postio-allow";

/// A verb per scheme, so an unrecognised verb is refused rather than mapped.
pub const REPLY_SCHEME: &str = "postio-reply";

pub const FORWARD_SCHEME: &str = "postio-forward";

/// A draft resumes via `CommandId::OpenMessage`, not as a reply (#1212).
pub const CONTINUE_SCHEME: &str = "postio-continue";

/// A class, not a drawn mark: the stylesheet is where it belongs (#1444).
pub const MINE_CLASS: &str = "postio-mine";

/// With script off a verb is a navigation, intercepted by scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageVerb {
    Reply,
    Forward,
    Continue,
    Allow,
}

/// Matched on the scheme alone: a sender controls link text and class, not these schemes.
/// An unknown verb, or one naming no message, is not a verb.
pub fn verb_of(uri: &str) -> Option<(MessageVerb, String)> {
    [
        (REPLY_SCHEME, MessageVerb::Reply),
        (FORWARD_SCHEME, MessageVerb::Forward),
        (CONTINUE_SCHEME, MessageVerb::Continue),
        (ALLOW_SCHEME, MessageVerb::Allow),
    ]
    .into_iter()
    .find_map(|(scheme, verb)| {
        uri.strip_prefix(scheme)
            .and_then(|rest| rest.strip_prefix(':'))
            .filter(|scope| !scope.is_empty())
            .map(|scope| (verb, scope.to_owned()))
    })
}

/// A stable id so a fold stays open or closed across re-renders. A message's fold is its anchor, a
/// quote's is `<anchor>-q<n>`.
pub const FOLD_ATTRIBUTE: &str = "data-postio-fold";

/// One function so the id and the `#m-3` fragment agree; a mismatch scrolls nowhere, silently
/// (#1386).
/// Takes the scope raw and escapes it here.
pub fn message_anchor(scope: &str) -> String {
    format!("m-{}", escape(scope))
}

/// A single message is a thread of one, so one set of chrome serves both.
pub struct Entry<'a> {
    /// Must be the scope the body was sanitised under. Unreserved characters only: it goes into a
    /// URI unescaped.
    pub scope: &'a str,
    pub sender: &'a str,
    pub address: &'a str,
    pub when: &'a str,
    pub preview: &'a str,
    pub expanded: bool,
    /// A draft offers `Continue editing`, neither reply nor forward (#1212).
    pub draft: bool,
    pub mine: bool,
    /// Always false for a thread of one.
    pub latest: bool,
    pub blocked: u32,
    /// Already sanitised under `scope`.
    pub body: &'a str,
    /// Per message: the one that added two hundred recipients is worth noticing before reply-all
    /// (#1427).
    /// Empty draws nothing.
    pub recipients: &'a str,
    pub cc: &'a str,
    pub sheet: Sheet,
    /// Must be the scoped output: an unscoped rule restyles every other sender on the page.
    pub styles: &'a str,
}

/// Postio's own script, evaluated by the frontend; the page's script is off (ADR 0003).
pub fn scroll_script(anchor: &str) -> String {
    format!(
        "(() => {{ const target = document.getElementById(\"{}\"); \
         if (target) {{ target.scrollIntoView(); }} }})()",
        quoted(anchor)
    )
}

/// Acts on the document's state: with script off the host cannot see a click on a summary.
pub fn toggle_script(anchor: &str) -> String {
    format!(
        "(() => {{ const message = document.getElementById(\"{}\"); \
         if (message) {{ message.open = !message.open; }} }})()",
        quoted(anchor)
    )
}

pub fn expand_all_script() -> String {
    "(() => { for (const message of document.querySelectorAll('details.postio-message')) \
     { message.open = true; } })()"
        .to_owned()
}

/// Greatest visible area (the rail's rule), debounced, and reported once at load. Posts the scope,
/// which a frontend checks against its thread, since the page holds several senders' markup.
/// `handler` is reduced to identifier characters so nothing passed in can become script.
pub fn observer_script(handler: &str) -> String {
    let handler: String = handler
        .chars()
        .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
        .collect();
    format!(
        "(() => {{\
           const post = () => {{\
             const view = document.documentElement.clientHeight;\
             let best = null, most = 0;\
             for (const el of document.querySelectorAll('.postio-message')) {{\
               const box = el.getBoundingClientRect();\
               const visible = Math.max(0, Math.min(box.bottom, view) - Math.max(box.top, 0));\
               if (visible > most) {{ most = visible; best = el.id; }}\
             }}\
             if (best) {{\
               window.webkit.messageHandlers.{handler}.postMessage(best.replace(/^m-/, ''));\
             }}\
           }};\
           let pending = null;\
           addEventListener('scroll', () => {{\
             clearTimeout(pending);\
             pending = setTimeout(post, 100);\
           }}, {{ passive: true }});\
           post();\
         }})()"
    )
}

fn quoted(text: &str) -> String {
    text.chars()
        .map(|character| match character {
            '\\' => "\\\\".to_owned(),
            '"' => "\\\"".to_owned(),
            '\n' | '\r' => String::new(),
            other => other.to_string(),
        })
        .collect()
}

/// A message before its body is drawn; `compose` turns it into an `Entry` (#1595).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThreadMessage {
    /// The message id in decimal: unreserved characters only, since it goes into a URI unescaped.
    pub scope: String,
    pub sender: String,
    /// Also what the per-sender image decision is made on.
    pub address: String,
    pub when: String,
    pub recipients: String,
    pub cc: String,
    pub preview: String,
    pub expanded: bool,
    /// Only the open message shows the "no body yet" plate; a thread of thirty must not carry
    /// thirty.
    pub absent: bool,
    pub latest: bool,
    pub draft: bool,
    pub mine: bool,
    /// Unsanitised; `compose` sanitises it under `scope`.
    pub body: postio_model::MessageBody,
}

/// `originals` holds the scopes the reader asked to see as sent (`⌃O`, #1398).
/// `renders` caches each drawn body, so a redraw sanitises only what changed.
pub fn compose(
    messages: &[ThreadMessage],
    allowed: impl Fn(&str) -> bool,
    originals: &std::collections::HashMap<String, super::document::Rendering>,
    renders: &mut super::document::RenderCache,
) -> String {
    use super::document::{Absent, Rendered, absent_html};

    // Rendered first and held, because `Entry` borrows the markup.
    let mut sheets = Vec::with_capacity(messages.len());
    let rendered: Vec<Rendered> = messages
        .iter()
        .map(|message| {
            let rendering = originals
                .get(&message.scope)
                .copied()
                .unwrap_or_else(super::document::opening_rendering);
            // Per message, from its own sender, so one allowed correspondent does not carry the
            // thread (#1353).
            let remote = if allowed(&message.address) {
                RemoteImages::Allowed
            } else {
                RemoteImages::Blocked
            };
            // The verdict is cached with the render, so this costs a lookup, not a parse.
            sheets.push(if message.absent {
                Sheet::Theme
            } else {
                super::document::sheet_for(
                    rendering,
                    renders.suits_reader_view(&message.scope, &message.body),
                )
            });
            if message.absent && message.expanded {
                // `Partial`: the thread knows only that no body is here yet; `role="status"`
                // announces it once.
                return Rendered {
                    html: absent_html(Absent::Partial),
                    ..Rendered::default()
                };
            }
            renders.render(&message.scope, &message.body, remote, rendering)
        })
        .collect();
    renders.keep_only(messages.iter().map(|message| message.scope.as_str()));
    let entries: Vec<Entry<'_>> = messages
        .iter()
        .zip(&rendered)
        .zip(&sheets)
        .map(|((message, rendered), sheet)| Entry {
            scope: &message.scope,
            sender: &message.sender,
            address: &message.address,
            when: &message.when,
            preview: &message.preview,
            expanded: message.expanded,
            latest: message.latest,
            draft: message.draft,
            mine: message.mine,
            blocked: rendered.held_back.remote_images,
            body: &rendered.html,
            styles: &rendered.styles,
            recipients: &message.recipients,
            cc: &message.cc,
            sheet: *sheet,
        })
        .collect();

    // The CSP is one policy per page (ADR 0032), so it opens only when some sender is allowed;
    // the sanitizer keeps blocked senders' images out.
    let anyone_allowed = messages.iter().any(|message| allowed(&message.address));
    conversation_document(
        &entries,
        if anyone_allowed {
            RemoteImages::Allowed
        } else {
            RemoteImages::Blocked
        },
        super::document::Sheet::Theme,
    )
}

/// Each message is a `<details>`; bodies sit in `.postio-body` so several senders on one page keep
/// a visible edge (#323).
pub fn conversation_document(entries: &[Entry<'_>], remote: RemoteImages, sheet: Sheet) -> String {
    let mut content = String::new();
    // Postio's styles first, then the senders': theirs win where they collide.
    // Safe because every sender rule is rewritten under its message's container (#1326).
    content.push_str("<style>");
    content.push_str(THREAD_CSS);
    if entries.iter().any(|entry| entry.sheet == Sheet::Senders) {
        content.push_str(&super::document::senders_sheet_css());
    }
    content.push_str("</style>");
    let senders: String = entries
        .iter()
        .map(|entry| entry.styles)
        .collect::<Vec<_>>()
        .join("\n");
    content.push_str(&senders_stylesheet(&senders));
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
    // The link is the consent: a notice without a way to unblock is a dead end. Per message since
    // #1353.
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
    // The header bar is fixed to the latest message, so these are the only way to reply to an older
    // one.
    // Out of flow, so they reserve no space and shift nothing; named for the message so six of them
    // differ to a screen reader.
    // A draft gets one verb instead of the pair (#1212).
    let actions = if entry.draft {
        format!(
            "<span class=\"postio-message-actions\">\
             <a class=\"postio-message-action\" href=\"{CONTINUE_SCHEME}:{scope}\" \
             aria-label=\"Continue editing this draft\" \
             title=\"Continue editing this draft\">Continue editing</a>\
             </span>",
            scope = escape(entry.scope),
        )
    } else {
        format!(
            "<span class=\"postio-message-actions\">\
         <a class=\"postio-message-action\" href=\"{REPLY_SCHEME}:{scope}\" \
         aria-label=\"Reply to {sender}\" title=\"Reply to {sender}\">Reply</a>\
         <a class=\"postio-message-action\" href=\"{FORWARD_SCHEME}:{scope}\" \
         aria-label=\"Forward {sender}&#39;s message\" \
         title=\"Forward {sender}&#39;s message\">Forward</a>\
         </span>",
            // Escaped: a scope with a quote would close the `href` and inject an attribute.
            scope = escape(entry.scope),
            sender = sender,
        )
    };
    // Named, so this message's own rules (scoped to the name) match.
    let body = contain_body_in(entry.body, Some(entry.scope));
    let anchor = message_anchor(entry.scope);
    let body = number_quote_folds(&body, &format!("{anchor}-"));
    let recipients = recipients_html(entry.recipients, entry.cc);
    // A normal string, not raw: a raw string cannot be line-continued.
    let mine = if entry.mine {
        format!(" {MINE_CLASS}")
    } else {
        String::new()
    };
    let paper = match entry.sheet {
        Sheet::Senders => format!(" {}", super::document::SENDERS_SHEET_CLASS),
        Sheet::Theme => String::new(),
    };
    format!(
        "<details class=\"postio-message{mine}{paper}\" id=\"{anchor}\" \
         {FOLD_ATTRIBUTE}=\"{anchor}\"{open}>\
         <summary class=\"postio-message-head\">\
         <span class=\"postio-recipients-label\">From</span>\
         <span class=\"postio-from\">{sender}</span>\
         <span class=\"postio-address\">{address}</span>\
         <span class=\"postio-preview\">{preview}</span>\
         {latest}\
         <span class=\"postio-when\">{when}</span>\
         {actions}\
         </summary>{recipients}{blocked}{body}</details>"
    )
}

/// Outside the `<summary>`, which a collapsed message shows: recipients belong with the opened
/// message.
fn recipients_html(recipients: &str, cc: &str) -> String {
    let mut rows = String::new();
    for (label, value) in [("To", recipients), ("Cc", cc)] {
        if value.trim().is_empty() {
            continue;
        }
        rows.push_str(&format!(
            "<div class=\"postio-message-recipients\">\
             <span class=\"postio-recipients-label\">{label}</span>\
             <span class=\"postio-recipients-value\">{}</span></div>",
            escape(value)
        ));
    }
    rows
}

/// Sender-controlled strings (name, preview) go into markup here; the body arrives already
/// sanitised.
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
    fn a_draft_offers_continue_editing_and_neither_reply_nor_forward() {
        // #1212: a draft has no correspondent, so reply and forward are wrong; it offers Continue
        // editing.
        let mut draft = entry("7", "Ada", "<p>unsent</p>", true);
        draft.draft = true;
        let document = conversation_document(
            &[draft],
            postio_body::RemoteImages::Blocked,
            crate::reader::document::Sheet::Theme,
        );
        assert!(
            document.contains(&format!("{CONTINUE_SCHEME}:7")),
            "a draft was not offered the one verb that is true of it"
        );
        assert!(
            !document.contains(REPLY_SCHEME),
            "a draft was offered a reply, which would quote the user's own \
             unsent text back at them"
        );
        assert!(
            !document.contains(FORWARD_SCHEME),
            "a draft was offered a forward of a message that was never sent"
        );
    }

    #[test]
    fn only_the_draft_of_a_thread_loses_its_reply() {
        let sent = entry("1", "Ada", "<p>sent</p>", true);
        let mut draft = entry("2", "Bo", "<p>unsent</p>", true);
        draft.draft = true;
        let document = conversation_document(
            &[sent, draft],
            postio_body::RemoteImages::Blocked,
            crate::reader::document::Sheet::Theme,
        );
        assert!(
            document.contains(&format!("{REPLY_SCHEME}:1")),
            "the sent message lost the reply that is right for it"
        );
        assert!(
            !document.contains(&format!("{REPLY_SCHEME}:2")),
            "the draft kept a reply because a sibling had one"
        );
        assert!(document.contains(&format!("{CONTINUE_SCHEME}:2")));
    }

    #[test]
    fn a_message_from_the_user_is_marked_as_theirs() {
        let mut mine = entry("1", "Ada", "<p>hi</p>", true);
        mine.mine = true;
        let theirs = entry("2", "Bo", "<p>hi</p>", true);
        let document = conversation_document(
            &[mine, theirs],
            postio_body::RemoteImages::Blocked,
            crate::reader::document::Sheet::Theme,
        );
        let first = document
            .split("<details")
            .nth(1)
            .expect("the first message");
        let second = document
            .split("<details")
            .nth(2)
            .expect("the second message");
        assert!(
            first.contains(MINE_CLASS),
            "the user's own message is not marked as theirs"
        );
        assert!(
            !second.contains(MINE_CLASS),
            "a correspondent's message is marked as the user's own"
        );
    }

    #[test]
    fn a_blocked_message_offers_a_way_to_show_its_images() {
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

    use postio_body::sanitize;

    use super::*;

    fn entry<'a>(scope: &'a str, sender: &'a str, body: &'a str, expanded: bool) -> Entry<'a> {
        Entry {
            scope,
            sender,
            address: "ada@example.com",
            when: "09:14",
            preview: "the first line of it",
            expanded,
            draft: false,
            mine: false,
            latest: false,
            blocked: 0,
            styles: "",
            recipients: "",
            cc: "",
            body,
            sheet: Sheet::Theme,
        }
    }

    #[test]
    fn a_thread_document_is_counted_like_a_single_one() {
        // The document-size guard must see conversation documents, not only `document_for` (#749).
        let built_before = crate::test_support::documents_built();
        let bytes_before = crate::test_support::document_bytes();
        let document = conversation_document(
            &[entry("1", "Ada Lovelace", "<p>first</p>", true)],
            postio_body::RemoteImages::Blocked,
            crate::reader::document::Sheet::Theme,
        );
        assert_eq!(
            crate::test_support::documents_built() - built_before,
            1,
            "a thread document was built and the guard did not count it"
        );
        assert_eq!(
            crate::test_support::document_bytes() - bytes_before,
            document.len() as u64,
            "the guard counted something other than the document's bytes"
        );
    }

    #[test]
    fn a_thread_is_one_document_holding_every_message() {
        let entries = [
            entry("1", "Ada Lovelace", "<p>first</p>", true),
            entry("2", "Grace Hopper", "<p>second</p>", false),
            entry("3", "Ada Lovelace", "<p>third</p>", false),
        ];
        let document = conversation_document(&entries, RemoteImages::Blocked, Sheet::Theme);

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
        // Entry markup only: the generated stylesheet legitimately contains backslashes.
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
        assert!(document.contains(r#"id="m-2" data-postio-fold="m-2" open>"#));
        assert!(document.contains(r#"id="m-1" data-postio-fold="m-1">"#));
    }

    #[test]
    fn every_body_keeps_its_own_bounded_surface() {
        let entries = [
            entry("1", "Ada", "<p>a</p>", true),
            entry("2", "Grace", "<p>b</p>", true),
        ];
        let document = conversation_document(&entries, RemoteImages::Blocked, Sheet::Theme);
        assert_eq!(
            document
                .matches(&format!(r#"<div class="{}""#, sanitize::BODY_CLASS))
                .count(),
            2
        );
        for scope in ["1", "2"] {
            assert!(
                document.contains(&format!(r#"{}="{scope}""#, sanitize::MESSAGE_ATTRIBUTE)),
                "message {scope} is not named: {document}"
            );
        }
    }

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

    #[test]
    fn one_message_is_a_thread_of_one() {
        let mut only = entry("7", "Marketside", "<p>out for delivery</p>", true);
        only.address = "orders@marketside.example";
        let document = conversation_document(&[only], RemoteImages::Blocked, Sheet::Theme);

        assert_eq!(document.matches("<details").count(), 1);
        assert!(
            document.contains(r#"id="m-7" data-postio-fold="m-7" open>"#),
            "the one message has to be open, or a single message opens closed"
        );
        assert!(document.contains("orders@marketside.example"));
        assert!(document.contains("<p>out for delivery</p>"));
        assert!(
            !document.contains(r#"<span class="postio-latest">"#),
            "a thread of one has nothing for `latest` to distinguish"
        );
    }

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
        // Against `entry_html`, not a whole document: `conversation_document` bumps a global
        // counter in
        // `reader::cost`, and assembling one here races that test (#1390).
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

    #[test]
    fn a_quote_folds_inside_the_message_it_belongs_to() {
        let quoted = "<p>my reply</p><details class=\"postio-quote\"><summary>quoted \
                      text</summary><blockquote>theirs</blockquote></details>";
        let document = conversation_document(
            &[
                entry("7", "Ada", quoted, true),
                entry("11", "Grace", "<p>no quote here</p>", true),
            ],
            postio_body::RemoteImages::Blocked,
            crate::reader::document::Sheet::Theme,
        );

        let ada = document
            .split(&format!("id=\"{}\"", message_anchor("7")))
            .nth(1)
            .and_then(|rest| {
                rest.split(&format!("id=\"{}\"", message_anchor("11")))
                    .next()
            })
            .expect("Ada's message is in the document");
        assert!(
            ada.contains("postio-quote"),
            "the fold is not inside the message that owns it: {ada}"
        );
        assert!(
            ada.contains("my reply"),
            "and the prose it belongs with is there too: {ada}"
        );

        let grace = document
            .split(&format!("id=\"{}\"", message_anchor("11")))
            .nth(1)
            .expect("Grace's message is in the document");
        assert!(
            !grace.contains("postio-quote"),
            "a message with nothing to fold gained a fold from its neighbour: \
             {grace}"
        );
    }

    #[test]
    fn a_conversations_quote_fold_says_how_many_lines_it_hides() {
        let body = postio_model::MessageBody {
            text: None,
            html: Some("<p>ok</p><blockquote><p>one</p><p>two</p></blockquote>".to_owned()),
        };
        let rendered = crate::reader::document::body_html_in(
            &body,
            RemoteImages::Blocked,
            crate::reader::document::Rendering::Original,
            Some("7"),
        );
        let document = conversation_document(
            &[entry("7", "Ada", &rendered.html, true)],
            RemoteImages::Blocked,
            Sheet::Theme,
        );
        let fold = r#"<details class="postio-quote" data-postio-fold="m-7-q0"><summary>2 quoted lines</summary>"#;
        assert!(document.contains(fold), "{fold} is not in {document}");
    }

    #[test]
    fn every_fold_carries_a_stable_id_unique_in_the_document() {
        let quoted = "<p>a</p><details class=\"postio-quote\"><summary>q</summary>\
                      <blockquote>one</blockquote></details><details class=\"postio-quote\">\
                      <summary>q</summary><blockquote>two</blockquote></details>";
        let compose = || {
            conversation_document(
                &[
                    entry("7", "Ada", quoted, true),
                    entry("11", "Grace", quoted, true),
                ],
                postio_body::RemoteImages::Blocked,
                crate::reader::document::Sheet::Theme,
            )
        };
        let document = compose();
        let ids: Vec<&str> = document
            .split("<details")
            .skip(1)
            .map(|tag| {
                tag.split_once("data-postio-fold=\"")
                    .filter(|(before, _)| !before.contains('>'))
                    .and_then(|(_, rest)| rest.split('"').next())
                    .unwrap_or_else(|| panic!("a fold without an id: <details{tag}"))
            })
            .collect();
        assert_eq!(ids.len(), 6, "two messages and four quotes: {ids:?}");
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len(), "fold ids collide: {ids:?}");
        assert_eq!(compose(), document, "the ids are not stable");
        assert!(ids.contains(&message_anchor("7").as_str()));
    }
}

#[cfg(test)]
mod recipients_tests {
    use super::*;

    #[test]
    fn to_and_cc_are_labelled_rows_and_empty_ones_are_left_out() {
        let rows = recipients_html("Quinn <quinn@example.net>", "");
        assert!(rows.contains(">To</span>"), "{rows}");
        assert!(
            !rows.contains(">Cc</span>"),
            "an empty Cc drew a row: {rows}"
        );
        assert!(
            rows.contains("Quinn &lt;quinn@example.net&gt;"),
            "the address was not escaped: {rows}"
        );
        let both = recipients_html("a@example.com", "b@example.com");
        assert_eq!(
            both.matches("postio-message-recipients").count(),
            2,
            "{both}"
        );
        assert_eq!(
            recipients_html("  ", ""),
            "",
            "blank recipients draw nothing"
        );
    }

    #[test]
    fn chrome_text_escapes_everything_markup_could_use() {
        assert_eq!(
            escape(r#"<b>"Ada" & 'Bo'</b>"#),
            "&lt;b&gt;&quot;Ada&quot; &amp; &#39;Bo&#39;&lt;/b&gt;"
        );
    }
}

#[cfg(test)]
mod compose_tests {
    use super::*;
    use crate::reader::document::RenderCache;
    use postio_model::MessageBody;
    use std::collections::HashMap;

    fn from(scope: &str, address: &str) -> ThreadMessage {
        ThreadMessage {
            scope: scope.to_owned(),
            sender: address.to_owned(),
            address: address.to_owned(),
            when: "10:40".to_owned(),
            preview: "the gate".to_owned(),
            expanded: true,
            body: MessageBody {
                text: None,
                html: Some(format!(
                    r#"<p>From {scope}.</p><img src="https://images.example.com/{scope}.png" alt="pixel">"#
                )),
            },
            ..ThreadMessage::default()
        }
    }

    fn newsletter(scope: &str) -> ThreadMessage {
        ThreadMessage {
            body: MessageBody {
                text: None,
                html: Some(
                    "<table><tr><td><table><tr><td><p style=\"color:#333\">Good morning.</p>\
                     </td></tr></table></td></tr></table>"
                        .to_owned(),
                ),
            },
            ..from(scope, "news@example.com")
        }
    }

    fn opening_of(document: &str, scope: &str) -> String {
        let marker = format!(r#"id="{}""#, message_anchor(scope));
        let at = document
            .find(&marker)
            .expect("the message is in the document");
        let start = document[..at].rfind('<').expect("an opening tag");
        let end = at + document[at..].find('>').expect("a closed tag");
        document[start..=end].to_owned()
    }

    #[test]
    fn a_bulk_message_shown_as_sent_is_on_its_senders_paper_in_a_conversation() {
        // Canvas 20: per message, a newsletter gets the sender's paper while correspondence beside
        // it keeps the theme.
        let messages = [newsletter("1"), from("2", "ada@example.com")];
        let document = compose(
            &messages,
            |_| false,
            &HashMap::new(),
            &mut RenderCache::default(),
        );

        assert!(
            opening_of(&document, "1").contains(crate::reader::document::SENDERS_SHEET_CLASS),
            "the newsletter is on its own paper: {}",
            opening_of(&document, "1")
        );
        assert!(
            !opening_of(&document, "2").contains(crate::reader::document::SENDERS_SHEET_CLASS),
            "the reply keeps the theme"
        );
        assert!(
            document.contains(&format!(
                ".{} .postio-body",
                crate::reader::document::SENDERS_SHEET_CLASS
            )),
            "and the page carries the rule that paints the paper"
        );
    }

    #[test]
    fn reader_view_of_a_newsletter_is_drawn_in_the_theme() {
        let messages = [newsletter("1")];
        let mut chosen = HashMap::new();
        chosen.insert("1".to_owned(), crate::reader::document::Rendering::Reader);
        let document = compose(&messages, |_| false, &chosen, &mut RenderCache::default());
        assert!(!opening_of(&document, "1").contains(crate::reader::document::SENDERS_SHEET_CLASS));
    }

    #[test]
    fn a_sender_the_user_allowed_keeps_their_images_and_nobody_else_does() {
        let messages = [from("1", "ada@example.com"), from("2", "bo@example.org")];
        let document = compose(
            &messages,
            |address| address == "ada@example.com",
            &HashMap::new(),
            &mut RenderCache::default(),
        );
        assert!(
            document.contains("images.example.com/1.png"),
            "the allowed sender's image went"
        );
        assert!(
            !document.contains("images.example.com/2.png"),
            "a stranger's image rode in on somebody else's grant"
        );
    }

    #[test]
    fn the_page_opens_to_remote_images_only_when_someone_in_it_is_allowed() {
        let messages = [from("1", "ada@example.com")];
        let nobody = compose(
            &messages,
            |_| false,
            &HashMap::new(),
            &mut RenderCache::default(),
        );
        assert!(
            !nobody.contains("img-src postio-cid: data: http: https:"),
            "{nobody}"
        );
        let someone = compose(
            &messages,
            |_| true,
            &HashMap::new(),
            &mut RenderCache::default(),
        );
        assert!(someone.contains("img-src postio-cid: data: http: https:"));
    }

    #[test]
    fn a_body_still_coming_says_so_only_where_it_is_open() {
        let mut open = from("1", "ada@example.com");
        open.absent = true;
        let mut shut = from("2", "bo@example.org");
        shut.absent = true;
        shut.expanded = false;
        let document = compose(
            &[open, shut],
            |_| false,
            &HashMap::new(),
            &mut RenderCache::default(),
        );
        assert_eq!(document.matches("role=\"status\"").count(), 1, "{document}");
    }

    #[test]
    fn every_message_has_the_anchor_the_pane_scrolls_to() {
        let messages = [from("1", "ada@example.com"), from("2", "bo@example.org")];
        let document = compose(
            &messages,
            |_| false,
            &HashMap::new(),
            &mut RenderCache::default(),
        );
        for scope in ["1", "2"] {
            assert!(
                document.contains(&format!("id=\"{}\"", message_anchor(scope))),
                "message {scope} cannot be scrolled to"
            );
        }
    }
}

#[cfg(test)]
mod verb_tests {
    use super::*;

    #[test]
    fn each_verb_scheme_names_its_verb_and_its_message() {
        assert_eq!(
            verb_of("postio-reply:42"),
            Some((MessageVerb::Reply, "42".to_owned()))
        );
        assert_eq!(
            verb_of("postio-forward:42"),
            Some((MessageVerb::Forward, "42".to_owned()))
        );
        assert_eq!(
            verb_of("postio-continue:7"),
            Some((MessageVerb::Continue, "7".to_owned()))
        );
        assert_eq!(
            verb_of("postio-allow:9"),
            Some((MessageVerb::Allow, "9".to_owned()))
        );
    }

    #[test]
    fn anything_else_is_not_a_verb() {
        assert_eq!(verb_of("https://example.com/postio-reply:42"), None);
        assert_eq!(verb_of("postio-reply:"), None, "a verb naming no message");
        assert_eq!(
            verb_of("postio-delete:42"),
            None,
            "an unknown verb is refused, not guessed"
        );
    }

    #[test]
    fn the_markup_and_the_parse_agree() {
        let entry = Entry {
            scope: "12",
            sender: "Ada",
            address: "ada@example.com",
            when: "",
            preview: "",
            expanded: true,
            draft: false,
            mine: false,
            latest: false,
            blocked: 2,
            body: "",
            recipients: "",
            cc: "",
            styles: "",
            sheet: Sheet::Theme,
        };
        let html = entry_html(&entry);
        for (scheme, verb) in [
            (REPLY_SCHEME, MessageVerb::Reply),
            (FORWARD_SCHEME, MessageVerb::Forward),
            (ALLOW_SCHEME, MessageVerb::Allow),
        ] {
            let href = format!("{scheme}:12");
            assert!(
                html.contains(&format!("href=\"{href}\"")),
                "{scheme} not in the page"
            );
            assert_eq!(verb_of(&href), Some((verb, "12".to_owned())));
        }
    }
}

#[cfg(test)]
mod script_tests {
    use super::*;

    #[test]
    fn scrolling_to_a_message_finds_it_by_id() {
        let script = scroll_script(&message_anchor("42"));
        assert!(script.contains(r#"getElementById("m-42")"#), "{script}");
        assert!(script.contains("scrollIntoView"), "{script}");
    }

    #[test]
    fn an_anchor_cannot_close_the_literal_it_is_quoted_in() {
        let script = scroll_script("m-\"); alert(1); (\"");
        assert!(
            script.contains(r#"getElementById("m-\"); alert(1); (\"")"#),
            "the quote was not escaped, so it closed the literal: {script}"
        );
        let script = toggle_script("a\\\nb");
        assert!(
            !script.contains('\n'),
            "a line break ends a literal: {script}"
        );
    }

    #[test]
    fn folding_toggles_one_message_and_expanding_opens_every_one() {
        let fold = toggle_script(&message_anchor("7"));
        assert!(
            fold.contains(r#"getElementById("m-7")"#) && fold.contains(".open = !"),
            "{fold}"
        );
        let all = expand_all_script();
        assert!(
            all.contains("querySelectorAll") && all.contains(".open = true"),
            "{all}"
        );
    }
}

#[cfg(test)]
mod observer_tests {
    use super::*;

    #[test]
    fn the_observer_reports_to_the_handler_it_is_given() {
        let script = observer_script("postioRail");
        assert!(
            script.contains("window.webkit.messageHandlers.postioRail.postMessage"),
            "{script}"
        );
    }

    #[test]
    fn it_settles_rather_than_tracking_every_frame() {
        let script = observer_script("postioRail");
        assert!(
            script.contains("setTimeout") && script.contains("clearTimeout"),
            "{script}"
        );
    }

    #[test]
    fn it_reports_the_message_by_the_scope_the_page_named_it_with() {
        let script = observer_script("postioRail");
        assert!(script.contains(".postio-message"), "{script}");
        assert!(script.contains("replace(/^m-/, '')"), "{script}");
    }

    #[test]
    fn a_handler_name_cannot_become_script() {
        let script = observer_script("rail.postMessage('x'); evil");
        assert!(!script.contains("evil"), "{script}");
    }
}
