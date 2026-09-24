//! vCard import and export.
//!
//! This crate maps between vCard files and Postio's contacts: which card
//! properties become a person's name, addresses, organisation and note, which
//! cards are groups, and how a card Postio has edited is written back out. It
//! does no storage and no I/O of its own — the app reads the file the user
//! picked, hands the bytes here, and writes what comes back into the store.
//!
//! Parsing is Pimalaya's `vcard-rs`, chosen because it round-trips every
//! property it does not understand byte-for-byte, which is the property an
//! honest import-then-export needs (`specs/005-contacts/research.md` R9). It
//! is a leaf: no database engine, no toolkit, no async runtime —
//! `scripts/checks/check-crate-boundaries.py` holds it to that.

use std::borrow::Cow;

use postio_model::EmailAddress;
use vcard::tree::cst::VcardCst;
use vcard::tree::line::VcardLine;
use vcard::tree::param::node::VcardParamNode;

/// What a file held: the cards that read, and the ones that did not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Import {
    /// Every card that could be read, in file order.
    pub cards: Vec<ParsedCard>,
    /// Every card that could not, with why.
    pub skipped: Vec<Skip>,
}

/// A card that was not imported (FR-054).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skip {
    /// Its position in the file, from zero.
    pub index: usize,
    /// Why, for the summary.
    pub reason: String,
}

/// One card, as Postio reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedCard {
    /// A person.
    Person(ParsedPerson),
    /// A `KIND:group` card.
    Group(ParsedGroup),
}

/// A person's card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedPerson {
    /// `UID`.
    pub uid: Option<String>,
    /// `FN`.
    pub name: Option<String>,
    /// Every `EMAIL`, and whether it is the preferred one; exactly one is.
    pub emails: Vec<(EmailAddress, bool)>,
    /// `ORG`.
    pub organization: Option<String>,
    /// `NOTE`.
    pub note: Option<String>,
    /// The card as it arrived, for `contacts.vcard`.
    pub raw: String,
}

/// A group card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedGroup {
    /// `UID`.
    pub uid: Option<String>,
    /// `FN`.
    pub name: String,
    /// Its `MEMBER`s.
    pub members: Vec<Member>,
    /// The card as it arrived.
    pub raw: String,
}

/// A `MEMBER` of a group card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Member {
    /// Another card, by its `UID`.
    Uid(String),
    /// An address, from a `mailto:` URI.
    Address(String),
}

/// A person as the store hands them to export.
#[derive(Debug, Clone, Copy)]
pub struct ExportPerson<'a> {
    /// Their `UID`.
    pub uid: &'a str,
    /// Their name.
    pub name: Option<&'a str>,
    /// Their addresses, and which is preferred.
    pub emails: &'a [(String, bool)],
    /// Their organisation.
    pub organization: Option<&'a str>,
    /// The user's note.
    pub note: Option<&'a str>,
}

/// Reads a `.vcf` file: every card that reads, and every one that does not
/// with the reason (FR-054). A file is split into cards here rather than by
/// the parser's own iterator, which stops at the first bad card -- one broken
/// card must not cost the user the rest of their address book.
pub fn parse(bytes: &[u8]) -> Import {
    let text = String::from_utf8_lossy(bytes);
    let mut import = Import::default();
    for (index, chunk) in split_cards(&text).into_iter().enumerate() {
        match parse_card(&chunk) {
            Ok(card) => import.cards.push(card),
            Err(reason) => import.skipped.push(Skip { index, reason }),
        }
    }
    import
}

/// One 4.0 card for `person`. From a stored card, only the properties Postio
/// models are rewritten -- in place, their group prefix kept -- and every
/// other line is carried over byte for byte, save the four spellings 4.0
/// forbids in a 3.0 source (contracts/vcard.md). With no stored card, a
/// fresh one.
pub fn export(person: &ExportPerson<'_>, raw: Option<&str>) -> String {
    if let Some(card) = raw.and_then(|raw| VcardCst::parse(raw).ok()) {
        return rewrite(&card, &Modelled::Person(person));
    }
    let mut out = String::from("BEGIN:VCARD\r\nVERSION:4.0\r\n");
    push(&mut out, "", "UID", person.uid);
    if let Some(name) = person.name {
        push(&mut out, "", "FN", &escape(name));
    }
    for (address, preferred) in person.emails {
        push_email(&mut out, "", address, *preferred);
    }
    if let Some(organization) = person.organization {
        push(&mut out, "", "ORG", &escape(organization));
    }
    if let Some(note) = person.note {
        push(&mut out, "", "NOTE", &escape(note));
    }
    out.push_str("END:VCARD\r\n");
    out
}

