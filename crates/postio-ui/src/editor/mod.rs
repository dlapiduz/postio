//! The editor's toolkit-free half: the document every frontend edits in.
//!
//! The mirror of [`crate::reader`], and deliberately so. The reader's document
//! — stylesheet, ground colour, content policy, wrapping — lives here rather
//! than in a frontend because two frontends would otherwise each answer the
//! question and drift; `postio-gtk`'s reader module says as much where it
//! re-exports it, *"one implementation for every frontend… what remains in
//! this file is webkit6 glue"*.
//!
//! The editing surface had no such module, and no stylesheet at all: its
//! document was assembled inline in `postio-gtk` as a bare
//! `<body contenteditable="true">`, so it rendered in the engine's defaults
//! while everything around it used the application's tokens — the wrong
//! typeface, the wrong size, and a white page in dark mode. That is what this
//! module exists to fix (spec 002, FR-072 to FR-077).
//!
//! What is **not** shared with the reader is the content policy. The reader
//! permits neither script nor `contenteditable`; the editor requires both.
//! Two documents, two policies, one set of tokens — sharing the policy would
//! either loosen the reader or break the editor.

pub mod document;
pub mod markdown;
