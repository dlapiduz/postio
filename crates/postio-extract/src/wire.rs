//! The frame an extraction crosses a process boundary in (spec 010 D28).
//!
//! The indexer does not call [`extract`](crate::extract) in its own
//! process: it starts `postio-extract-helper`, writes one [request] to its
//! stdin and reads one [reply] from its stdout. This is both halves of
//! that exchange, so the writer and the reader are one piece of code and
//! cannot drift.
//!
//! # The format
//!
//! Hand-rolled, little-endian, every variable part length-prefixed, and
//! versioned twice: [`WIRE_VERSION`] for the shape of the frame, and the
//! reply's [`EXTRACTOR_VERSION`](crate::EXTRACTOR_VERSION) so a helper left
//! over from another build is told apart from a broken one
//! ([`Error::Extractor`]) rather than recorded as if this build had read
//! the file.
//!
//! ```text
//! request = "PXRQ" u16:wire
//!           str:mime  u8:has-name [str:name]
//!           u64:max_input u64:max_text u64:max_units u64:max_time_ms
//!           u64:max_entry u32:max_ratio
//!           u64:length bytes
//! reply   = "PXRS" u16:wire u32:extractor
//!           u8:outcome u8:detail
//!           u32:count { location str:text }*count
//! location = u8:kind, then u32 (page, slide, paragraph, line),
//!            str u32 (sheet name, row), u32 u32 (table, row), or nothing
//! str     = u32:length utf-8
//! ```
//!
//! # A reply is read as hostile
//!
//! The helper has just read a stranger's file, so what comes back is
//! decoded as carefully as the file was: no length is trusted to size an
//! allocation, the frame must end exactly where its count says, and
//! anything else is an [`Error`] the runner records as a failure.

use std::io::{self, Read, Write};
use std::time::Duration;

use crate::{Extracted, Limit, Limits, Location, Outcome, Skip, Unit};

/// The shape of the frame. Bump it whenever a field is added, removed or
/// reordered; a helper and an indexer that disagree then fail loudly
/// instead of misreading each other.
pub const WIRE_VERSION: u16 = 1;

const REQUEST_MAGIC: &[u8; 4] = b"PXRQ";
const REPLY_MAGIC: &[u8; 4] = b"PXRS";

/// The longest MIME type or file name a request may carry. Both come from
/// a message's headers, which are the sender's to write.
const MAX_LABEL: u32 = 64 * 1024;

/// The longest sheet name a reply may carry. Excel stops at 31
/// characters; this is room for any honest file and no hostile one.
const MAX_SHEET_NAME: u32 = 64 * 1024;

/// One extraction, as the helper receives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The attachment's bytes.
    pub bytes: Vec<u8>,
    /// Its MIME type, parameters and all.
    pub mime_type: String,
    /// Its file name, when it has one.
    pub name: Option<String>,
    /// How far the extraction may go.
    pub limits: Limits,
}

