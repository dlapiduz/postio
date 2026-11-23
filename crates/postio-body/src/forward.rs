//! Forwarding a whole thread: `forward_thread` (one readable message, attachments once) and
//! `forward_as_attachments` (one `.eml` per member).
//! Each member goes through `quote_of` and keeps its quoted history as `<blockquote>` or `>` lines,
//! so composer and recipient fold the same spans and no script or remote image is carried.
//! Attachments dedupe by `BlobId` (a content hash); an undownloaded part matches by name, type and
//! size, which can only keep a duplicate.

use std::collections::{HashMap, HashSet};

use postio_model::ids::{AttachmentId, MessageId};
use postio_model::{Attachment, MessageBody};

use crate::document::{Block, Document, Inline};
use crate::replying::{Quoted, quote_of};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub header: String,
    pub body: MessageBody,
    pub attachments: Vec<Attachment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutgoingBody {
    pub subject: String,
    pub body: MessageBody,
    pub attachments: Vec<Attachment>,
}

/// Opens forwarded content in both halves, matching `forward_header`; it separates the person's own
/// words above from the forwarded content below.
pub const MARKER: &str = "---------- Forwarded message";

pub const THREAD_MARKER: &str = "---------- Forwarded messages ----------";

/// What sits above `MARKER` in both halves, or the whole body when there is no marker.
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForwardedRow {
    pub number: usize,
    pub header: String,
    pub first: String,
}

impl ForwardedRow {
    pub fn words(&self) -> String {
        format!("{} \u{b7} {}", self.number, self.header)
    }
}

/// The block index of the `THREAD_MARKER` line; everything from it on is forwarded content.
pub fn thread_start(document: &Document) -> Option<usize> {
    document.blocks.iter().position(|block| {
        matches!(block, Block::Paragraph(_)) && block_text(block).trim() == THREAD_MARKER
    })
}

pub fn thread_rows(blocks: &[Block]) -> Vec<ForwardedRow> {
    let mut rows: Vec<ForwardedRow> = Vec::new();
    for block in blocks.iter().skip(1) {
        if let Some(header) = member_header(block) {
            rows.push(ForwardedRow {
                number: rows.len() + 1,
                header,
                first: String::new(),
            });
        } else if let Some(row) = rows.last_mut().filter(|row| row.first.is_empty()) {
            row.first = block_text(block)
                .lines()
                .map(|line| line.trim_start_matches(['>', ' ']).trim())
                .find(|line| !line.is_empty())
                .unwrap_or_default()
                .to_owned();
        }
    }
    rows
}

/// Bold in the html half and plain in the text half, which a plain-text composer loads.
fn member_header(block: &Block) -> Option<String> {
    if !matches!(block, Block::Paragraph(_)) {
        return None;
    }
    let text = block_text(block);
    let text = text.trim();
    let (who, when) = text.rsplit_once(" \u{b7} ")?;
    let shape = b"dddd-dd-dd dd:dd";
    let fits = when.len() == shape.len()
        && when.bytes().zip(shape).all(|(byte, shape)| match shape {
            b'd' => byte.is_ascii_digit(),
            other => byte == *other,
        });
    (fits && !who.is_empty()).then(|| text.to_owned())
}

fn block_text(block: &Block) -> String {
    Document {
        blocks: vec![block.clone()],
    }
    .to_text()
}

pub fn header(from: &str, date: &str) -> String {
    format!("{from} · {date}")
}

/// A subject already ending in a count has it replaced, not repeated.
pub fn thread_subject(subject: &str, messages: usize) -> String {
    let base = strip_count(subject.trim());
    let count = if messages == 1 {
        "(1 message)".to_owned()
    } else {
        format!("({messages} messages)")
    };
    format!("{} {count}", postio_model::subject::forward_subject(base))
}

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

/// A plain-text member's `>` lines become `<blockquote>`s first, so its history is a quote in the
/// html half too.
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

