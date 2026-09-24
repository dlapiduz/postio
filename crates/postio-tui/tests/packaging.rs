//! The two Flatpaks share one store (FR-040, research R10).
//!
//! Each package gets a private data directory unless it asks otherwise, and a
//! package that forgot one grant would quietly have a store, a config or a
//! socket of its own: two mailboxes that each look fine. So the grants are
//! checked side by side, and so is the daemon each package ships beside its
//! frontend, which is where `postio-client` looks for it first.

use std::path::PathBuf;

fn manifest(name: &str) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::read_to_string(root.join("flatpak").join(name))
        .unwrap_or_else(|error| panic!("flatpak/{name}: {error}"))
}

const SHARED: [&str; 3] = [
    "--filesystem=xdg-data/postio:create",
    "--filesystem=xdg-config/postio:create",
    "--filesystem=xdg-run/postio:create",
];

#[test]
fn both_packages_grant_the_shared_store_config_and_socket() {
    for name in ["dev.postio.Postio.json", "dev.postio.PostioTui.json"] {
        let manifest = manifest(name);
        for grant in SHARED {
            assert!(manifest.contains(grant), "{name} does not grant {grant}");
        }
    }
}

#[test]
fn each_package_installs_the_daemon_beside_its_frontend() {
    for (name, frontend) in [
        ("dev.postio.Postio.json", "/app/bin/postio\""),
        ("dev.postio.PostioTui.json", "/app/bin/postio-tui\""),
    ] {
        let manifest = manifest(name);
        assert!(manifest.contains(frontend), "{name} installs no {frontend}");
        assert!(
            manifest.contains("/app/bin/postio-daemon\""),
            "{name} ships no postio-daemon beside its frontend"
        );
    }
}
