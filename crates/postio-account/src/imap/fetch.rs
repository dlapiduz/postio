//! One `UID FETCH` per call for the given `UidSet`; batching is the caller's (`UidSet::chunks`).
//! Every fetch also asks `BODY.PEEK[HEADER.FIELDS (REFERENCES)]`, since `ENVELOPE` lacks it and JWZ
//! threading needs it; `.PEEK` so reading headers never sets `\Seen`.
//! `BODYSTRUCTURE` is flattened with RFC 3501 §6.4.5 section numbers; `MESSAGE/RFC822` content
//! numbers one level deeper.

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

use super::{ConnectionPool, ImapSession, Priority, undecoded, whole_messages};

/// `changed_since` requires CONDSTORE (`Unsupported` otherwise) and selects with `(CONDSTORE)` so
/// `CHANGEDSINCE` is legal (RFC 7162 §3.3.1).
pub async fn fetch_headers(
    pool: &ConnectionPool,
    mailbox: &str,
    uids: &UidSet,
    changed_since: Option<ModSeq>,
    priority: Priority,
    cancel: &CancelToken,
) -> BackendResult<Vec<FetchedMessage>> {
    fetch(
        pool,
        mailbox,
        uids,
        changed_since,
        Promoted::NotAsked,
        priority,
        cancel,
    )
    .await
}

/// Adds the three promoted headers to the same command; a first sync's `fetch_headers` does not,
/// since every enumerated message would pay for them (ADR 0025).
pub async fn fetch_headers_for_filing(
    pool: &ConnectionPool,
    mailbox: &str,
    uids: &UidSet,
    changed_since: Option<ModSeq>,
    priority: Priority,
    cancel: &CancelToken,
) -> BackendResult<Vec<FetchedMessage>> {
    fetch(
        pool,
        mailbox,
        uids,
        changed_since,
        Promoted::Asked,
        priority,
        cancel,
    )
    .await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Promoted {
    NotAsked,
    Asked,
}

async fn fetch(
    pool: &ConnectionPool,
    mailbox: &str,
    uids: &UidSet,
    changed_since: Option<ModSeq>,
    promoted: Promoted,
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

        fetch_batch(
            session,
            &mailbox,
            uid_validity,
            &uids,
            changed_since,
            promoted,
        )
        .await
    })
    .await
}

