//! Message bodies: compressed on the way into a row, decompressed on the way
//! out.
//!
//! # Where bodies live, and why it is here
//!
//! A message's decoded text, HTML and header block are columns on its
//! `messages` row (ADR 0020). They used to be files in the content-addressed
//! blob store, and the case for that store turned out to be a case about
//! *attachments*: they are large, they stream, and the same PDF really does
//! arrive five times. None of that is true of a body. The median one is 325
//! bytes, identical bodies are rare — a quoted reply resembles its parent, it
//! is not byte-equal — and a file per body leaks its size and its mtime to
//! anyone holding the directory, encrypted or not. Message sizes are a
//! fingerprint and mtimes trace when mail arrived and was read.
//!
//! In rows, SQLCipher covers them (#300) and none of that leaks.
//!
//! # Compression is the largest disk lever in the product
//!
//! Attachment payloads default to on-demand, so the default store is
//! essentially the text axis: 1.43 GB on the reference account. Per-value zstd
//! takes about 36% of that, and a dictionary trained on the mailbox itself
//! about a further 28%, because mail from one correspondence is full of the
//! same signatures, the same quoted headers and the same boilerplate.
//!
//! Those ratios (1.57x plain, 2.19x with a dictionary) come from this
//! project's own corpus and from ADR 0020's measurements. **Synthetic mail
//! compresses 6-7x and that number means nothing** — generated mail is far
//! more self-similar than the real thing.
//!
//! # In Rust, not in an extension
//!
//! `sqlite-zstd` does exactly this and cannot be linked: its latest release
//! wants `libsqlite3-sys ^0.33`, our rusqlite wants `^0.38`, and cargo's
//! `links = "sqlite3"` rule permits one. The remaining route is a loadable
//! `.so` and `load_extension`, which is an attack surface a mail client should
//! not open for forty lines of Rust. ADR 0020 has the survey.
//!
//! # Where this sits relative to encryption
//!
//! Above the pager; SQLCipher is below it. They never meet, and the resulting
//! order is compress-then-encrypt, which is the order ADR 0017 requires.
//!
//! # The dictionary is a row
//!
//! [`train_dictionary`] writes one into `body_dictionaries` and every body
//! written afterwards names it. A dictionary held as a file beside the
//! database would be a new way to lose mail — lose it and every body written
//! against it is gone — so it lives where it is backed up, encrypted and
//! restored with the data it decodes. Old dictionaries are never deleted while
//! a row still names one, which the schema enforces with `ON DELETE RESTRICT`
//! rather than trusting anybody to remember.

use std::collections::HashMap;
use std::sync::Arc;

use rusqlite::Connection;

use crate::error::{Error, Result};

/// Compression level for stored bodies.
///
/// 3 is zstd's own default and the knee of the curve for text. Mail is read
/// far more often than it is written, and zstd's decompression speed barely
/// varies with the level it was written at, so paying more here buys almost
/// nothing on the path that has a budget.
const LEVEL: i32 = 3;

/// The largest dictionary [`train_dictionary`] will produce.
///
/// 110 KiB is zstd's own recommendation and the size its tooling defaults to.
/// It is read into memory on the first body read of a session and kept there.
const MAX_DICTIONARY_BYTES: usize = 110 * 1024;

/// The fewest bodies worth training on.
///
/// Below this zstd's trainer either refuses or produces a dictionary that
/// describes the samples rather than the mailbox, which is worse than none:
/// every body written against it pays the lookup and gains nothing.
const MIN_SAMPLES: usize = 32;

/// The fewest bytes of corpus worth training on.
const MIN_SAMPLE_BYTES: usize = 64 * 1024;

/// The most bodies read into memory to train from.
///
/// Training is an idle-time pass and this bounds what it costs: a few thousand
/// bodies describe a mailbox's vocabulary as well as all of them do.
const MAX_SAMPLES: usize = 4_096;

/// The most corpus bytes held at once while training.
const MAX_SAMPLE_BYTES: usize = 32 * 1024 * 1024;

/// How much the corpus must grow before training again is worth it.
///
/// ADR 0017's heuristic. A dictionary trained on the first 500 messages of a
/// mailbox that now holds 80,000 is describing a different mailbox; one
/// retrained after every hundred arrivals is churn that buys nothing and
/// leaves a table of near-identical dictionaries nothing may delete.
const REGROWTH_FACTOR: i64 = 10;

/// The id of a trained dictionary row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DictionaryId(i64);