/// Why a frame could not be read.
#[derive(Debug)]
pub enum Error {
    /// The stream failed or ended before the frame did.
    Io(io::Error),
    /// The bytes are not a frame: wrong magic, an impossible length, an
    /// unknown tag, text that is not UTF-8, or bytes after the end.
    Malformed(&'static str),
    /// A frame of another [`WIRE_VERSION`].
    Wire(u16),
    /// A reply from another [`EXTRACTOR_VERSION`](crate::EXTRACTOR_VERSION):
    /// a helper from another build, not a broken one.
    Extractor(u32),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "the frame could not be read: {error}"),
            Self::Malformed(what) => write!(f, "the frame is malformed: {what}"),
            Self::Wire(version) => write!(f, "a frame of wire version {version}"),
            Self::Extractor(version) => {
                write!(f, "a reply from extractor version {version}")
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Write one request: what the indexer sends the helper.
pub fn write_request(
    out: &mut impl Write,
    bytes: &[u8],
    mime_type: &str,
    name: Option<&str>,
    limits: &Limits,
) -> io::Result<()> {
    let mut header = Vec::with_capacity(128);
    header.extend_from_slice(REQUEST_MAGIC);
    header.extend_from_slice(&WIRE_VERSION.to_le_bytes());
    put_str(&mut header, mime_type);
    match name {
        Some(name) => {
            header.push(1);
            put_str(&mut header, name);
        }
        None => header.push(0),
    }
    header.extend_from_slice(&limits.max_input.to_le_bytes());
    header.extend_from_slice(&(limits.max_text as u64).to_le_bytes());
    header.extend_from_slice(&(limits.max_units as u64).to_le_bytes());
    let millis = u64::try_from(limits.max_time.as_millis()).unwrap_or(u64::MAX);
    header.extend_from_slice(&millis.to_le_bytes());
    header.extend_from_slice(&limits.max_entry.to_le_bytes());
    header.extend_from_slice(&limits.max_ratio.to_le_bytes());
    header.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    out.write_all(&header)?;
    out.write_all(bytes)?;
    out.flush()
}

/// Read one request: what the helper receives. The attachment's length is
/// held to the request's own `max_input`, so the helper never reads more
/// than the limits it was given allow.
pub fn read_request(input: &mut impl Read) -> Result<Request, Error> {
    let mut magic = [0u8; 4];
    input.read_exact(&mut magic)?;
    if &magic != REQUEST_MAGIC {
        return Err(Error::Malformed("not a request"));
    }
    let wire = read_u16(input)?;
    if wire != WIRE_VERSION {
        return Err(Error::Wire(wire));
    }
    let mime_type = read_str(input, MAX_LABEL)?;
    let name = match read_u8(input)? {
        0 => None,
        1 => Some(read_str(input, MAX_LABEL)?),
        _ => return Err(Error::Malformed("a name flag that is neither")),
    };
    let max_input = read_u64(input)?;
    let max_text = usize::try_from(read_u64(input)?).unwrap_or(usize::MAX);
    let max_units = usize::try_from(read_u64(input)?).unwrap_or(usize::MAX);
    let max_time = Duration::from_millis(read_u64(input)?);
    let max_entry = read_u64(input)?;
    let max_ratio = read_u32(input)?;
    let limits = Limits {
        max_input,
        max_text,
        max_units,
        max_time,
        max_entry,
        max_ratio,
    };
    let length = read_u64(input)?;
    if length > limits.max_input {
        return Err(Error::Malformed("an attachment past its own input limit"));
    }
    let bytes = read_exactly(input, length)?;
    Ok(Request {
        bytes,
        mime_type,
        name,
        limits,
    })
}

/// Write one reply: what the helper answers.
pub fn write_reply(out: &mut impl Write, extracted: &Extracted) -> io::Result<()> {
    let mut frame = Vec::with_capacity(
        64 + extracted
            .units
            .iter()
            .map(|unit| unit.text.len() + 16)
            .sum::<usize>(),
    );
    frame.extend_from_slice(REPLY_MAGIC);
    frame.extend_from_slice(&WIRE_VERSION.to_le_bytes());
    frame.extend_from_slice(&crate::EXTRACTOR_VERSION.to_le_bytes());
    let (outcome, detail) = outcome_tag(extracted.outcome);
    frame.push(outcome);
    frame.push(detail);
    let count = u32::try_from(extracted.units.len()).unwrap_or(u32::MAX);
    frame.extend_from_slice(&count.to_le_bytes());
    for unit in extracted.units.iter().take(count as usize) {
        put_location(&mut frame, &unit.location);
        put_str(&mut frame, &unit.text);
    }
    out.write_all(&frame)?;
    out.flush()
}

/// Read one reply: what the indexer receives. The whole of `frame` must be
/// the reply, and nothing after it.
pub fn read_reply(frame: &[u8]) -> Result<Extracted, Error> {
    let mut input = frame;
    let mut magic = [0u8; 4];
    input.read_exact(&mut magic)?;
    if &magic != REPLY_MAGIC {
        return Err(Error::Malformed("not a reply"));
    }
    let wire = read_u16(&mut input)?;
    if wire != WIRE_VERSION {
        return Err(Error::Wire(wire));
    }
    let extractor = read_u32(&mut input)?;
    if extractor != crate::EXTRACTOR_VERSION {
        return Err(Error::Extractor(extractor));
    }
    let outcome = outcome_of(read_u8(&mut input)?, read_u8(&mut input)?)?;
    let count = read_u32(&mut input)?;
    // Each unit is at least a kind and an empty string: five bytes. A count
    // the rest of the frame cannot hold is a lie, refused before it sizes
    // anything.
    if u64::from(count) * 5 > input.len() as u64 {
        return Err(Error::Malformed("more units than the frame holds"));
    }
    let mut units = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let location = read_location(&mut input)?;
        let left = u32::try_from(input.len()).unwrap_or(u32::MAX);
        let text = read_str(&mut input, left)?;
        units.push(Unit { location, text });
    }
    if !input.is_empty() {
        return Err(Error::Malformed("bytes after the reply"));
    }
    Ok(Extracted { units, outcome })
}

/// The helper's whole job, over any two streams: read a request, extract
/// it, write the reply. `postio-extract-helper` is this over stdin and
/// stdout; tests call it over buffers.
pub fn serve(input: &mut impl Read, output: &mut impl Write) -> Result<(), Error> {
    let request = read_request(input)?;
    let extracted = crate::extract(
        &request.bytes,
        &request.mime_type,
        request.name.as_deref(),
        &request.limits,
    );
    write_reply(output, &extracted)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// The pieces.
// ---------------------------------------------------------------------------

fn outcome_tag(outcome: Outcome) -> (u8, u8) {
    match outcome {
        Outcome::Complete => (0, 0),
        Outcome::Truncated(limit) => (
            1,
            match limit {
                Limit::Text => 0,
                Limit::Units => 1,
                Limit::Time => 2,
                Limit::Entry => 3,
                Limit::Ratio => 4,
            },
        ),
        Outcome::Skipped(skip) => (
            2,
            match skip {
                Skip::Encrypted => 0,
                Skip::Unsupported => 1,
                Skip::Empty => 2,
                Skip::TooLarge => 3,
                Skip::Unavailable => 4,
            },
        ),
        Outcome::Failed => (3, 0),
    }
}

fn outcome_of(tag: u8, detail: u8) -> Result<Outcome, Error> {
    Ok(match (tag, detail) {
        (0, 0) => Outcome::Complete,
        (1, 0) => Outcome::Truncated(Limit::Text),
        (1, 1) => Outcome::Truncated(Limit::Units),
        (1, 2) => Outcome::Truncated(Limit::Time),
        (1, 3) => Outcome::Truncated(Limit::Entry),
        (1, 4) => Outcome::Truncated(Limit::Ratio),
        (2, 0) => Outcome::Skipped(Skip::Encrypted),
        (2, 1) => Outcome::Skipped(Skip::Unsupported),
        (2, 2) => Outcome::Skipped(Skip::Empty),
        (2, 3) => Outcome::Skipped(Skip::TooLarge),
        (2, 4) => Outcome::Skipped(Skip::Unavailable),
        (3, 0) => Outcome::Failed,
        _ => return Err(Error::Malformed("an unknown outcome")),
    })
}

fn put_location(out: &mut Vec<u8>, location: &Location) {
    match location {
        Location::Page(n) => {
            out.push(0);
            out.extend_from_slice(&n.to_le_bytes());
        }
        Location::Sheet { name, row } => {
            out.push(1);
            put_str(out, name);
            out.extend_from_slice(&row.to_le_bytes());
        }
        Location::Slide(n) => {
            out.push(2);
            out.extend_from_slice(&n.to_le_bytes());
        }
        Location::Paragraph(n) => {
            out.push(3);
            out.extend_from_slice(&n.to_le_bytes());
        }
        Location::Line(n) => {
            out.push(4);
            out.extend_from_slice(&n.to_le_bytes());
        }
        Location::Table { index, row } => {
            out.push(5);
            out.extend_from_slice(&index.to_le_bytes());
            out.extend_from_slice(&row.to_le_bytes());
        }
        Location::ImageText => out.push(6),
    }
}

fn read_location(input: &mut &[u8]) -> Result<Location, Error> {
    Ok(match read_u8(input)? {
        0 => Location::Page(read_u32(input)?),
        1 => Location::Sheet {
            name: read_str(input, MAX_SHEET_NAME)?,
            row: read_u32(input)?,
        },
        2 => Location::Slide(read_u32(input)?),
        3 => Location::Paragraph(read_u32(input)?),
        4 => Location::Line(read_u32(input)?),
        5 => Location::Table {
            index: read_u32(input)?,
            row: read_u32(input)?,
        },
        6 => Location::ImageText,
        _ => return Err(Error::Malformed("an unknown location")),
    })
}

/// A string as its length and its UTF-8. A string longer than a `u32`
/// cannot occur: every text is held under `Limits::max_text`.
fn put_str(out: &mut Vec<u8>, text: &str) {
    let length = u32::try_from(text.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&length.to_le_bytes());
    out.extend_from_slice(&text.as_bytes()[..length as usize]);
}

fn read_str(input: &mut impl Read, max: u32) -> Result<String, Error> {
    let length = read_u32(input)?;
    if length > max {
        return Err(Error::Malformed("a string past its limit"));
    }
    String::from_utf8(read_exactly(input, u64::from(length))?)
        .map_err(|_| Error::Malformed("a string that is not UTF-8"))
}

/// Exactly `length` bytes, grown as they arrive rather than allocated up
/// front from a length the other side wrote.
fn read_exactly(input: &mut impl Read, length: u64) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    input.take(length).read_to_end(&mut bytes)?;
    if bytes.len() as u64 != length {
        return Err(Error::Io(io::ErrorKind::UnexpectedEof.into()));
    }
    Ok(bytes)
}

fn read_u8(input: &mut impl Read) -> Result<u8, Error> {
    let mut buffer = [0u8; 1];
    input.read_exact(&mut buffer)?;
    Ok(buffer[0])
}

fn read_u16(input: &mut impl Read) -> Result<u16, Error> {
    let mut buffer = [0u8; 2];
    input.read_exact(&mut buffer)?;
    Ok(u16::from_le_bytes(buffer))
}

fn read_u32(input: &mut impl Read) -> Result<u32, Error> {
    let mut buffer = [0u8; 4];
    input.read_exact(&mut buffer)?;
    Ok(u32::from_le_bytes(buffer))
}

fn read_u64(input: &mut impl Read) -> Result<u64, Error> {
    let mut buffer = [0u8; 8];
    input.read_exact(&mut buffer)?;
    Ok(u64::from_le_bytes(buffer))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_location() -> Vec<Location> {
        vec![
            Location::Page(2),
            Location::Sheet {
                name: "Summary".to_owned(),
                row: 14,
            },
            Location::Slide(3),
            Location::Paragraph(4),
            Location::Line(5),
            Location::Table { index: 1, row: 7 },
            Location::ImageText,
        ]
    }

    fn every_outcome() -> Vec<Outcome> {
        vec![
            Outcome::Complete,
            Outcome::Truncated(Limit::Text),
            Outcome::Truncated(Limit::Units),
            Outcome::Truncated(Limit::Time),
            Outcome::Truncated(Limit::Entry),
            Outcome::Truncated(Limit::Ratio),
            Outcome::Skipped(Skip::Encrypted),
            Outcome::Skipped(Skip::Unsupported),
            Outcome::Skipped(Skip::Empty),
            Outcome::Skipped(Skip::TooLarge),
            Outcome::Skipped(Skip::Unavailable),
            Outcome::Failed,
        ]
    }

    fn reply_bytes(extracted: &Extracted) -> Vec<u8> {
        let mut frame = Vec::new();
        write_reply(&mut frame, extracted).expect("written");
        frame
    }

    fn sample() -> Extracted {
        Extracted {
            units: every_location()
                .into_iter()
                .enumerate()
                .map(|(n, location)| Unit {
                    location,
                    text: format!("unit {n}: the café's Atlas figures"),
                })
                .collect(),
            outcome: Outcome::Truncated(Limit::Units),
        }
    }

    #[test]
    fn a_request_round_trips() {
        let limits = Limits {
            max_input: 1_000,
            max_text: 200,
            max_units: 7,
            max_time: Duration::from_millis(1_500),
            max_entry: 900,
            max_ratio: 12,
        };
        for name in [Some("Atlas budget.xlsx"), None] {
            let mut frame = Vec::new();
            write_request(
                &mut frame,
                b"PK\x03\x04 bytes",
                "text/plain; charset=utf-8",
                name,
                &limits,
            )
            .expect("written");
            let request = read_request(&mut frame.as_slice()).expect("read");
            assert_eq!(
                request,
                Request {
                    bytes: b"PK\x03\x04 bytes".to_vec(),
                    mime_type: "text/plain; charset=utf-8".to_owned(),
                    name: name.map(str::to_owned),
                    limits: limits.clone(),
                }
            );
        }
    }

    #[test]
    fn a_request_past_its_own_input_limit_is_refused() {
        let limits = Limits {
            max_input: 3,
            ..Limits::default()
        };
        let mut frame = Vec::new();
        write_request(&mut frame, b"four", "text/plain", None, &limits).expect("written");
        assert!(matches!(
            read_request(&mut frame.as_slice()),
            Err(Error::Malformed(_))
        ));
    }

    #[test]
    fn every_location_and_every_outcome_round_trips() {
        let extracted = sample();
        assert_eq!(
            read_reply(&reply_bytes(&extracted)).expect("read"),
            extracted
        );
        for outcome in every_outcome() {
            let bare = Extracted {
                units: Vec::new(),
                outcome,
            };
            assert_eq!(read_reply(&reply_bytes(&bare)).expect("read"), bare);
        }
    }

    #[test]
    fn a_reply_cut_anywhere_is_an_error() {
        let frame = reply_bytes(&sample());
        for cut in 0..frame.len() {
            assert!(
                read_reply(&frame[..cut]).is_err(),
                "a reply cut at {cut} of {} was read",
                frame.len()
            );
        }
    }

    #[test]
    fn a_reply_with_anything_after_it_is_an_error() {
        let mut frame = reply_bytes(&sample());
        frame.push(0);
        assert!(matches!(read_reply(&frame), Err(Error::Malformed(_))));
    }

    #[test]
    fn a_reply_with_the_wrong_magic_or_wire_is_an_error() {
        let mut frame = reply_bytes(&sample());
        frame[0] = b'X';
        assert!(matches!(read_reply(&frame), Err(Error::Malformed(_))));

        let mut frame = reply_bytes(&sample());
        frame[4..6].copy_from_slice(&(WIRE_VERSION + 1).to_le_bytes());
        assert!(matches!(read_reply(&frame), Err(Error::Wire(_))));
    }

    #[test]
    fn a_reply_from_another_extractor_is_told_apart() {
        let mut frame = reply_bytes(&sample());
        frame[6..10].copy_from_slice(&(crate::EXTRACTOR_VERSION + 1).to_le_bytes());
        assert!(matches!(
            read_reply(&frame),
            Err(Error::Extractor(version)) if version == crate::EXTRACTOR_VERSION + 1
        ));
    }

    #[test]
    fn a_count_the_frame_cannot_hold_sizes_nothing() {
        let mut frame = reply_bytes(&Extracted {
            units: Vec::new(),
            outcome: Outcome::Complete,
        });
        let at = frame.len() - 4;
        frame[at..].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(read_reply(&frame), Err(Error::Malformed(_))));
    }

    #[test]
    fn garbage_is_not_a_reply() {
        assert!(read_reply(b"").is_err());
        assert!(read_reply(b"%PDF-1.7 printed by somebody's debug line").is_err());
    }

    #[test]
    fn serve_answers_what_extract_answers() {
        let text = b"Notes from the Atlas sync\nsecond line\n";
        let mut request = Vec::new();
        write_request(
            &mut request,
            text,
            "text/plain",
            Some("notes.txt"),
            &Limits::default(),
        )
        .expect("written");
        let mut reply = Vec::new();
        serve(&mut request.as_slice(), &mut reply).expect("served");
        assert_eq!(
            read_reply(&reply).expect("read"),
            crate::extract(text, "text/plain", Some("notes.txt"), &Limits::default())
        );
    }
}
