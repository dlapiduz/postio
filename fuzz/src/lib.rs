//! What the fuzz targets actually assert.
//!
//! A `fuzz_target!` body is unusually hard to test: it only runs under
//! libFuzzer, and the way you find out it was wrong is that it never fails.
//! So the judgement lives here as ordinary functions with ordinary tests, and
//! each target is a two-line wrapper that decodes its input and calls one.
//!
//! Two kinds of check, and the difference matters when triaging a find:
//!
//! * **Crash-freedom.** Simply calling the function is the assertion — a
//!   panic, an abort or an OOM is the find. This is most of what
//!   [`check_parse_message`] does, because a mail that crashes the client on
//!   sight is a denial of service the user re-triggers on every startup, and
//!   there is nothing else to say about a header block that parses.
//! * **Properties.** Something that must be true of the *output*, checked
//!   explicitly. These catch the failures that do not crash: a sanitizer that
//!   lets a scheme through, a span that points into the middle of a character.
//!   A property find is usually the more serious of the two.
//!
//! Every property here is written so that violating it is a bug in Postio,
//! never in the input. There is no such thing as a malformed input to any of
//! these functions — see each `check_*` for what "total" means for it.

pub mod properties;

pub use properties::{check_parse_message, check_parse_query, check_sanitize_html};
