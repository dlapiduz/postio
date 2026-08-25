//! A string → `postio_search`'s query parser.
//!
//! The same string reaches this parser from three surfaces — the search bar,
//! the sidebar's saved searches and `[filters]` in `config.toml` — so it has
//! to mean the same thing in all three, and it has to be total in all three.
//! A query that panics is a search box that kills the application on a
//! keystroke.
//!
//! The judgement is in `postio_fuzz::check_parse_query`.

#![no_main]

libfuzzer_sys::fuzz_target!(|raw: &[u8]| {
    let query = String::from_utf8_lossy(raw);
    postio_fuzz::check_parse_query(&query);
});