/// With `changed_since`, a response io-imap could not wholly decode fails with
/// `ResyncIntegrityLost`: an `Ok` with a line missing is indistinguishable from a complete pull.
async fn fetch_batch(
    session: &mut ImapSession,
    mailbox: &str,
    uid_validity: UidValidity,
    uids: &UidSet,
    changed_since: Option<ModSeq>,
    promoted: Promoted,
) -> BackendResult<Vec<FetchedMessage>> {
    let (messages, skipped) =
        fetch_batch_inner(session, uid_validity, uids, changed_since, promoted).await?;

    if changed_since.is_some() && skipped > 0 {
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
    promoted: Promoted,
) -> BackendResult<(Vec<FetchedMessage>, u64)> {
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
    if promoted == Promoted::Asked {
        item_names.push(promoted_item());
    }
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
    let (raw, skipped) = whole_messages(raw);

    let mut messages = Vec::new();
    for items in raw.into_values() {
        if let Some(message) = build_fetched_message(items, uid_validity)?
            && uids.contains(message.uid)
        {
            messages.push(message);
        }
    }
    Ok((messages, skipped))
}

/// Checked for undecoded lines like the `CHANGEDSINCE` fetch: a sweep that silently lost a
/// message's flags defeats its purpose; fails with `ResyncIntegrityLost`.
/// Selected without `(CONDSTORE)`: nothing here asks about modification sequences.
pub async fn fetch_flags(
    pool: &ConnectionPool,
    mailbox: &str,
    priority: Priority,
    cancel: &CancelToken,
) -> BackendResult<Option<Vec<(Uid, FlagSet)>>> {
    if cancel.is_cancelled() {
        return Err(BackendError::Cancelled);
    }
    let mailbox = mailbox.to_owned();

    pool.execute(priority, async |session| {
        session.ensure_selected(&mailbox, false).await?;
        if cancel.is_cancelled() {
            return Err(BackendError::Cancelled);
        }

        let sequence_set = sequence_set_for(&UidSet::all())?;
        let raw = session
            .fetch(
                sequence_set,
                MacroOrMessageDataItemNames::MessageDataItemNames(vec![
                    MessageDataItemName::Uid,
                    MessageDataItemName::Flags,
                ]),
                ImapMessageFetchOptions {
                    uid: true,
                    modifiers: Vec::new(),
                },
            )
            .await
            .map_err(|error| session.command_error("FETCH", error))?;
        let (raw, skipped) = whole_messages(raw);
        if skipped > 0 {
            return Err(BackendError::ResyncIntegrityLost {
                mailbox: mailbox.clone(),
                skipped,
            });
        }

        Ok(Some(
            raw.into_values()
                .filter_map(|items| flags_of(items.into_iter()))
                .collect(),
        ))
    })
    .await
}

fn flags_of(items: impl Iterator<Item = MessageDataItem<'static>>) -> Option<(Uid, FlagSet)> {
    let mut uid = None;
    let mut flags = FlagSet::new();
    for item in items {
        match item {
            MessageDataItem::Uid(value) => uid = Some(Uid::new(value.get())),
            MessageDataItem::Flags(list) => {
                flags = list.into_iter().map(flag_from_wire).collect();
            }
            _ => {}
        }
    }
    uid.map(|uid| (uid, flags))
}

/// Only the short direction counts: a listing longer than `EXISTS` is the ordinary race with new
/// mail, and refusing on it would discard healthy connections.
fn listing_was_misread(listed: usize, exists: u32) -> bool {
    (listed as u64) < u64::from(exists)
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

/// A separate item from `REFERENCES`: the server may return the headers in either order, and
/// `references_from_header` assumes the block holds exactly one header (#9).
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

/// A separate item, since the single-field parsers assume a one-header block; this block is read
/// whole (spec 007, R8).
fn promoted_item() -> MessageDataItemName<'static> {
    MessageDataItemName::BodyExt {
        section: Some(promoted_section()),
        partial: None,
        peek: true,
    }
}

fn promoted_section() -> Section<'static> {
    let names: Vec<AString<'static>> = postio_model::promoted::FIELDS
        .iter()
        .map(|name| {
            AString::try_from(name.to_ascii_uppercase()).expect("a header name is a valid AString")
        })
        .collect();
    Section::HeaderFields(
        None,
        Vec1::try_from(names).expect("the promoted fields are not an empty list"),
    )
}

/// A flag change on another message can ride in the same reply (RFC 3501 §7.4.2): an entry with
/// neither `INTERNALDATE` nor `ENVELOPE` is passed over, not an error. Treating it as one failed a
/// real archive's whole first sync; the next incremental pass reads the flag.
fn build_fetched_message(
    items: Vec1<MessageDataItem<'static>>,
    uid_validity: UidValidity,
) -> BackendResult<Option<FetchedMessage>> {
    let mut uid = None;
    let mut mod_seq = None;
    let mut flags = FlagSet::new();
    let mut internal_date = None;
    let mut size = 0u64;
    let mut envelope = None;
    let mut structure = None;
    let mut references = Vec::new();
    let mut list_id = None;
    let mut promoted = None;

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
            // Asked for, so known: a block with none of the three is a message that says none.
            MessageDataItem::BodyExt { section, data, .. }
                if section == Some(promoted_section()) =>
            {
                let block = nstring_to_string(data).unwrap_or_default();
                promoted = Some(postio_model::promoted::PromotedHeaders::from_headers(
                    &postio_model::headers::parse_block(block.trim_end()),
                ));
            }
            MessageDataItem::BodyExt { data, .. } => {
                references = references_from_header(nstring_to_string(data).as_deref());
            }
            _ => {}
        }
    }

    if internal_date.is_none() && envelope.is_none() {
        return Ok(None);
    }
    let uid = uid.ok_or_else(|| BackendError::Protocol {
        reason: "the server's FETCH response carried no UID".to_owned(),
    })?;
    let internal_date = internal_date.ok_or_else(|| BackendError::Protocol {
        reason: format!("the server's FETCH response for UID {uid} carried no INTERNALDATE"),
    })?;

    Ok(Some(FetchedMessage {
        content_identity: None,
        remote_id: crate::backend::identity::remote_id(uid_validity, uid),
        uid,
        uid_validity,
        mod_seq,
        flags,
        internal_date,
        size,
        envelope: envelope.map(|wire| envelope_from_wire(wire, references.split_off(0), list_id)),
        structure: structure.map(|wire| body_structure_from_wire(&wire)),
        promoted,
    }))
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

