# The Postio `.eml` test corpus

Realistic mail, on disk, so that **no test in the default suite has to touch the
network** (CLAUDE.md, "Test-driven development is mandatory"). Every crate in the
workspace tests against these files: MIME parsing, JWZ threading, the search
indexer, the reader's remote-content blocking, and the `MailBackend` mock all
draw from here.

Nothing in this directory is real. Every address sits under an RFC 2606 reserved
domain (`example.com`, `example.net`, `example.org`, and country variants plus
`.invalid`), and every body was invented for this corpus. A test in
`../corpus_loader.rs` enforces both rules, so a fixture pasted in from a real
mailbox will fail the build.

## Using the corpus

The loader lives in `postio-model` behind the off-by-default `test-corpus`
feature, and embeds these files with `include_bytes!` — there is no filesystem
lookup at run time, so any crate can use it without knowing where the repository
is checked out.

```toml
# Cargo.toml of the crate whose tests need mail
[dev-dependencies]
postio-model = { workspace = true, features = ["test-corpus"] }
```

```rust
use postio_model::test_corpus::{self, Category};

// One fixture, by name (the `.eml` suffix is optional).
let raw: &[u8] = test_corpus::load("html-newsletter").bytes();

// Or a whole category — this is how you write "every broken-header case"
// without a hard-coded list that goes stale.
for fixture in test_corpus::by_category(Category::MalformedHeaders) {
    let parsed = my_parser::parse(fixture.bytes());
    assert!(parsed.is_ok(), "{} must not fail the message", fixture.name());
}

// Intersections work too: the half-linked replies threading has to recover.
let hard = test_corpus::by_categories(&[Category::Threading, Category::BrokenReferences]);
```

The loader hands out **bytes**, never parsed messages, and `Fixture::bytes()`
stays the primary accessor for that reason: MIME parsing is a consumer of this
corpus, and the parser's own tests (`postio-model/tests/mime.rs`) call
`mime::parse` directly, never through the fixture, or they would be testing the
parser against itself. Several fixtures are also not valid UTF-8 by design, so
`Fixture::as_str()` is fallible.

Now that parsing exists, `Fixture::parse()` is a convenience for everyone else —
threading, search, reader tests — that just wants the domain `Message` without
calling `mime::parse` and `ParsedMessage::into_message` by hand:

```rust
let message = test_corpus::load("list-thread-04-reply-deep").parse();
assert!(message.in_reply_to.is_some());
```

It fills in a fixed, arbitrary `account_id`, `mailbox_id` and `received_at` —
these fixtures were never in a real mailbox, and most callers only care about
what the message itself carries.

## Categories

Fixtures are tagged, not filed — most carry several tags.

| Category | What it selects |
|---|---|
| `plain-text` | `text/plain` bodies, the baseline case |
| `html` | `text/html` bodies: newsletters, marketing, rich replies |
| `multipart-alternative` | pick exactly one part, never both |
| `multipart-mixed` | body plus attachments |
| `multipart-related` | parts referenced from HTML by `cid:` |
| `nested-multipart` | more than one level of nesting |
| `inline-image` | images shown inline through `cid:` |
| `attachment` | carries at least one attachment part |
| `large-attachment` | big enough that buffering it whole is a mistake |
| `non-utf8-charset` | ISO-8859-1, Shift_JIS, UTF-7, windows-1252 on the wire |
| `base64` | `Content-Transfer-Encoding: base64` |
| `quoted-printable` | `Content-Transfer-Encoding: quoted-printable` |
| `encoded-word` | RFC 2047 encoded words in headers |
| `malformed-headers` | header blocks a strict parser would reject |
| `malformed-structure` | broken or truncated MIME structure |
| `missing-headers` | no `Message-ID`, no `Date`, no body |
| `threading` | part of a conversation JWZ has to reassemble |
| `broken-references` | absent, malformed or dangling `References` |
| `mailing-list` | RFC 2369 list headers, `List-Id` |
| `pgp` | `multipart/signed` and `multipart/encrypted` |
| `remote-content` | remote images, tracking pixels — the reader must block these |
| `calendar` | `text/calendar` parts and `.ics` attachments |
| `delivery-status` | bounces: `multipart/report` |

## The fixtures

### Ordinary mail