/// One 4.0 group card: its name and members as the store has them now,
/// everything else from `raw` as [`export`] carries it.
pub fn export_group(uid: &str, name: &str, members: &[String], raw: Option<&str>) -> String {
    if let Some(card) = raw.and_then(|raw| VcardCst::parse(raw).ok()) {
        return rewrite(&card, &Modelled::Group { uid, name, members });
    }
    let mut out = String::from("BEGIN:VCARD\r\nVERSION:4.0\r\nKIND:group\r\n");
    push(&mut out, "", "UID", uid);
    push(&mut out, "", "FN", &escape(name));
    for member in members {
        push(&mut out, "", "MEMBER", member);
    }
    out.push_str("END:VCARD\r\n");
    out
}

// -- Reading ------------------------------------------------------------------

/// The file's cards, `BEGIN` to `END`, each as it was written.
fn split_cards(text: &str) -> Vec<String> {
    let mut cards = Vec::new();
    let mut current: Option<String> = None;
    for line in text.split_inclusive('\n') {
        let bare = line.trim();
        if bare.eq_ignore_ascii_case("BEGIN:VCARD") {
            current = Some(String::new());
        }
        if let Some(card) = current.as_mut() {
            card.push_str(line);
        }
        if bare.eq_ignore_ascii_case("END:VCARD")
            && let Some(card) = current.take()
        {
            cards.push(card);
        }
    }
    cards
}

fn parse_card(chunk: &str) -> Result<ParsedCard, String> {
    let card = VcardCst::parse(chunk).map_err(|error| error.to_string())?;
    let (mut uid, mut name, mut organization, mut note) = (None, None, None, None);
    let mut structured_name = None;
    let mut group = false;
    let mut emails: Vec<(EmailAddress, bool)> = Vec::new();
    let mut members = Vec::new();
    for line in &card.props {
        let (_, property) = split_name(line.name.get());
        let text = || {
            let value = line.value.decode().trim().to_owned();
            (!value.is_empty()).then_some(value)
        };
        match property.to_ascii_uppercase().as_str() {
            "UID" => uid = uid.or_else(text),
            "FN" => name = name.or_else(text),
            "N" => {
                let given = line.value.decode_component(1).trim().to_owned();
                let family = line.value.decode_component(0).trim().to_owned();
                let joined = format!("{given} {family}").trim().to_owned();
                if !joined.is_empty() {
                    structured_name = structured_name.or(Some(joined));
                }
            }
            "ORG" => {
                if organization.is_none() {
                    let org = line.value.decode_component(0).trim().to_owned();
                    organization = (!org.is_empty()).then_some(org);
                }
            }
            "NOTE" => note = note.or_else(text),
            "KIND" => group = text().is_some_and(|kind| kind.eq_ignore_ascii_case("group")),
            "EMAIL" => {
                if let Some(value) = text() {
                    let address = value
                        .strip_prefix("mailto:")
                        .unwrap_or(&value)
                        .trim()
                        .to_owned();
                    if !emails
                        .iter()
                        .any(|(a, _)| a.address.eq_ignore_ascii_case(&address))
                    {
                        emails.push((
                            EmailAddress::new(None::<String>, address),
                            is_preferred(line),
                        ));
                    }
                }
            }
            "MEMBER" => {
                if let Some(value) = text() {
                    members.push(match value.strip_prefix("mailto:") {
                        Some(address) => Member::Address(address.trim().to_lowercase()),
                        None => Member::Uid(value),
                    });
                }
            }
            _ => {}
        }
    }
    let name = name.or(structured_name);
    let raw = chunk.to_owned();
    if group {
        return Ok(ParsedCard::Group(ParsedGroup {
            uid,
            name: name.unwrap_or_else(|| "Unnamed group".to_owned()),
            members,
            raw,
        }));
    }
    if emails.is_empty() {
        return Err("it has no email address".to_owned());
    }
    // Exactly one preferred: the first the card marks, else the first.
    let preferred = emails.iter().position(|(_, p)| *p).unwrap_or(0);
    for (index, (_, flag)) in emails.iter_mut().enumerate() {
        *flag = index == preferred;
    }
    Ok(ParsedCard::Person(ParsedPerson {
        uid,
        name,
        emails,
        organization,
        note,
        raw,
    }))
}

