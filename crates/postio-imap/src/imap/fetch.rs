//! `UID FETCH` of `ENVELOPE`, `BODYSTRUCTURE`, flags, size and internal
//! date — everything an initial sync needs before a single body byte exists
//! locally.
//!
//! # Bounded memory is the caller's job, by design
//!
//! [`fetch_headers`] issues exactly one `UID FETCH` for whatever [`UidSet`] it
//! is given and returns everything that command reported. It does not chunk
//! internally: `backend::mod`'s rule 3 puts batching on the caller, via
//! [`UidSet::chunks`], so a ten-thousand-message initial sync is many calls
//! into this function rather than one call that buffers ten thousand
//! messages. Passing an already-chunked set is what keeps this off the heap.
//!
//! # `References` is not part of `ENVELOPE`
//!
//! JWZ threading needs the `References` header, but RFC 3501's `ENVELOPE`
//! carries only `In-Reply-To` and `Message-ID`. So every fetch also asks for
//! `BODY.PEEK[HEADER.FIELDS (REFERENCES)]` alongside `ENVELOPE` — one extra
//! item on the same command, not a second round trip — and `.PEEK` so reading
//! headers never sets `\Seen`.
//!
//! # `BODYSTRUCTURE` is flattened, not kept as a tree
//!
//! `imap_types::body::BodyStructure` is a recursive `Single`/`Multi` enum.
//! [`flatten`] walks it into the flat [`PartNode`] list `BodyStructure` wants,
//! assigning RFC 3501 §6.4.5 section numbers as it goes: a non-multipart
//! message is section `1`; a multipart's children are `1`, `2`, …; and a
//! part of type `MESSAGE/RFC822` numbers its own encapsulated content one
//! level deeper (`3.1`, `3.2`, …), exactly as a nested multipart would.

use std::num::NonZeroU64;

use io_imap::client::ImapClientAsync;
use io_imap::rfc3501::fetch::ImapMessageFetchOptions;
use io_imap::types::body::{
    Body as WireBody, BodyStructure as WireBodyStructure, Disposition as WireDisposition,
    SpecificFields,
};
use io_imap::types::command::FetchModifier;
use io_imap::types::core::{AString, IString, NString, Vec1};
use io_imap::types::envelope::{Address as WireAddress, Envelope as WireEnvelope};
use io_imap::types::fetch::Section;
use io_imap::types::fetch::{MacroOrMessageDataItemNames, MessageDataItem, MessageDataItemName};
use io_imap::types::flag::FlagFetch;
use io_imap::types::sequence::SequenceSet;
use postio_model::{
    Disposition, EmailAddress, Flag, FlagSet, ModSeq, RfcMessageId, Uid, UidValidity,
};

use crate::backend::{
    BackendError, BackendResult, BodyStructure, Capability, Envelope, FetchedMessage, PartNode,
    UidSet,
};
use crate::cancel::CancelToken;

use super::{ConnectionPool, ImapSession, Priority, skip_counter};

/// Fetches metadata for `uids` in `mailbox` — no body bytes.
///
/// See the [module docs](self) for why this does not chunk `uids` itself.
/// `changed_since` requires the server to speak `CONDSTORE`
/// ([`BackendError::Unsupported`] otherwise) and selects the mailbox with
/// `(CONDSTORE)` so `CHANGEDSINCE` is legal on it (RFC 7162 §3.3.1).
pub async fn fetch_headers(
    pool: &ConnectionPool,
    mailbox: &str,
    uids: &UidSet,
    changed_since: Option<ModSeq>,
    priority: Priority,
    cancel: &CancelToken,
) -> BackendResult<Vec<FetchedMessage>> {
    if uids.is_empty() {
        return Ok(Vec::new());
    }
    if cancel.is_cancelled() {
        return Err(BackendError::Cancelled);
    }

    let mailbox = mailbox.to_owned();
    let uids = uids.clone();

    pool.execute(priority, async |session| {
        let uid_validity = session
            .ensure_selected(&mailbox, changed_since.is_some())
            .await?;

        if cancel.is_cancelled() {
            return Err(BackendError::Cancelled);
        }

        fetch_batch(session, &mailbox, uid_validity, &uids, changed_since).await
    })
    .await
}

