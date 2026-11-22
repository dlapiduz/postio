#![allow(dead_code)]

use std::path::Path;

use postio_runtime::engine::Engine;
use tempfile::TempDir;

/// Locals drop in reverse order, so the blob directory would be removed while the engine thread is
/// still fetching (#724);
/// stopping the engine in `Drop` is ordering no call site can get wrong.
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
        self.engine.stop();
    }
}