/// For prose fields (subject, display name) that can carry an RFC 2047 encoded word; ids and the
/// date stay on `nstring_to_string`.
fn nstring_to_header_text(value: NString<'static>) -> Option<String> {
    value
        .into_option()
        .map(|bytes| postio_model::mime::decode_header_text(&bytes))
}

fn istring_to_string(value: &IString<'static>) -> String {
    String::from_utf8_lossy(&value.clone().into_inner()).into_owned()
}

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

/// The same reduction `postio_model::mime::list_id_from_text` does for a full parse, so a row shows
/// the same list either way.
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

fn body_structure_from_wire(wire: &WireBodyStructure<'static>) -> BodyStructure {
    let mut parts = Vec::new();
    flatten(wire, &[], &mut parts);
    BodyStructure::from_parts(root_content_type(wire), parts)
}

/// `Multi`'s own `subtype` is the only carrier of a multipart's type; `flatten` records only
/// children.
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

/// Numbered one level deeper than the enclosing part (`3.1`, `3.2`), never reusing `3`.
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
        if let Some(filename) = decoded_parameter(params, "filename") {
            node = node.with_filename(filename);
        }
    }

    if node.filename().is_none()
        && let Some(name) = decoded_parameter(&body.basic.parameter_list, "name")
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

fn decoded_parameter(list: &[(IString<'static>, IString<'static>)], key: &str) -> Option<String> {
    let pairs: Vec<(String, String)> = list
        .iter()
        .map(|(name, value)| (istring_to_string(name), istring_to_string(value)))
        .collect();
    postio_model::mime::parameter_value(&pairs, key)
}

fn disposition_from(kind: &str) -> Disposition {
    match kind.to_ascii_lowercase().as_str() {
        "inline" => Disposition::Inline,
        "attachment" => Disposition::Attachment,
        other => Disposition::Other(other.to_owned()),
    }
}