/// Issues one `UID FETCH` and maps the response.
///
/// When `changed_since` is set this is an incremental resync's fetch, and
/// [`skip_counter`] brackets the round trip: `io-imap` completing `Ok` after
/// silently dropping an untagged line it could not decode is exactly the
/// case a plain result can't distinguish from a genuinely complete pull. See
/// the [module docs](self) and [`crate::backend::BackendError::ResyncIntegrityLost`].
async fn fetch_batch(
    session: &mut ImapSession,
    mailbox: &str,
    uid_validity: UidValidity,
    uids: &UidSet,
    changed_since: Option<ModSeq>,
) -> BackendResult<Vec<FetchedMessage>> {
    if changed_since.is_none() {
        return fetch_batch_inner(session, uid_validity, uids, changed_since).await;
    }

    skip_counter::install();
    let _exclusive = skip_counter::exclusive_measurement().await;
    let before_skips = skip_counter::skipped_untagged_responses();

    let messages = fetch_batch_inner(session, uid_validity, uids, changed_since).await?;

    let skipped = skip_counter::skipped_untagged_responses() - before_skips;
    if skipped > 0 {
        return Err(BackendError::ResyncIntegrityLost {
            mailbox: mailbox.to_owned(),
            skipped,
        });
    }

    Ok(messages)
}

async fn fetch_batch_inner(
    session: &mut ImapSession,
    uid_validity: UidValidity,
    uids: &UidSet,
    changed_since: Option<ModSeq>,
) -> BackendResult<Vec<FetchedMessage>> {
    let sequence_set = sequence_set_for(uids)?;
    let condstore = session.capabilities().contains(Capability::CondStore);

    let mut item_names = vec![
        MessageDataItemName::Uid,
        MessageDataItemName::Flags,
        MessageDataItemName::InternalDate,
        MessageDataItemName::Rfc822Size,
        MessageDataItemName::Envelope,
        MessageDataItemName::BodyStructure,
        references_item(),
        list_id_item(),
    ];
    if condstore {
        item_names.push(MessageDataItemName::ModSeq);
    }

    let modifiers = match changed_since {
        Some(mod_seq) => vec![FetchModifier::ChangedSince(non_zero_mod_seq(mod_seq)?)],
        None => Vec::new(),
    };

    let opts = ImapMessageFetchOptions {
        uid: true,
        modifiers,
    };

    let raw = session
        .fetch(
            sequence_set,
            MacroOrMessageDataItemNames::MessageDataItemNames(item_names),
            opts,
        )
        .await;
    let raw = raw.map_err(|error| session.command_error("FETCH", error))?;

    raw.into_values()
        .map(|items| build_fetched_message(items, uid_validity))
        .collect()
}

fn sequence_set_for(uids: &UidSet) -> BackendResult<SequenceSet> {
    SequenceSet::try_from(uids.to_sequence_set().as_str()).map_err(|error| BackendError::Protocol {
        reason: format!("{uids} is not a sequence set IMAP can carry: {error}"),
    })
}

fn non_zero_mod_seq(mod_seq: ModSeq) -> BackendResult<NonZeroU64> {
    NonZeroU64::new(mod_seq.get()).ok_or_else(|| BackendError::Protocol {
        reason: "CHANGEDSINCE requires a non-zero modification sequence".to_owned(),
    })
}

/// `BODY.PEEK[HEADER.FIELDS (REFERENCES)]` — see the [module docs](self).
fn references_item() -> MessageDataItemName<'static> {
    MessageDataItemName::BodyExt {
        section: Some(references_section()),
        partial: None,
        peek: true,
    }
}

fn references_section() -> Section<'static> {
    Section::HeaderFields(
        None,
        Vec1::from(AString::try_from("REFERENCES").expect("REFERENCES is a valid AString")),
    )
}

