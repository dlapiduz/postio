//! Paths a person types: `~` expanded, and Tab completing from the disk.

use std::path::{Path, PathBuf};

/// `typed` with a leading `~/` meaning the home directory, as in a shell.
pub fn expand(typed: &str) -> PathBuf {
    match (typed.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => Path::new(&home).join(rest),
        _ => PathBuf::from(typed),
    }
}

/// `typed`, completed as far as the entries on disk agree: the longest
/// prefix every matching name shares, and a `/` after a directory that is
/// the only match. Unchanged when nothing matches. Hidden entries are only
/// offered once a `.` has been typed.
pub fn complete(typed: &str) -> String {
    let (folder, partial) = match typed.rfind('/') {
        Some(slash) => (&typed[..=slash], &typed[slash + 1..]),
        None => ("", typed),
    };
    let listed = if folder.is_empty() {
        PathBuf::from(".")
    } else {
        expand(folder)
    };
    let Ok(entries) = std::fs::read_dir(&listed) else {
        return typed.to_owned();
    };
    let mut names: Vec<(String, bool)> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let is_dir = entry.file_type().ok()?.is_dir();
            (name.starts_with(partial) && (partial.starts_with('.') || !name.starts_with('.')))
                .then_some((name, is_dir))
        })
        .collect();
    names.sort();
    let Some((first, _)) = names.first() else {
        return typed.to_owned();
    };
    let mut shared = first.clone();
    for (name, _) in &names[1..] {
        let common = shared
            .char_indices()
            .zip(name.chars())
            .take_while(|((_, a), b)| a == b)
            .last()
            .map_or(0, |((at, c), _)| at + c.len_utf8());
        shared.truncate(common);
    }
    let slash = if names.len() == 1 && names[0].1 {
        "/"
    } else {
        ""
    };
    format!("{folder}{shared}{slash}")
}

/// What to call `path` when telling someone about it: its file name.
pub fn name_of(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_stops_where_the_names_part() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["minutes-june.pdf", "minutes-july.pdf", ".hidden"] {
            std::fs::write(dir.path().join(name), b"").unwrap();
        }
        std::fs::create_dir(dir.path().join("photos")).unwrap();
        let at = |rest: &str| format!("{}/{rest}", dir.path().display());

        assert_eq!(complete(&at("min")), at("minutes-ju"));
        assert_eq!(
            complete(&at("ph")),
            at("photos/"),
            "a lone directory gets its slash"
        );
        assert_eq!(
            complete(&at("zzz")),
            at("zzz"),
            "nothing matches, nothing changes"
        );
        assert_eq!(
            complete(&at("")),
            at(""),
            "no common start among the visible ones"
        );
        assert_eq!(complete(&at(".h")), at(".hidden"));
    }

    #[test]
    fn a_tilde_is_home() {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        if let Some(home) = home {
            assert_eq!(expand("~/notes.txt"), home.join("notes.txt"));
        }
        assert_eq!(expand("./notes.txt"), PathBuf::from("./notes.txt"));
    }
}
