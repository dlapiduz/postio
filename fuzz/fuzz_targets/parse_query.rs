//! A string into the query parser, which the search bar, saved searches and `config.toml` all
//! reach: it must be total in all three.

#![no_main]

libfuzzer_sys::fuzz_target!(|raw: &[u8]| {
    let query = String::from_utf8_lossy(raw);
    postio_fuzz::check_parse_query(&query);
});
