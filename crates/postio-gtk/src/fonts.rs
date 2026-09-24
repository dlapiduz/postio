//! Barlow, Barlow Condensed and IBM Plex Mono, resolved from the binary.
//!
//! The design depends on all three faces; a fallback would silently wreck the
//! layout. The bytes are `postio-ui`'s (#799, ADR 0023) — [`FACES`] and
//! [`LICENSES`] are the same table the reader's `postio-font:` scheme handler
//! serves — under the SIL Open Font License, together with their licence
//! files.
//!
//! Pango can only take a font from a *path*, so the embedded faces are
//! unpacked once into the user's cache directory — content-addressed, so a
//! rebuild with new font data lands in a new directory and a second run is a
//! no-op — and handed to the default `PangoFontMap` with
//! `pango_font_map_add_font_file()`. Nothing is installed system-wide and
//! nothing is fetched: the bytes come from the executable itself.
//!
//! Call [`install`] once, **before** building any widgets: a `PangoContext`
//! that has already resolved a family keeps its answer, so fonts added later
//! would not reach labels that already exist.

use std::path::{Path, PathBuf};

use pango::prelude::*;
use postio_ui::reader::document::{FACES, LICENSES};

/// The three families this design needs, by the name CSS asks for.
pub const FAMILIES: [&str; 3] = ["Barlow", "Barlow Condensed", "IBM Plex Mono"];

#[derive(Debug)]
pub enum FontError {
    /// The cache directory could not be written.
    Cache(std::io::Error),
    /// Pango refused a face.
    Pango(glib::Error),
}

impl std::fmt::Display for FontError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FontError::Cache(e) => write!(f, "cannot unpack the embedded fonts: {e}"),
            FontError::Pango(e) => write!(f, "pango rejected an embedded font: {e}"),
        }
    }
}

impl std::error::Error for FontError {}

/// Unpack the embedded faces and register them with the default font map.
///
/// Returns the paths that were handed to Pango, in bundle order.
pub fn install() -> Result<Vec<PathBuf>, FontError> {
    let font_map = pangocairo::FontMap::default();
    install_into(font_map.upcast_ref::<pango::FontMap>())
}

/// As [`install`], for a font map you own — used by the tests.
pub fn install_into(font_map: &pango::FontMap) -> Result<Vec<PathBuf>, FontError> {
    let dir = cache_dir()?;
    let mut installed = Vec::new();

    for face in FACES {
        let file = dir.join(face.name);
        write_if_missing(&file, face.bytes).map_err(FontError::Cache)?;
        let name = file.to_string_lossy().into_owned();
        font_map.add_font_file(&name).map_err(FontError::Pango)?;
        installed.push(file);
    }

    Ok(installed)
}

/// The licence text shipped with each family, as `(family, OFL text)`.
///
/// The About dialog attributes the fonts from here rather than from a string
/// someone has to remember to update.
pub fn licenses() -> Vec<(String, String)> {
    LICENSES
        .iter()
        .map(|(family, text)| ((*family).to_owned(), (*text).to_owned()))
        .collect()
}

/// `~/.cache/postio/fonts/<digest>/` — the digest covers the bundled font
/// bytes, so an upgraded font never collides with a stale copy.
fn cache_dir() -> Result<PathBuf, FontError> {
    let mut digest = Fnv::new();
    for face in FACES {
        digest.write(face.name.as_bytes());
        digest.write(face.bytes);
    }
    let dir = glib::user_cache_dir()
        .join("postio")
        .join("fonts")
        .join(format!("{:016x}", digest.finish()));
    std::fs::create_dir_all(&dir).map_err(FontError::Cache)?;
    Ok(dir)
}

fn write_if_missing(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Ok(existing) = std::fs::metadata(path)
        && existing.len() == bytes.len() as u64
    {
        return Ok(());
    }
    // Write beside the target and rename, so a second process never sees a
    // half-written face.
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// FNV-1a's step, 64 bit, taken a word at a time. Enough to name a cache
/// directory, and it keeps this crate free of a hashing dependency.
///
/// A word rather than a byte per step because this runs on every launch,
/// over every embedded face -- 910 KB -- on the way to the first frame
/// (#1604), and a byte-wise FNV is a dependent multiply per byte. Not the
/// published FNV-1a any more, which nothing needs: the digest only has to
/// change when the fonts do.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }

    fn write(&mut self, bytes: &[u8]) {
        let words = bytes.chunks_exact(8);
        let tail = words.remainder();
        for word in words {
            let word = u64::from_le_bytes(word.try_into().expect("eight bytes"));
            self.0 ^= word;
            self.0 = self.0.wrapping_mul(0x1000_0000_01b3);
        }
        for b in tail {
            self.0 ^= *b as u64;
            self.0 = self.0.wrapping_mul(0x1000_0000_01b3);
        }
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::Fnv;

    fn digest(bytes: &[u8]) -> u64 {
        let mut digest = Fnv::new();
        digest.write(bytes);
        digest.finish()
    }

    #[test]
    fn a_changed_byte_anywhere_changes_the_directory() {
        // The cache directory is named for the fonts, so an upgraded face
        // must never land on a stale copy: every byte counts, including the
        // ones past the last whole word.
        let face: Vec<u8> = (0..=250u8).collect();
        let original = digest(&face);
        for position in [0, 7, 8, 100, 247, 248, 250] {
            let mut changed = face.clone();
            changed[position] ^= 1;
            assert_ne!(digest(&changed), original, "byte {position} was not hashed");
        }
    }
}
