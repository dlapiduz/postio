//! Handing a draft to another editor, and taking it back (#1270).
//!
//! Canvas 26 puts `⌃⌘E` on the compose window: open this draft in the
//! editor you actually like. What that costs is a file — the draft has to
//! exist somewhere the other program can open — and a file holding unsent
//! mail is a privacy question before it is a convenience.
//!
//! # Where it goes, and who can read it
//!
//! A private directory of the user's own, created 0700, with the file inside
//! it 0600. **Not the shared temp directory**: `/tmp` is world-readable on
//! every Unix, and a draft is mail that has not been sent yet — often the
//! most private mail there is, because it is the part still being thought
//! about.
//!
//! The file is named for the draft rather than randomly, so a second
//! hand-off of the same draft reuses it and an editor that reopens "the last
//! file" gets the right one.
//!
//! # Coming back
//!
//! Reading it back is a whole-file read, and the frontend decides *when* —
//! most naturally when its window returns to the front. What must not happen
//! is a read that races the editor's write: an editor that saves by writing
//! a new file and renaming it over the old one is atomic, and one that
//! truncates and writes is not, which is why an empty read is refused rather
//! than treated as "they deleted everything".

use std::io::Write;
use std::path::{Path, PathBuf};

/// A draft that is out with another editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handoff {
    /// The file the other editor has.
    pub path: PathBuf,
}

/// Write `body` where another editor can open it.
pub fn begin(directory: &Path, draft: i64, body: &str) -> Result<Handoff, String> {
    std::fs::create_dir_all(directory)
        .map_err(|error| format!("Postio could not make a place for the draft: {error}"))?;
    restrict(directory, 0o700)?;

    // Named for the draft, so a second hand-off reuses it. `.eml` would
    // invite a mail client; `.txt` is what an editor opens without asking.
    let path = directory.join(format!("postio-draft-{draft}.txt"));
    let mut file = std::fs::File::create(&path)
        .map_err(|error| format!("The draft could not be written: {error}"))?;
    file.write_all(body.as_bytes())
        .map_err(|error| format!("The draft could not be written: {error}"))?;
    file.flush()
        .map_err(|error| format!("The draft could not be written: {error}"))?;
    drop(file);
    restrict(&path, 0o600)?;

    Ok(Handoff { path })
}

/// Read back what the other editor left.
///
/// An **empty** file is refused rather than believed. An editor that saves by
/// truncating and writing is briefly empty on disk, and a frontend polling on
/// window focus can land in that window — where believing it would silently
/// throw away everything the user had written.
pub fn read_back(handoff: &Handoff) -> Result<String, String> {
    let text = std::fs::read_to_string(&handoff.path)
        .map_err(|error| format!("The edited draft could not be read: {error}"))?;
    if text.trim().is_empty() {
        return Err(
            "The edited file was empty, so nothing was taken back. Clear the \
             message in Postio if that is what you meant."
                .to_owned(),
        );
    }
    Ok(text)
}

/// Take the file away again.
///
/// Best effort: a file left behind is a draft's text sitting on disk, which
/// is worth trying to avoid and not worth failing an edit over.
pub fn finish(handoff: &Handoff) {
    if let Err(error) = std::fs::remove_file(&handoff.path) {
        tracing::debug!(%error, "a hand-off file could not be removed");
    }
}

/// Unix permissions, and a no-op anywhere else.
fn restrict(path: &Path, mode: u32) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            .map_err(|error| format!("The draft's file could not be made private: {error}"))?;
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    Ok(())
}
