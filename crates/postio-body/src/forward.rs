//! Forwarding a whole thread (spec 013 US7, FR-023, research R12).
//!
//! Three forms of "forward", and this is the two that are not the single
//! message `postio_model::reply::forward` already builds:
//!
//! * [`forward_thread`]: every member oldest first in one readable message,
//!   each under a "From · date" [`header`], with its own words and the
//!   history it quoted, and the attachments of every member **once**.
//! * [`forward_as_attachments`]: each member's raw source as a
//!   `message/rfc822` part, one `.eml` per member.
//!
//! # Folded as the reader folds
//!
//! A member's quoted history is not cut or rewritten here. It stays in the
//! markup as a `<blockquote>` (or as `>` lines in the plain half), which are
//! exactly the stretches [`crate::quote::html_stretches`] and
//! [`crate::quote::text_stretches`] find, so the composer and the recipient's
//! reader fold the same spans the reader folds today. Each member goes through
//! [`crate::quote_of`], the one gate a reply's quote goes through, so a
//! forwarded thread carries no script and no remote image either.
//!
//! # Deduplicated by content hash
//!
//! The same file is attached to a message and again to the reply that
//! quotes it. An attachment's [`BlobId`](postio_model::ids::BlobId) is its
//! content hash (the blob store is content addressed), so two attachments
//! with the same one are one file and go once. A part whose bytes were never
//! downloaded has no hash; it is matched by name, type and size instead, which
//! can only ever keep a duplicate, never drop a different file.

use std::collections::{HashMap, HashSet};

use postio_model::ids::{AttachmentId, MessageId};
use postio_model::{Attachment, MessageBody};

use crate::document::{Block, Document, Inline};
use crate::replying::{Quoted, quote_of};

/// One member of a thread, ready to be forwarded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    /// The line it is headed by: [`header`].
    pub header: String,
    /// Its body as stored.
    pub body: MessageBody,
    /// Its attachments.
    pub attachments: Vec<Attachment>,
}

/// A forwarded thread: what the draft starts from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutgoingBody {
    /// The subject: [`thread_subject`].
    pub subject: String,
    /// Every member's words.
    pub body: MessageBody,
    /// Every member's attachments, each file once, ready to belong to a draft.
    pub attachments: Vec<Attachment>,
}

/// What opens forwarded content, in every form and in both halves: the
/// conventional line `postio_model::reply::forward_header` starts with. A
/// whole thread's says "messages". It is how the person's own words, above
/// it, are told from the forwarded content, below it.
pub const MARKER: &str = "---------- Forwarded message";

/// The line that opens a forwarded thread.
pub const THREAD_MARKER: &str = "---------- Forwarded messages ----------";

/// The person's own words in a forward in progress: what sits above
/// [`MARKER`], in both halves, or the whole body when there is no marker
/// (the "as attachments" form carries none).
pub fn typed_above(body: &MessageBody) -> MessageBody {
    MessageBody {
        text: body.text.as_deref().map(|text| {
            let end = text
                .match_indices(MARKER)
                .map(|(at, _)| at)
                .find(|&at| at == 0 || text[..at].ends_with('\n'))
                .unwrap_or(text.len());
            text[..end].trim_end_matches('\n').to_owned()
        }),
        html: body.html.as_deref().map(|html| {
            let end = html.find(MARKER).map_or(html.len(), |at| {
                // Back to the block that holds the marker line.
                html[..at]
                    .rfind("<p")
                    .into_iter()
                    .chain(html[..at].rfind("<div"))
                    .max()
                    .unwrap_or(at)
            });
            html[..end].to_owned()
        }),
    }
}

/// `typed` above `content`, where `content` begins with [`MARKER`] (or is
/// empty). With nothing typed, a blank line is left above for the caret.
pub fn with_typed(typed: &MessageBody, content: &MessageBody) -> MessageBody {
    let words = typed.text.as_deref().unwrap_or_default();
    let text = content.text.as_deref().map(|content| {
        if words.trim().is_empty() {
            format!("\n\n{content}")
        } else {
            format!("{words}\n\n{content}")
        }
    });
    let html = content.html.as_deref().map(|content| {
        let above = typed.html.as_deref().unwrap_or_default();
        if above.trim().is_empty() {
            // The caret's line: an empty paragraph would be narrowed away.
            let caret = Document {
                blocks: vec![Block::Paragraph(vec![Inline::Break])],
            };
            format!("{}{content}", caret.to_html())
        } else {
            format!("{above}{content}")
        }
    });
    MessageBody { text, html }
}