| File | Exercises |
|---|---|
| `plain-text-simple.eml` | The smallest realistic message: 7bit us-ascii, a `-- ` signature delimiter, a `Return-Path`. The happy path everything else is measured against. |
| `plain-text-flowed-reply.eml` | `format=flowed; delsp=yes` with quoted parent text. Reflowing, quote-depth detection, and a correctly linked reply. |
| `headers-only-no-body.eml` | The file ends after the last header: no blank line, no body. Trivially breaks any parser that splits on `\r\n\r\n` without a fallback. |
| `header-folding-received-chain.eml` | A three-hop `Received` chain, `DKIM-Signature`, multi-line `Authentication-Results`, a folded `Subject` and a folded multi-recipient `To`. Header unfolding, at length. |
| `charset-utf-8-emoji-rtl.eml` | Valid UTF-8 that is still hard to render: ZWJ emoji sequences, flags, RTL Arabic and Hebrew, combining marks, astral-plane glyphs, an embedded BOM. Byte length, char length and grapheme count all differ. |

### Transfer encodings

| File | Exercises |
|---|---|
| `transfer-encoding-base64.eml` | A plain-text body encoded base64 for no reason, as export tools do. |
| `transfer-encoding-quoted-printable.eml` | Soft line breaks, a literal `=` as `=3D`, encoded trailing whitespace, accented runs and currency symbols. |

### MIME structure

| File | Exercises |
|---|---|
| `multipart-alternative.eml` | The canonical text + HTML pair, with a preamble before the first boundary and an epilogue after the closing one; both must be discarded. |
| `nested-multipart.eml` | `mixed` > `alternative` > `related`, three levels deep, with an inline PNG in the innermost part and an attachment after the whole nest. |
| `inline-image-cid.eml` | Two inline PNGs addressed by `cid:`, one with `name=` and one without, plus a third `cid:` reference with **no matching part** — the dangling case. |
| `attachment-pdf.eml` | Two attachments, `Content-Description`, `size=` and `creation-date=` disposition parameters, `Cc` recipients. |
| `attachment-large.eml` | ~256 KiB of base64. This is the fixture that proves attachments stream to the blob store instead of sitting in memory; it is the only file here over 64 KiB, and a test enforces that. |
| `attachment-rfc2231-filename.eml` | Three spellings of one non-ASCII filename: RFC 2231 continuations (`name*0*`, `name*1*`), the `charset'language'value` form, and the RFC 2047 encoded-word-in-a-parameter abuse that is illegal and ubiquitous. |
| `calendar-invite.eml` | `text/calendar; method=REQUEST` inside an alternative inside a mixed part, plus the same ICS again as an attachment. Line folding and escaping inside the ICS itself. |
| `bounce-delivery-status.eml` | A Postfix bounce: `multipart/report; report-type=delivery-status`, a `message/delivery-status` part, and the original message embedded as `message/rfc822` — nested message parsing. |

### HTML and remote content

| File | Exercises |
|---|---|
| `html-newsletter.eml` | A newsletter shaped like the real thing: nested layout tables, inline CSS, a `@media` query, an XHTML doctype, `List-Unsubscribe` with One-Click, quoted-printable. The stress case for HTML sanitizing and for text extraction into the search index. |
| `html-tracking-pixel-remote-images.eml` | A 1×1 open-rate beacon, remote `<img>` over both https and http, CSS `background-image` URLs, a `url()` inside a stylesheet, and a click-tracking redirect. The reader's remote-content blocking must catch **all** of these, not just `<img src>`. |

### Character sets

| File | Exercises |
|---|---|
| `charset-iso-8859-1.eml` | Raw 8-bit latin-1 body with Q-encoded latin-1 `Subject` and `From`. Not valid UTF-8 as stored. |
| `charset-shift-jis.eml` | Japanese mail: Shift_JIS body in base64, Shift_JIS encoded words in `Subject` and the display name. |
| `charset-utf-7.eml` | A UTF-7 body and a UTF-7 encoded word, plus an IMAP modified-UTF-7 mailbox name in a header. The classic failure is emitting raw `+AOk-` escapes. |
| `charset-windows-1252-mislabeled.eml` | Bytes in the C1 range labelled `iso-8859-1`: curly quotes, em dash, ellipsis, bullet, trademark. Strict latin-1 decoding yields control characters; every real client treats `iso-8859-1` as windows-1252 instead, and so must we. |

### RFC 2047 encoded words

