//! The demo store and window Focus's `shot` and its storyboard runner share.
//!
//! `examples/shot.rs` and the storyboard runner both need a real
//! [`FocusWindow`] over a real, migrated,
//! seeded store, started and adopted the way the application starts it. That
//! setup lives here so the two share it rather than each keeping a copy
//! (specs/008-storyboards R11); argument parsing, staging a numbered screen
//! and the pictures stay with the tools.
//!
//! Behind the `demo` feature, which turns on `postio-storage/test-support`:
//! the seeds and the in-memory store are test-support code and must not
//! reach a normal build.
//!
//! The store is today's inbox in the references' shape, over the storage
//! seed. Every name is invented and every address is on a reserved domain.
//! Nothing touches the network.

#![allow(missing_docs)]

pub mod storyboard;

/// The stores, which every app shares (`postio-demo`).
pub use postio_demo::*;

use gtk::prelude::*;
use postio_model::AccountId;
use postio_storage::{BlobStore, Store};

use crate::startup::Session;
use crate::window::FocusWindow;

/// The runtime the demo's reads are driven on.
///
/// `block_on` polls the future on *this* thread, where GTK lives, and
/// `multi_thread` because the store's reads from a synchronous callback reach
/// for `block_in_place`.
pub fn on_runtime<T>(future: impl std::future::Future<Output = T>) -> T {
    use std::sync::OnceLock;
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("a runtime for the demo")
        })
        .block_on(future)
}

/// A window adopted over a started host, and what a tool needs to drive it.
pub struct Started {
    /// Focus's window, presented and adopted.
    pub window: FocusWindow,
    /// The session over the host: stop it before the window goes.
    pub session: Session,
    /// Where the engine's events enter: a tool says what sync would have.
    pub sink: postio_core::bridge::EventSink,
    /// The account the store was seeded with.
    pub account: AccountId,
    /// The blob directory the host reads, kept until the demo ends.
    _blobs: tempfile::TempDir,
}

impl Started {
    /// Take the window down, stopping its session first.
    pub fn finish(self) {
        self.window.destroy();
        self.session.stop();
    }
}

/// Start the host over `database`, present a Focus window of `size` and
/// adopt it, as the application starts: the path `shot` and the storyboard
/// runner share.
///
/// `config` is the TOML the window runs under, written to `config_path`
/// first: Settings shows the file it writes, and writes to it.
pub fn start(
    database: Store,
    account: AccountId,
    config: &str,
    config_path: &std::path::Path,
    size: (i32, i32),
) -> Result<Started, String> {
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("no config folder: {error}"))?;
    }
    std::fs::write(config_path, config).map_err(|error| format!("no config: {error}"))?;
    let config = postio_config::Config::from_toml_str(config)
        .map_err(|error| format!("the demo's config: {error}"))?;
    let blobs_dir = tempfile::tempdir().map_err(|error| format!("no scratch: {error}"))?;
    let blobs = BlobStore::open(
        blobs_dir.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .map_err(|error| format!("no blob store: {error}"))?;
    let sink = std::rc::Rc::new(std::cell::RefCell::new(None));
    let host = postio_host::Host::start(database, blobs, {
        let sink = std::rc::Rc::clone(&sink);
        move |wiring| {
            sink.replace(Some(wiring.events.clone()));
            wiring
        }
    })
    .map_err(|error| format!("the host did not start: {error}"))?;
    let sink = sink.take().ok_or("the host kept its events to itself")?;
    let window = FocusWindow::new(None);
    window.set_default_size(size.0, size.1);
    window.present();
    let session = crate::startup::adopt_at(&window, host, &config, Some(config_path));
    Ok(Started {
        window,
        session,
        sink,
        account,
        _blobs: blobs_dir,
    })
}