/// The line a member is headed by: who, then when (`date` already spelled).
pub fn header(from: &str, date: &str) -> String {
    format!("{from} · {date}")
}

/// "Fwd: <subject> (N messages)", with no second "Fwd:".
///
/// A subject that already ends in a count (a thread forwarded again, or the
/// same call made twice) has it replaced rather than repeated.
pub fn thread_subject(subject: &str, messages: usize) -> String {
    let base = strip_count(subject.trim());
    let count = if messages == 1 {
        "(1 message)".to_owned()
    } else {
        format!("({messages} messages)")
    };
    format!("{} {count}", postio_model::subject::forward_subject(base))
}

/// `subject` without a trailing " (N messages)".
fn strip_count(subject: &str) -> &str {
    let Some(open) = subject.rfind(" (") else {
        return subject;
    };
    let tail = &subject[open + 2..];
    let counted = tail
        .strip_suffix(" messages)")
        .or_else(|| tail.strip_suffix(" message)"))
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    if counted {
        subject[..open].trim_end()
    } else {
        subject
    }
}

/// The whole thread as one body: see the module doc.
pub fn forward_thread(members: &[Member], subject: &str) -> OutgoingBody {
    let mut blocks = vec![Block::Paragraph(vec![Inline::Text(
        THREAD_MARKER.to_owned(),
    )])];
    let mut text = format!("{THREAD_MARKER}\n\n");
    for (n, member) in members.iter().enumerate() {
        let (quoted, own_text) = carried(member, n);
        blocks.push(Block::Paragraph(vec![Inline::Strong(vec![Inline::Text(
            member.header.clone(),
        )])]));
        if !quoted.is_empty() {
            blocks.push(Block::Quoted(quoted.carried()));
        }
        if n > 0 {
            text.push_str("\n\n");
        }
        text.push_str(&member.header);
        text.push_str("\n\n");
        text.push_str(own_text.trim_end());
    }
    text.push('\n');
    OutgoingBody {
        subject: thread_subject(subject, members.len()),
        body: MessageBody {
            text: Some(text),
            html: Some(Document { blocks }.to_html()),
        },
        attachments: distinct_attachments(members),
    }
}

/// One member through the reply quote's gate, and its plain words.
///
/// A plain-text member's `>` lines are made `<blockquote>`s first, so its
/// history is a quote in the html half as well as in the text half.
fn carried(member: &Member, n: usize) -> (Quoted, String) {
    let text = member.body.text.clone().unwrap_or_default();
    let html = member
        .body
        .html
        .clone()
        .filter(|html| !html.trim().is_empty())
        .or_else(|| {
            (!text.trim().is_empty()).then(|| Document::from_text_quoting(&text).to_html())
        });
    let quoted = quote_of(html.as_deref(), &text, &format!("fwd{}", n + 1));
    let words = if text.trim().is_empty() {
        quoted.text().to_owned()
    } else {
        text
    };
    (quoted, words)
}

/// Every member's attachments, each file once, as a draft's own.
fn distinct_attachments(members: &[Member]) -> Vec<Attachment> {
    let mut seen_hashes = HashSet::new();
    let mut seen_unfetched = HashSet::new();
    let mut out = Vec::new();
    for attachment in members.iter().flat_map(|member| &member.attachments) {
        let new = match &attachment.blob_id {
            Some(hash) => seen_hashes.insert(hash.clone()),
            None => seen_unfetched.insert((
                attachment.filename.clone(),
                attachment.mime_type.clone(),
                attachment.size,
            )),
        };
        if new {
            let mut carried = attachment.clone();
            carried.id = AttachmentId::UNASSIGNED;
            carried.message_id = MessageId::UNASSIGNED;
            out.push(carried);
        }
    }
    out
}

