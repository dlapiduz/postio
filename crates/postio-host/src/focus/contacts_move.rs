//! The one-shot move of address-keyed entries from `config.toml` onto the
//! correspondents they name (specs/011-focus-contacts S3, plan 4.3, ADR 0048).
//!
//! The first time Focus mode starts on a store that has `contact_settings`,
//! what `config.toml` holds that a correspondent can hold -- a pinned
//! address, a marker turned off for an address, a `[[focus.digests]]` rule
//! that is one `from:` query for one address -- is given to that
//! correspondent and taken out of the file, which is saved as an editor
//! saves it. The file as it was is kept beside it, once, as
//! `config.toml.pre-contacts`, so the person can see what moved.
//!
//! Everything a correspondent cannot hold stays: a domain, a `list:` or
//! `subject:` rule, a rule over several senders, anything validation
//! reports. And so does an entry that would collide with what its
//! correspondent already has: a digest for a pinned address (one arrival
//! cannot be both), or a second digest for one person. The engine reads both
//! homes, so nothing that stays changes what happens to any message.
//!
//! The log carries counts, never an address.

use std::path::{Path, PathBuf};

use postio_config::{Config, FocusConfig};
use postio_model::{ContactArrival, EmailAddress};
use postio_storage::Store;
use postio_storage::repository::{ContactRepository, ContactSettingsRepository};

/// What a move did, as counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Moved {
    /// Addresses pinned on their correspondent.
    pub pins: usize,
    /// Marker kinds turned off on a correspondent.
    pub stops: usize,
    /// Digest rules given to a correspondent.
    pub digests: usize,
    /// Entries that could be held and were left in the file.
    pub left: usize,
}

impl Moved {
    /// Whether the file changed.
    pub fn any(&self) -> bool {
        self.pins + self.stops + self.digests > 0
    }
}

/// Where the file as it was is kept: `config.toml.pre-contacts`.
pub fn pre_contacts_path(config: &Path) -> PathBuf {
    let mut name = config.file_name().unwrap_or_default().to_os_string();
    name.push(".pre-contacts");
    config.with_file_name(name)
}

/// Give the correspondents what they can hold of the file at `path`.
/// Answers what moved and `[focus]` as the file now reads, or `None` when
/// nothing moved.
pub async fn run(database: &Store, path: &Path) -> Result<Option<(Moved, FocusConfig)>, String> {
    let failed = |reason: &str| reason.to_owned();
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(None);
    };
    let Ok(config) = Config::from_toml_str(&text) else {
        // Somebody's edit, mid-way: not ours to rewrite.
        return Ok(None);
    };
    let movable = postio_config::movable(&config.focus);
    if movable.pins.is_empty() && movable.stops.is_empty() && movable.digests.is_empty() {
        return Ok(None);
    }

    let connection = database
        .connect()
        .await
        .map_err(|_| failed("could not open the store to move senders"))?;
    let people = ContactRepository::new(&connection);
    let settings = ContactSettingsRepository::new(&connection);
    let store = |_| failed("could not keep a sender's setting");
    let mut moved = Moved::default();
    let mut edited = text.clone();

    // Stops first: they never collide with anything.
    for (address, kind) in &movable.stops {
        let Ok(person) = people
            .person_for_email(&EmailAddress::new(None::<String>, address.clone()))
            .await
        else {
            moved.left += 1;
            continue;
        };
        let mut held = settings.get(person).await.map_err(store)?;
        held.set_stop(*kind, true);
        settings.put(person, &held).await.map_err(store)?;
        if let Some(next) =
            postio_config::focus_edit::set_stop_marker(&edited, address, kind.as_str(), false)
                .map_err(|_| failed("could not take a marker out of the file"))?
        {
            edited = next;
        }
        moved.stops += 1;
    }
    for address in &movable.pins {
        let Ok(person) = people
            .person_for_email(&EmailAddress::new(None::<String>, address.clone()))
            .await
        else {
            moved.left += 1;
            continue;
        };
        let mut held = settings.get(person).await.map_err(store)?;
        if held.arrival == Some(ContactArrival::Digest) {
            moved.left += 1;
            continue;
        }
        held.arrival = Some(ContactArrival::Inbox);
        settings.put(person, &held).await.map_err(store)?;
        if let Some(next) = postio_config::focus_edit::set_never(&edited, address, false)
            .map_err(|_| failed("could not take a pin out of the file"))?
        {
            edited = next;
        }
        moved.pins += 1;
    }
    for (address, rule) in &movable.digests {
        let Ok(person) = people
            .person_for_email(&EmailAddress::new(None::<String>, address.clone()))
            .await
        else {
            moved.left += 1;
            continue;
        };
        let mut held = settings.get(person).await.map_err(store)?;
        if held.arrival == Some(ContactArrival::Inbox) || held.digest.is_some() {
            moved.left += 1;
            continue;
        }
        held.arrival = Some(ContactArrival::Digest);
        held.digest = Some(postio_config::setting_of(rule));
        settings.put(person, &held).await.map_err(store)?;
        if let Some((next, _, _)) =
            postio_config::focus_edit::remove_digest_rule(&edited, &rule.name)
                .map_err(|_| failed("could not take a rule out of the file"))?
        {
            edited = next;
        }
        moved.digests += 1;
    }

    if !moved.any() {
        return Ok(None);
    }
    let kept = pre_contacts_path(path);
    if !kept.exists() {
        std::fs::write(&kept, &text)
            .map_err(|_| failed("could not keep config.toml.pre-contacts"))?;
    }
    let reread = Config::from_toml_str(&edited)
        .map_err(|_| failed("the file would not read after the move"))?;
    postio_config::save::write_atomically(path, &edited)
        .map_err(|_| failed("could not save config.toml"))?;
    tracing::info!(
        pins = moved.pins,
        stops = moved.stops,
        digests = moved.digests,
        left = moved.left,
        "moved senders' settings from config.toml onto the senders"
    );
    Ok(Some((moved, reread.focus)))
}

