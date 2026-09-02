//! The sidebar draws one collapsible section per account. #185, the drawing.
//!
//! `gtk_folder_sections.rs` is the other half: the feed can read every
//! account's tree. This is what becomes of those rows — a header per account
//! and that account's folders under it, folded away and brought back.
//!
//! # What makes this worth asserting rather than looking at
//!
//! Folding hides widgets, and a hidden `GtkListBox` still *holds* its rows.
//! So the visible thing and the reachable thing can disagree: the folders
//! disappear while `j` still steps through them, landing the keyboard on rows
//! nobody can see. `step` is the only way to ask what the keyboard can reach,
//! so it is what this drives.
//!
//! The other half is that a section's boxes must survive a re-render.
//! `sync_folder_rows` updates rows by index, so a box rebuilt under the
//! selection would take it — and a count arriving is a re-render.
//!
//! One test function: GTK is single-threaded and initialised once per binary.
//! Nothing here touches the network.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This sets it before anything else runs, which is the one
// moment it is sound.

use gtk::gdk;
use postio_gtk::sidebar::Sidebar;
use postio_gtk::{app, fonts, style};
use postio_model::AccountScope;
use postio_model::ids::AccountId;
use postio_model::mailbox::Mailbox;

/// One account's folders, named so a failure says which account is missing.
fn tree(account: AccountId, paths: &[&str]) -> Vec<Mailbox> {
    paths
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let mut mailbox = Mailbox::new(account, *path, Some('/'));
            mailbox.id = postio_model::ids::MailboxId::new(account.get() * 100 + index as i64);
            mailbox.selectable = true;
            mailbox
        })
        .collect()
}

pub fn each_account_folds_away_its_own_folders() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let sidebar = Sidebar::new();
    let work = AccountId::new(1);
    let home = AccountId::new(2);

    let mut mailboxes = tree(work, &["INBOX", "Projects", "Projects/Harbour"]);
    mailboxes.extend(tree(home, &["INBOX", "Recipes"]));

    sidebar.set_accounts(
        &[(work, "Work".to_owned()), (home, "Home".to_owned())],
        AccountScope::Unified,
        true,
    );
    sidebar.set_mailboxes(&mailboxes);

    // ── both accounts' folders are reachable ─────────────────────────────
    let reachable = |sidebar: &Sidebar| {
        let mut seen = Vec::new();
        // `step` from a standing start walks the whole sidebar; asking it
        // rather than counting widgets is the point, because what the
        // keyboard reaches is the thing folding has to change.
        while let Some(id) = sidebar.step(1) {
            if seen.contains(&id) {
                break;
            }
            seen.push(id);
        }
        seen
    };

    let all = reachable(&sidebar);
    assert!(
        all.len() >= mailboxes.len(),
        "both accounts' folders should be reachable, got {} of {}",
        all.len(),
        mailboxes.len()
    );

    // ── folding one account takes only its folders away ──────────────────
    sidebar.toggle_account(home);
    let folded = reachable(&sidebar);
    let home_ids: Vec<_> = tree(home, &["INBOX", "Recipes"])
        .iter()
        .map(|mailbox| mailbox.id)
        .collect();
    assert!(
        home_ids.iter().all(|id| !folded.contains(id)),
        "a folded account's folders are hidden, so the keyboard must not \
         step into them: {folded:?}"
    );
    let work_ids: Vec<_> = tree(work, &["INBOX", "Projects", "Projects/Harbour"])
        .iter()
        .map(|mailbox| mailbox.id)
        .collect();
    assert!(
        work_ids.iter().any(|id| folded.contains(id)),
        "folding one account must not take the other's folders with it"
    );

    // ── and brings them back ─────────────────────────────────────────────
    sidebar.toggle_account(home);
    let restored = reachable(&sidebar);
    assert_eq!(
        restored.len(),
        all.len(),
        "unfolding should restore exactly what folding removed"
    );

    // ── a re-render keeps the boxes, so the selection survives a count ───
    let anchor = all[0];
    sidebar.select(anchor);
    assert_eq!(
        sidebar.selected(),
        Some(anchor),
        "select did not take effect at all, before any re-render"
    );
    sidebar.set_mailboxes(&mailboxes);
    assert_eq!(
        sidebar.selected(),
        Some(anchor),
        "a re-render rebuilt the boxes under the selection and lost it -- \
         `sync_folder_rows` updates by index and needs the same boxes back"
    );
}