/// `BODY.PEEK[HEADER.FIELDS (LIST-ID)]`, alongside `ENVELOPE` for the same
/// reason `references_item` is: RFC 3501's `ENVELOPE` has no field for it,
/// but it is what lets a mailing list be detected without the user naming
/// it anywhere (#9). A field of its own rather than folded into the same
/// `HEADER.FIELDS` request as `REFERENCES`: the server may return the
/// requested headers in either order in one raw block, and
/// `references_from_header`'s own parsing assumes the block holds exactly
/// one header. Two items in one `FETCH` command is still one round trip.
fn list_id_item() -> MessageDataItemName<'static> {
    MessageDataItemName::BodyExt {
        section: Some(list_id_section()),
        partial: None,
        peek: true,
    }
}

fn list_id_section() -> Section<'static> {
    Section::HeaderFields(
        None,
        Vec1::from(AString::try_from("LIST-ID").expect("LIST-ID is a valid AString")),
    )
}

// ---------------------------------------------------------------------------
// Response mapping
// ---------------------------------------------------------------------------

fn build_fetched_message(
    items: Vec1<MessageDataItem<'static>>,
    uid_validity: UidValidity,
) -> BackendResult<FetchedMessage> {
    let mut uid = None;
    let mut mod_seq = None;
    let mut flags = FlagSet::new();
    let mut internal_date = None;
    let mut size = 0u64;
    let mut envelope = None;
    let mut structure = None;
    let mut references = Vec::new();
    let mut list_id = None;

    for item in items {
        match item {
            MessageDataItem::Uid(value) => uid = Some(Uid::new(value.get())),
            MessageDataItem::Flags(list) => {
                flags = list.into_iter().map(flag_from_wire).collect();
            }
            MessageDataItem::InternalDate(date) => {
                internal_date = Some(date.as_ref().with_timezone(&chrono::Utc));
            }
            MessageDataItem::Rfc822Size(value) => size = u64::from(value),
            MessageDataItem::ModSeq(value) => mod_seq = Some(ModSeq::new(value.get())),
            MessageDataItem::Envelope(wire) => envelope = Some(wire),
            MessageDataItem::BodyStructure(wire) => structure = Some(wire),
            MessageDataItem::BodyExt { section, data, .. }
                if section == Some(list_id_section()) =>
            {
                list_id = list_id_from_header(nstring_to_string(data).as_deref());
            }
            MessageDataItem::BodyExt { data, .. } => {
                references = references_from_header(nstring_to_string(data).as_deref());
            }
            _ => {}
        }
    }

    let uid = uid.ok_or_else(|| BackendError::Protocol {
        reason: "the server's FETCH response carried no UID".to_owned(),
    })?;
    let internal_date = internal_date.ok_or_else(|| BackendError::Protocol {
        reason: format!("the server's FETCH response for UID {uid} carried no INTERNALDATE"),
    })?;

    Ok(FetchedMessage {
        remote_id: crate::backend::identity::remote_id(uid_validity, uid),
        uid,
        uid_validity,
        mod_seq,
        flags,
        internal_date,
        size,
        envelope: envelope.map(|wire| envelope_from_wire(wire, references.split_off(0), list_id)),
        structure: structure.map(|wire| body_structure_from_wire(&wire)),
    })
}

pub(super) fn flag_from_wire(flag: FlagFetch<'static>) -> Flag {
    match flag {
        FlagFetch::Flag(flag) => Flag::parse(flag.to_string()),
        FlagFetch::Recent => Flag::Recent,
    }
}

