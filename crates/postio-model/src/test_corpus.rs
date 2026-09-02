//! The `.eml` test corpus and its loader.
//!
//! Every crate in Postio has to be testable without a network connection —
//! CLAUDE.md makes that a hard rule: *no test in the default suite may touch the
//! network.* This module is what makes that possible. It carries a curated set
//! of raw RFC 5322 messages, embedded into the binary at compile time with
//! [`include_bytes!`], and hands them out by name or by category.
//!
//! The files themselves live in `crates/postio-model/tests/corpus/`, one
//! `.eml` per fixture, with a `README.md` next to them explaining what each one
//! exercises. Nothing here is real: every address is under an RFC 2606 reserved
//! domain, and every body was invented for this corpus.
//!
//! # Availability
//!
//! This module is behind the off-by-default `test-corpus` cargo feature so that
//! ordinary builds of `postio-model` carry none of it. Downstream crates opt in
//! from their dev-dependencies only:
//!
//! ```toml
//! [dev-dependencies]
//! postio-model = { workspace = true, features = ["test-corpus"] }
//! ```
//!
//! # Using it
//!
//! ```
//! use postio_model::test_corpus;
//!
//! // One fixture by name, with or without the `.eml` suffix.
//! let raw = test_corpus::load("plain-text-simple").bytes();
//! assert!(raw.starts_with(b"Return-Path:"));
//!
//! // Or every fixture in a category.
//! for fixture in test_corpus::by_category(test_corpus::Category::BrokenReferences) {
//!     println!("{}: {}", fixture.name(), fixture.description());
//! }
//! ```
//!
//! # Scope
//!
//! The loader deliberately deals in **bytes**, not parsed messages. MIME parsing
//! into the domain model is a separate concern that consumes this corpus; if the
//! loader parsed, every parser test would be testing the parser against itself.
//! Fixtures are also not all valid UTF-8 — several exist precisely because they
//! are ISO-8859-1, Shift_JIS or windows-1252 on the wire — so [`Fixture::bytes`]
//! is the primary accessor and [`Fixture::as_str`] is fallible by design.

use std::borrow::Cow;

use chrono::{DateTime, Utc};

use crate::ids::{AccountId, MailboxId};
use crate::message::Message;
use crate::mime;

/// Path of the corpus directory, relative to the workspace root.
///
/// Useful in error messages and in tests that want to walk the directory rather
/// than the embedded table.
pub const CORPUS_DIR: &str = "crates/postio-model/tests/corpus";

/// What a fixture is *for*: the behaviour it is meant to put under test.
///
/// A fixture usually carries several categories — a Japanese newsletter is both
/// [`Category::Html`] and [`Category::NonUtf8Charset`] — so callers should treat
/// these as tags, not as a taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Category {
    /// A `text/plain` body, the baseline case.
    PlainText,
    /// A `text/html` body: newsletters, marketing mail, rich replies.
    Html,
    /// `multipart/alternative` — the reader has to pick one part, never both.
    MultipartAlternative,
    /// `multipart/mixed` — a body plus attachments.
    MultipartMixed,
    /// `multipart/related` — parts referenced from the HTML by `cid:`.
    MultipartRelated,
    /// Nesting deeper than one level of multipart.
    NestedMultipart,
    /// Images displayed inline through a `cid:` reference.
    InlineImage,
    /// Carries at least one attachment part.
    Attachment,
    /// Carries an attachment big enough that buffering it whole is a mistake.
    LargeAttachment,
    /// The raw bytes are not UTF-8: ISO-8859-1, Shift_JIS, UTF-7, windows-1252.
    NonUtf8Charset,
    /// Uses `Content-Transfer-Encoding: base64`.
    Base64,
    /// Uses `Content-Transfer-Encoding: quoted-printable`.
    QuotedPrintable,
    /// RFC 2047 encoded words in the headers (subjects, display names).
    EncodedWord,
    /// Header block that a strict parser would reject; recovery is the point.
    MalformedHeaders,
    /// MIME structure that is broken or truncated rather than merely unusual.
    MalformedStructure,
    /// Required headers are absent — no `Message-ID`, no `Date`, no body.
    MissingHeaders,
    /// Part of a conversation the JWZ threading pass has to reassemble.
    Threading,
    /// `References`/`In-Reply-To` that are absent, malformed or dangling.
    BrokenReferences,
    /// Carries `List-Id` and the rest of the RFC 2369 list headers.
    MailingList,
    /// OpenPGP: `multipart/signed` or `multipart/encrypted`.
    Pgp,
    /// Loads resources from the network — remote images, tracking pixels.
    RemoteContent,
    /// Carries a `text/calendar` part or an `.ics` attachment.
    Calendar,
    /// A bounce: `multipart/report` with a `message/delivery-status` part.
    DeliveryStatus,
}

