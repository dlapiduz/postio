//! The two Flatpaks share one store (FR-040, research R10).
//!
//! Each package gets a private data directory unless it asks otherwise, and a
//! package that forgot one grant would quietly have a store or a config of
//! its own: two mailboxes that each look fine. So the grants are checked side
//! by side. Each package runs its own app and nothing else: whichever opens
//! the store first has it, and the other is told to close it -- there is no
//! background service to ship, and no runtime directory to reach one in.

use std::path::PathBuf;

fn manifest(name: &str) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::read_to_string(root.join("flatpak").join(name))
        .unwrap_or_else(|error| panic!("flatpak/{name}: {error}"))
}

const PACKAGES: [(&str, &str); 2] = [
    ("dev.postio.Postio.json", "/app/bin/postio\""),
    ("dev.postio.PostioTui.json", "/app/bin/postio-tui\""),
];

const SHARED: [&str; 2] = [
    "--filesystem=xdg-data/postio:create",
    "--filesystem=xdg-config/postio:create",
];

#[test]
fn both_packages_grant_the_shared_store_and_config() {
    for (name, _) in PACKAGES {
        let manifest = manifest(name);
        for grant in SHARED {
            assert!(manifest.contains(grant), "{name} does not grant {grant}");
        }
    }
}

#[test]
fn each_package_installs_its_own_app_and_no_daemon() {
    for (name, frontend) in PACKAGES {
        let manifest = manifest(name);
        assert!(manifest.contains(frontend), "{name} installs no {frontend}");
        assert!(
            !manifest.contains("postio-daemon"),
            "{name} still builds or ships postio-daemon"
        );
        assert!(
            !manifest.contains("xdg-run/postio"),
            "{name} still grants the daemon's runtime directory"
        );
    }
}