impl DictionaryId {
    /// The underlying row id.
    pub fn get(self) -> i64 {
        self.0
    }
}

/// Compresses one value for storage.
///
/// `dictionary` is the trained dictionary to compress against, or `None` to
/// compress the value on its own. The caller records which was used; a zstd
/// frame can only be read back with the dictionary it was written against, and
/// a value that does not record one is a value that becomes unreadable the
/// moment a second dictionary exists.
pub(crate) fn compress(value: &str, dictionary: Option<&[u8]>) -> Result<Vec<u8>> {
    let bytes = value.as_bytes();
    let compressed = match dictionary {
        None => zstd::bulk::compress(bytes, LEVEL),
        Some(dictionary) => zstd::bulk::Compressor::with_dictionary(LEVEL, dictionary)
            .and_then(|mut compressor| compressor.compress(bytes)),
    };
    compressed.map_err(|source| Error::UnreadableBody {
        reason: format!("a body could not be compressed: {source}"),
    })
}

/// Reads one stored value back.
///
/// Streaming rather than a sized buffer, matching the blob store: the frame
/// header's claimed size is not something a corrupt row should get to turn
/// into an allocation.
///
/// # Errors
///
/// [`Error::UnreadableBody`] if the frame will not decode, or decodes to bytes
/// that are not UTF-8. Both mean the row and this build disagree about what is
/// stored, and a body is not a thing to hand back a guess for.
pub(crate) fn decompress(stored: &[u8], dictionary: Option<&[u8]>) -> Result<String> {
    use std::io::Read;

    let plain = match dictionary {
        None => zstd::stream::decode_all(stored),
        Some(dictionary) => zstd::stream::read::Decoder::with_dictionary(stored, dictionary)
            .and_then(|mut decoder| {
                let mut plain = Vec::new();
                decoder.read_to_end(&mut plain)?;
                Ok(plain)
            }),
    }
    .map_err(|source| Error::UnreadableBody {
        reason: format!("a stored body could not be decompressed: {source}"),
    })?;

    String::from_utf8(plain).map_err(|_| Error::UnreadableBody {
        reason: "a stored body decompressed to bytes that are not UTF-8".to_owned(),
    })
}

/// Dictionaries read from the database, kept for as long as their holder
/// lives.
///
/// One dictionary is ~110 KiB and building a zstd decoding table from it is
/// not free, so a pass that reads a batch of bodies should not do it once per
/// body. A repository built for a single read gets no reuse and needs none;
/// the body-index catch-up builds one repository outside its loop and gets all
/// of it.
///
/// Deliberately *not* a process-wide cache keyed by row id: two databases open
/// at once — which is the ordinary state of the test suite — both have a
/// dictionary 1, and they are not the same dictionary.
#[derive(Debug, Default)]
pub(crate) struct Dictionaries {
    loaded: HashMap<i64, Arc<Vec<u8>>>,
}

impl Dictionaries {
    /// The dictionary a new write should use, if there is one.
    ///
    /// **Which id is newest is looked up every time**, and only the bytes are
    /// cached. Memoizing the id instead saves a `max()` over a table with a
    /// handful of rows and costs correctness: a repository built before a
    /// training pass would go on writing against no dictionary — or an older
    /// one — for as long as it lived, and the caller has no way to know it
    /// should build a new one.
    pub(crate) fn newest(
        &mut self,
        connection: &Connection,
    ) -> Result<Option<(i64, Arc<Vec<u8>>)>> {
        let newest: Option<i64> =
            connection.query_row("SELECT max(id) FROM body_dictionaries", [], |row| {
                row.get(0)
            })?;
        let Some(id) = newest else {
            return Ok(None);
        };
        Ok(Some((id, self.get(connection, id)?)))
    }

    /// One dictionary by id, loading it if this is the first ask.
    pub(crate) fn get(&mut self, connection: &Connection, id: i64) -> Result<Arc<Vec<u8>>> {
        if let Some(dictionary) = self.loaded.get(&id) {
            return Ok(Arc::clone(dictionary));
        }
        let bytes: Vec<u8> = connection
            .query_row(
                "SELECT dictionary FROM body_dictionaries WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .map_err(|source| match source {
                // A row names a dictionary that is not there. The schema's
                // `ON DELETE RESTRICT` is what makes this unreachable, so
                // reaching it means the database was edited by hand.
                rusqlite::Error::QueryReturnedNoRows => Error::UnreadableBody {
                    reason: format!("a body names dictionary {id}, which is not in this database"),
                },
                other => Error::Sqlite(other),
            })?;
        let dictionary = Arc::new(bytes);
        self.loaded.insert(id, Arc::clone(&dictionary));
        Ok(dictionary)
    }
}