/// Each raw source as a `message/rfc822` attachment named `.eml`, in order.
///
/// The attachments hold no bytes (`blob_id` is `None`): the caller stores
/// `raw[n]` and sets the `n`th one's `blob_id`, because the blob store is not
/// below this crate. Each is named by its `Subject:` when the source has a
/// plain one, else "message N".
pub fn forward_as_attachments(raw: &[Vec<u8>]) -> Vec<Attachment> {
    let mut used: HashMap<String, u32> = HashMap::new();
    raw.iter()
        .enumerate()
        .map(|(n, bytes)| {
            let stem = subject_of(bytes)
                .map(|subject| safe_name(&subject))
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| format!("message {}", n + 1));
            let times = used.entry(stem.to_lowercase()).or_insert(0);
            *times += 1;
            let name = if *times == 1 {
                format!("{stem}.eml")
            } else {
                format!("{stem} {times}.eml")
            };
            let mut attachment =
                Attachment::new(MessageId::UNASSIGNED, "message/rfc822", bytes.len() as u64);
            attachment.filename = Some(name);
            attachment
        })
        .collect()
}

/// The `Subject:` of a raw message, unfolded; `None` when absent or encoded
/// (an encoded word is not worth a half-decoded file name).
fn subject_of(raw: &[u8]) -> Option<String> {
    let head_end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .or_else(|| raw.windows(2).position(|w| w == b"\n\n"))
        .unwrap_or(raw.len());
    let head = String::from_utf8_lossy(&raw[..head_end]).replace("\r\n", "\n");
    let mut lines = head.split('\n').peekable();
    while let Some(line) = lines.next() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if !name.eq_ignore_ascii_case("subject") {
            continue;
        }
        let mut subject = value.trim().to_owned();
        while let Some(next) = lines.peek() {
            if next.starts_with([' ', '\t']) {
                subject.push(' ');
                subject.push_str(next.trim());
                lines.next();
            } else {
                break;
            }
        }
        return (!subject.contains("=?")).then_some(subject);
    }
    None
}

