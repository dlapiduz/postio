//! Capture into a throwaway vault (spec 007 T157, US15 scenario 1): the
//! exact bytes appended, and nothing else in the vault changed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use postio_model::MessageId;
use postio_vault::{NoteEntry, Reason, Task, Vault, VaultError};

/// Every file in the vault and its bytes.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("a folder") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let bytes = std::fs::read(&path).expect("a file");
                files.insert(path.strip_prefix(root).unwrap().to_path_buf(), bytes);
            }
        }
    }
    files
}

/// A vault with a tasks note, two projects -- one by frontmatter, one by
/// folder -- and a note that is neither.
fn vault() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a vault");
    let write = |name: &str, text: &str| {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    write("Tasks.md", "# Tasks\n\n- [ ] Water the plants\n");
    write(
        "Quarter close.md",
        "---\ntype: project\nstatus: active\n---\n# Quarter close\n",
    );
    write("Projects/Harbour redevelopment.md", "# Harbour\n");
    write("Journal/2026-09-28.md", "Rain again.\n");
    write(".obsidian/app.json", "{}");
    dir
}

fn open(dir: &tempfile::TempDir) -> Vault {
    Vault::open(dir.path(), None, Some(Path::new("Projects"))).expect("the vault")
}

fn day(month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, month, day).expect("a day")
}

#[test]
fn a_captured_task_is_exactly_one_line_appended_and_nothing_else_changes() {
    // US15 scenario 1, spec C21: the link before the date, where the Tasks
    // plugin still reads the date.
    let dir = vault();
    let before = snapshot(dir.path());

    let captured = open(&dir)
        .append_task(
            None,
            &Task {
                text: "Please leave comments by Wednesday".to_owned(),
                message: MessageId::new(42),
                due: Some(day(9, 30)),
            },
        )
        .expect("captured");

    let line = "- [ ] Please leave comments by Wednesday [\u{2709}](postio://message/42) \u{1F4C5} 2026-09-30";
    assert_eq!(captured.line, line);
    assert_eq!(captured.note, PathBuf::from("Tasks.md"));
    let mut after = snapshot(dir.path());
    let tasks = after.remove(Path::new("Tasks.md")).expect("the tasks note");
    let mut expected = before[Path::new("Tasks.md")].clone();
    expected.extend_from_slice(format!("{line}\n").as_bytes());
    assert_eq!(
        String::from_utf8(tasks).unwrap(),
        String::from_utf8(expected).unwrap()
    );
    let mut rest = before;
    rest.remove(Path::new("Tasks.md"));
    assert_eq!(after, rest, "nothing else in the vault changed");
}

#[test]
fn a_task_with_no_due_date_ends_in_its_link_and_is_one_line() {
    let task = Task {
        text: "Send the\nfigures  ".to_owned(),
        message: MessageId::new(7),
        due: None,
    };
    assert_eq!(
        task.line(),
        "- [ ] Send the figures [\u{2709}](postio://message/7)"
    );
}

#[test]
fn a_note_without_a_final_newline_gets_one_before_the_line_and_nothing_more() {
    let dir = vault();
    std::fs::write(dir.path().join("Tasks.md"), "- [ ] Water the plants").unwrap();

    open(&dir)
        .append_task(
            None,
            &Task {
                text: "Book the room".to_owned(),
                message: MessageId::new(3),
                due: None,
            },
        )
        .expect("captured");

    assert_eq!(
        std::fs::read_to_string(dir.path().join("Tasks.md")).unwrap(),
        "- [ ] Water the plants\n- [ ] Book the room [\u{2709}](postio://message/3)\n"
    );
}

#[test]
fn a_missing_tasks_note_is_created_with_the_line() {
    let dir = tempfile::tempdir().expect("a vault");
    let vault =
        Vault::open(dir.path(), Some(Path::new("Inbox/Captured.md")), None).expect("the vault");

    let captured = vault
        .append_task(
            None,
            &Task {
                text: "Reply to Quinn".to_owned(),
                message: MessageId::new(9),
                due: Some(day(10, 2)),
            },
        )
        .expect("captured");

    assert_eq!(captured.note, PathBuf::from("Inbox/Captured.md"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("Inbox/Captured.md")).unwrap(),
        "- [ ] Reply to Quinn [\u{2709}](postio://message/9) \u{1F4C5} 2026-10-02\n"
    );
}