fn nstring_to_string(value: NString<'static>) -> Option<String> {
    value
        .into_option()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

/// As [`nstring_to_string`], for a field that carries prose rather than a
/// structured token — the subject, or a display name — and so can carry an
/// RFC 2047 encoded word: a sender's client folds non-ASCII into the
/// envelope this way, and the ENVELOPE response hands it over undecoded.
/// `mailbox`, `host`, message ids and the date stay on
/// [`nstring_to_string`]; none of them is header prose an encoder ever
/// touches.
fn nstring_to_header_text(value: NString<'static>) -> Option<String> {
    value
        .into_option()
        .map(|bytes| postio_model::mime::decode_header_text(&bytes))
}

fn istring_to_string(value: &IString<'static>) -> String {
    String::from_utf8_lossy(&value.clone().into_inner()).into_owned()
}

/// Pulls `Message-ID`-shaped tokens out of a raw, possibly folded
/// `References:` header line.
fn references_from_header(raw: Option<&str>) -> Vec<RfcMessageId> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    let Some((_, value)) = raw.split_once(':') else {
        return Vec::new();
    };
    value
        .split_whitespace()
        .filter(|token| token.starts_with('<'))
        .map(RfcMessageId::new)
        .collect()
}

/// Pulls the bracketed identifier out of a raw `List-Id:` header line — the
/// same reduction [`postio_model::mime::list_id_from_text`] does for a full
/// message parse, so a row shows the same list whether it arrived through
/// `ENVELOPE`'s companion fetch or through a fully parsed body.
fn list_id_from_header(raw: Option<&str>) -> Option<String> {
    let (_, value) = raw?.split_once(':')?;
    postio_model::mime::list_id_from_text(value)
}

fn parse_rfc2822(text: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc2822(text.trim())
        .ok()
        .map(|date| date.with_timezone(&chrono::Utc))
}

fn envelope_from_wire(
    wire: WireEnvelope<'static>,
    references: Vec<RfcMessageId>,
    list_id: Option<String>,
) -> Envelope {
    Envelope {
        date: nstring_to_string(wire.date).and_then(|text| parse_rfc2822(&text)),
        subject: nstring_to_header_text(wire.subject),
        from: wire
            .from
            .into_iter()
            .filter_map(address_from_wire)
            .collect(),
        sender: wire.sender.into_iter().find_map(address_from_wire),
        reply_to: wire
            .reply_to
            .into_iter()
            .filter_map(address_from_wire)
            .collect(),
        to: wire.to.into_iter().filter_map(address_from_wire).collect(),
        cc: wire.cc.into_iter().filter_map(address_from_wire).collect(),
        bcc: wire.bcc.into_iter().filter_map(address_from_wire).collect(),
        message_id: nstring_to_string(wire.message_id).map(RfcMessageId::new),
        in_reply_to: nstring_to_string(wire.in_reply_to).map(RfcMessageId::new),
        references,
        list_id,
    }
}

/// `None` for RFC 2822 group syntax (`host` `NIL`), which is not an address.
fn address_from_wire(address: WireAddress<'static>) -> Option<EmailAddress> {
    let mailbox = nstring_to_string(address.mailbox)?;
    let host = nstring_to_string(address.host)?;
    let name = nstring_to_header_text(address.name);
    Some(EmailAddress::new(name, format!("{mailbox}@{host}")))
}

// ---------------------------------------------------------------------------
// BODYSTRUCTURE flattening
// ---------------------------------------------------------------------------

fn body_structure_from_wire(wire: &WireBodyStructure<'static>) -> BodyStructure {
    let mut parts = Vec::new();
    flatten(wire, &[], &mut parts);
    BodyStructure::from_parts(root_content_type(wire), parts)
}

/// The message's own content type — `BODYSTRUCTURE`'s root, not any one
/// part's.
///
/// `Multi`'s own `subtype` (`mixed`, `alternative`, `related`, …) is exactly
/// this and nothing else carries it: [`flatten`] only ever numbers and
/// records the *children*, so a multipart message's own type has no other
/// way out of the wire structure.
fn root_content_type(wire: &WireBodyStructure<'static>) -> String {
    match wire {
        WireBodyStructure::Multi { subtype, .. } => {
            format!("multipart/{}", istring_to_string(subtype))
        }
        WireBodyStructure::Single { body, .. } => {
            let (media_type, subtype) = mime_type_of(&body.specific);
            format!("{media_type}/{subtype}")
        }
    }
}

