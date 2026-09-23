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

use super::document::{Sheet, contain_body_in, scroll_markers, senders_stylesheet, wrap_document};

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

/// The scheme a draft's `Continue editing` uses. See [`REPLY_SCHEME`].
///
/// A draft has one verb that is true of it and it is not a reply (#1212), so
/// it does not share the other two: routing it separately is what lets the
/// frontend raise `CommandId::OpenMessage` -- the same command activating the
/// row raises -- rather than inventing a second way to resume a composer.
pub const CONTINUE_SCHEME: &str = "postio-continue";

/// The class the user's own messages wear, for the mark in
/// `Design/screens/18-conversation-row-states.png` (#1241).
///
/// A class rather than a drawn mark, because the document is HTML and the
/// stylesheet is where a mark belongs -- the stacked pane drew it into a
/// widget's `snapshot()` and that is precisely why it did not survive the
/// pane (#1444).
pub const MINE_CLASS: &str = "postio-mine";

/// A verb a message offers for itself, inside a conversation document.
///
/// With JavaScript off, a verb in the page is a navigation, and the pane
/// intercepts it by scheme before anything can reach the browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageVerb {
    /// Reply to this message rather than to the latest one (FR-009).
    Reply,
    /// Forward this message.
    Forward,
    /// Resume the composer on this draft (#1212).
    Continue,
    /// Show this sender's remote images -- the consent the blocked notice
    /// asks for.
    Allow,
}

/// Which verb a navigation inside the document asks for, and for which
/// message's scope -- or `None` when it is not a verb at all.
///
/// Matched on the scheme alone, and the whole prefix: a sender controls a
/// link's text and class and neither of the schemes the sanitizer will emit,
/// so the scheme is the only thing worth trusting. An unknown verb is refused
/// rather than mapped to the nearest one, and a verb naming no message is not
/// a verb.
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
    /// Whether this is a draft: a message the user wrote and never sent.
    ///
    /// It changes which verbs the message offers -- `Continue editing`, and
    /// neither reply nor forward (#1212).
    pub draft: bool,
    /// Whether the message came from one of the account's own addresses.
    ///
    /// Draws the user's own side of the conversation with the outlined mark
    /// of `Design/screens/18-conversation-row-states.png` (#1241). The
    /// comparison is folded once by the frontend, which is the only layer
    /// that knows the account's identities.
    pub mine: bool,
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
    /// Who it went to, already drawn by
    /// [`crate::reader::header::recipient_line`] -- "Ada, Bob and 197
    /// others".
    ///
    /// Per message, not per thread: a conversation's messages go to different
    /// people, and the one that added two hundred recipients is the one worth
    /// noticing before reply-all. The stacked pane drew this on every
    /// expanded entry, and one document has to keep it or the default pane
    /// says less than the pane it replaced (#1427).
    ///
    /// Empty when there are none, and then nothing is drawn.
    pub recipients: &'a str,
    /// Who else was copied, drawn by the same rule. Empty when nobody was.
    pub cc: &'a str,
    /// This message's own stylesheets, scoped to it
    /// (`postio_body::sanitize::Sanitized::styles`). Empty for most mail.
    ///
    /// Must be the scoped output, for the same reason [`Entry::body`] must:
    /// a rule that arrived unscoped restyles every other sender on this page.
    pub styles: &'a str,
}

/// A host script that scrolls the page to `anchor` -- `J`, `K`, the rail.
///
/// Postio's own script, evaluated by the frontend: the page's own script is
/// off (ADR 0003). `getElementById` takes a string and never parses a
/// selector, so the only thing to escape against is the literal it is quoted
/// in -- and the ids are Postio's own, so that escape is belt and braces
/// rather than the control. The rule GTK's reader uses for its fragments.
pub fn scroll_script(anchor: &str) -> String {
    format!(
        "(() => {{ const target = document.getElementById(\"{}\"); \
         if (target) {{ target.scrollIntoView(); }} }})()",
        quoted(anchor)
    )
}

/// A host script that folds or unfolds the message at `anchor` -- `z`.
///
/// Expansion is the document's state, not a model's: with the page's script
/// off, the application cannot see a person click a summary, so the toggle
/// acts on what is on screen rather than on what the host believes is.
pub fn toggle_script(anchor: &str) -> String {
    format!(
        "(() => {{ const message = document.getElementById(\"{}\"); \
         if (message) {{ message.open = !message.open; }} }})()",
        quoted(anchor)
    )
}

