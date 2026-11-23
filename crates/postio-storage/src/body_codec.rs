//! A message body in its column: a zstd frame when smaller, else the text itself.
//! A frame's magic cannot start UTF-8 text, so a reader tells the two apart per row with no version
//! byte (ADR 0020).

use crate::blob::format_level as LEVEL;

const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

pub fn pack(text: &str) -> Vec<u8> {
    match zstd::bulk::compress(text.as_bytes(), LEVEL) {
        Ok(frame) if frame.len() < text.len() => frame,
        _ => text.as_bytes().to_vec(),
    }
}

/// A frame that will not decode (truncated write, corrupted page) comes back as the lossy reading
/// of the bytes, not an empty pane.
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

/// Bounds decoding so a corrupted frame cannot ask for the machine.
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

    /// Bytes from zstd 0.13: stores on disk hold such frames, and a newer zstd must read them.
    const FRAME_FROM_ZSTD_0_13: &str = "28b52ffd606800850100540244656172204164612c0a74686520696e766f6963652069732061747461636865643b202e0a0200295fa9785ccf27";

    fn frame_from_zstd_0_13() -> Vec<u8> {
        (0..FRAME_FROM_ZSTD_0_13.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&FRAME_FROM_ZSTD_0_13[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn a_body_compressed_by_an_older_zstd_still_reads() {
        let text = "Dear Ada,\nthe invoice is attached; the invoice is attached.\n".repeat(6);
        assert_eq!(unpack(frame_from_zstd_0_13()), text);
    }

    #[test]
    fn a_frame_from_an_older_zstd_still_reads_through_the_stream_decoder() {
        // The blob store reads through `stream::read::Decoder`, not `bulk`.
        let text = "Dear Ada,\nthe invoice is attached; the invoice is attached.\n".repeat(6);
        let frame = frame_from_zstd_0_13();
        let mut decoder = zstd::stream::read::Decoder::new(frame.as_slice()).unwrap();
        let mut out = String::new();
        std::io::Read::read_to_string(&mut decoder, &mut out).unwrap();
        assert_eq!(out, text);
    }
}
