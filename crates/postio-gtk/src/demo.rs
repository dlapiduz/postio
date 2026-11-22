//! The demo store and window `shot` and the storyboard runner share: a real `FocusWindow` over a
//! migrated, seeded store, started and adopted as the app starts it.
//! Behind the `demo` feature, which turns on `postio-storage/test-support`: that code must not
//! reach a normal build.

pub mod storyboard;

pub use postio_demo::*;

use gtk::prelude::*;
use postio_model::AccountId;
use postio_storage::{BlobStore, Store};

use crate::startup::Session;
use crate::window::FocusWindow;

/// `block_on` polls on this thread, where GTK lives; `multi_thread` because the store's reads from
/// a synchronous callback reach for `block_in_place`.
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

pub struct Started {
    pub window: FocusWindow,
    pub session: Session,
    pub sink: postio_core::bridge::EventSink,
    pub account: AccountId,
    pub store: Store,
    _blobs: tempfile::TempDir,
    kept: Vec<tempfile::TempDir>,
}

impl Started {
    pub fn keep(&mut self, dir: tempfile::TempDir) {
        self.kept.push(dir);
    }

    pub fn finish(self) {
        self.window.destroy();
        self.session.stop();
    }
}

/// `config` is the TOML the window runs under, written to `config_path` first: Settings shows the
/// file it writes, and writes to it.
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
    // The search seed's files have their bytes on this machine, so the attachment indexer reads
    // them; another seed names none of them.
    postio_demo::store_search_blobs(&blobs).map_err(|error| format!("no search files: {error}"))?;
    postio_demo::store_threads_blobs(&blobs)
        .map_err(|error| format!("no thread sources: {error}"))?;
    let store = database.clone();
    let files = blobs.clone();
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
    // Index every body and file before the first frame: left to the idle passes, a word was found
    // or not by how far they had got,
    // and a count grew between frames of one storyboard. Run on the host's runtime and waited for
    // here: the runner calls this from inside another, where a `block_on` may not run.
    let (indexed, done) = std::sync::mpsc::channel();
    host.runtime().spawn({
        let store = store.clone();
        async move {
            let result = async {
                postio_session::index_local_bodies(&store).await?;
                postio_session::index_local_attachments(&store, &files).await
            }
            .await
            .map_err(|error| error.to_string());
            let _ = indexed.send(result);
        }
    });
    done.recv()
        .map_err(|_| "the demo's index never answered".to_owned())?
        .map_err(|error| format!("the demo's mail was not indexed: {error}"))?;
    let window = FocusWindow::new(None);
    window.set_default_size(size.0, size.1);
    window.present();
    let session = crate::startup::adopt_at(&window, host, &config, Some(config_path));
    Ok(Started {
        window,
        session,
        sink,
        account,
        store,
        _blobs: blobs_dir,
        kept: Vec::new(),
    })
}