/// A host script that opens every message -- *Expand all*.
pub fn expand_all_script() -> String {
    "(() => { for (const message of document.querySelectorAll('details.postio-message')) \
     { message.open = true; } })()"
        .to_owned()
}

/// A host script that reports which message fills the pane, to
/// `window.webkit.messageHandlers.<handler>` -- the rail's observer.
///
/// **Greatest visible area**, the rail's rule: what a person is reading is
/// what fills the screen, not the shortest message that happens to be whole.
/// Debounced rather than continuous -- "jitter during a flick-scroll is worse
/// than lag" -- and reported once at once, so a freshly loaded page says
/// where it is. What is posted is the message's scope, the anchor with its
/// prefix off; a frontend checks it against the thread it drew, because the
/// report arrives from a page holding several senders' markup.
///
/// `handler` is Postio's own name for the channel, reduced to the characters
/// a JavaScript identifier may hold, so nothing passed here can become
/// script. The script GTK's reader has run since it had a rail.
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

/// `text` inside a double-quoted script literal: its escapes and its quote
/// escaped, line breaks dropped -- a literal cannot hold one.
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

/// One message of a conversation document, before its body is drawn.
///
/// The frontend's view of a message -- who, when, whether it is open, and its
/// body still unsanitised -- which [`compose`] turns into an [`Entry`]. It
/// lived in `postio-gtk`'s reader, and moved here with [`compose`] so the
/// macOS pane composes the same document from the same decisions rather than
/// a second copy of them (#1595).
///
/// `Clone` because a reader keeps the thread it drew: the `Show` verb inside
/// the document has to re-render after granting consent, and it re-renders
/// the same messages rather than asking for them again.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThreadMessage {
    /// What this message's `cid:` references are stamped with, and what the
    /// scheme handler routes on. The message id in decimal: unreserved
    /// characters only, since it goes into a URI unescaped.
    pub scope: String,
    /// Who it is from, as a person reads it.
    pub sender: String,
    /// Their address, shown beside the name on an open message (canvas 17),
    /// and what the per-sender image decision is made on.
    pub address: String,
    /// When, already formatted.
    pub when: String,
    /// Who it went to, already drawn by
    /// [`crate::reader::header::recipient_line`] (#1427).
    pub recipients: String,
    /// Who else was copied, by the same rule. Empty when nobody was.
    pub cc: String,
    /// The one line a collapsed message shows.
    pub preview: String,
    /// Whether it starts open.
    pub expanded: bool,
    /// Whether the body has not been backfilled yet.
    ///
    /// A message with no body used to contribute an empty section and say
    /// nothing. Only the message that is *open* shows the plate: everything
    /// unfetched stays the one line it already was, or a thread of thirty
    /// would carry thirty explanations of one fact.
    pub absent: bool,
    /// Whether this is the newest message in the thread -- canvas 17's badge.
    pub latest: bool,
    /// Whether this is a draft: written here and never sent (#1212).
    pub draft: bool,
    /// Whether it came from one of the account's own addresses (#1241).
    pub mine: bool,
    /// The message body, unsanitised -- [`compose`] sanitises it under
    /// [`scope`](Self::scope), which is the only way the reference stamping
    /// can be guaranteed.
    pub body: postio_model::MessageBody,
}