impl Category {
    /// Every category, in declaration order.
    pub const ALL: &'static [Category] = &[
        Category::PlainText,
        Category::Html,
        Category::MultipartAlternative,
        Category::MultipartMixed,
        Category::MultipartRelated,
        Category::NestedMultipart,
        Category::InlineImage,
        Category::Attachment,
        Category::LargeAttachment,
        Category::NonUtf8Charset,
        Category::Base64,
        Category::QuotedPrintable,
        Category::EncodedWord,
        Category::MalformedHeaders,
        Category::MalformedStructure,
        Category::MissingHeaders,
        Category::Threading,
        Category::BrokenReferences,
        Category::MailingList,
        Category::Pgp,
        Category::RemoteContent,
        Category::Calendar,
        Category::DeliveryStatus,
    ];

    /// The category's stable, lower-kebab-case name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Category::PlainText => "plain-text",
            Category::Html => "html",
            Category::MultipartAlternative => "multipart-alternative",
            Category::MultipartMixed => "multipart-mixed",
            Category::MultipartRelated => "multipart-related",
            Category::NestedMultipart => "nested-multipart",
            Category::InlineImage => "inline-image",
            Category::Attachment => "attachment",
            Category::LargeAttachment => "large-attachment",
            Category::NonUtf8Charset => "non-utf8-charset",
            Category::Base64 => "base64",
            Category::QuotedPrintable => "quoted-printable",
            Category::EncodedWord => "encoded-word",
            Category::MalformedHeaders => "malformed-headers",
            Category::MalformedStructure => "malformed-structure",
            Category::MissingHeaders => "missing-headers",
            Category::Threading => "threading",
            Category::BrokenReferences => "broken-references",
            Category::MailingList => "mailing-list",
            Category::Pgp => "pgp",
            Category::RemoteContent => "remote-content",
            Category::Calendar => "calendar",
            Category::DeliveryStatus => "delivery-status",
        }
    }

    /// Look a category up by the name [`Category::as_str`] returns.
    pub fn from_name(name: &str) -> Option<Category> {
        Category::ALL.iter().copied().find(|c| c.as_str() == name)
    }
}

impl std::fmt::Display for Category {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One `.eml` fixture: its raw bytes plus what it is there to exercise.
///
/// Values of this type are `'static` and embedded in the binary; there is no
/// filesystem access at run time, which is what lets other crates use the
/// corpus without knowing where the repository lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fixture {
    name: &'static str,
    file_name: &'static str,
    description: &'static str,
    categories: &'static [Category],
    bytes: &'static [u8],
}

