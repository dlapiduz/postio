//! The one process that owns Postio's store.
//!
//! Turso holds an exclusive lock on the database file, and the operation
//! queue is correct only with one drainer, so exactly one process opens the
//! store and runs the engines: this one (ADR 0041). Frontends reach it through
//! `postio-client`, over a socket as `postio-daemon` or in-process where no
//! other frontend can share the store.