/// The whole thread as one document, deciding each message on its own terms.
///
/// `allowed` answers whether a sender's remote images are allowed, and
/// `originals` holds the scopes the reader asked to see as sent (`⌃O`,
/// #1398). Both frontends call this, so a conversation reads the same on
/// either (#1595).
pub fn compose(
    messages: &[ThreadMessage],
    allowed: impl Fn(&str) -> bool,
    originals: &std::collections::HashSet<String>,
) -> String {
    use super::document::{Absent, Rendered, Rendering, absent_html, body_html_in};

    // Rendered first, and held, because `Entry` borrows the markup. Reader
    // view is decided per message, from the message: bulk mail opens reduced,
    // correspondence never does, and a thread can hold both.
    let rendered: Vec<Rendered> = messages
        .iter()
        .map(|message| {
            // The reader's own choice first: `⌃O` on a message overrules what
            // its content suggests, for that message and no other (#1398).
            let rendering = if originals.contains(&message.scope) {
                Rendering::Original
            } else if super::document::suits_reader_view(&message.body) {
                Rendering::Reader
            } else {
                Rendering::Original
            };
            // Per **message**, from its own sender (`PRODUCT.md` §21), so one
            // allowed correspondent does not carry the rest of the thread with
            // them (#1353).
            let remote = if allowed(&message.address) {
                RemoteImages::Allowed
            } else {
                RemoteImages::Blocked
            };
            if message.absent && message.expanded {
                // The single-message pane's own words, and its `role="status"`
                // live region with them, so a screen reader is told once.
                // `Partial`: the thread knows only that no body is here yet.
                return Rendered {
                    html: absent_html(Absent::Partial),
                    ..Rendered::default()
                };
            }
            body_html_in(&message.body, remote, rendering, Some(&message.scope))
        })
        .collect();
    let entries: Vec<Entry<'_>> = messages
        .iter()
        .zip(&rendered)
        .map(|(message, rendered)| Entry {
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
        })
        .collect();

    // The document's `Content-Security-Policy` is one policy for the whole
    // page, with no per-message form -- the limitation ADR 0032 names. So it
    // opens only when some message is from a sender the user allowed, and the
    // *sanitizer* keeps the others out: a blocked sender's `src` is dropped
    // before the markup is composed. For such a page the policy is no longer a
    // second, independent refusal; it is still the only refusal for every
    // thread where nobody is allowed, which is the ordinary case.
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

/// The whole conversation, as one hardened document.
///
/// Each message is a `<details>` whose `<summary>` is its header, and whose
/// body is wrapped in `.postio-body` — #323's visible edge between what Postio
/// wrote and what arrived, which matters more here than in a single-message
/// document, not less: several senders share this page.
pub fn conversation_document(entries: &[Entry<'_>], remote: RemoteImages, sheet: Sheet) -> String {
    let mut content = String::new();
    // Postio's first, then the senders'. Ours is the ground the page sits on;
    // theirs describes their own message and wins where the two genuinely
    // collide (FR-019).
    //
    // A sender's rules used to be simply absent here -- `sanitize` stripped
    // `<style>` tag-and-contents -- and that was what made it safe for several
    // senders to share one page. #1326 admits them, so what makes it safe now
    // is that every rule has been rewritten under its own message's container
    // (`postio_body::styles`). That is a stronger claim resting on a parser
    // rather than on a deletion, which is why it is tested against the engine
    // and not only against the markup.
    content.push_str("<style>");
    content.push_str(THREAD_CSS);
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
    // A draft's one verb. The other two are the correspondent's, and a draft
    // has no correspondent yet (#1212) -- so this is not an addition to the
    // pair below but a replacement for it.
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
        )
    };
    // Named, not just contained: the name is what this message's own rules
    // were scoped to, and without it they match nothing.
    let body = contain_body_in(entry.body, Some(entry.scope));
    let anchor = message_anchor(entry.scope);
    let recipients = recipients_html(entry.recipients, entry.cc);
    // A normal string, not a raw one: a raw string cannot be line-continued,
    // and the backslash would be a character in the markup — which is what
    // `the_markup_is_well_formed` caught.
    let mine = if entry.mine {
        format!(" {MINE_CLASS}")
    } else {
        String::new()
    };
    format!(
        "<details class=\"postio-message{mine}\" id=\"{anchor}\"{open}>\
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

/// The `to` line of one message, or nothing when it has no recipients.
///
/// Outside the `<summary>` on purpose: the summary is what a *collapsed*
/// message shows, and it already carries sender, preview, date and the verbs.
/// Who it went to belongs with the message you have opened, which is where
/// the stacked pane drew it too.
fn recipients_html(recipients: &str, cc: &str) -> String {
    let mut rows = String::new();
    for (label, value) in [("To", recipients), ("Cc", cc)] {
        if value.trim().is_empty() {
            continue;
        }
        // Label and value as two cells, so `To` and `Cc` line up with each
        // other and their addresses start at the same column. Drawn bare,
        // the recipients read as a stray line of text under the sender --
        // which is exactly how it looked when this had no label at all.
        rows.push_str(&format!(
            "<div class=\"postio-message-recipients\">\
             <span class=\"postio-recipients-label\">{label}</span>\
             <span class=\"postio-recipients-value\">{}</span></div>",
            escape(value)
        ));
    }
    rows
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
    fn a_draft_offers_continue_editing_and_neither_reply_nor_forward() {
        // #1212: a draft is a message you wrote and never sent, so reply,
        // reply-all and forward are the correspondent's verbs and it has no
        // correspondent yet -- a `Reply` here quotes your own unsent text back
        // at you. The verb that is right was already reachable and
        // unannounced: activating it resumes the composer on the draft.
        //
        // The stacked pane knew this and drew `DRAFT_ACTIONS` on the entry's
        // own bar. The one document had no equivalent and offered `Reply`
        // (#1444).
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
        // The draft is not necessarily the row the list holds, and the verbs
        // are per message: the message it answers keeps its own reply.
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
        // #1241: `Design/screens/18-conversation-row-states.png` gives the
        // user's own side of a conversation an outlined mark rather than a
        // filled one. The stacked pane drew it per entry; the document draws
        // it as a class the stylesheet hangs the mark on (#1444).
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
        }
    }

    #[test]
    fn a_thread_document_is_counted_like_a_single_one() {
        // The document-size guard (`cost::note_document`: documents built,
        // bytes, largest) is what catches #749's bulk regression, and the
        // one-document conversation is the document the app actually ships
        // for a thread. It was invisible to the guard: `note_document` ran
        // only in `document_for`, and this path went to `wrap_document`
        // directly. The 2026-09-08 note recorded the gap.
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
        assert_eq!(
            document
                .matches(&format!(r#"<div class="{}""#, sanitize::BODY_CLASS))
                .count(),
            2
        );
        // And each one named, because the name is what #1326 scoped that
        // message's own stylesheet to. A container that lost it is a message
        // that renders unstyled.
        for scope in ["1", "2"] {
            assert!(
                document.contains(&format!(r#"{}="{scope}""#, sanitize::MESSAGE_ATTRIBUTE)),
                "message {scope} is not named: {document}"
            );
        }
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

    #[test]
    fn a_quote_folds_inside_the_message_it_belongs_to() {
        // Each message is already a `<details>` whose `<summary>` is its
        // header, so a folded quote is a second `<details>` nested inside one.
        // "It folded" and "it folded inside the right message" are different
        // claims in a document holding several senders, and only the second is
        // worth anything -- a quote that escaped its message would be attached
        // to somebody else's mail.
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
}

#[cfg(test)]
mod compose_tests {
    use super::*;
    use postio_model::MessageBody;
    use std::collections::HashSet;

    /// One message from `address`, open, with a remote image in its body.
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

    #[test]
    fn a_sender_the_user_allowed_keeps_their_images_and_nobody_else_does() {
        // Per sender, never per page (#1353, `PRODUCT.md` §21): one allowed
        // correspondent must not carry the rest of the thread with them.
        let messages = [from("1", "ada@example.com"), from("2", "bo@example.org")];
        let document = compose(
            &messages,
            |address| address == "ada@example.com",
            &HashSet::new(),
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
        // The policy is one per page, so it is the sanitizer that keeps a
        // blocked sender out -- and the policy is still the only refusal for
        // the ordinary thread, where nobody is allowed.
        let messages = [from("1", "ada@example.com")];
        let nobody = compose(&messages, |_| false, &HashSet::new());
        assert!(
            !nobody.contains("img-src postio-cid: data: http: https:"),
            "{nobody}"
        );
        let someone = compose(&messages, |_| true, &HashSet::new());
        assert!(someone.contains("img-src postio-cid: data: http: https:"));
    }

    #[test]
    fn a_body_still_coming_says_so_only_where_it_is_open() {
        // One explanation, not thirty: a collapsed message is its one line.
        let mut open = from("1", "ada@example.com");
        open.absent = true;
        let mut shut = from("2", "bo@example.org");
        shut.absent = true;
        shut.expanded = false;
        let document = compose(&[open, shut], |_| false, &HashSet::new());
        assert_eq!(document.matches("role=\"status\"").count(), 1, "{document}");
    }

    #[test]
    fn every_message_has_the_anchor_the_pane_scrolls_to() {
        let messages = [from("1", "ada@example.com"), from("2", "bo@example.org")];
        let document = compose(&messages, |_| false, &HashSet::new());
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
        // A sender controls a link's text and class, never these schemes --
        // the sanitizer will not emit them -- so the scheme is all that is
        // matched, and a sender's own link is a link.
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
        // The page writes these links and the pane reads them back; if the
        // two drifted, every per-message verb would go to the browser.
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
        // The ids are Postio's own, so this is belt and braces -- but a quote
        // or a backslash in one would otherwise end the string and run the
        // rest as script.
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
        // "Jitter during a flick-scroll is worse than lag."
        let script = observer_script("postioRail");
        assert!(
            script.contains("setTimeout") && script.contains("clearTimeout"),
            "{script}"
        );
    }

    #[test]
    fn it_reports_the_message_by_the_scope_the_page_named_it_with() {
        // The anchor is `message_anchor(scope)`, so taking the prefix off is
        // the scope back -- and a frontend checks it against the thread it
        // drew rather than trusting a page holding several senders' markup.
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
