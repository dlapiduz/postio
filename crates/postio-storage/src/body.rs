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
}
