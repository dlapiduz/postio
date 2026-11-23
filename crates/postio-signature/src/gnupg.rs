use pgp::composed::{Deserializable, SignedPublicKey};

use crate::keys::KeyInfo;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyRead {
    pub blocks: Vec<KeyInfo>,
    pub skipped: u32,
}

const TRUST_PACKET: u8 = 12;
const BLOB_OPENPGP: u8 = 2;

pub fn keyblocks_from_pubring_gpg(bytes: &[u8]) -> KeyRead {
    let mut kept = Vec::with_capacity(bytes.len());
    let mut at = 0usize;
    let mut cut_short = false;
    while at < bytes.len() {
        let Some((tag, end)) = packet(bytes, at) else {
            cut_short = true;
            break;
        };
        if tag != TRUST_PACKET {
            kept.extend_from_slice(&bytes[at..end]);
        }
        at = end;
    }
    let mut read = parse_many(&kept);
    read.skipped += u32::from(cut_short);
    read
}

pub fn keyblocks_from_keybox(bytes: &[u8]) -> KeyRead {
    let mut read = KeyRead::default();
    let mut at = 0usize;
    while at < bytes.len() {
        let Some(length) = be32(bytes, at).map(|length| length as usize) else {
            read.skipped += 1;
            break;
        };
        let Some(blob) = at
            .checked_add(length)
            .filter(|end| length >= 6 && *end <= bytes.len())
            .map(|end| &bytes[at..end])
        else {
            read.skipped += 1;
            break;
        };
        at += length;
        if blob[4] != BLOB_OPENPGP {
            continue;
        }
        let keyblock = be32(blob, 8)
            .zip(be32(blob, 12))
            .and_then(|(offset, size)| {
                let start = offset as usize;
                start
                    .checked_add(size as usize)
                    .filter(|end| *end <= blob.len())
                    .map(|end| &blob[start..end])
            });
        match keyblock.and_then(parse_one) {
            Some(info) => read.blocks.push(info),
            None => read.skipped += 1,
        }
    }
    read
}

