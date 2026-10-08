//! `config.toml` applied live through the shared bridge (specs/007-postio-focus
//! T022): an edit on disk reaches the app on the main loop, validated, and a
//! broken edit leaves the last good keymap in force.
//!
//! What is asserted is the keymap the app would draw its hints from, read in
//! the callback, not that the watcher was told anything.

use std::cell::RefCell;
use std::ops::ControlFlow;
use std::rc::Rc;

use postio_core::CommandId;
use postio_core::config::ConfigService;

use crate::support::until;

/// What the app saw on each reload: the archive key, and whether the reload
/// was applied.
type Seen = Rc<RefCell<Vec<(Option<String>, bool)>>>;

pub fn an_edit_reaches_the_app_and_a_broken_one_keeps_the_last_good_keys() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[keys]\narchive = \"y\"\n").expect("the first config");
    let service = ConfigService::load(&path);
    assert_eq!(service.keymap().binding(CommandId::Archive), Some("y"));

    let seen: Seen = Rc::default();
    let following = postio_widgets::present::config::follow(service, {
        let seen = seen.clone();
        move |service, update| {
            seen.borrow_mut().push((
                service
                    .keymap()
                    .binding(CommandId::Archive)
                    .map(str::to_owned),
                update.applied(),
            ));
            ControlFlow::Continue(())
        }
    });
    assert!(following, "the file could not be watched");

    std::fs::write(&path, "[keys]\narchive = \"z\"\n").expect("an edit");
    assert!(
        until(|| seen
            .borrow()
            .iter()
            .any(|(key, _)| key.as_deref() == Some("z"))),
        "the edit never reached the app: {:?}",
        seen.borrow()
    );

    let before = seen.borrow().len();
    std::fs::write(&path, "[keys\narchive = ").expect("a broken edit");
    assert!(
        until(|| seen.borrow().len() > before),
        "the broken edit was never reported"
    );
    let (key, applied) = seen.borrow().last().cloned().expect("a reload");
    assert!(!applied, "a file that does not parse was applied");
    assert_eq!(
        key.as_deref(),
        Some("z"),
        "a broken file took the working keymap away"
    );
}
