//! Handing a draft to another editor, and taking it back (#1270).
//! The draft goes in a private directory (0700) with the file 0600, never shared `/tmp`: unsent
//! mail is private. It is named for the draft so a second hand-off reuses it.
//! Reading back refuses an empty file: an editor that truncates then writes is briefly empty.

use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handoff {
    pub path: PathBuf,
}

pub fn begin(directory: &Path, draft: i64, body: &str) -> Result<Handoff, String> {
    std::fs::create_dir_all(directory)
        .map_err(|error| format!("Postio could not make a place for the draft: {error}"))?;
    restrict(directory, 0o700)?;

    // `.txt`, not `.eml`: an editor opens it without asking.
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

/// Read back what the other editor left. An empty file is refused, not believed: an editor that
/// truncates and writes is briefly empty, and believing it would discard the draft.
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

/// Take the file away again. Best effort: a leftover is worth avoiding, not worth failing an edit
/// over.
pub fn finish(handoff: &Handoff) {
    if let Err(error) = std::fs::remove_file(&handoff.path) {
        tracing::debug!(%error, "a hand-off file could not be removed");
    }
}

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