/// The attachments hold no bytes (`blob_id` is `None`): the caller stores `raw[n]` and sets it,
/// since the blob store is below this crate.
/// Named "2 · Lena Park · subject.eml"; an absent or encoded sender or subject is left out, and a
/// message with neither is "message N".
pub fn forward_as_attachments(raw: &[Vec<u8>]) -> Vec<Attachment> {
    let mut used: HashMap<String, u32> = HashMap::new();
    raw.iter()
        .enumerate()
        .map(|(n, bytes)| {
            let who = header_of(bytes, "from").and_then(|from| sender_name(&from));
            let subject = header_of(bytes, "subject")
                .filter(|subject| !subject.contains("=?"))
                .map(|subject| bare_subject(&subject).to_owned())
                .filter(|subject| !subject.is_empty());
            let parts: Vec<String> = who.into_iter().chain(subject).collect();
            let stem = if parts.is_empty() {
                format!("message {}", n + 1)
            } else {
                format!("{} \u{b7} {}", n + 1, parts.join(" \u{b7} "))
            };
            let stem = safe_name(&stem);
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

fn bare_subject(subject: &str) -> &str {
    let mut rest = subject.trim();
    loop {
        let lower = rest.to_ascii_lowercase();
        let Some(prefix) = ["re:", "fwd:", "fw:"]
            .into_iter()
            .find(|prefix| lower.starts_with(prefix))
        else {
            return rest;
        };
        rest = rest[prefix.len()..].trim_start();
    }
}

/// `None` for an encoded name.
fn sender_name(from: &str) -> Option<String> {
    let from = from.trim();
    let name = match from.split_once('<') {
        Some((name, _)) => name.trim().trim_matches('"').trim(),
        None => "",
    };
    if name.contains("=?") {
        return None;
    }
    let name = if name.is_empty() {
        let address = from.trim_matches(['<', '>']);
        address.split('@').next().unwrap_or("").trim()
    } else {
        name
    };
    (!name.is_empty()).then(|| name.to_owned())
}

fn header_of(raw: &[u8], wanted: &str) -> Option<String> {
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
        if !name.eq_ignore_ascii_case(wanted) {
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
        return (!subject.is_empty()).then_some(subject);
    }
    None
}

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
    fn a_forwarded_thread_is_cut_at_its_marker_and_read_as_one_row_a_message() {
        let body = forward_thread(&six(), "Plan");
        let mut document = crate::parse(body.body.html.as_deref().expect("an html half"));
        document.blocks.insert(
            0,
            Block::Paragraph(vec![Inline::Text("Typed above.".to_owned())]),
        );
        let start = thread_start(&document).expect("the marker is found");
        assert_eq!(start, 1, "the typed words stay above it");
        let rows = thread_rows(&document.blocks[start..]);
        assert_eq!(rows.len(), 6);
        assert_eq!(
            rows[1].words(),
            format!("2 \u{b7} Sender 2 \u{b7} {}", at(2))
        );
        assert_eq!(rows[0].first, "words 1");
        let plain = Document::from_text(body.body.text.as_deref().expect("a text half"));
        let rows = thread_rows(&plain.blocks[thread_start(&plain).expect("a marker")..]);
        assert_eq!(rows.len(), 6);
        assert_eq!(rows[5].header, format!("Sender 6 \u{b7} {}", at(6)));
        assert_eq!(
            thread_start(&Document::from_text("Nothing forwarded")),
            None
        );
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
        assert_eq!(set[0].filename.as_deref(), Some("1 \u{b7} one.eml"));
        assert_eq!(set[1].filename.as_deref(), Some("2 \u{b7} two.eml"));
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
        assert_eq!(
            names,
            ["1 \u{b7} a-b.eml", "2 \u{b7} a-b.eml", "message 3.eml"]
        );
    }

    #[test]
    fn eml_names_say_the_place_the_sender_and_the_subject_without_re() {
        let raw = vec![
            b"From: Ana Ruiz <ana@example.com>\r\nSubject: Harbor API draft v3\r\n\r\n".to_vec(),
            b"From: \"Lena Park\" <lena@example.com>\r\nSubject: Re: Harbor API draft v3\r\n\r\n"
                .to_vec(),
            b"From: ben@example.com\r\nSubject: RE: Fwd: Harbor: the plan?\r\n\r\n".to_vec(),
            b"From: =?utf-8?q?L=C3=A9na?= <l@example.com>\r\nSubject: =?utf-8?q?x?=\r\n\r\n"
                .to_vec(),
        ];
        let names: Vec<_> = forward_as_attachments(&raw)
            .into_iter()
            .map(|a| a.filename.expect("a name"))
            .collect();
        assert_eq!(
            names,
            [
                "1 \u{b7} Ana Ruiz \u{b7} Harbor API draft v3.eml",
                "2 \u{b7} Lena Park \u{b7} Harbor API draft v3.eml",
                "3 \u{b7} ben \u{b7} Harbor- the plan-.eml",
                "message 4.eml",
            ]
        );
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