| File | Exercises |
|---|---|
| `encoded-word-subject-and-names.eml` | The correct cases: adjacent encoded words that must be joined **without** a space, two charsets in one field, a folded `Subject` whose continuation begins with an encoded word, B and Q encodings side by side, and one plain ASCII display name. |
| `encoded-word-broken.eml` | The hostile cases: unterminated, invalid base64 payload, unknown charset, unknown encoding letter, and one that blows past the 75-character limit. Every one must degrade to raw text rather than failing the message. |

### Malformed and missing

| File | Exercises |
|---|---|
| `malformed-headers.eml` | A dozen distinct header sins in one block: no colon, no field name, `Subject` twice, an unparseable `Date`, unbalanced angle brackets, an empty recipient, an unclosed quoted string, control characters in a value, an empty `boundary=` on a non-multipart type. Recovery, not rejection. |
| `malformed-bare-lf.eml` | Bare LF line endings everywhere, including at the MIME boundaries. Anything written against a strict CRLF reading of RFC 5322 finds no body, or no boundary, or neither. |
| `malformed-truncated-multipart.eml` | Delivery cut short mid-attachment: no closing boundary, a truncated base64 run. The first part must still reach the reader, and nothing may hang. |
| `missing-message-id-and-date.eml` | No `Message-ID`, no `Date`, no `To`. Storage has to synthesize a stable local identity, the list has to sort it anyway, and threading has nothing to link on. |
| `duplicate-message-id.eml` | Deliberately reuses the `Message-ID` of `plain-text-simple.eml` with a different body and a later `Date`. Servers really do deliver a `Message-ID` twice, so deduplication must not key on it alone and threading must not build a cycle from the pair. |
| `broken-references.eml` | Every way a `References` header can be broken, in one header: an unterminated angle-addr, an empty `<>`, a bare non-angle token, a duplicate entry, a reference to a message not in the store, and an `In-Reply-To` that matches nothing. |

### The mailing-list thread

Seven messages from `harbour-dev@lists.example.org`, all carrying `List-Id` and
the rest of the RFC 2369 headers. Together they are the JWZ threading test case:
a root, a well-linked reply, a sibling, a grandchild, and then three that are
linked badly in three different real-world ways.

| File | Position | Exercises |
|---|---|---|
| `list-thread-01-root.eml` | root | The message everything else hangs off. |
| `list-thread-02-reply.eml` | child of 01 | Well-formed: both `In-Reply-To` and `References`. |
| `list-thread-03-reply-sibling.eml` | child of 01 | Replies to the root, not to 02 — must render as a sibling, not a grandchild. |
| `list-thread-04-reply-deep.eml` | child of 02 | Depth 3, with the full two-entry `References` chain folded across lines. |
| `list-thread-05-reply-no-references.eml` | child of 04 | `In-Reply-To` and **no** `References` at all. The most common way a real thread arrives half-linked; a pass that reads only `References` drops this message out of the tree. |
| `list-thread-06-reply-subject-only.eml` | child of the thread | Neither `In-Reply-To` nor `References` — a gateway stripped both. Only subject normalization can attach it, which is precisely the JWZ subject-matching fallback. |
| `list-thread-07-subject-change.eml` | child of 05 | Subject changed with `(was: ...)` and `References` truncated to the last two entries, so the root is reachable only transitively through the parent's own references. |

### Signed and encrypted

| File | Exercises |
|---|---|
| `pgp-signed.eml` | PGP/MIME `multipart/signed; micalg=pgp-sha256`. The signed part must be preserved byte-exactly, headers and transfer encoding included — canonicalization mistakes here are the classic reason a good signature reports as broken. The armour is synthetic and will not verify. |
| `pgp-encrypted.eml` | PGP/MIME `multipart/encrypted`: the `Version: 1` control part plus an armoured blob. Synthetic; it will not decrypt. |

## Extending the corpus

Extend this corpus rather than starting another one somewhere else. Adding a
fixture is three steps, and the tests will tell you if you miss one:

1. Drop the `.eml` file in this directory. Use CRLF line endings unless the
   fixture exists *because* it does not, and keep it under 64 KiB unless it is
   tagged `large-attachment`.
2. Add a line to the `corpus!` table in
   `../../src/test_corpus.rs`: the file stem, its categories, and one sentence
   on what it exercises and why.
3. Add a row to the tables above.

Then run `cargo test -p postio-model`. `corpus_loader.rs` fails if a file is not
in the table, if the table names a file that is not on disk, if a fixture is
undocumented here, if it carries a domain that is not RFC 2606 reserved, or if a
category ends up with no fixtures.

Invent the content. Never paste in a real message, a real address, or anything
out of a real mail client's configuration.
