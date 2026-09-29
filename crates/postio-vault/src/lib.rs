//! Obsidian capture for Postio Focus (spec 007 milestone 3, FR-180, FR-181,
//! US15, research R16).
//!
//! A vault is a folder of markdown on this computer. Postio writes to it the
//! way a person with a text editor would, and nothing more:
//!
//! - **Tasks** are Obsidian Tasks lines,
//!   `- [ ] <text> [✉](postio://message/<id>) 📅 YYYY-MM-DD`, appended to a
//!   project's note or the tasks note. The link goes **before** the date: the
//!   Tasks plugin reads its fields from the end of the line and allows only
//!   tags and block ids after them, so a link after the date would hide the
//!   date from it (spec C21). 📅 is U+1F4C5.
//! - **Notes** are created, or appended to, with a quoted excerpt only when
//!   the person asks for one.
//! - **Nothing is ever edited** beyond appending: the bytes a note had stay
//!   the bytes it has, in front of what is added.
//! - **Projects** are notes with `type: project` in their frontmatter, or
//!   notes in the configured projects folder, and one is suggested for a
//!   capture with the reason why.
//! - **Finished tasks are read back** -- `- [x] … ✅ YYYY-MM-DD` -- so Focus
//!   can show "Task in <project> · due <day>" on a row and offer to archive
//!   the conversation once the task is ticked.
//!
//! It is plain file access: no network, no plugin, no store and no toolkit,
//! which `scripts/checks/check-crate-boundaries.py` holds. Every path it
//! writes is inside the vault; a note named outside it is refused.

use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use postio_model::MessageId;

/// Why the vault could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    /// The configured vault is not a folder on this computer.
    #[error("the vault is not a folder on this computer")]
    NotAFolder,
    /// A note was named outside the vault, or by an absolute path.
    #[error("that note is not inside the vault")]
    OutsideTheVault,
    /// Reading or writing a note failed.
    #[error("the vault could not be read or written: {0}")]
    Io(#[from] std::io::Error),
}

/// A vault on this computer, as `[focus.vault]` names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vault {
    root: PathBuf,
    tasks_note: PathBuf,
    projects: Option<PathBuf>,
}

/// A task to capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    /// What to do: the action sentence, verbatim, or the subject.
    pub text: String,
    /// The message it came from, which its link opens.
    pub message: MessageId,
    /// When it is due, if it is.
    pub due: Option<NaiveDate>,
}

/// A note entry to capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteEntry {
    /// What the person wrote.
    pub text: String,
    /// The message it came from, which its link opens.
    pub message: MessageId,
    /// An excerpt of the message, only when the person asked to quote it.
    pub quote: Option<String>,
}

/// A project note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    /// Its name: the note's file name without `.md`.
    pub name: String,
    /// The note, relative to the vault.
    pub note: PathBuf,
}

/// A project suggested for a capture, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// The project.
    pub project: Project,
    /// Why this one.
    pub reason: Reason,
}

/// Why a project was suggested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// A word of its name is in the message's subject.
    NamedInSubject(String),
}

/// A task Postio captured, as the vault has it now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedTask {
    /// The note it is in, relative to the vault.
    pub note: PathBuf,
    /// The message its link names.
    pub message: MessageId,
    /// Its text.
    pub text: String,
    /// When it is due, if it says.
    pub due: Option<NaiveDate>,
    /// Whether it is ticked.
    pub finished: bool,
    /// When it was done, if the Tasks plugin wrote it.
    pub done: Option<NaiveDate>,
}

/// What a capture appended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Captured {
    /// The note, relative to the vault.
    pub note: PathBuf,
    /// The line appended.
    pub line: String,
}

/// The link a captured line carries, before its message id.
const LINK: &str = "[\u{2709}](postio://message/";

/// The Tasks plugin's due date signifier, U+1F4C5.
const DUE: char = '\u{1F4C5}';

/// The Tasks plugin's done date signifier, U+2705.
const DONE: char = '\u{2705}';

/// The emoji the Tasks plugin reads fields by. A capture's text never carries
/// one, or the plugin would read a field the person never set.
const SIGNIFIERS: &[char] = &[
    '\u{1F4C5}', // due
    '\u{2705}',  // done
    '\u{23F3}',  // scheduled
    '\u{1F6EB}', // start
    '\u{2795}',  // created
    '\u{274C}',  // cancelled
    '\u{1F501}', // recurring
    '\u{23EB}',  // highest
    '\u{1F53A}', // high
    '\u{1F53C}', // medium
    '\u{1F53D}', // low
    '\u{23EC}',  // lowest
];

