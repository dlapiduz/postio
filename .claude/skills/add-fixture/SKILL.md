---
name: add-fixture
description: Add a .eml message fixture to the Postio test corpus correctly — the file, the loader table, its categories, and the README row — so the three tests that enforce all four stay green. Use whenever a bug or feature needs new test mail.
---

# Add a corpus fixture

The corpus at `crates/postio-model/tests/corpus/` is how every crate tests
against realistic mail without touching the network. Three separate tests
enforce that a fixture is registered in all the right places, so adding the
file alone will turn the suite red.

Read `crates/postio-model/tests/corpus/README.md` first — it documents every
existing fixture and what it exercises. Extend the corpus rather than
duplicating a fixture that already covers your case.

## 1. Write the `.eml`

Save it as `crates/postio-model/tests/corpus/<descriptive-name>.eml`, named for
what it *exercises*, not what it contains — `broken-references`, not
`email-7`.

**Invent everything.** Every address must use a reserved domain
(`example.com`, `.test`, `.invalid`). Never a real person's name or address,
least of all the maintainer's — `scripts/check-no-personal-data.py` runs in CI
and a corpus test scans specifically for this.

Use real message bytes: CRLF line endings, genuine headers, correct MIME
boundaries. A fixture that is not actually well-formed mail teaches the parser
the wrong lesson.

## 2. Register it in the loader

Add an entry to the fixture table in `crates/postio-model/src/test_corpus.rs`:
the name, a one-line description of what it exercises, and its categories.

Pick from the existing `Category` values where one fits. Add a new category
only when nothing existing describes the case, and then document it in the
README's category table too.

## 3. Document it in the README

Add a row to the fixture table in
`crates/postio-model/tests/corpus/README.md` saying what the fixture exercises
and why it exists. A fixture nobody can explain gets deleted by someone later.

## 4. Verify all four landed

```bash
cargo test -p postio-model
python3 scripts/check-no-personal-data.py
```

The tests that will catch a partial job:

- `loader_reaches_every_file_on_disk_and_nothing_else` — file present but not
  registered, or registered but missing
- `every_fixture_reads_back_exactly_as_stored` — embedded bytes match disk
- `the_readme_documents_every_fixture` — README row missing
- `every_address_in_the_corpus_is_an_invented_reserved_domain` — real-looking
  address slipped in

## 5. Use it

Downstream crates reach the corpus through the `test-corpus` feature:

```toml
[dev-dependencies]
postio-model = { workspace = true, features = ["test-corpus"] }
```

The loader hands back **raw bytes**, not parsed messages — deliberately, so
parser tests cannot become circular.
