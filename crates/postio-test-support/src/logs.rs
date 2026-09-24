//! Log lines a test can read back.
//!
//! Eight test modules each wrote this writer out -- a buffer behind a mutex,
//! `io::Write` into it and `MakeWriter` handing out clones -- so here it is
//! once, behind the `logs` feature, for the crates that assert on what
//! reached the log.

use std::sync::{Arc, Mutex};

/// A `tracing_subscriber` writer that keeps everything written to it.
#[derive(Clone, Default)]
pub struct Captured(pub Arc<Mutex<Vec<u8>>>);

impl Captured {
    /// Everything written so far, as text.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("not poisoned")).into_owned()
    }
}

impl std::io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("not poisoned").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
    type Writer = Captured;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}
