//! How a message body sits in its column: zstd when that is smaller, the
//! text itself when it is not.
//!
//! Bodies are text in a column rather than files (ADR 0020), and they are
//! most of a store's bytes -- `table_shape` measured 91% of the messages
//! table as body bytes on a real account. They were zstd frames until the
//! engine changed and the full-text index moved onto the column, because an
//! index cannot tokenise compressed bytes; the index reads its own folded
//! table now (`message_search_bodies`), so the column is free to be small
//! again.
//!
//! # One column, two encodings, no version byte
//!
//! A zstd frame begins with a magic number that no UTF-8 text can start
//! with, so a reader tells the two apart by looking rather than by being
//! told. That is what makes the encoding a per-row decision -- a short or
//! incompressible body is stored as it is, since a frame around it would be
//! larger -- and what lets a store written before this module read without a
//! migration: its bodies are text, and text is one of the two shapes.
//!
//! Level 3 is zstd's default and the blob store's choice too
//! (`blob::format::LEVEL`): the ratio hardly moves above it and the cost
//! does, and this runs on the backfill's write path.

/// The compression level; see the module docs.
const LEVEL: i32 = 3;

/// The first four bytes of every zstd frame (RFC 8878 §3.1.1).
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

/// `text` as it goes into the column: a zstd frame when that is smaller,
/// the bytes of the text when it is not.
pub fn pack(text: &str) -> Vec<u8> {
    match zstd::bulk::compress(text.as_bytes(), LEVEL) {
        Ok(frame) if frame.len() < text.len() => frame,
        _ => text.as_bytes().to_vec(),
    }
}

/// The text a column holds, whichever shape [`pack`] chose for it.
///
/// A frame that will not decode -- a truncated write, a corrupted page --
/// comes back as the lossy reading of the bytes rather than nothing, the
/// same "show what arrived" promise the parser keeps: the reader says
/// something rather than an empty pane.
pub fn unpack(bytes: Vec<u8>) -> String {
    if bytes.starts_with(&ZSTD_MAGIC)
        && let Ok(text) = zstd::bulk::decompress(&bytes, MAX_BODY_BYTES)
    {
        return String::from_utf8(text)
            .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned());
    }
    String::from_utf8(bytes)
        .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned())
}

/// The most a decoded body may be, so a corrupted frame cannot ask for the
/// machine. Well above `[sync] max_body_bytes`' ceiling for what is
/// fetched at all.
const MAX_BODY_BYTES: usize = 256 * 1024 * 1024;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repetitive_body_round_trips_through_a_smaller_column() {
        let text = "<p>the same line, over and over</p>\n".repeat(200);
        let packed = pack(&text);
        assert!(packed.starts_with(&ZSTD_MAGIC));
        assert!(
            packed.len() < text.len() / 4,
            "{} of {}",
            packed.len(),
            text.len()
        );
        assert_eq!(unpack(packed), text);
    }

    #[test]
    fn a_body_that_would_not_shrink_is_stored_as_it_is() {
        for text in ["", "hi", "a short line nothing repeats in"] {
            let packed = pack(text);
            assert_eq!(packed, text.as_bytes(), "{text:?}");
            assert_eq!(unpack(packed), text);
        }
    }

    #[test]
    fn a_column_written_as_text_before_this_module_reads_as_it_did() {
        let text = "café — written by the store that came before";
        assert_eq!(unpack(text.as_bytes().to_vec()), text);
    }

    #[test]
    fn a_frame_that_will_not_decode_reads_as_something_rather_than_nothing() {
        let mut broken = pack(&"x".repeat(1000));
        broken.truncate(8);
        assert!(broken.starts_with(&ZSTD_MAGIC));
        let _ = unpack(broken);
    }
}
