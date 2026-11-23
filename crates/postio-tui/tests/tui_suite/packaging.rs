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

/// The helper recovers from a page that panics (`catch_unwind`), which `panic = "abort"` would
/// turn into a lost file, so the package builds it under `release` (#1824).
#[test]
fn the_terminal_package_ships_an_unwinding_extraction_helper() {
    let manifest = manifest("dev.postio.PostioTui.json");
    let build = manifest
        .lines()
        .find(|line| line.contains("--bin postio-extract-helper"))
        .expect("the terminal manifest never builds postio-extract-helper");
    assert!(
        build.contains("--release") && !build.contains("release-tui"),
        "the helper must build under an unwinding profile: {build}"
    );
    assert!(
        manifest.contains("target/release/postio-extract-helper /app/bin/postio-extract-helper"),
        "the terminal manifest never installs /app/bin/postio-extract-helper"
    );
}