/// Whether the corpus has grown enough to be worth training a dictionary from.
///
/// Cheap enough to ask on an idle tick: two counts against an index.
///
/// # Errors
///
/// [`Error::Sqlite`] if the counts cannot be read.
pub fn should_train(connection: &Connection) -> Result<bool> {
    let bodies: i64 = connection.query_row(
        "SELECT count(*) FROM messages WHERE body_text IS NOT NULL",
        [],
        |row| row.get(0),
    )?;
    if bodies < MIN_SAMPLES as i64 {
        return Ok(false);
    }
    let trained_on: Option<i64> = connection
        .query_row(
            "SELECT sample_count FROM body_dictionaries ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .ok();
    match trained_on {
        // Never trained: any corpus at all is worth the first one.
        None => Ok(true),
        // ADR 0017's heuristic: a tenfold corpus is a different mailbox.
        Some(previous) => Ok(bodies >= previous.saturating_mul(REGROWTH_FACTOR)),
    }
}

/// Trains a dictionary from the bodies already stored, and records it.
///
/// Answers `None` when there is not enough local mail to train from — the
/// ordinary state of a store that has just been created, and not a fault.
/// Bodies written before this ran keep naming whatever they were written
/// against and go on reading; nothing is rewritten.
///
/// This decompresses its samples, so it is an idle-time pass and not something
/// to call on a path with a budget. [`should_train`] is the cheap question.
///
/// # Errors
///
/// [`Error::Sqlite`] if the corpus cannot be read or the row cannot be
/// written, and [`Error::UnreadableBody`] if a sample will not decompress.
pub fn train_dictionary(connection: &Connection) -> Result<Option<DictionaryId>> {
    let samples = read_samples(connection)?;
    if samples.len() < MIN_SAMPLES {
        return Ok(None);
    }
    let sample_bytes: usize = samples.iter().map(String::len).sum();
    if sample_bytes < MIN_SAMPLE_BYTES {
        return Ok(None);
    }

    let dictionary = match zstd::dict::from_samples(&samples, MAX_DICTIONARY_BYTES) {
        Ok(dictionary) => dictionary,
        // zstd's trainer declines corpora it cannot find enough structure in.
        // That is a legitimate answer about this mailbox, not a failure: the
        // bodies already written are unaffected and the next pass may find
        // more to work with.
        Err(source) => {
            tracing::debug!(
                samples = samples.len(),
                bytes = sample_bytes,
                %source,
                "no body dictionary could be trained from the local corpus"
            );
            return Ok(None);
        }
    };

    // The count is what `should_train` compares against next time, so it must
    // be the size of the corpus rather than of the sample taken from it.
    let corpus: i64 = connection.query_row(
        "SELECT count(*) FROM messages WHERE body_text IS NOT NULL",
        [],
        |row| row.get(0),
    )?;
    connection.execute(
        "INSERT INTO body_dictionaries (dictionary, sample_count, sample_bytes, created_at)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![
            dictionary,
            corpus,
            sample_bytes as i64,
            chrono::Utc::now().timestamp_millis(),
        ],
    )?;
    let id = connection.last_insert_rowid();

    // Ids, counts and sizes: never a byte of what was trained on.
    tracing::info!(
        dictionary = id,
        samples = samples.len(),
        sample_bytes,
        dictionary_bytes = dictionary.len(),
        "trained a body compression dictionary"
    );
    Ok(Some(DictionaryId(id)))
}