impl Fixture {
    /// The fixture's name: its file name without the `.eml` suffix.
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// The fixture's file name, including the `.eml` suffix.
    pub const fn file_name(&self) -> &'static str {
        self.file_name
    }

    /// Path to the fixture relative to the workspace root.
    pub fn source_path(&self) -> String {
        format!("{CORPUS_DIR}/{}", self.file_name)
    }

    /// One line on what this fixture exercises and why it is in the corpus.
    pub const fn description(&self) -> &'static str {
        self.description
    }

    /// The behaviours this fixture is tagged with.
    pub const fn categories(&self) -> &'static [Category] {
        self.categories
    }

    /// Whether this fixture carries `category`.
    pub fn has_category(&self, category: Category) -> bool {
        self.categories.contains(&category)
    }

    /// The message exactly as it would arrive from a server.
    ///
    /// Line endings are CRLF except where a fixture exists precisely because
    /// they are not, and the bytes are not necessarily valid UTF-8.
    pub const fn bytes(&self) -> &'static [u8] {
        self.bytes
    }

    /// Length of the raw message in bytes.
    pub const fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Whether the fixture is empty. Nothing in the corpus is; this exists
    /// because clippy asks for it wherever there is a `len`.
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// The raw message as `&str`, or `None` when it is not valid UTF-8.
    ///
    /// Several fixtures are deliberately ISO-8859-1, Shift_JIS or windows-1252,
    /// so a caller that needs text must decide what to do about them.
    pub fn as_str(&self) -> Option<&'static str> {
        std::str::from_utf8(self.bytes).ok()
    }

    /// The raw message as text, replacing invalid sequences with `U+FFFD`.
    ///
    /// Convenient for assertions about header names, which are ASCII in every
    /// fixture regardless of what the body is encoded in.
    pub fn text_lossy(&self) -> Cow<'static, str> {
        String::from_utf8_lossy(self.bytes)
    }

    /// Parses this fixture into the domain [`Message`], via
    /// [`mime::parse`](crate::mime::parse).
    ///
    /// A thin convenience so threading, search and reader tests do not each
    /// invoke the parser and [`ParsedMessage::into_message`](crate::mime::ParsedMessage::into_message)
    /// by hand. `account_id` and `mailbox_id` are both `1`, and `received_at`
    /// is a fixed sentinel: none of these fixtures came from a real mailbox,
    /// and a caller that cares about scoping or receipt time is not using
    /// this corpus for that.
    pub fn parse(&self) -> Message {
        mime::parse(self.bytes).into_message(
            AccountId::new(1),
            MailboxId::new(1),
            sentinel_received_at(),
        )
    }
}

/// A fixed, arbitrary receive time for [`Fixture::parse`].
fn sentinel_received_at() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).expect("a valid fixed unix timestamp")
}

macro_rules! corpus {
    ( $( $name:literal : [ $( $cat:ident ),+ $(,)? ] => $doc:literal ),+ $(,)? ) => {
        /// Every fixture in the corpus, in file-name order.
        static FIXTURES: &[Fixture] = &[
            $(
                Fixture {
                    name: $name,
                    file_name: concat!($name, ".eml"),
                    description: $doc,
                    categories: &[ $( Category::$cat ),+ ],
                    bytes: include_bytes!(concat!("../tests/corpus/", $name, ".eml")),
                }
            ),+
        ];
    };
}

