//! Scaffolding shared by the engine tests.
//!
//! Each test file here is its own binary, so anything more than one of them
//! needs lives in this module rather than being copied.
#![allow(dead_code)]

use std::path::Path;

use postio_runtime::engine::Engine;
use tempfile::TempDir;

/// A test engine's blob directory, and the engine that has to stop before it
/// can go.
///
/// # Why this is not a bare `TempDir`
///
/// Locals drop in reverse declaration order, so the directory a builder hands
/// back last is the *first* thing a test body releases — while the engine
/// thread is still fetching. `TempDir::drop` calls `remove_dir_all` and
/// swallows the error, the sync pass then commits a blob, `BlobStore`
/// recreates the parent it needs, and the tree is left behind under
/// `target/tmp`. That is #724: nondeterministic by construction, because
/// whether anything was in flight at that instant decides it.
///
/// Stopping the engine in a `Drop` body is ordering no call site can get
/// wrong — not a tuple's field order, and not the order a test happens to
/// name its bindings in.
pub struct BlobDir {
    engine: Engine,
    directory: TempDir,
}

impl BlobDir {
    pub fn new(engine: Engine, directory: TempDir) -> Self {
        Self { engine, directory }
    }

    pub fn path(&self) -> &Path {
        self.directory.path()
    }
}

impl Drop for BlobDir {
    fn drop(&mut self) {
        // Synchronous and idempotent: it closes the job channel and joins the
        // engine thread, so nothing is still writing when `TempDir` removes
        // the tree on the next line.
        self.engine.stop();
    }
}