fn be32(bytes: &[u8], at: usize) -> Option<u32> {
    let slice = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn packet(bytes: &[u8], at: usize) -> Option<(u8, usize)> {
    let first = *bytes.get(at)?;
    if first & 0x80 == 0 {
        return None;
    }
    let (tag, body_at, length) = if first & 0x40 != 0 {
        let tag = first & 0x3f;
        let octet = *bytes.get(at + 1)? as usize;
        match octet {
            0..=191 => (tag, at + 2, octet),
            192..=223 => {
                let second = *bytes.get(at + 2)? as usize;
                (tag, at + 3, ((octet - 192) << 8) + second + 192)
            }
            255 => (tag, at + 6, be32(bytes, at + 2)? as usize),
            _ => return None,
        }
    } else {
        let tag = (first >> 2) & 0x0f;
        match first & 0x03 {
            0 => (tag, at + 2, *bytes.get(at + 1)? as usize),
            1 => {
                let high = *bytes.get(at + 1)? as usize;
                let low = *bytes.get(at + 2)? as usize;
                (tag, at + 3, (high << 8) | low)
            }
            2 => (tag, at + 5, be32(bytes, at + 1)? as usize),
            _ => (tag, at + 1, bytes.len().checked_sub(at + 1)?),
        }
    };
    let end = body_at
        .checked_add(length)
        .filter(|end| *end <= bytes.len())?;
    Some((tag, end))
}

fn parse_many(bytes: &[u8]) -> KeyRead {
    let mut read = KeyRead::default();
    let parsed = std::panic::catch_unwind(|| {
        SignedPublicKey::from_bytes_many(bytes)
            .map(|keys| keys.collect::<Vec<_>>())
            .unwrap_or_default()
    });
    for key in parsed.unwrap_or_default() {
        match key.ok().as_ref().and_then(KeyInfo::of) {
            Some(info) => read.blocks.push(info),
            None => read.skipped += 1,
        }
    }
    read
}

fn parse_one(bytes: &[u8]) -> Option<KeyInfo> {
    std::panic::catch_unwind(|| SignedPublicKey::from_bytes(bytes).ok())
        .ok()
        .flatten()
        .as_ref()
        .and_then(KeyInfo::of)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyblock(name: &str) -> Vec<u8> {
        let path = format!(
            "{}/../postio-model/tests/corpus/keys/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        crate::keys::keys_in(&std::fs::read(path).expect("a corpus key"))
            .remove(0)
            .keyblock
    }

    fn trust() -> Vec<u8> {
        vec![0xb0, 0x02, 0x06, 0x00]
    }

    fn blob(kind: u8, payload: &[u8]) -> Vec<u8> {
        let length = 20 + payload.len();
        let mut out = Vec::with_capacity(length);
        out.extend_from_slice(&(length as u32).to_be_bytes());
        out.push(kind);
        out.push(1);
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&20u32.to_be_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(payload);
        out
    }

    fn header() -> Vec<u8> {
        let mut out = 32u32.to_be_bytes().to_vec();
        out.push(1);
        out.extend_from_slice(&[1; 27]);
        out
    }

    #[test]
    fn pubring_gpg_skips_the_trust_packets_between_keys() {
        let mut bytes = keyblock("ada-a.asc");
        bytes.extend(trust());
        bytes.extend(keyblock("ada-b.asc"));
        bytes.extend(trust());
        let read = keyblocks_from_pubring_gpg(&bytes);
        assert_eq!((read.blocks.len(), read.skipped), (2, 0), "{read:?}");
        assert!(
            read.blocks
                .iter()
                .all(|key| key.addresses == ["ada@example.com"])
        );
    }

    #[test]
    fn a_keybox_yields_its_openpgp_blobs_and_passes_over_x509() {
        let mut bytes = header();
        bytes.extend(blob(2, &keyblock("ada-a.asc")));
        bytes.extend(blob(3, b"0\x82 not really DER"));
        bytes.extend(blob(2, &keyblock("ada-b.asc")));
        let read = keyblocks_from_keybox(&bytes);
        assert_eq!((read.blocks.len(), read.skipped), (2, 0), "{read:?}");
    }

    #[test]
    fn a_truncated_blob_stops_the_read_and_is_counted() {
        let mut bytes = header();
        bytes.extend(blob(2, &keyblock("ada-a.asc")));
        let mut cut = blob(2, &keyblock("ada-b.asc"));
        cut.truncate(cut.len() / 2);
        bytes.extend(cut);
        let read = keyblocks_from_keybox(&bytes);
        assert_eq!((read.blocks.len(), read.skipped), (1, 1), "{read:?}");
    }

    #[test]
    fn a_keyblock_whose_stated_length_overruns_its_blob_is_skipped() {
        let mut overrun = blob(2, &keyblock("ada-a.asc"));
        overrun[12..16].copy_from_slice(&u32::MAX.to_be_bytes());
        let mut bytes = header();
        bytes.extend(overrun);
        bytes.extend(blob(2, &keyblock("ada-b.asc")));
        let read = keyblocks_from_keybox(&bytes);
        assert_eq!((read.blocks.len(), read.skipped), (1, 1), "{read:?}");
    }

    #[test]
    fn garbage_is_nothing_and_never_a_panic() {
        for bytes in [
            &b""[..],
            b"\x00",
            b"\xff\xff\xff\xff",
            b"\x00\x00\x00\x05\x02",
            b"\xb0",
            b"\xc6\xff\xff\xff\xff\xff",
        ] {
            let _ = keyblocks_from_keybox(bytes);
            let _ = keyblocks_from_pubring_gpg(bytes);
        }
        let read = keyblocks_from_pubring_gpg(b"\xb0\x05ab");
        assert!(read.blocks.is_empty());
    }
}