/// Walks one body structure, assigning RFC 3501 §6.4.5 section numbers.
///
/// `path` is the section number of the closest enclosing part, as path
/// components — `[]` at the top of the message, `[3]` for the third child of
/// a top-level multipart, and so on. See the [module docs](self).
fn flatten(structure: &WireBodyStructure<'static>, path: &[u32], out: &mut Vec<PartNode>) {
    match structure {
        WireBodyStructure::Multi { bodies, .. } => {
            for (index, child) in bodies.as_ref().iter().enumerate() {
                let mut child_path = path.to_vec();
                child_path.push(u32::try_from(index + 1).unwrap_or(u32::MAX));
                flatten(child, &child_path, out);
            }
        }
        WireBodyStructure::Single {
            body,
            extension_data,
        } => {
            let section = section_string(path);
            out.push(part_node(&section, body, extension_data.as_ref()));

            if let SpecificFields::Message { body_structure, .. } = &body.specific {
                flatten_encapsulated(body_structure, path, out);
            }
        }
    }
}

/// A `MESSAGE/RFC822` part's own body is numbered one level deeper than the
/// part that encloses it — `3.1`, `3.2`, … for `3` — never reusing `3`
/// itself, which the wrapper already claimed.
fn flatten_encapsulated(
    structure: &WireBodyStructure<'static>,
    path: &[u32],
    out: &mut Vec<PartNode>,
) {
    match structure {
        WireBodyStructure::Multi { .. } => flatten(structure, path, out),
        WireBodyStructure::Single { .. } => {
            let mut child_path = path.to_vec();
            child_path.push(1);
            flatten(structure, &child_path, out);
        }
    }
}

fn section_string(path: &[u32]) -> String {
    if path.is_empty() {
        "1".to_owned()
    } else {
        path.iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(".")
    }
}

fn part_node(
    section: &str,
    body: &WireBody<'static>,
    extension: Option<&io_imap::types::body::SinglePartExtensionData<'static>>,
) -> PartNode {
    let (media_type, subtype) = mime_type_of(&body.specific);
    let mut node = PartNode::new(section, format!("{media_type}/{subtype}"), basic_size(body));

    node = node.with_encoding(istring_to_string(&body.basic.content_transfer_encoding));

    if let Some(charset) = parameter(&body.basic.parameter_list, "charset") {
        node = node.with_charset(charset);
    }
    if let Some(id) = nstring_to_string(body.basic.id.clone()) {
        node = node.with_content_id(id);
    }

    if let Some((kind, params)) = disposition_of(extension) {
        node = node.with_disposition(disposition_from(&kind));
        if let Some(filename) = parameter(params, "filename") {
            node = node.with_filename(filename);
        }
    }

    if node.filename().is_none()
        && let Some(name) = parameter(&body.basic.parameter_list, "name")
    {
        node = node.with_filename(name);
    }

    node
}

fn basic_size(body: &WireBody<'static>) -> u64 {
    u64::from(body.basic.size)
}