/// A server may refuse a given `SEARCH`; the caller treats any failure as "walk instead".
/// `Ok(None)` is reserved for a server that answered with nothing useful (#727).
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
        // Checked for undecoded lines: io-imap drops an unparseable `* SEARCH` line and completes
        // `Ok`, silently removing its UIDs; an empty result looks like an empty mailbox (a 60,934
        // message Archive listed nothing).
        let found = session
            .search(
                Vec1::from(SearchKey::All),
                ImapMessageSearchOptions { uid: true },
            )
            .await
            .map_err(|error| session.command_error("SEARCH", error))?;
        let skipped = undecoded(&found);
        let found = found.data;

        if skipped > 0 {
            return Err(BackendError::Disconnected {
                context: format!("SEARCH on {mailbox}"),
                reason: format!(
                    "{skipped} untagged response(s) could not be decoded, so the \
                     UID listing is missing whatever they carried"
                ),
            });
        }

        // A listing short of `EXISTS` is a misread, not an answer (iCloud returned nothing for a
        // 60,934-message Archive). Refused as a transient error so `ConnectionPool::execute`
        // discards the connection: `ImapSession` keeps one `Fragmentizer`, so unread bytes
        // desynchronise every later command (#1538).
        // `initial::existing_uids` treats the refusal as a downgrade and walks the UID space.
        if let Some(exists) = session.selected_exists(&mailbox)
            && listing_was_misread(found.len(), exists)
        {
            return Err(BackendError::Disconnected {
                context: format!("SEARCH on {mailbox}"),
                reason: format!(
                    "the server listed {} UIDs for a mailbox it says holds \
                     {exists}; the response was not read whole, and this \
                     connection cannot be trusted to parse the next one",
                    found.len()
                ),
            });
        }

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
        // `flatten` used to discard `Multi`'s `subtype`, so every multipart read back as
        // `multipart/mixed`.
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

    fn part_named(disposition: &[(&str, &str)], name: Option<&str>) -> WireBodyStructure<'static> {
        let mut part = attachment_part("placeholder.pdf", 10);
        if let WireBodyStructure::Single {
            body,
            extension_data,
        } = &mut part
        {
            body.basic.parameter_list = name
                .map(|name| {
                    vec![(
                        IString::try_from("name").unwrap(),
                        IString::try_from(name.to_owned()).unwrap(),
                    )]
                })
                .unwrap_or_default();
            let params = disposition
                .iter()
                .map(|(key, value)| {
                    (
                        IString::try_from((*key).to_owned()).unwrap(),
                        IString::try_from((*value).to_owned()).unwrap(),
                    )
                })
                .collect();
            *extension_data = Some(io_imap::types::body::SinglePartExtensionData {
                md5: NString::NIL,
                tail: Some(WireDisposition {
                    disposition: Some((IString::try_from("attachment").unwrap(), params)),
                    tail: None,
                }),
            });
        }
        part
    }

    #[test]
    fn an_rfc_2231_filename_is_decoded_rather_than_lost() {
        // `filename*` is how a non-ASCII name travels, undecoded; looking only for `filename` left
        // the part nameless (#1686).
        let structure = body_structure_from_wire(&part_named(
            &[("filename*", "UTF-8''Rechnung%20M%C3%A4rz.pdf")],
            None,
        ));
        assert_eq!(structure.parts()[0].filename(), Some("Rechnung März.pdf"));
    }

    #[test]
    fn an_rfc_2231_filename_in_continuations_is_joined() {
        let structure = body_structure_from_wire(&part_named(
            &[
                ("filename*0*", "UTF-8''Quartals"),
                ("filename*1*", "bericht%20%C3%9Cbersicht"),
                ("filename*2", ".pdf"),
            ],
            None,
        ));
        assert_eq!(
            structure.parts()[0].filename(),
            Some("Quartalsbericht Übersicht.pdf")
        );
    }

    #[test]
    fn an_encoded_word_name_is_decoded() {
        // Not what RFC 2047 allows in a parameter, but many clients send it.
        let structure = body_structure_from_wire(&part_named(
            &[],
            Some("=?UTF-8?B?UHLDpHNlbnRhdGlvbi5wZGY=?="),
        ));
        assert_eq!(structure.parts()[0].filename(), Some("Präsentation.pdf"));
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

#[cfg(test)]
mod a_short_listing_is_a_misread {
    use super::listing_was_misread;

    /// iCloud answered `UID SEARCH ALL` with nothing for a 60,934-message Archive; the unread
    /// response then corrupted later commands on the connection (`SELECT` with a `UIDVALIDITY` of
    /// 5).
    #[test]
    fn a_listing_short_of_exists_is_refused() {
        assert!(listing_was_misread(0, 60_934), "the Archive, as measured");
        assert!(listing_was_misread(0, 20_324), "Sent Messages, as measured");
        assert!(listing_was_misread(0, 35), "a small mailbox, still empty");
        assert!(listing_was_misread(0, 1), "one message is still a mailbox");
        assert!(listing_was_misread(172, 173), "one short is still short");
    }

    /// The control: refusing costs the connection, so a rule firing on a healthy answer would churn
    /// connections.
    #[test]
    fn a_listing_that_covers_exists_is_believed() {
        assert!(!listing_was_misread(173, 173), "exactly right");
        assert!(!listing_was_misread(60_934, 60_934), "exactly right, large");
        assert!(!listing_was_misread(174, 173), "a delivery mid-command");
        assert!(
            !listing_was_misread(0, 0),
            "an empty mailbox really is empty"
        );
    }
}