/// `name` as a file's stem: no path separators or controls, bounded.
fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '-'
            } else {
                c
            }
        })
        .take(80)
        .collect();
    cleaned.trim().trim_matches('.').trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_model::attachment::Disposition;
    use postio_model::ids::BlobId;

    fn at(day: u32) -> String {
        format!("2026-03-{day:02} 09:30")
    }

    fn file(name: &str, blob: &str, size: u64) -> Attachment {
        let mut attachment = Attachment::new(MessageId::new(7), "application/pdf", size);
        attachment.id = postio_model::ids::AttachmentId::new(40);
        attachment.filename = Some(name.to_owned());
        attachment.blob_id = Some(BlobId::new(blob));
        attachment
    }

    /// Six members, oldest first. Member 2 has quoted history in HTML,
    /// member 3 in plain `>` lines. Members 2 and 5 attach the same bytes
    /// under different names; member 4 attaches another file.
    fn six() -> Vec<Member> {
        (1..=6u32)
            .map(|n| {
                let body = match n {
                    2 => MessageBody {
                        text: Some(format!("words {n}\n")),
                        html: Some(format!(
                            "<p>words {n}</p><blockquote><p>history of one</p></blockquote>"
                        )),
                    },
                    3 => MessageBody {
                        text: Some(format!(
                            "words {n}\n\nOn Mon, Ada wrote:\n> history of two\n"
                        )),
                        html: None,
                    },
                    _ => MessageBody {
                        text: Some(format!("words {n}\n")),
                        html: None,
                    },
                };
                let attachments = match n {
                    2 => vec![file("plan.pdf", "hash-a", 100)],
                    4 => vec![file("map.pdf", "hash-b", 200)],
                    5 => vec![file("plan-final.pdf", "hash-a", 100)],
                    _ => Vec::new(),
                };
                Member {
                    header: header(&format!("Sender {n}"), &at(n)),
                    body,
                    attachments,
                }
            })
            .collect()
    }

    #[test]
    fn a_header_says_who_then_when() {
        assert_eq!(
            header("Ada Lovelace", &at(3)),
            "Ada Lovelace · 2026-03-03 09:30"
        );
    }

    #[test]
    fn six_members_come_oldest_first_each_under_its_header() {
        let out = forward_thread(&six(), "Tide gate");
        let text = out.body.text.expect("a text half");
        let html = out.body.html.expect("an html half");
        for (name, part) in [("text", &text), ("html", &html)] {
            let mut last = 0;
            for n in 1..=6u32 {
                let header = header(&format!("Sender {n}"), &at(n));
                let found = part
                    .find(&header)
                    .unwrap_or_else(|| panic!("{name}: no header for member {n}: {part}"));
                assert!(found >= last, "{name}: member {n} is out of order");
                last = found;
                let words = part
                    .find(&format!("words {n}"))
                    .unwrap_or_else(|| panic!("{name}: no words for member {n}"));
                assert!(
                    words > found,
                    "{name}: member {n}'s words are above its header"
                );
            }
        }
    }

    #[test]
    fn quoted_history_is_left_for_the_reader_to_fold() {
        let out = forward_thread(&six(), "Tide gate");
        let html = out.body.html.expect("html");
        let text = out.body.text.expect("text");
        assert!(
            crate::quote::html_stretches(&html).iter().any(
                |s| matches!(s, crate::quote::Stretch::Quoted(q) if q.contains("history of one"))
            ),
            "the html member's history is a quote the reader folds: {html}"
        );
        assert!(
            crate::quote::html_stretches(&html).iter().any(
                |s| matches!(s, crate::quote::Stretch::Quoted(q) if q.contains("history of two"))
            ),
            "the plain member's `>` lines are a quote in the html half too: {html}"
        );
        assert!(
            crate::quote::text_stretches(&text).iter().any(
                |s| matches!(s, crate::quote::Stretch::Quoted(q) if q.contains("history of two"))
            ),
            "and stay `>` lines in the plain half: {text}"
        );
    }

    #[test]
    fn each_attachment_goes_once_by_content_hash() {
        let out = forward_thread(&six(), "Tide gate");
        let hashes: Vec<_> = out
            .attachments
            .iter()
            .map(|a| a.blob_id.as_ref().expect("a blob").as_str().to_owned())
            .collect();
        assert_eq!(
            hashes,
            ["hash-a", "hash-b"],
            "same bytes once, in thread order"
        );
        assert_eq!(
            out.attachments[0].filename.as_deref(),
            Some("plan.pdf"),
            "the first member's name wins"
        );
        for attachment in &out.attachments {
            assert_eq!(attachment.id, postio_model::ids::AttachmentId::UNASSIGNED);
            assert_eq!(attachment.message_id, MessageId::UNASSIGNED);
        }
    }

    #[test]
    fn a_part_with_no_bytes_is_matched_by_name_type_and_size() {
        let mut a = file("a.pdf", "x", 5);
        a.blob_id = None;
        let b = a.clone();
        let mut c = a.clone();
        c.size = 6;
        let members = vec![
            Member {
                header: "h".into(),
                body: MessageBody::default(),
                attachments: vec![a, b],
            },
            Member {
                header: "h".into(),
                body: MessageBody::default(),
                attachments: vec![c],
            },
        ];
        assert_eq!(forward_thread(&members, "S").attachments.len(), 2);
    }

    #[test]
    fn the_eml_set_is_one_rfc822_part_per_member_in_order() {
        let raw = vec![
            b"Subject: one\r\n\r\nA".to_vec(),
            b"Subject: two\r\n\r\nBB".to_vec(),
        ];
        let set = forward_as_attachments(&raw);
        assert_eq!(set.len(), 2);
        for (n, attachment) in set.iter().enumerate() {
            assert_eq!(attachment.mime_type, "message/rfc822");
            assert_eq!(attachment.size, raw[n].len() as u64);
            assert_eq!(attachment.disposition, Disposition::Attachment);
            assert!(attachment.blob_id.is_none());
        }
        assert_eq!(set[0].filename.as_deref(), Some("one.eml"));
        assert_eq!(set[1].filename.as_deref(), Some("two.eml"));
    }

    #[test]
    fn eml_names_are_unique_and_safe() {
        let raw = vec![
            b"Subject: a/b\r\n\r\n".to_vec(),
            b"Subject: a/b\r\n\r\n".to_vec(),
            b"X: y\r\n\r\n".to_vec(),
        ];
        let names: Vec<_> = forward_as_attachments(&raw)
            .into_iter()
            .map(|a| a.filename.expect("a name"))
            .collect();
        assert_eq!(names, ["a-b.eml", "a-b 2.eml", "message 3.eml"]);
    }

    #[test]
    fn the_subject_counts_the_messages_and_never_doubles_fwd() {
        assert_eq!(
            thread_subject("Tide gate", 6),
            "Fwd: Tide gate (6 messages)"
        );
        assert_eq!(
            thread_subject("Fwd: Tide gate", 6),
            "Fwd: Tide gate (6 messages)"
        );
        assert_eq!(
            thread_subject("FW: Tide gate", 2),
            "FW: Tide gate (2 messages)"
        );
        assert_eq!(
            thread_subject("Tide gate (6 messages)", 6),
            "Fwd: Tide gate (6 messages)"
        );
        assert_eq!(thread_subject("", 3), "Fwd: (3 messages)");
        assert_eq!(
            forward_thread(&six(), "Tide gate").subject,
            "Fwd: Tide gate (6 messages)"
        );
    }

    fn marked_plain(typed: &str) -> MessageBody {
        MessageBody {
            text: Some(format!(
                "{typed}\n\n{MARKER} ----------\nFrom: Ada\n\nthe old words\n"
            )),
            html: None,
        }
    }

    #[test]
    fn a_thread_forward_opens_with_its_marker_in_both_halves() {
        let out = forward_thread(&six(), "Tide gate").body;
        assert!(out.text.as_deref().unwrap().starts_with(THREAD_MARKER));
        assert!(out.html.as_deref().unwrap().contains(THREAD_MARKER));
    }

    #[test]
    fn the_typed_words_are_what_sits_above_the_marker() {
        let typed = typed_above(&marked_plain("my words\n\nsecond"));
        assert_eq!(typed.text.as_deref(), Some("my words\n\nsecond"));
        assert_eq!(typed.html, None);
        let rich = MessageBody {
            text: Some(format!("mine\n\n{MARKER} ----------\nx")),
            html: Some(format!(
                "<p>mine</p><p>{MARKER} ----------<br>From: x</p><div>old</div>"
            )),
        };
        let typed = typed_above(&rich);
        assert_eq!(typed.text.as_deref(), Some("mine"));
        assert_eq!(typed.html.as_deref(), Some("<p>mine</p>"));
    }

    #[test]
    fn a_body_with_no_marker_is_all_typed() {
        let body = MessageBody {
            text: Some("just me\n".into()),
            html: None,
        };
        assert_eq!(typed_above(&body).text.as_deref(), Some("just me"));
        assert_eq!(typed_above(&MessageBody::default()), MessageBody::default());
    }

    #[test]
    fn typed_words_survive_a_swap_of_what_is_forwarded() {
        let thread = forward_thread(&six(), "Tide gate").body;
        let swapped = with_typed(&typed_above(&marked_plain("keep me")), &thread);
        let text = swapped.text.expect("text");
        assert!(
            text.starts_with(&format!("keep me\n\n{THREAD_MARKER}")),
            "{text}"
        );
        assert!(!text.contains("the old words"));
        // And again: the second swap finds the same typed words.
        assert_eq!(
            typed_above(&MessageBody {
                text: Some(text),
                html: None
            })
            .text
            .as_deref(),
            Some("keep me")
        );
    }

    #[test]
    fn with_nothing_typed_a_blank_line_is_left_for_the_caret() {
        let thread = forward_thread(&six(), "Tide gate").body;
        let body = with_typed(&MessageBody::default(), &thread);
        assert!(
            body.text
                .expect("text")
                .starts_with(&format!("\n\n{THREAD_MARKER}"))
        );
    }
}