#[cfg(test)]
mod tests {
    use postio_storage::test_support;

    use super::*;

    const FILE: &str = r#"# Mine.
[ui]
density = "compact"

[focus.filter]
# Pinned.
never = ["ada@example.org", "@example.net", "news@ledger.example"]
stop_markers = [
  { sender = "ada@example.org", kind = "question" },
  { sender = "@example.net", kind = "todo" },
]

[[focus.digests]]
name = "Ledger"
match = ["from:news@ledger.example"]
cadence = "weekly"
day = "saturday"
at = "16:00"

[[focus.digests]]
name = "Lists"
match = ["list:dev.lists.example"]
cadence = "daily"
at = "08:00"

[[focus.digests]]
name = "Weekly"
match = ["from:letters@weekly.example"]
cadence = "weekly"
day = "friday"
at = "17:00"
"#;

    fn setup(text: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, text).unwrap();
        (dir, path)
    }

    #[tokio::test]
    async fn what_a_person_can_hold_moves_and_the_rest_stays_with_the_file_kept_beside_it() {
        let database = test_support::memory().await;
        let (_dir, path) = setup(FILE);

        let (moved, focus) = run(&database, &path)
            .await
            .unwrap()
            .expect("something moved");

        // The ledger address is pinned, so its digest stays in the file: one
        // arrival cannot be both.
        assert_eq!(
            moved,
            Moved {
                pins: 2,
                stops: 1,
                digests: 1,
                left: 1
            }
        );
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(
            after.starts_with("# Mine.\n[ui]\ndensity = \"compact\"\n"),
            "{after}"
        );
        assert!(
            after.contains("@example.net") && after.contains("Lists") && after.contains("Ledger")
        );
        assert!(
            !after.contains("ada@example.org") && !after.contains("Weekly"),
            "{after}"
        );
        assert_eq!(focus.filter.never, ["@example.net"]);
        assert_eq!(focus.filter.stop_markers.len(), 1, "the domain's");
        assert_eq!(
            std::fs::read_to_string(pre_contacts_path(&path)).unwrap(),
            FILE
        );

        let connection = database.connect().await.unwrap();
        let derived = ContactSettingsRepository::new(&connection)
            .derived()
            .await
            .unwrap();
        let ada = derived
            .iter()
            .find(|d| d.addresses == ["ada@example.org"])
            .unwrap();
        assert_eq!(ada.settings.arrival, Some(ContactArrival::Inbox));
        assert!(ada.settings.stops(postio_model::StoppedMarker::Question));
        let weekly = derived
            .iter()
            .find(|d| d.addresses == ["letters@weekly.example"])
            .unwrap();
        assert_eq!(weekly.settings.digest.as_ref().unwrap().name, "Weekly");
        for one in &derived {
            let connection = database.connect().await.unwrap();
            let person = ContactRepository::new(&connection)
                .get(one.contact)
                .await
                .unwrap()
                .unwrap();
            assert!(!person.is_saved(), "a setting is not a save");
        }
    }

    #[tokio::test]
    async fn a_second_run_moves_nothing_and_leaves_the_kept_file_as_the_first_made_it() {
        let database = test_support::memory().await;
        let (_dir, path) = setup(FILE);
        run(&database, &path).await.unwrap();
        let after_first = std::fs::read_to_string(&path).unwrap();

        // What stayed because it collided stays; nothing else is touched.
        assert_eq!(run(&database, &path).await.unwrap(), None);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), after_first);
        assert_eq!(
            std::fs::read_to_string(pre_contacts_path(&path)).unwrap(),
            FILE
        );
    }

    #[tokio::test]
    async fn a_file_with_nothing_to_move_is_not_touched_and_no_copy_is_made() {
        let database = test_support::memory().await;
        let text = "[focus.filter]\nnever = [\"@example.net\"]\n";
        let (_dir, path) = setup(text);
        assert_eq!(run(&database, &path).await.unwrap(), None);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        assert!(!pre_contacts_path(&path).exists());
    }

    #[tokio::test]
    async fn a_file_that_does_not_read_is_never_rewritten() {
        let database = test_support::memory().await;
        let text = "[focus.filter\nnever = [\"ada@example.org\"]\n";
        let (_dir, path) = setup(text);
        assert_eq!(run(&database, &path).await.unwrap(), None);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }
}
