//! Obsidian capture for Postio Focus. A vault is a folder of markdown; Postio only appends to it,
//! as a person with a text editor would, and refuses a note outside it.
//! A task is a Tasks line `- [ ] <text> [✉](postio://message/<id>) 📅 YYYY-MM-DD`; the link goes
//! before the date because the Tasks plugin reads its fields from the end of the line and a link
//! after the date would hide it (spec C21). Finished tasks (`- [x] … ✅`) are read back so Focus can
//! offer to archive the conversation.

use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use postio_model::MessageId;

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("the vault is not a folder on this computer")]
    NotAFolder,
    #[error("that note is not inside the vault")]
    OutsideTheVault,
    #[error("the vault could not be read or written: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vault {
    root: PathBuf,
    tasks_note: PathBuf,
    projects: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub text: String,
    pub message: MessageId,
    pub due: Option<NaiveDate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteEntry {
    pub text: String,
    pub message: MessageId,
    pub quote: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub name: String,
    pub note: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub project: Project,
    pub reason: Reason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    NamedInSubject(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedTask {
    pub note: PathBuf,
    pub message: MessageId,
    pub text: String,
    pub due: Option<NaiveDate>,
    pub finished: bool,
    pub done: Option<NaiveDate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Captured {
    pub note: PathBuf,
    pub line: String,
}

const LINK: &str = "[\u{2709}](postio://message/";

const DUE: char = '\u{1F4C5}';

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

const FRONTMATTER_BYTES: u64 = 4 * 1024;

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

    pub fn root(&self) -> &Path {
        &self.root
    }

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

    pub fn append_note(&self, note: &Path, entry: &NoteEntry) -> Result<Captured, VaultError> {
        let note = inside(note)?;
        let lines = entry.lines();
        self.append(&note, &lines)?;
        Ok(Captured {
            note,
            line: lines.trim_end().to_owned(),
        })
    }

    /// In one write, after a newline only if the note did not end in one; the note's own bytes are
    /// never rewritten.
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
