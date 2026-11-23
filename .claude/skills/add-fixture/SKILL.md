---
name: add-fixture
description: Add a .eml fixture to the Postio test corpus correctly (file, loader table, categories, README row) so the three tests that enforce all four stay green. Use whenever a bug or feature needs new test mail.
---

# Add a corpus fixture

`crates/postio-model/tests/corpus/` lets every crate test against realistic mail without the network. Three tests enforce registration, so a bare file turns the suite red. Read `crates/postio-model/tests/corpus/README.md` first and extend an existing fixture rather than duplicate one.

## 1. Write the `.eml`

Save as `crates/postio-model/tests/corpus/<descriptive-name>.eml`, named for what it *exercises* (`broken-references`, not `email-7`).

- **Invent everything.** Reserved domains only (`example.com`, `.test`, `.invalid`); never a real person's name or address, least of all the maintainer's. `scripts/checks/check-no-personal-data.py` runs in CI and a corpus test scans for this.
- Use real message bytes: CRLF line endings, genuine headers, correct MIME boundaries.

## 2. Register it in the loader

Add an entry to the fixture table in `crates/postio-model/src/test_corpus.rs`: name, a one-line description, categories. Use an existing `Category` where one fits; add one only when none does, and document it in the README's category table.

## 3. Document it in the README

Add a row to the fixture table in `crates/postio-model/tests/corpus/README.md`: what it exercises and why.

## 4. Verify

```bash
cargo test -p postio-model
python3 scripts/checks/check-no-personal-data.py
```

The tests that catch a partial job:
- `loader_reaches_every_file_on_disk_and_nothing_else`: file present but not registered, or the reverse
- `every_fixture_reads_back_exactly_as_stored`: embedded bytes match disk
- `the_readme_documents_every_fixture`: README row missing
- `every_address_in_the_corpus_is_an_invented_reserved_domain`: a real-looking address

## 5. Use it

```toml
[dev-dependencies]
postio-model = { workspace = true, features = ["test-corpus"] }
```

The loader returns **raw bytes**, not parsed messages, so parser tests cannot become circular.