fn mime_type_of(specific: &SpecificFields<'static>) -> (String, String) {
    match specific {
        SpecificFields::Basic { r#type, subtype } => {
            (istring_to_string(r#type), istring_to_string(subtype))
        }
        SpecificFields::Message { .. } => ("message".to_owned(), "rfc822".to_owned()),
        SpecificFields::Text { subtype, .. } => ("text".to_owned(), istring_to_string(subtype)),
    }
}

type DispositionParams<'a> = &'a [(IString<'static>, IString<'static>)];

fn disposition_of<'a>(
    extension: Option<&'a io_imap::types::body::SinglePartExtensionData<'static>>,
) -> Option<(String, DispositionParams<'a>)> {
    let disposition: &'a WireDisposition<'static> = extension?.tail.as_ref()?;
    let (kind, params) = disposition.disposition.as_ref()?;
    Some((istring_to_string(kind), params.as_slice()))
}

fn parameter(list: &[(IString<'static>, IString<'static>)], key: &str) -> Option<String> {
    list.iter()
        .find(|(name, _)| istring_to_string(name).eq_ignore_ascii_case(key))
        .map(|(_, value)| istring_to_string(value))
}

fn disposition_from(kind: &str) -> Disposition {
    match kind.to_ascii_lowercase().as_str() {
        "inline" => Disposition::Inline,
        "attachment" => Disposition::Attachment,
        other => Disposition::Other(other.to_owned()),
    }
}

/// Every UID that exists in `mailbox`, via `UID SEARCH ALL`.
///
/// The cheap answer to "what is actually here", and what stops a first sync
/// walking a UID space that is mostly gaps. See
/// [`MailBackend::existing_uids`](crate::backend::MailBackend::existing_uids)
/// for why that matters and #727 for what it cost.
///
/// `SEARCH` is in RFC 3501 — it is not an extension and there is no
/// capability to check — but a server may still refuse a given one, and the
/// caller treats any failure as "walk instead" rather than as a failed sync.
/// Returning `Ok(None)` is therefore reserved for a server that answered and
/// said nothing useful; a refusal comes back as `Err` and is downgraded one
/// layer up, where the decision to fall back belongs.
pub async fn existing_uids(
    pool: &ConnectionPool,
    mailbox: &str,
    priority: Priority,
    cancel: &CancelToken,
) -> BackendResult<Option<Vec<Uid>>> {
    use io_imap::rfc3501::search::ImapMessageSearchOptions;
    use io_imap::types::core::Vec1;
    use io_imap::types::search::SearchKey;

    if cancel.is_cancelled() {
        return Err(BackendError::Cancelled);
    }
    let mailbox = mailbox.to_owned();

    pool.execute(priority, async |session| {
        session.ensure_selected(&mailbox, false).await?;
        if cancel.is_cancelled() {
            return Err(BackendError::Cancelled);
        }
        let found = session
            .search(
                Vec1::from(SearchKey::All),
                ImapMessageSearchOptions { uid: true },
            )
            .await
            .map_err(|error| session.command_error("SEARCH", error))?;
        Ok(Some(
            found.into_iter().map(|uid| Uid::new(uid.get())).collect(),
        ))
    })
    .await
}

#[cfg(test)]
mod tests {
    use io_imap::types::body::{BasicFields, MultiPartExtensionData};
    use io_imap::types::core::NString;

    use super::*;

    fn basic_fields(size: u32) -> BasicFields<'static> {
        BasicFields {
            parameter_list: Vec::new(),
            id: NString::NIL,
            description: NString::NIL,
            content_transfer_encoding: IString::try_from("7BIT").unwrap(),
            size,
        }
    }

    fn text_part(size: u32) -> WireBodyStructure<'static> {
        WireBodyStructure::Single {
            body: WireBody {
                basic: BasicFields {
                    parameter_list: vec![(
                        IString::try_from("charset").unwrap(),
                        IString::try_from("us-ascii").unwrap(),
                    )],
                    ..basic_fields(size)
                },
                specific: SpecificFields::Text {
                    subtype: IString::try_from("plain").unwrap(),
                    number_of_lines: 3,
                },
            },
            extension_data: None,
        }
    }

    fn attachment_part(filename: &str, size: u32) -> WireBodyStructure<'static> {
        WireBodyStructure::Single {
            body: WireBody {
                basic: BasicFields {
                    parameter_list: vec![(
                        IString::try_from("name").unwrap(),
                        IString::try_from(filename.to_owned()).unwrap(),
                    )],
                    ..basic_fields(size)
                },
                specific: SpecificFields::Basic {
                    r#type: IString::try_from("application").unwrap(),
                    subtype: IString::try_from("pdf").unwrap(),
                },
            },
            extension_data: Some(io_imap::types::body::SinglePartExtensionData {
                md5: NString::NIL,
                tail: Some(WireDisposition {
                    disposition: Some((
                        IString::try_from("attachment").unwrap(),
                        vec![(
                            IString::try_from("filename").unwrap(),
                            IString::try_from(filename.to_owned()).unwrap(),
                        )],
                    )),
                    tail: None,
                }),
            }),
        }
    }

    fn multipart(children: Vec<WireBodyStructure<'static>>) -> WireBodyStructure<'static> {
        WireBodyStructure::Multi {
            bodies: Vec1::try_from(children).expect("at least one child"),
            subtype: IString::try_from("mixed").unwrap(),
            extension_data: None::<MultiPartExtensionData<'static>>,
        }
    }

    #[test]
    fn a_simple_single_part_message_is_section_one() {
        let structure = body_structure_from_wire(&text_part(120));

        let parts = structure.parts();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].section(), "1");
        assert_eq!(parts[0].mime_type(), "text/plain");
        assert_eq!(parts[0].charset(), Some("us-ascii"));
        assert_eq!(parts[0].size(), 120);
        assert!(parts[0].is_body_text());
    }

    #[test]
    fn a_single_part_message_s_own_content_type_is_its_one_part_s() {
        let structure = body_structure_from_wire(&text_part(120));
        assert_eq!(structure.content_type(), "text/plain");
    }

    #[test]
    fn a_multipart_message_s_own_content_type_carries_the_wire_subtype() {
        // The bug this covers: `flatten` used to discard `Multi`'s own
        // `subtype` (`WireBodyStructure::Multi { bodies, .. }`), so every
        // multipart message read back as `multipart/mixed` regardless of
        // what the server actually sent -- wrong for `multipart/related`
        // (inline images) and `multipart/alternative` alike.
        let alternative = WireBodyStructure::Multi {
            bodies: Vec1::try_from(vec![text_part(10), attachment_part("report.pdf", 20)])
                .expect("at least one child"),
            subtype: IString::try_from("alternative").unwrap(),
            extension_data: None::<MultiPartExtensionData<'static>>,
        };
        let structure = body_structure_from_wire(&alternative);
        assert_eq!(structure.content_type(), "multipart/alternative");

        let mixed = body_structure_from_wire(&multipart(vec![
            text_part(10),
            attachment_part("report.pdf", 20),
        ]));
        assert_eq!(mixed.content_type(), "multipart/mixed");
    }

    #[test]
    fn a_multipart_numbers_its_children_from_one_not_one_dot_one() {
        let structure = body_structure_from_wire(&multipart(vec![
            text_part(51),
            attachment_part("report.pdf", 4554),
        ]));

        let sections: Vec<&str> = structure.parts().iter().map(|p| p.section()).collect();
        assert_eq!(sections, ["1", "2"]);
    }

    #[test]
    fn an_attachment_is_named_from_the_disposition_filename_and_known_before_any_body_fetch() {
        let structure = body_structure_from_wire(&multipart(vec![
            text_part(51),
            attachment_part("report.pdf", 4554),
        ]));

        let attachment = structure
            .parts()
            .iter()
            .find(|part| !part.is_body_text())
            .expect("an attachment part");

        assert_eq!(attachment.section(), "2");
        assert_eq!(attachment.mime_type(), "application/pdf");
        assert_eq!(attachment.filename(), Some("report.pdf"));
        assert_eq!(attachment.disposition(), &Disposition::Attachment);
        assert_eq!(attachment.size(), 4554);
    }

    #[test]
    fn a_filename_falls_back_to_the_basic_name_parameter_without_a_disposition() {
        let mut part = attachment_part("ignored.pdf", 10);
        if let WireBodyStructure::Single { extension_data, .. } = &mut part {
            *extension_data = None;
        }
        let structure = body_structure_from_wire(&part);

        assert_eq!(structure.parts()[0].filename(), Some("ignored.pdf"));
    }

    #[test]
    fn an_encapsulated_message_numbers_its_own_parts_one_level_deeper() {
        let inner = multipart(vec![text_part(31), attachment_part("nested.bin", 12)]);
        let wrapper = WireBodyStructure::Single {
            body: WireBody {
                basic: basic_fields(200),
                specific: SpecificFields::Message {
                    envelope: Box::new(WireEnvelope {
                        date: NString::NIL,
                        subject: NString::NIL,
                        from: Vec::new(),
                        sender: Vec::new(),
                        reply_to: Vec::new(),
                        to: Vec::new(),
                        cc: Vec::new(),
                        bcc: Vec::new(),
                        in_reply_to: NString::NIL,
                        message_id: NString::NIL,
                    }),
                    body_structure: Box::new(inner),
                    number_of_lines: 6,
                },
            },
            extension_data: None,
        };
        let structure = body_structure_from_wire(&multipart(vec![text_part(10), wrapper]));

        let sections: Vec<&str> = structure.parts().iter().map(|p| p.section()).collect();
        assert_eq!(sections, ["1", "2", "2.1", "2.2"]);
    }

    #[test]
    fn a_folded_references_header_yields_every_message_id() {
        let raw = "References: <a@example.com>\r\n <b@example.com>\r\n";
        let ids = references_from_header(Some(raw));

        assert_eq!(
            ids,
            vec![
                RfcMessageId::new("<a@example.com>"),
                RfcMessageId::new("<b@example.com>"),
            ]
        );
    }

    #[test]
    fn a_missing_references_header_is_an_empty_list_not_an_error() {
        assert_eq!(references_from_header(None), Vec::new());
    }

    #[test]
    fn a_group_syntax_address_is_not_reported_as_an_email_address() {
        let group_start = WireAddress {
            name: NString::try_from("A Group").unwrap(),
            adl: NString::NIL,
            mailbox: NString::NIL,
            host: NString::NIL,
        };
        assert_eq!(address_from_wire(group_start), None);
    }

    #[test]
    fn an_ordinary_address_keeps_its_display_name() {
        let address = WireAddress {
            name: NString::try_from("Ada Lovelace").unwrap(),
            adl: NString::NIL,
            mailbox: NString::try_from("ada").unwrap(),
            host: NString::try_from("example.com").unwrap(),
        };

        let mapped = address_from_wire(address).expect("a real address");
        assert_eq!(mapped.name.as_deref(), Some("Ada Lovelace"));
        assert_eq!(mapped.address, "ada@example.com");
    }

    #[test]
    fn an_encoded_word_display_name_decodes_rather_than_reaching_the_row_raw() {
        // Base64 UTF-8 for "Café" — a display name a sender's client folded
        // into RFC 2047 rather than sending as raw UTF-8.
        let address = WireAddress {
            name: NString::try_from("=?UTF-8?B?Q2Fmw6k=?=").unwrap(),
            adl: NString::NIL,
            mailbox: NString::try_from("cafe").unwrap(),
            host: NString::try_from("example.com").unwrap(),
        };

        let mapped = address_from_wire(address).expect("a real address");
        assert_eq!(mapped.name.as_deref(), Some("Café"));
    }

    fn empty_envelope() -> WireEnvelope<'static> {
        WireEnvelope {
            date: NString::NIL,
            subject: NString::NIL,
            from: Vec::new(),
            sender: Vec::new(),
            reply_to: Vec::new(),
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            in_reply_to: NString::NIL,
            message_id: NString::NIL,
        }
    }

    #[test]
    fn an_encoded_word_subject_is_decoded_not_shown_raw() {
        let wire = WireEnvelope {
            subject: NString::try_from("=?UTF-8?B?Q2Fmw6k=?=").unwrap(),
            ..empty_envelope()
        };

        let envelope = envelope_from_wire(wire, Vec::new(), None);
        assert_eq!(envelope.subject.as_deref(), Some("Café"));
    }

    #[test]
    fn a_list_id_header_line_is_reduced_to_its_bracketed_identifier() {
        assert_eq!(
            list_id_from_header(Some("List-Id: Harbour dev <harbour-dev.lists.example.org>")),
            Some("harbour-dev.lists.example.org".to_string())
        );
        assert_eq!(list_id_from_header(None), None);
    }

    #[test]
    fn a_date_header_the_sender_claimed_parses_as_rfc_2822() {
        let parsed = parse_rfc2822("Mon, 7 Feb 1994 21:52:25 -0800").expect("a valid date");
        assert_eq!(parsed.to_rfc3339(), "1994-02-08T05:52:25+00:00");
    }

    #[test]
    fn an_unparseable_date_is_none_not_an_error() {
        assert_eq!(parse_rfc2822("not a date"), None);
    }
}