#[test]
fn projects_come_from_frontmatter_and_the_projects_folder_and_one_is_suggested() {
    // FR-181: `type: project` frontmatter, or a configured folder; the
    // suggestion says why.
    let dir = vault();
    let vault = open(&dir);

    let mut names: Vec<String> = vault
        .projects()
        .expect("projects")
        .into_iter()
        .map(|project| project.name)
        .collect();
    names.sort();
    assert_eq!(names, ["Harbour redevelopment", "Quarter close"]);

    let suggestion = vault
        .suggest("Re: Q3 close -- approvals by Friday")
        .expect("a read")
        .expect("a suggestion");
    assert_eq!(suggestion.project.name, "Quarter close");
    assert_eq!(
        suggestion.reason,
        Reason::NamedInSubject("close".to_owned())
    );
    assert_eq!(
        vault.suggest("Lunch on Friday?").expect("a read"),
        None,
        "nothing suggested on a guess"
    );
}

#[test]
fn a_task_goes_to_its_project_s_note() {
    let dir = vault();
    let vault = open(&dir);
    let harbour = vault
        .projects()
        .expect("projects")
        .into_iter()
        .find(|project| project.name == "Harbour redevelopment")
        .expect("the project");

    let captured = vault
        .append_task(
            Some(&harbour),
            &Task {
                text: "Review revision D".to_owned(),
                message: MessageId::new(5),
                due: None,
            },
        )
        .expect("captured");

    assert_eq!(
        captured.note,
        PathBuf::from("Projects/Harbour redevelopment.md")
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("Projects/Harbour redevelopment.md")).unwrap(),
        "# Harbour\n- [ ] Review revision D [\u{2709}](postio://message/5)\n"
    );
}

#[test]
fn a_note_is_created_or_appended_and_quotes_only_when_asked() {
    let dir = vault();
    let vault = open(&dir);
    let entry = |quote: Option<&str>| NoteEntry {
        text: "The station opens in 2028.".to_owned(),
        message: MessageId::new(11),
        quote: quote.map(str::to_owned),
    };

    vault
        .append_note(Path::new("Reading/Rail.md"), &entry(None))
        .expect("created");
    vault
        .append_note(
            Path::new("Reading/Rail.md"),
            &entry(Some(
                "Work starts in March,\nand the station opens in 2028.",
            )),
        )
        .expect("appended");

    assert_eq!(
        std::fs::read_to_string(dir.path().join("Reading/Rail.md")).unwrap(),
        "- The station opens in 2028. [\u{2709}](postio://message/11)\n\
         - The station opens in 2028. [\u{2709}](postio://message/11)\n  \
         > Work starts in March,\n  > and the station opens in 2028.\n"
    );
}

#[test]
fn a_note_outside_the_vault_is_refused() {
    let dir = vault();
    let vault = open(&dir);
    let entry = NoteEntry {
        text: "x".to_owned(),
        message: MessageId::new(1),
        quote: None,
    };
    for note in [
        "../Elsewhere.md",
        "/tmp/Elsewhere.md",
        "Reading/../../Out.md",
    ] {
        assert!(
            matches!(
                vault.append_note(Path::new(note), &entry),
                Err(VaultError::OutsideTheVault)
            ),
            "{note}"
        );
    }
    assert!(matches!(
        Vault::open(dir.path(), Some(Path::new("../Tasks.md")), None),
        Err(VaultError::OutsideTheVault)
    ));
    assert!(matches!(
        Vault::open(dir.path().join("Nope"), None, None),
        Err(VaultError::NotAFolder)
    ));
}

#[test]
fn finished_tasks_are_read_back_with_their_done_date() {
    // FR-181: ticked in Obsidian, a task reads back finished, so Focus can
    // offer to archive the conversation.
    let dir = vault();
    std::fs::write(
        dir.path().join("Tasks.md"),
        "# Tasks\n\
         - [ ] Water the plants\n\
         - [ ] Reply to Quinn [\u{2709}](postio://message/9) \u{1F4C5} 2026-10-02\n\
         - [x] Approve the figures [\u{2709}](postio://message/42) \u{1F4C5} 2026-09-30 \u{2705} 2026-09-29\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("Projects/Harbour redevelopment.md"),
        "# Harbour\n- [X] Review revision D [\u{2709}](postio://message/5)\n",
    )
    .unwrap();

    let mut tasks = open(&dir).tasks().expect("read back");
    tasks.sort_by_key(|task| task.message);

    let summary: Vec<(i64, bool, Option<NaiveDate>, Option<NaiveDate>)> = tasks
        .iter()
        .map(|task| (task.message.get(), task.finished, task.due, task.done))
        .collect();
    assert_eq!(
        summary,
        [
            (5, true, None, None),
            (9, false, Some(day(10, 2)), None),
            (42, true, Some(day(9, 30)), Some(day(9, 29))),
        ]
    );
    assert_eq!(tasks[2].text, "Approve the figures");
    assert_eq!(
        tasks[0].note,
        PathBuf::from("Projects/Harbour redevelopment.md")
    );
}