corpus! {
    "attachment-large": [MultipartMixed, Attachment, LargeAttachment, Base64] =>
        "A ~256 KiB base64 attachment: the case that must stream to the blob store rather than sit in memory.",
    "attachment-pdf": [MultipartMixed, Attachment, Base64] =>
        "Ordinary mail with two attachments, one of them a PDF with Content-Description and disposition parameters.",
    "attachment-rfc2231-filename": [MultipartMixed, Attachment, EncodedWord] =>
        "Three spellings of a non-ASCII filename: RFC 2231 continuations, charset'language form, and encoded-word abuse.",
    "bounce-delivery-status": [DeliveryStatus, MultipartMixed, PlainText] =>
        "A Postfix bounce: multipart/report with message/delivery-status and the original message embedded as message/rfc822.",
    "broken-references": [BrokenReferences, Threading, PlainText] =>
        "Every way a References header can be broken at once, plus an In-Reply-To that matches nothing.",
    "calendar-invite": [Calendar, MultipartAlternative, MultipartMixed, NestedMultipart, Attachment] =>
        "A meeting request: text/calendar METHOD=REQUEST inside an alternative, and the same ICS again as an attachment.",
    "charset-iso-8859-1": [NonUtf8Charset, PlainText] =>
        "Raw 8-bit ISO-8859-1 body with a Q-encoded latin-1 subject and display name. Not valid UTF-8.",
    "charset-shift-jis": [NonUtf8Charset, Base64, EncodedWord] =>
        "Japanese mail: Shift_JIS body in base64, Shift_JIS encoded words in Subject and From.",
    "charset-utf-7": [NonUtf8Charset, PlainText, EncodedWord] =>
        "UTF-7 body and UTF-7 encoded word, plus an IMAP modified-UTF-7 mailbox name in a header.",
    "charset-utf-8-emoji-rtl": [PlainText] =>
        "Valid UTF-8 stressing the renderer: ZWJ emoji, RTL Arabic and Hebrew, combining marks, astral-plane glyphs.",
    "charset-windows-1252-mislabeled": [NonUtf8Charset, PlainText] =>
        "windows-1252 bytes in the C1 range labelled iso-8859-1 — the mislabelling every real client silently forgives.",
    "duplicate-message-id": [Threading, PlainText] =>
        "Reuses the Message-ID of plain-text-simple with a different body: deduplication must not key on Message-ID alone.",
    "encoded-word-broken": [EncodedWord, MalformedHeaders] =>
        "Encoded words that are unterminated, invalid base64, an unknown charset, an unknown encoding letter, or over-long.",
    "encoded-word-subject-and-names": [EncodedWord, PlainText] =>
        "Correct RFC 2047: adjacent encoded words joined without a space, two charsets in one field, a folded Subject.",
    "header-folding-received-chain": [PlainText] =>
        "Deeply folded headers: a three-hop Received chain, DKIM-Signature, multi-line Authentication-Results and Subject.",
    "headers-only-no-body": [MissingHeaders, PlainText] =>
        "A message that ends after its headers, with no blank line and no body at all.",
    "html-newsletter": [Html, MultipartAlternative, QuotedPrintable, MailingList] =>
        "A real-shaped newsletter: nested layout tables, inline CSS, a media query, List-Unsubscribe and One-Click.",
    "html-tracking-pixel-remote-images": [Html, RemoteContent, QuotedPrintable] =>
        "A 1x1 open-rate beacon, remote <img> tags, CSS background-image URLs and a click-tracking redirect.",
    "inline-disposed-body": [MultipartAlternative, PlainText, Html, QuotedPrintable] =>
        "Both alternatives carry Content-Disposition: inline \u{2014} the part that *is* the message, marked the way an attachment is.",
    "inline-image-cid": [MultipartRelated, InlineImage, Html, Base64, Attachment] =>
        "Two inline PNGs referenced by cid:, plus a third cid: reference with no matching part.",
    "list-thread-01-root": [Threading, MailingList, PlainText] =>
        "Thread root: the message every other list-thread fixture hangs off.",
    "list-thread-02-reply": [Threading, MailingList, PlainText] =>
        "Well-formed depth-2 reply with both In-Reply-To and References.",
    "list-thread-03-reply-sibling": [Threading, MailingList, PlainText] =>
        "A second reply to the root: must render as a sibling of 02, not a child of it.",
    "list-thread-04-reply-deep": [Threading, MailingList, PlainText] =>
        "Depth-3 reply carrying the full two-entry References chain, folded across lines.",
    "list-thread-05-reply-no-references": [Threading, MailingList, BrokenReferences, PlainText] =>
        "In-Reply-To but no References at all — the most common way a real thread arrives half-linked.",
    "list-thread-06-reply-subject-only": [Threading, MailingList, BrokenReferences, PlainText] =>
        "Neither In-Reply-To nor References: only JWZ subject matching can attach it to the thread.",
    "list-thread-07-subject-change": [Threading, MailingList, BrokenReferences, PlainText] =>
        "Subject changed with 'was:' and References truncated to the last two entries, so the root is only reachable transitively.",
    "malformed-bare-lf": [MalformedStructure, MultipartMixed] =>
        "Bare LF line endings everywhere, including the MIME boundaries. Strict CRLF parsers find no body.",
    "malformed-headers": [MalformedHeaders] =>
        "A header block wrong in a dozen ways: no colon, no field name, duplicate Subject, unparseable Date, empty boundary.",
    "malformed-truncated-multipart": [MalformedStructure, MultipartMixed, Base64] =>
        "Delivery cut short mid-attachment: no closing boundary. The first part must still reach the reader.",
    "missing-message-id-and-date": [MissingHeaders, Threading, PlainText] =>
        "No Message-ID, no Date, no To: storage has to synthesize an identity and the list has to sort it anyway.",
    "multipart-alternative": [MultipartAlternative, PlainText, Html, QuotedPrintable] =>
        "The canonical text/plain + text/html pair, with a preamble and an epilogue that must both be discarded.",
    "nested-multipart": [NestedMultipart, MultipartMixed, MultipartAlternative, MultipartRelated, InlineImage, Attachment] =>
        "mixed > alternative > related, three levels deep, with an inline image and a trailing attachment.",
    "pgp-encrypted": [Pgp, MultipartMixed] =>
        "PGP/MIME multipart/encrypted: the version part plus an armoured OpenPGP blob.",
    "pgp-signed": [Pgp, MultipartMixed, PlainText] =>
        "PGP/MIME multipart/signed with micalg=pgp-sha256; the signed part must be preserved byte-exactly.",
    "plain-text-flowed-reply": [PlainText, Threading] =>
        "text/plain with format=flowed and delsp=yes, quoting its parent — reflowing and quote detection.",
    "plain-text-simple": [PlainText, Threading] =>
        "The smallest realistic message: 7bit us-ascii, a signature delimiter, nothing unusual.",
    "transfer-encoding-base64": [Base64, PlainText] =>
        "A plain-text body encoded base64, as export tools emit even when there is nothing to escape.",
    "transfer-encoding-quoted-printable": [QuotedPrintable, PlainText, EncodedWord] =>
        "Quoted-printable with soft line breaks, a literal =3D, encoded trailing whitespace and accented runs.",
}