/// `item1.EMAIL` → (`item1.`, `EMAIL`).
fn split_name(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(dot) => (&name[..=dot], &name[dot + 1..]),
        None => ("", name),
    }
}

/// `PREF=1` (4.0), `TYPE=pref` (3.0) or a bare `PREF` (2.1).
fn is_preferred(line: &VcardLine<'_>) -> bool {
    line.params.iter().any(|param| {
        let name = param.name.get();
        name.eq_ignore_ascii_case("PREF")
            || (name.eq_ignore_ascii_case("TYPE")
                && param
                    .values
                    .iter()
                    .any(|v| v.get().eq_ignore_ascii_case("pref")))
    })
}

// -- Writing ------------------------------------------------------------------

/// What the store says about the card, for the lines it owns.
enum Modelled<'a> {
    Person(&'a ExportPerson<'a>),
    Group {
        uid: &'a str,
        name: &'a str,
        members: &'a [String],
    },
}

/// Walks a stored card, writing the modelled lines from the store and
/// carrying every other one over.
fn rewrite(card: &VcardCst<'_>, modelled: &Modelled<'_>) -> String {
    let from_3 = card
        .version_line()
        .is_some_and(|line| line.raw_value_str().trim() != "4.0");
    let (uid, name) = match modelled {
        Modelled::Person(person) => (person.uid, person.name),
        Modelled::Group { uid, name, .. } => (*uid, Some(*name)),
    };
    let mut out = String::new();
    let mut done: Vec<&'static str> = Vec::new();
    if let Some(begin) = &card.begin {
        out.push_str(&line_text(begin));
    }
    for line in &card.props {
        let (group, property) = split_name(line.name.get());
        let upper = property.to_ascii_uppercase();
        let eol = line.eol.get();
        let once = |done: &mut Vec<&'static str>, key: &'static str| {
            let first = !done.contains(&key);
            done.push(key);
            first
        };
        match upper.as_str() {
            "VERSION" => out.push_str(&format!("VERSION:4.0{eol}")),
            "UID" => {
                if once(&mut done, "UID") {
                    out.push_str(&format!("{group}UID:{uid}{eol}"));
                }
            }
            "FN" => {
                if once(&mut done, "FN")
                    && let Some(name) = name
                {
                    out.push_str(&format!("{group}FN:{}{eol}", escape(name)));
                }
            }
            "EMAIL" => {
                if let Modelled::Person(person) = modelled
                    && once(&mut done, "EMAIL")
                {
                    for (address, preferred) in person.emails {
                        write_email(&mut out, card, group, address, *preferred, from_3);
                    }
                }
            }
            "ORG" | "NOTE" => {
                let (key, value) = match (upper.as_str(), modelled) {
                    ("ORG", Modelled::Person(p)) => ("ORG", p.organization),
                    ("NOTE", Modelled::Person(p)) => ("NOTE", p.note),
                    _ => {
                        out.push_str(&carried(line, from_3));
                        continue;
                    }
                };
                if once(&mut done, key)
                    && let Some(value) = value
                {
                    out.push_str(&format!("{group}{key}:{}{eol}", escape(value)));
                }
            }
            "MEMBER" => {
                if let Modelled::Group { members, .. } = modelled
                    && once(&mut done, "MEMBER")
                {
                    for member in *members {
                        out.push_str(&format!("{group}MEMBER:{member}{eol}"));
                    }
                }
            }
            _ => out.push_str(&carried(line, from_3)),
        }
    }
    // What the store has that the card did not.
    if !done.contains(&"UID") {
        push(&mut out, "", "UID", uid);
    }
    if !done.contains(&"FN")
        && let Some(name) = name
    {
        push(&mut out, "", "FN", &escape(name));
    }
    match modelled {
        Modelled::Person(person) => {
            if !done.contains(&"EMAIL") {
                for (address, preferred) in person.emails {
                    push_email(&mut out, "", address, *preferred);
                }
            }
            for (key, value) in [("ORG", person.organization), ("NOTE", person.note)] {
                if !done.contains(&key)
                    && let Some(value) = value
                {
                    push(&mut out, "", key, &escape(value));
                }
            }
        }
        Modelled::Group { members, .. } => {
            if !done.contains(&"MEMBER") {
                for member in *members {
                    push(&mut out, "", "MEMBER", member);
                }
            }
        }
    }
    match &card.end {
        Some(end) => out.push_str(&line_text(end)),
        None => out.push_str("END:VCARD\r\n"),
    }
    out
}