/// How much of a note is read to find its frontmatter.
const FRONTMATTER_BYTES: u64 = 4 * 1024;

/// What a capture's text becomes on its line: one line, runs of whitespace
/// one space, and no field signifier.
fn one_line(text: &str) -> String {
    text.split_whitespace()
        .map(|word| {
            word.chars()
                .filter(|c| !SIGNIFIERS.contains(c))
                .collect::<String>()
        })
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

impl Task {
    /// The Obsidian Tasks line: `- [ ] <text> [✉](postio://message/<id>)`,
    /// then ` 📅 YYYY-MM-DD` when it is due.
    pub fn line(&self) -> String {
        let mut line = format!(
            "- [ ] {} {LINK}{})",
            one_line(&self.text),
            self.message.get()
        );
        if let Some(due) = self.due {
            line.push_str(&format!(" {DUE} {}", due.format("%Y-%m-%d")));
        }
        line
    }
}

impl NoteEntry {
    /// The lines the entry appends: the text and its link, then the quote,
    /// when asked for, as quoted lines under it.
    pub fn lines(&self) -> String {
        let mut lines = format!("- {} {LINK}{})\n", one_line(&self.text), self.message.get());
        if let Some(quote) = &self.quote {
            for line in quote.lines() {
                lines.push_str(&format!("  > {line}\n"));
            }
        }
        lines
    }
}

/// `note` if it names a place inside the vault: relative, and never
/// climbing out of it.
fn inside(note: &Path) -> Result<PathBuf, VaultError> {
    use std::path::Component;
    let mut kept = PathBuf::new();
    for component in note.components() {
        match component {
            Component::Normal(part) => kept.push(part),
            Component::CurDir => {}
            _ => return Err(VaultError::OutsideTheVault),
        }
    }
    if kept.as_os_str().is_empty() {
        return Err(VaultError::OutsideTheVault);
    }
    Ok(kept)
}

impl Vault {
    /// The vault at `root`, with its tasks note (`Tasks.md` unless given)
    /// and, optionally, a folder of project notes, both relative to it.
    pub fn open(
        root: impl Into<PathBuf>,
        tasks_note: Option<&Path>,
        projects: Option<&Path>,
    ) -> Result<Self, VaultError> {
        let root = root.into();
        if !root.is_dir() {
            return Err(VaultError::NotAFolder);
        }
        Ok(Self {
            root,
            tasks_note: inside(tasks_note.unwrap_or(Path::new("Tasks.md")))?,
            projects: projects.map(inside).transpose()?,
        })
    }

    /// The vault's folder.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Append `task` to `project`'s note, or to the tasks note: exactly one
    /// line, after a newline only if the note did not end in one.
    pub fn append_task(
        &self,
        project: Option<&Project>,
        task: &Task,
    ) -> Result<Captured, VaultError> {
        let note = match project {
            Some(project) => inside(&project.note)?,
            None => self.tasks_note.clone(),
        };
        let line = task.line();
        self.append(&note, &format!("{line}\n"))?;
        Ok(Captured { note, line })
    }

    /// Append `entry` to `note`, creating it if it is not there.
    pub fn append_note(&self, note: &Path, entry: &NoteEntry) -> Result<Captured, VaultError> {
        let note = inside(note)?;
        let lines = entry.lines();
        self.append(&note, &lines)?;
        Ok(Captured {
            note,
            line: lines.trim_end().to_owned(),
        })
    }

    /// Append `text` to the note at `note`, relative to the vault: in one
    /// write, after a newline only if the note did not end in one. The
    /// note's own bytes are never rewritten.
    fn append(&self, note: &Path, text: &str) -> Result<(), VaultError> {
        use std::io::{Read, Seek, SeekFrom, Write};
        let path = self.root.join(note);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(&path)?;
        let length = file.metadata()?.len();
        let mut needs_newline = false;
        if length > 0 {
            file.seek(SeekFrom::Start(length - 1))?;
            let mut last = [0u8; 1];
            file.read_exact(&mut last)?;
            needs_newline = last[0] != b'\n';
        }
        let mut bytes = Vec::with_capacity(text.len() + 1);
        if needs_newline {
            bytes.push(b'\n');
        }
        bytes.extend_from_slice(text.as_bytes());
        file.write_all(&bytes)?;
        file.flush()?;
        Ok(())
    }

    /// The vault's projects: notes with `type: project` in their
    /// frontmatter, and the notes in the projects folder. Hidden folders --
    /// `.obsidian`, `.trash` -- are not read.
    pub fn projects(&self) -> Result<Vec<Project>, VaultError> {
        let mut projects = Vec::new();
        for note in self.notes()? {
            let in_folder = self
                .projects
                .as_ref()
                .is_some_and(|folder| note.starts_with(folder));
            if in_folder || self.is_project(&note)? {
                let name = note
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_default();
                projects.push(Project { name, note });
            }
        }
        projects.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(projects)
    }

    /// Every markdown note in the vault, relative to it.
    fn notes(&self) -> Result<Vec<PathBuf>, VaultError> {
        let mut notes = Vec::new();
        let mut folders = vec![PathBuf::new()];
        while let Some(folder) = folders.pop() {
            for entry in std::fs::read_dir(self.root.join(&folder))? {
                let entry = entry?;
                let name = entry.file_name();
                if name.to_string_lossy().starts_with('.') {
                    continue;
                }
                let relative = folder.join(&name);
                let kind = entry.file_type()?;
                if kind.is_dir() {
                    folders.push(relative);
                } else if kind.is_file()
                    && relative
                        .extension()
                        .is_some_and(|extension| extension == "md")
                {
                    notes.push(relative);
                }
            }
        }
        notes.sort();
        Ok(notes)
    }

    /// Whether `note`'s frontmatter says `type: project`.
    fn is_project(&self, note: &Path) -> Result<bool, VaultError> {
        use std::io::Read;
        let mut head = String::new();
        std::fs::File::open(self.root.join(note))?
            .take(FRONTMATTER_BYTES)
            .read_to_string(&mut head)
            .ok();
        let mut lines = head.lines();
        if lines.next().map(str::trim_end) != Some("---") {
            return Ok(false);
        }
        for line in lines {
            let line = line.trim_end();
            if line == "---" {
                break;
            }
            if let Some((key, value)) = line.split_once(':')
                && key.trim().eq_ignore_ascii_case("type")
                && value
                    .trim()
                    .trim_matches(|c| c == '"' || c == '\'')
                    .eq_ignore_ascii_case("project")
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// The project to suggest for mail with `subject`, and why: the one with
    /// the most words of its name in the subject. Words of fewer than four
    /// letters are not counted, and nothing is suggested on a guess.
    pub fn suggest(&self, subject: &str) -> Result<Option<Suggestion>, VaultError> {
        let words = |text: &str| -> Vec<String> {
            text.split(|c: char| !c.is_alphanumeric())
                .filter(|word| word.chars().count() >= 4)
                .map(str::to_lowercase)
                .collect()
        };
        let subject = words(subject);
        let mut best: Option<(usize, Project, String)> = None;
        for project in self.projects()? {
            let shared: Vec<String> = words(&project.name)
                .into_iter()
                .filter(|word| subject.contains(word))
                .collect();
            let Some(first) = shared.first().cloned() else {
                continue;
            };
            if best
                .as_ref()
                .is_none_or(|(count, ..)| shared.len() > *count)
            {
                best = Some((shared.len(), project, first));
            }
        }
        Ok(best.map(|(_, project, word)| Suggestion {
            project,
            reason: Reason::NamedInSubject(word),
        }))
    }

    /// Every task Postio captured -- a Tasks line with a `postio://` link --
    /// in the tasks note and the project notes.
    pub fn tasks(&self) -> Result<Vec<CapturedTask>, VaultError> {
        let mut notes = vec![self.tasks_note.clone()];
        for project in self.projects()? {
            if !notes.contains(&project.note) {
                notes.push(project.note);
            }
        }
        let mut tasks = Vec::new();
        for note in notes {
            let text = match std::fs::read_to_string(self.root.join(&note)) {
                Ok(text) => text,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            tasks.extend(text.lines().filter_map(|line| captured(&note, line)));
        }
        Ok(tasks)
    }
}

/// `line` as a task Postio captured, if it is one.
fn captured(note: &Path, line: &str) -> Option<CapturedTask> {
    let line = line.trim_start();
    let (finished, rest) = match line.strip_prefix("- [ ] ") {
        Some(rest) => (false, rest),
        None => (
            true,
            line.strip_prefix("- [x] ")
                .or_else(|| line.strip_prefix("- [X] "))?,
        ),
    };
    let (text, after) = rest.split_once(LINK)?;
    let (id, fields) = after.split_once(')')?;
    let message = MessageId::new(id.parse().ok()?);
    let date_after = |signifier: char| {
        let (_, after) = fields.split_once(signifier)?;
        NaiveDate::parse_from_str(after.trim_start().get(..10)?, "%Y-%m-%d").ok()
    };
    Some(CapturedTask {
        note: note.to_path_buf(),
        message,
        text: text.trim().to_owned(),
        due: date_after(DUE),
        finished,
        done: date_after(DONE),
    })
}