/// Every fixture in the corpus, in file-name order.
pub fn all() -> &'static [Fixture] {
    FIXTURES
}

/// How many fixtures the corpus holds.
pub fn count() -> usize {
    FIXTURES.len()
}

/// The name of every fixture, in file-name order.
pub fn names() -> impl Iterator<Item = &'static str> {
    FIXTURES.iter().map(Fixture::name)
}

/// Look up one fixture by name, with or without its `.eml` suffix.
///
/// Returns `None` if there is no such fixture; see [`load`] for the panicking
/// form, which is usually what a test wants.
pub fn get(name: &str) -> Option<&'static Fixture> {
    let stem = name.strip_suffix(".eml").unwrap_or(name);
    FIXTURES.iter().find(|f| f.name == stem)
}

/// Look up one fixture by name, panicking with the list of known names if it
/// is missing.
///
/// This is the one-call entry point: `test_corpus::load("html-newsletter")`.
///
/// # Panics
///
/// If no fixture has that name. A typo in a test should fail loudly and say
/// what the alternatives were.
pub fn load(name: &str) -> &'static Fixture {
    get(name).unwrap_or_else(|| {
        let known: Vec<&str> = names().collect();
        panic!(
            "no fixture named `{name}` in {CORPUS_DIR}; known fixtures: {}",
            known.join(", ")
        )
    })
}

/// Every fixture tagged with `category`, in file-name order.
///
/// This is how a test asks for "all the broken-header ones" without hard-coding
/// a list that goes stale the moment someone adds a fixture.
pub fn by_category(category: Category) -> Vec<&'static Fixture> {
    FIXTURES
        .iter()
        .filter(|f| f.has_category(category))
        .collect()
}

/// Every fixture tagged with *all* of `categories`.
///
/// With an empty slice this returns the whole corpus.
pub fn by_categories(categories: &[Category]) -> Vec<&'static Fixture> {
    FIXTURES
        .iter()
        .filter(|f| categories.iter().all(|c| f.has_category(*c)))
        .collect()
}