/// One address, from its original line when the card had it -- its `TYPE`
/// and group kept -- with `PREF=1` on the preferred one only.
fn write_email(
    out: &mut String,
    card: &VcardCst<'_>,
    group: &str,
    address: &str,
    preferred: bool,
    from_3: bool,
) {
    let original = card.props.iter().find(|line| {
        let (_, property) = split_name(line.name.get());
        property.eq_ignore_ascii_case("EMAIL")
            && line
                .value
                .decode()
                .trim()
                .trim_start_matches("mailto:")
                .eq_ignore_ascii_case(address)
    });
    let Some(original) = original else {
        push_email(out, group, address, preferred);
        return;
    };
    let mut line = original.clone();
    drop_preference(&mut line);
    if from_3 {
        line.params
            .retain(|param| !param.name.get().eq_ignore_ascii_case("CHARSET"));
    }
    if preferred {
        line.params.push(VcardParamNode::parse("PREF=1"));
    }
    out.push_str(&line_text(&line));
}

/// Takes every way of saying "preferred" off a line.
fn drop_preference(line: &mut VcardLine<'_>) {
    line.params
        .retain(|param| !param.name.get().eq_ignore_ascii_case("PREF"));
    for param in &mut line.params {
        if param.name.get().eq_ignore_ascii_case("TYPE") {
            param
                .values
                .retain(|v| !v.get().eq_ignore_ascii_case("pref"));
        }
    }
    line.params.retain(|param| {
        !(param.name.get().eq_ignore_ascii_case("TYPE") && param.values.is_empty())
    });
}

/// A line Postio does not model: byte for byte from a 4.0 card; from a 3.0
/// one, with only what 4.0 forbids respelt -- `TYPE=PREF` as `PREF=1`, no
/// `CHARSET`, and an inline `ENCODING=b` value as a `data:` URI.
fn carried(line: &VcardLine<'_>, from_3: bool) -> String {
    let untouched = line_text(line);
    if !from_3 {
        return untouched;
    }
    let mut line = line.clone();
    let mut changed = false;
    let before = line.params.len();
    line.params
        .retain(|param| !param.name.get().eq_ignore_ascii_case("CHARSET"));
    changed |= line.params.len() != before;
    if is_preferred(&line) {
        drop_preference(&mut line);
        line.params.push(VcardParamNode::parse("PREF=1"));
        changed = true;
    }
    let inline = line.params.iter().position(|param| {
        param.name.get().eq_ignore_ascii_case("ENCODING")
            && param.values.iter().any(|v| {
                v.get().eq_ignore_ascii_case("b") || v.get().eq_ignore_ascii_case("base64")
            })
    });
    let Some(inline) = inline else {
        return if changed { line_text(&line) } else { untouched };
    };
    line.params.remove(inline);
    let media = line
        .params
        .iter()
        .position(|param| param.name.get().eq_ignore_ascii_case("TYPE"))
        .map(|at| line.params.remove(at))
        .and_then(|param| param.values.first().map(|v| v.get().to_ascii_lowercase()));
    let (_, property) = split_name(line.name.get());
    let family = match property.to_ascii_uppercase().as_str() {
        "PHOTO" | "LOGO" => "image/",
        "SOUND" => "audio/",
        _ => "application/",
    };
    let media = match media {
        Some(media) if media.contains('/') => media,
        Some(media) => format!("{family}{media}"),
        None => "application/octet-stream".to_owned(),
    };
    let data = line.raw_value_str().trim().to_owned();
    let text = line_text(&line);
    let head = text.split_once(':').map_or(text.as_str(), |(head, _)| head);
    format!("{head}:data:{media};base64,{data}{}", line.eol.get())
}

/// One line, serialized exactly as the tree would write it.
fn line_text(line: &VcardLine<'_>) -> String {
    let card = VcardCst {
        begin: None,
        props: vec![line.clone()],
        end: None,
        trailing: Cow::Borrowed(""),
    };
    String::from_utf8_lossy(&card.to_bytes()).into_owned()
}

fn push(out: &mut String, group: &str, property: &str, value: &str) {
    out.push_str(&format!("{group}{property}:{value}\r\n"));
}

fn push_email(out: &mut String, group: &str, address: &str, preferred: bool) {
    let pref = if preferred { ";PREF=1" } else { "" };
    out.push_str(&format!("{group}EMAIL{pref}:{address}\r\n"));
}

/// RFC 6350 3.4's text escapes.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            ',' => out.push_str("\\,"),
            ';' => out.push_str("\\;"),
            c => out.push(c),
        }
    }
    out
}