/// The stored bodies to train from, decompressed.
///
/// Newest first: a dictionary should describe the mail arriving now, and a
/// mailbox's vocabulary drifts over years.
fn read_samples(connection: &Connection) -> Result<Vec<String>> {
    let mut statement = connection.prepare(
        "SELECT body_text, body_dictionary_id FROM messages
          WHERE body_text IS NOT NULL
          ORDER BY received_at DESC, id DESC
          LIMIT ?1",
    )?;
    let rows = statement.query_map([MAX_SAMPLES as i64], |row| {
        Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Option<i64>>(1)?))
    })?;

    let mut dictionaries = Dictionaries::default();
    let mut samples = Vec::new();
    let mut bytes = 0usize;
    for row in rows {
        let (stored, dictionary_id) = row?;
        let dictionary = match dictionary_id {
            None => None,
            Some(id) => Some(dictionaries.get(connection, id)?),
        };
        let sample = decompress(&stored, dictionary.as_ref().map(|d| d.as_slice()))?;
        // A body of nothing teaches the trainer nothing and counts against the
        // sample budget.
        if sample.is_empty() {
            continue;
        }
        bytes += sample.len();
        samples.push(sample);
        if bytes >= MAX_SAMPLE_BYTES {
            break;
        }
    }
    Ok(samples)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_round_trips_without_a_dictionary() {
        let text = "Subject: lunch\r\n\r\nOn my way.".repeat(20);
        let stored = compress(&text, None).expect("compress");
        assert_eq!(decompress(&stored, None).expect("decompress"), text);
    }

    #[test]
    fn a_value_round_trips_against_a_dictionary() {
        let samples: Vec<String> = (0..64)
            .map(|n| format!("From: ada@example.com\r\nSubject: invoice {n}\r\n\r\nthanks\r\n"))
            .collect();
        let dictionary = zstd::dict::from_samples(&samples, MAX_DICTIONARY_BYTES).expect("train");

        let text = "From: ada@example.com\r\nSubject: invoice 99\r\n\r\nthanks\r\n";
        let stored = compress(text, Some(&dictionary)).expect("compress");
        assert_eq!(
            decompress(&stored, Some(&dictionary)).expect("decompress"),
            text
        );
    }

    #[test]
    fn the_wrong_dictionary_is_refused_rather_than_guessed_at() {
        let samples: Vec<String> = (0..64)
            .map(|n| format!("From: ada@example.com\r\nSubject: invoice {n}\r\n\r\nthanks\r\n"))
            .collect();
        let dictionary = zstd::dict::from_samples(&samples, MAX_DICTIONARY_BYTES).expect("train");
        let stored =
            compress("a body written against a dictionary", Some(&dictionary)).expect("compress");

        assert!(
            matches!(decompress(&stored, None), Err(Error::UnreadableBody { .. })),
            "reading a dictionary frame without its dictionary must fail loudly"
        );
    }

    #[test]
    fn an_empty_value_round_trips_as_empty() {
        let stored = compress("", None).expect("compress");
        assert_eq!(decompress(&stored, None).expect("decompress"), "");
    }

    #[test]
    fn damaged_bytes_are_an_error_and_not_a_guess() {
        let stored = compress("the real body", None).expect("compress");
        let mut damaged = stored.clone();
        let last = damaged.len() - 1;
        damaged[last] ^= 0xff;
        damaged.truncate(last);
        assert!(matches!(
            decompress(&damaged, None),
            Err(Error::UnreadableBody { .. })
        ));
    }

    // --- what zstd 0.13 wrote, read by whatever is linked now (#1304) -----
    //
    // The 0.13 -> 0.14 bump is a version bump of a *format*, and every stored
    // body in every existing install is a frame the old release produced. A
    // round-trip test cannot see that: it compresses and decompresses with
    // the same build, so it would stay green through a format change that
    // orphaned the whole store.
    //
    // These three constants were produced by a scratch binary pinned to
    //  -- the release this crate depended on before the
    // bump -- and are read here by the release it depends on now. The
    // dictionary path is included because that is where the subtle
    // incompatibility would live: a frame written against a dictionary
    // carries its id, and  has to be handed the same bytes back.
    //
    // Regenerate only if the *fixture* is wrong. If a future zstd cannot read
    // these, that is the finding, not a stale fixture.
    const FRAME_0_13: &[u8] = &[
        0x28, 0xb5, 0x2f, 0xfd, 0x20, 0xc5, 0xad, 0x04, 0x00, 0x92, 0x8a, 0x20, 0x22, 0xa0, 0x35,
        0xe9, 0xff, 0xff, 0xff, 0xe8, 0xea, 0x07, 0x3e, 0xc5, 0x26, 0x6d, 0x40, 0x44, 0xda, 0x25,
        0x56, 0xee, 0x4d, 0xd2, 0x6e, 0xdb, 0xce, 0x5a, 0xfa, 0x75, 0x9c, 0x41, 0xc2, 0x51, 0x15,
        0x83, 0x07, 0x47, 0xa2, 0x57, 0x7d, 0x63, 0x18, 0x89, 0xac, 0xb7, 0xd6, 0x7b, 0xd6, 0x3a,
        0xd5, 0xc8, 0x5a, 0x85, 0x67, 0x48, 0x93, 0x8c, 0xa5, 0xe2, 0xe3, 0x29, 0xbc, 0x8f, 0xf0,
        0xed, 0x8a, 0x72, 0x09, 0xd3, 0xfa, 0x4d, 0x84, 0x93, 0xe5, 0x6f, 0xac, 0xb7, 0xd6, 0x47,
        0xd7, 0xac, 0x53, 0x7d, 0xcc, 0x64, 0x3d, 0xcb, 0x7b, 0x56, 0x80, 0xa3, 0x56, 0xc9, 0x12,
        0x1e, 0x1f, 0x04, 0x67, 0x60, 0x3c, 0xf0, 0x1e, 0x2b, 0x73, 0x6d, 0x2d, 0x10, 0x30, 0xe0,
        0x93, 0x0a, 0xc7, 0xb1, 0xa0, 0x64, 0xed, 0x8b, 0xe7, 0xef, 0x51, 0x7e, 0xba, 0xf5, 0xd7,
        0x9a, 0xb0, 0xe8, 0x0b, 0x45, 0x23, 0x11, 0x05, 0x00, 0x99, 0x21, 0x46, 0x64, 0x75, 0xbb,
        0x51, 0xed, 0x74, 0xea, 0x0a, 0x69, 0x95, 0x1e,
    ];
    const DICTIONARY_0_13: &[u8] = &[
        0x37, 0xa4, 0x30, 0xec, 0x24, 0x3b, 0x8d, 0x4a, 0x13, 0x10, 0x18, 0x7e, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x28, 0x95, 0x52, 0x4a, 0x69, 0xa6, 0xa8, 0x83, 0x41,
        0x20, 0x10, 0x08, 0x84, 0x9d, 0xb9, 0x97, 0x88, 0x88, 0x88, 0x3c, 0x54, 0xa0, 0x40, 0x41,
        0x81, 0x08, 0x95, 0x82, 0x82, 0x82, 0x82, 0x82, 0x82, 0x02, 0x15, 0x93, 0x82, 0x82, 0x82,
        0x82, 0x82, 0x82, 0x82, 0x84, 0x82, 0x82, 0x82, 0x82, 0x82, 0x82, 0x82, 0x82, 0xc2, 0x56,
        0xa1, 0x50, 0x58, 0x26, 0x45, 0x29, 0xa5, 0x94, 0x52, 0x4a, 0x55, 0x55, 0x0f, 0x14, 0xf3,
        0x64, 0x30, 0x18, 0x0c, 0x06, 0x83, 0xc1, 0x60, 0x30, 0x18, 0x0c, 0xc3, 0x30, 0x0c, 0xc3,
        0x30, 0x0c, 0xc3, 0x30, 0x8c, 0x31, 0xc6, 0x18, 0x63, 0x66, 0x07, 0x01, 0x00, 0x00, 0x00,
        0x04, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x20, 0x6f, 0x6e, 0x20, 0x74, 0x68, 0x65,
        0x6d, 0x20, 0x69, 0x73, 0x20, 0x77, 0x6f, 0x72, 0x74, 0x68, 0x20, 0x68, 0x61, 0x76, 0x69,
        0x6e, 0x67, 0x2e, 0x0a, 0x53, 0x75, 0x62, 0x6a, 0x65, 0x63, 0x74, 0x3a, 0x20, 0x73, 0x61,
        0x6d, 0x70, 0x6c, 0x65, 0x20, 0x39, 0x0a, 0x0a, 0x41, 0x20, 0x62, 0x6f, 0x64, 0x64, 0x20,
        0x6f, 0x6e, 0x20, 0x74, 0x68, 0x65, 0x6d, 0x20, 0x69, 0x73, 0x20, 0x77, 0x6f, 0x72, 0x74,
        0x68, 0x20, 0x68, 0x61, 0x76, 0x69, 0x6e, 0x67, 0x2e, 0x0a, 0x53, 0x75, 0x62, 0x6a, 0x65,
        0x63, 0x74, 0x3a, 0x20, 0x73, 0x61, 0x6d, 0x70, 0x6c, 0x65, 0x20, 0x31, 0x0a, 0x0a, 0x41,
        0x20, 0x62, 0x6f, 0x20, 0x6f, 0x6e, 0x20, 0x74, 0x68, 0x65, 0x6d, 0x20, 0x69, 0x73, 0x20,
        0x77, 0x6f, 0x72, 0x74, 0x68, 0x20, 0x68, 0x61, 0x76, 0x69, 0x6e, 0x67, 0x2e, 0x0a, 0x53,
        0x75, 0x62, 0x6a, 0x65, 0x63, 0x74, 0x3a, 0x20, 0x73, 0x61, 0x6d, 0x70, 0x6c, 0x65, 0x20,
        0x31, 0x33, 0x0a, 0x0a, 0x41, 0x20, 0x62, 0x6f, 0x20, 0x6f, 0x6e, 0x20, 0x74, 0x68, 0x65,
        0x6d, 0x20, 0x69, 0x73, 0x20, 0x77, 0x6f, 0x72, 0x74, 0x68, 0x20, 0x68, 0x61, 0x76, 0x69,
        0x6e, 0x67, 0x2e, 0x0a, 0x53, 0x75, 0x62, 0x6a, 0x65, 0x63, 0x74, 0x3a, 0x20, 0x73, 0x61,
        0x6d, 0x70, 0x6c, 0x65, 0x20, 0x38, 0x0a, 0x0a, 0x41, 0x20, 0x62, 0x6f, 0x64, 0x64, 0x20,
        0x6f, 0x6e, 0x20, 0x74, 0x68, 0x65, 0x6d, 0x20, 0x69, 0x73, 0x20, 0x77, 0x6f, 0x72, 0x74,
        0x68, 0x20, 0x68, 0x61, 0x76, 0x69, 0x6e, 0x67, 0x2e, 0x0a, 0x53, 0x75, 0x62, 0x6a, 0x65,
        0x63, 0x74, 0x3a, 0x20, 0x73, 0x61, 0x6d, 0x70, 0x6c, 0x65, 0x20, 0x34, 0x0a, 0x0a, 0x41,
        0x20, 0x62, 0x6f, 0x20, 0x6f, 0x6e, 0x20, 0x74, 0x68, 0x65, 0x6d, 0x20, 0x69, 0x73, 0x20,
        0x77, 0x6f, 0x72, 0x74, 0x68, 0x20, 0x68, 0x61, 0x76, 0x69, 0x6e, 0x67, 0x2e, 0x0a, 0x53,
        0x75, 0x62, 0x6a, 0x65, 0x63, 0x74, 0x3a, 0x20, 0x73, 0x61, 0x6d, 0x70, 0x6c, 0x65, 0x20,
        0x31, 0x32, 0x0a, 0x0a, 0x41, 0x20, 0x62, 0x6f, 0x20, 0x6f, 0x6e, 0x20, 0x74, 0x68, 0x65,
        0x6d, 0x20, 0x69, 0x73, 0x20, 0x77, 0x6f, 0x72, 0x74, 0x68, 0x20, 0x68, 0x61, 0x76, 0x69,
        0x6e, 0x67, 0x2e, 0x0a, 0x53, 0x75, 0x62, 0x6a, 0x65, 0x63, 0x74, 0x3a, 0x20, 0x73, 0x61,
        0x6d, 0x70, 0x6c, 0x65, 0x20, 0x37, 0x0a, 0x0a, 0x41, 0x20, 0x62, 0x6f, 0x64, 0x53, 0x75,
        0x62, 0x6a, 0x65, 0x63, 0x74, 0x3a, 0x20, 0x73, 0x61, 0x6d, 0x70, 0x6c, 0x65, 0x20, 0x30,
        0x0a, 0x0a, 0x41, 0x20, 0x62, 0x6f, 0x6f, 0x6e, 0x20, 0x74, 0x68, 0x65, 0x6d, 0x20, 0x69,
        0x73, 0x20, 0x77, 0x6f, 0x72, 0x74, 0x68, 0x20, 0x68, 0x61, 0x76, 0x69, 0x6e, 0x67, 0x2e,
        0x0a, 0x53, 0x75, 0x62, 0x6a, 0x65, 0x63, 0x74, 0x3a, 0x20, 0x73, 0x61, 0x6d, 0x70, 0x6c,
        0x65, 0x20, 0x31, 0x34, 0x0a, 0x0a, 0x41, 0x20, 0x62, 0x6f, 0x64, 0x20, 0x6f, 0x6e, 0x20,
        0x74, 0x68, 0x65, 0x6d, 0x20, 0x69, 0x73, 0x20, 0x77, 0x6f, 0x72, 0x74, 0x68, 0x20, 0x68,
        0x61, 0x76, 0x69, 0x6e, 0x67, 0x2e, 0x0a, 0x53, 0x75, 0x62, 0x6a, 0x65, 0x63, 0x74, 0x3a,
        0x20, 0x73, 0x61, 0x6d, 0x70, 0x6c, 0x65, 0x20, 0x36, 0x0a, 0x0a, 0x41, 0x20, 0x62, 0x6f,
        0x64, 0x20, 0x6f, 0x6e, 0x20, 0x74, 0x68, 0x65, 0x6d, 0x20, 0x69, 0x73, 0x20, 0x77, 0x6f,
        0x72, 0x74, 0x68, 0x20, 0x68, 0x61, 0x76, 0x69, 0x6e, 0x67, 0x2e, 0x0a, 0x53, 0x75, 0x62,
        0x6a, 0x65, 0x63, 0x74, 0x3a, 0x20, 0x73, 0x61, 0x6d, 0x70, 0x6c, 0x65, 0x20, 0x33, 0x0a,
        0x0a, 0x41, 0x20, 0x62, 0x6f, 0x64, 0x6f, 0x6e, 0x20, 0x74, 0x68, 0x65, 0x6d, 0x20, 0x69,
        0x73, 0x20, 0x77, 0x6f, 0x72, 0x74, 0x68, 0x20, 0x68, 0x61, 0x76, 0x69, 0x6e, 0x67, 0x2e,
        0x0a, 0x53, 0x75, 0x62, 0x6a, 0x65, 0x63, 0x74, 0x3a, 0x20, 0x73, 0x61, 0x6d, 0x70, 0x6c,
        0x65, 0x20, 0x31, 0x31, 0x0a, 0x0a, 0x41, 0x20, 0x62, 0x6f, 0x64, 0x20, 0x77, 0x6f, 0x72,
        0x74, 0x68, 0x20, 0x68, 0x61, 0x76, 0x69, 0x6e, 0x67, 0x2e, 0x0a, 0x53, 0x75, 0x62, 0x6a,
        0x65, 0x63, 0x74, 0x3a, 0x20, 0x73, 0x61, 0x6d, 0x70, 0x6c, 0x65, 0x20, 0x35, 0x0a, 0x0a,
        0x41, 0x20, 0x62, 0x6f, 0x64, 0x20, 0x6f, 0x6e, 0x20, 0x74, 0x68, 0x65, 0x6d, 0x20, 0x69,
        0x73, 0x20, 0x77, 0x6f, 0x72, 0x74, 0x68, 0x20, 0x68, 0x61, 0x76, 0x69, 0x6e, 0x67, 0x2e,
        0x0a, 0x53, 0x75, 0x62, 0x6a, 0x65, 0x63, 0x74, 0x3a, 0x20, 0x73, 0x61, 0x6d, 0x70, 0x6c,
        0x65, 0x20, 0x32, 0x0a, 0x0a, 0x41, 0x20, 0x62, 0x6f, 0x64, 0x6f, 0x6e, 0x20, 0x74, 0x68,
        0x65, 0x6d, 0x20, 0x69, 0x73, 0x20, 0x77, 0x6f, 0x72, 0x74, 0x68, 0x20, 0x68, 0x61, 0x76,
        0x69, 0x6e, 0x67, 0x2e, 0x0a, 0x53, 0x75, 0x62, 0x6a, 0x65, 0x63, 0x74, 0x3a, 0x20, 0x73,
        0x61, 0x6d, 0x70, 0x6c, 0x65, 0x20, 0x31, 0x30, 0x0a, 0x0a, 0x41, 0x20, 0x62, 0x6f, 0x64,
        0x20, 0x73, 0x6f, 0x20, 0x61, 0x20, 0x64, 0x69, 0x63, 0x74, 0x69, 0x6f, 0x6e, 0x61, 0x72,
        0x79, 0x20, 0x74, 0x72, 0x61, 0x69, 0x6e, 0x65, 0x64, 0x20, 0x6f, 0x6e, 0x20, 0x74, 0x68,
        0x65, 0x6d, 0x20, 0x69, 0x73, 0x20, 0x77, 0x6f, 0x72, 0x74, 0x68, 0x20, 0x68, 0x61, 0x76,
        0x69, 0x6e, 0x67, 0x2e, 0x0a, 0x0a, 0x0a, 0x41, 0x20, 0x62, 0x6f, 0x64, 0x79, 0x20, 0x74,
        0x68, 0x61, 0x74, 0x20, 0x6c, 0x6f, 0x6f, 0x6b, 0x73, 0x20, 0x6c, 0x69, 0x6b, 0x65, 0x20,
        0x74, 0x68, 0x65, 0x20, 0x6f, 0x74, 0x68, 0x65, 0x72, 0x73, 0x2c, 0x20, 0x73, 0x6f, 0x20,
        0x61, 0x20, 0x64, 0x69, 0x63, 0x74, 0x69, 0x6f, 0x6e, 0x61,
    ];
    const FRAME_0_13_WITH_DICTIONARY: &[u8] = &[
        0x28, 0xb5, 0x2f, 0xfd, 0x23, 0x24, 0x3b, 0x8d, 0x4a, 0xc5, 0x9d, 0x05, 0x00, 0x64, 0x09,
        0x74, 0x68, 0x65, 0x20, 0x74, 0x69, 0x64, 0x65, 0x20, 0x67, 0x61, 0x74, 0x65, 0x20, 0x69,
        0x6e, 0x74, 0x65, 0x72, 0x6c, 0x6f, 0x63, 0x6b, 0x0a, 0x0a, 0x41, 0x64, 0x61, 0x20, 0xe2,
        0x80, 0x94, 0x20, 0x74, 0x72, 0x69, 0x70, 0x73, 0x20, 0x61, 0x74, 0x20, 0x34, 0x20, 0x70,
        0x43, 0x69, 0x2f, 0x4c, 0x20, 0x61, 0x6e, 0x64, 0x6c, 0x6f, 0x67, 0x20, 0x73, 0x61, 0x79,
        0x73, 0x20, 0x69, 0x74, 0x20, 0x68, 0x61, 0x73, 0x20, 0x64, 0x6f, 0x6e, 0x65, 0x74, 0x77,
        0x69, 0x63, 0x65, 0x20, 0x74, 0x68, 0x65, 0x65, 0x6b, 0x2e, 0x20, 0x4e, 0x6f, 0x74, 0x65,
        0x73, 0x20, 0x64, 0x69, 0x66, 0x66, 0x65, 0x72, 0x65, 0x6e, 0x63, 0x65, 0x20, 0x65, 0x6e,
        0x67, 0x69, 0x6e, 0x65, 0x20, 0x66, 0x6f, 0x6c, 0x6c, 0x6f, 0x77, 0x2c, 0x6c, 0x65, 0x6e,
        0x67, 0x74, 0x68, 0x2c, 0x20, 0x73, 0x6f, 0x20, 0x74, 0x68, 0x61, 0x74, 0x20, 0x74, 0x68,
        0x69, 0x73, 0x20, 0x63, 0x6f, 0x6d, 0x70, 0x72, 0x65, 0x73, 0x73, 0x65, 0x73, 0x2e, 0x0a,
        0x08, 0xfc, 0x9a, 0xf4, 0x9c, 0x92, 0xc0, 0x86, 0x33, 0x3c, 0x4b, 0x6c, 0xd5, 0x53, 0x6e,
        0x12, 0x4c, 0x35, 0x51, 0x3e, 0x20, 0x31, 0x00, 0x14, 0xdc, 0x9a, 0x04,
    ];
    /// The payload the three constants above were made from.
    const BODY_0_13: &str = "Subject: the tide gate interlock

Ada — the interlock trips at 4 pCi/L and the log says it has done so twice this week. Notes on the difference engine follow, at length, so that this compresses.
";

    #[test]
    fn a_body_written_by_zstd_0_13_still_reads() {
        assert_eq!(
            decompress(FRAME_0_13, None).expect("a 0.13 frame must still decode"),
            BODY_0_13,
            "zstd read the old frame and got different bytes out"
        );
    }

    #[test]
    fn a_body_written_by_zstd_0_13_against_a_dictionary_still_reads() {
        assert_eq!(
            decompress(FRAME_0_13_WITH_DICTIONARY, Some(DICTIONARY_0_13))
                .expect("a 0.13 dictionary frame must still decode"),
            BODY_0_13,
            "the frame decoded against its own dictionary and gave different bytes"
        );
    }

    #[test]
    fn a_dictionary_frame_will_not_decode_without_its_dictionary() {
        // The guard the two above rest on: if the dictionary were being
        // ignored, both would pass for the wrong reason.
        assert!(matches!(
            decompress(FRAME_0_13_WITH_DICTIONARY, None),
            Err(Error::UnreadableBody { .. })
        ));
    }
}
