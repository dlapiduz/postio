//! The `.eml` test corpus, embedded with `include_bytes!` so no test needs the network. Behind the
//! off-by-default `test-corpus` feature; dependents opt in from dev-dependencies.
//! The loader deals in bytes, not parsed messages, or parser tests would test the parser against
//! itself; `Fixture::as_str` is fallible because some fixtures are not UTF-8.

use std::borrow::Cow;

use chrono::{DateTime, Utc};

use crate::ids::{AccountId, MailboxId};
use crate::message::Message;
use crate::mime;

pub const CORPUS_DIR: &str = "crates/postio-model/tests/corpus";

/// A fixture usually carries several categories; treat them as tags, not a taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Category {
    PlainText,
    Html,
    MultipartAlternative,
    MultipartMixed,
    MultipartRelated,
    NestedMultipart,
    InlineImage,
    Attachment,
    LargeAttachment,
    NonUtf8Charset,
    Base64,
    QuotedPrintable,
    EncodedWord,
    MalformedHeaders,
    MalformedStructure,
    MissingHeaders,
    Threading,
    ThreadingRules,
    BrokenReferences,
    MailingList,
    Pgp,
    /// A signature to check, or the provider's record of one: the corpus keys verify these.
    Signature,
    RemoteContent,
    Calendar,
    /// A bounce: `multipart/report` with a `message/delivery-status` part.
    DeliveryStatus,
    Designed,
    Hostile,
    ThemeContrast,
    /// Data, never instructions (ADR 0009 Q4, ADR 0010).
    PromptInjection,
}

impl Category {
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
        Category::ThreadingRules,
        Category::BrokenReferences,
        Category::MailingList,
        Category::Pgp,
        Category::Signature,
        Category::RemoteContent,
        Category::Calendar,
        Category::DeliveryStatus,
        Category::Designed,
        Category::Hostile,
        Category::ThemeContrast,
        Category::PromptInjection,
    ];

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
            Category::ThreadingRules => "threading-rules",
            Category::BrokenReferences => "broken-references",
            Category::MailingList => "mailing-list",
            Category::Pgp => "pgp",
            Category::Signature => "signature",
            Category::RemoteContent => "remote-content",
            Category::Calendar => "calendar",
            Category::DeliveryStatus => "delivery-status",
            Category::Designed => "designed",
            Category::Hostile => "hostile",
            Category::ThemeContrast => "theme-contrast",
            Category::PromptInjection => "prompt-injection",
        }
    }

    pub fn from_name(name: &str) -> Option<Category> {
        Category::ALL.iter().copied().find(|c| c.as_str() == name)
    }
}

impl std::fmt::Display for Category {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fixture {
    name: &'static str,
    file_name: &'static str,
    description: &'static str,
    categories: &'static [Category],
    bytes: &'static [u8],
}

impl Fixture {
    pub const fn name(&self) -> &'static str {
        self.name
    }

    pub const fn file_name(&self) -> &'static str {
        self.file_name
    }

    pub fn source_path(&self) -> String {
        format!("{CORPUS_DIR}/{}", self.file_name)
    }

    pub const fn description(&self) -> &'static str {
        self.description
    }

    pub const fn categories(&self) -> &'static [Category] {
        self.categories
    }

    pub fn has_category(&self, category: Category) -> bool {
        self.categories.contains(&category)
    }

    /// CRLF line endings except where a fixture exists because they are not; not necessarily valid
    /// UTF-8.
    pub const fn bytes(&self) -> &'static [u8] {
        self.bytes
    }

    pub const fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Nothing in the corpus is empty; clippy asks for this beside `len`.
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// `None` when not valid UTF-8; several fixtures are deliberately ISO-8859-1, Shift_JIS or
    /// windows-1252.
    pub fn as_str(&self) -> Option<&'static str> {
        std::str::from_utf8(self.bytes).ok()
    }

    pub fn text_lossy(&self) -> Cow<'static, str> {
        String::from_utf8_lossy(self.bytes)
    }

    /// `account_id` and `mailbox_id` are both `1` and `received_at` is a fixed sentinel: a caller
    /// that cares about scoping or receipt time is not using the corpus for that.
    pub fn parse(&self) -> Message {
        mime::parse(self.bytes).into_message(
            AccountId::new(1),
            MailboxId::new(1),
            sentinel_received_at(),
        )
    }
}

fn sentinel_received_at() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).expect("a valid fixed unix timestamp")
}

macro_rules! corpus {
    ( $( $name:literal : [ $( $cat:ident ),+ $(,)? ] => $doc:literal ),+ $(,)? ) => {
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
    "auth-results-arc-list": [PlainText, MailingList, Signature] =>
        "A list re-signed the mail and sealed an ARC set: DKIM passes only for the list's domain, and the sender's own failed.",
    "auth-results-forged-lower": [PlainText, Hostile] =>
        "The receiving server's failing Authentication-Results above a passing one the sender wrote: only the topmost counts.",
    "auth-results-two-dkim": [PlainText, Signature] =>
        "Two passing DKIM signatures, one aligned with From and one by the sending service, which is not.",
    "auth-results-verified": [PlainText] =>
        "A receiving server's Authentication-Results with DKIM, SPF and DMARC all passing: nothing to warn about.",
    "autocrypt-header": [PlainText, Signature] =>
        "An unsigned message whose Autocrypt header carries corpus key A for its From address.",
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
    "display-name-impersonation": [PlainText, Hostile] =>
        "A passing message whose display name is a different address (billing@bank.example) than the one it came from.",
    "duplicate-message-id": [Threading, PlainText] =>
        "Reuses the Message-ID of plain-text-simple with a different body: deduplication must not key on Message-ID alone.",
    "encoded-word-broken": [EncodedWord, MalformedHeaders] =>
        "Encoded words that are unterminated, invalid base64, an unknown charset, an unknown encoding letter, or over-long.",
    "encoded-word-control-chars-in-header": [EncodedWord, MalformedHeaders] =>
        "Encoded words that decode to a forged From: and Date: line, a terminal escape, NUL, BEL and U+2028: a header value that must reach a reader as one line (#1770).",
    "encoded-word-crlf-in-header": [EncodedWord, MalformedHeaders] =>
        "An encoded word whose decoded text contains CR and LF: line breaks inside a header value that unfolding cannot remove, because the octets were never folding whitespace.",
    "encoded-word-subject-and-names": [EncodedWord, PlainText] =>
        "Correct RFC 2047: adjacent encoded words joined without a space, two charsets in one field, a folded Subject.",
    "header-folding-received-chain": [PlainText] =>
        "Deeply folded headers: a three-hop Received chain, DKIM-Signature, multi-line Authentication-Results and Subject.",
    "headers-only-no-body": [MissingHeaders, PlainText] =>
        "A message that ends after its headers, with no blank line and no body at all.",
    "hostile-remote-attribute-variants": [Html, RemoteContent, Hostile] =>
        "Legacy and oddly spelled attributes that name a remote resource.",
    "hostile-remote-css-obfuscated": [Html, RemoteContent, Hostile] =>
        "CSS that names a URL without a plain url( token: image-set, escapes, string @import, comments.",
    "hostile-remote-frames-embeds": [Html, RemoteContent, Hostile] =>
        "iframe, frameset, object, embed, applet and an img inside object fallback.",
    "hostile-remote-media-poster": [Html, RemoteContent, Hostile] =>
        "video poster and src, audio, source and track.",
    "hostile-remote-meta-refresh": [Html, RemoteContent, Hostile] =>
        "meta http-equiv refresh and a link prefetch.",
    "hostile-remote-srcset-picture": [Html, RemoteContent, Hostile] =>
        "srcset, picture/source and imagesrcset.",
    "hostile-remote-stylesheet-link": [Html, RemoteContent, Hostile] =>
        "A link rel=stylesheet, a link rel=preload and a base href.",
    "hostile-remote-svg-images": [Html, RemoteContent, Hostile] =>
        "inline svg with image href, xlink:href, use, feImage and a style @import.",
    "html-cjk-emoji": [Html] =>
        "Chinese, Japanese and Korean paragraphs and ZWJ emoji in HTML: shaping and font fallback with no tofu (spec 006 T006).",
    "html-class-styled": [Html, Designed] =>
        "A <style> block that targets its own classes and an id: what the sanitizer used to strip before any rule could match (#1545).",
    "html-dark-aware": [Html, ThemeContrast] =>
        "Declares color-scheme light dark and restyles itself under prefers-color-scheme: dark through class selectors.",
    "html-dark-text-no-background": [Html, ThemeContrast] =>
        "Sets dark text colours and no background anywhere: black text on a dark ground unless something adapts it.",
    "html-deep-nesting": [Html, Hostile] =>
        "Four hundred nested divs: the input that overflows a recursive layout's stack.",
    "html-designed-three-column": [Html, MultipartRelated, InlineImage, Base64, Designed, MailingList] =>
        "A three-column campaign in nested layout tables with a cid: hero image, coloured cards and a button.",
    "html-escaping-styles": [Html, Hostile] =>
        "A message whose inline styling tries to leave its own block: position fixed and absolute, a viewport-sized overlay, z-index, a transform, and viewport units \u{2014} beside ordinary colour and a layout table that must survive.",
    "html-every-url-vector": [Html, RemoteContent, Hostile] =>
        "Every way HTML and CSS name a remote resource: @import, @font-face, backgrounds, list markers, cursors, border images, content, a conditional rule, a background attribute, a pixel.",
    "html-gmail-reply-chain": [Html, MultipartAlternative, PlainText, Threading] =>
        "A webmail reply chain: two nested quoted blockquotes, an attribution line each, no colours or page of its own.",
    "html-illegible-sender-dark": [Html, ThemeContrast] =>
        "The sender's own dark-mode styling sets #333 text on #222: honoured, but not trusted.",
    "html-legacy-font-center": [Html] =>
        "Early-2000s markup: <font color face size>, <center>, and body bgcolor, text, link and vlink attributes.",
    "html-legacy-table-attrs": [Html, MultipartRelated, InlineImage, Base64] =>
        "Table presentational attributes browsers still honour: valign, cellpadding, cellspacing, border, align=center, img align, a cid: cell background.",
    "html-malformed-image": [Html, MultipartRelated, InlineImage, Base64, Hostile] =>
        "A cid: PNG cut off halfway: the decoder must fail into a sized placeholder.",
    "html-newsletter": [Html, MultipartAlternative, QuotedPrintable, MailingList, Designed] =>
        "A real-shaped newsletter: nested layout tables, inline CSS, a media query, List-Unsubscribe and One-Click.",
    "html-newsletter-many-tables": [Html, MultipartAlternative, MailingList, Designed, RemoteContent] =>
        "A long release-notes newsletter: two dozen nested layout tables, remote images, no page of its own; app colours, and a text part.",
    "html-newsletter-own-page": [Html, MailingList, Designed, ThemeContrast] =>
        "A newsletter that paints its own page (a body background) in a 640px layout table: rendered as sent, on paper.",
    "html-oversized-image": [Html, MultipartRelated, InlineImage, Base64, Hostile] =>
        "A PNG whose header declares 20,000 x 20,000 pixels in under a kilobyte: decode limits, not trust.",
    "html-receipt-fixed-width": [Html, RemoteContent, Designed] =>
        "A receipt with no background anywhere but a 560px layout table and a remote logo: paper by its layout alone.",
    "html-responsive-media": [Html, Designed] =>
        "Two columns that stack under @media (max-width: 600px): responsive rules judged against the pane's width.",
    "html-responsive-stacked-cells": [Html, MultipartAlternative, MailingList, Designed] =>
        "Layout-table cells made display:block by @media (max-width: 600px): the stacked columns are drawn in the 480px column.",
    "html-rtl-mixed": [Html] =>
        "Arabic and Hebrew paragraphs with Latin runs inside, plus an English line: bidirectional layout.",
    "html-script-forms": [Html, Hostile] =>
        "A script block, onload and onerror handlers, javascript: links in two spellings, and a credentials form.",
    "html-svg-local-file": [Html, MultipartRelated, InlineImage, Base64, Hostile] =>
        "A cid: SVG whose inner <image> elements name local files: an image must never read the disk.",
    "html-tracking-pixel-remote-images": [Html, RemoteContent, QuotedPrintable, Hostile] =>
        "A 1x1 open-rate beacon, remote <img> tags, CSS background-image URLs and a click-tracking redirect.",
    "html-transactional-receipt": [Html, Designed] =>
        "A receipt: a hidden preheader, a data: logo, and a totals table whose cells must copy in reading order.",
    "html-transparent-logo": [Html, MultipartRelated, InlineImage, Base64, ThemeContrast] =>
        "A dark logo PNG with a transparent ground, drawn for a white page: invisible if a dark ground shows through.",
    "html-very-tall": [Html, Hostile] =>
        "About 40,000 pixels of lines with a unique last one: no height at which content may be cut off.",
    "html-white-page-reply": [Html, ThemeContrast, Threading] =>
        "A desktop-client reply that stamps a white page and black text on everything: correspondence, not design.",
    "html-work-black-text": [Html, ThemeContrast] =>
        "Office-client work mail: black text set on every span, Calibri, a bordered table with tinted headers, one red sentence.",
    "inline-disposed-body": [MultipartAlternative, PlainText, Html, QuotedPrintable] =>
        "Both alternatives carry Content-Disposition: inline \u{2014} the part that *is* the message, marked the way an attachment is.",
    "inline-image-cid": [MultipartRelated, InlineImage, Html, Base64, Attachment] =>
        "Two inline PNGs referenced by cid:, plus a third cid: reference with no matching part.",
    "invite-cancel": [Calendar, MultipartAlternative, PlainText, QuotedPrintable, Base64] =>
        "An Outlook-style METHOD:CANCEL for invite-windows-zone's UID, at SEQUENCE 1: the marker says cancelled and offers no answer.",
    "invite-iana-zone": [Calendar, MultipartMixed, MultipartAlternative, NestedMultipart, Attachment, PlainText, Html, Base64, QuotedPrintable] =>
        "A Google-style request on an IANA zone with its VTIMEZONE, an attendee inside a VALARM, and the same ICS again as an attachment.",
    "invite-quoted-printable": [Calendar, MultipartAlternative, PlainText, QuotedPrintable] =>
        "An Apple-style request whose calendar part is quoted-printable, with quoted CN and EMAIL parameters, the week after the clocks go back.",
    "invite-update-sequence": [Calendar, MultipartAlternative, PlainText, Html, Base64, QuotedPrintable] =>
        "invite-iana-zone's event moved to a new time: the same UID at SEQUENCE 1 with a later DTSTAMP, which replaces the marker's time.",
    "invite-utc-times": [Calendar, MultipartMixed, MultipartAlternative, NestedMultipart, Attachment, PlainText, Html, Base64] =>
        "A Zoom-style request in UTC with no VTIMEZONE, a stray TZID property to ignore, and the calendar attached rather than an alternative.",
    "invite-weekly-exdate": [Calendar, MultipartMixed, MultipartAlternative, NestedMultipart, Attachment, PlainText, Base64] =>
        "A Thunderbird-style weekly event with two EXDATEs, six-digit zone offsets, and occurrences on both sides of a DST change.",
    "invite-windows-zone": [Calendar, MultipartAlternative, PlainText, Html, QuotedPrintable, Base64] =>
        "An Outlook-style request on a Windows zone name with its VTIMEZONE, base64 calendar part, and X-MICROSOFT properties to ignore.",
    "invite-zone-without-vtimezone": [Calendar, MultipartMixed, Attachment, PlainText] =>
        "A booking-page request whose DTSTART names Asia/Kolkata and carries no VTIMEZONE: the zone is resolved by name, half-hour offset and all.",
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
    "multipart-boundary-never-appears": [MalformedStructure, MultipartAlternative] =>
        "A multipart whose boundary parameter names a delimiter that appears nowhere in the body, which is what a gateway that rewrote one and not the other leaves behind.",
    "multipart-boundary-quoted-in-body": [MalformedStructure, MultipartMixed, PlainText, Attachment] =>
        "A body that quotes its own boundary three ways: mid-line, at a line start followed by words, and as the prefix of a longer token. None is a delimiter, so the whole text survives and the attachment after it is still found.",
    "nested-multipart": [NestedMultipart, MultipartMixed, MultipartAlternative, MultipartRelated, InlineImage, Attachment] =>
        "mixed > alternative > related, three levels deep, with an inline image and a trailing attachment.",
    "pgp-encrypted": [Pgp, MultipartMixed] =>
        "PGP/MIME multipart/encrypted: the version part plus an armoured OpenPGP blob.",
    "pgp-inline-signed": [PlainText, Pgp, Signature] =>
        "An inline cleartext signature by corpus key A, with a dash-escaped line.",
    "pgp-key-attached": [MultipartMixed, Attachment, Pgp, Signature] =>
        "Corpus key A attached as application/pgp-keys, offered as Import key.",
    "pgp-signed": [Pgp, MultipartMixed, PlainText] =>
        "PGP/MIME multipart/signed with micalg=pgp-sha256; the signed part must be preserved byte-exactly.",
    "pgp-signed-good": [Pgp, Signature, PlainText] =>
        "PGP/MIME signed by corpus key A over SHA-256: good against keys/ada-a.asc.",
    "pgp-signed-list-footer": [Pgp, Signature, MultipartMixed, NestedMultipart, MailingList] =>
        "A list wrapped the good signed part in multipart/mixed and added an unsigned footer: only part of the message is signed.",
    "pgp-signed-no-key": [Pgp, Signature, PlainText] =>
        "Signed by a key that was never exported: no key for it exists anywhere.",
    "pgp-signed-other-key": [Pgp, Signature, PlainText] =>
        "The good fixture's text from the same address, signed by corpus key B (RSA 3072, SHA-512).",
    "pgp-signed-qp": [Pgp, Signature, QuotedPrintable] =>
        "A quoted-printable signed part with soft breaks: the signature covers the encoded form.",
    "pgp-signed-reencoded": [Pgp, Signature, Base64] =>
        "The quoted-printable fixture's signature over the same text re-encoded as base64: bad.",
    "pgp-signed-sha1": [Pgp, Signature, PlainText] =>
        "Signed by corpus key B over SHA-1, which is no longer safe: never good.",
    "pgp-signed-tampered-body": [Pgp, Signature, PlainText] =>
        "The good fixture with one body byte changed after signing: bad.",
    "pgp-signed-tampered-header": [Pgp, Signature, PlainText] =>
        "The good fixture with its signed part's transfer-encoding header changed: bad.",
    "plain-text-flowed-reply": [PlainText, Threading] =>
        "text/plain with format=flowed and delsp=yes, quoting its parent — reflowing and quote detection.",
    "plain-text-simple": [PlainText, Threading] =>
        "The smallest realistic message: 7bit us-ascii, a signature delimiter, nothing unusual.",
    "thread-archive-1-parent": [Threading, ThreadingRules, PlainText] =>
        "Archive pair, parent: belongs in the Inbox; its reply is filed in Archive.",
    "thread-archive-2-reply": [Threading, ThreadingRules, PlainText] =>
        "Archive pair, reply: links to the parent by In-Reply-To and References; belongs in Archive.",
    "thread-fork-1-root": [Threading, ThreadingRules, PlainText] =>
        "Fork, root: both replies answer this message.",
    "thread-fork-2-reply-a": [Threading, ThreadingRules, PlainText] =>
        "Fork, first reply to the root; the fourth message answers this one.",
    "thread-fork-3-reply-b": [Threading, ThreadingRules, PlainText] =>
        "Fork, second reply to the root, from the account's own address: a sibling of the first reply.",
    "thread-fork-4-reply-to-a": [Threading, ThreadingRules, PlainText] =>
        "Fork, reply to the first reply: References carry root then parent.",
    "thread-hello-30d-1": [Threading, ThreadingRules, PlainText] =>
        "'Re: Hello' with no references: the earlier of two unrelated messages 30 days apart.",
    "thread-hello-30d-2": [Threading, ThreadingRules, PlainText] =>
        "'Re: Hello' with no references, 30 days after its namesake: outside the subject window, so a separate thread.",
    "thread-hello-3d-1": [Threading, ThreadingRules, PlainText] =>
        "'Re: Hello' with no references: the earlier of two messages 3 days apart.",
    "thread-hello-3d-2": [Threading, ThreadingRules, PlainText] =>
        "'Re: Hello' with no references, 3 days after its namesake: inside the 7-day subject window, so one thread.",
    "thread-list-1-root": [Threading, ThreadingRules, MailingList, PlainText] =>
        "List discussion, root: carries List-Id and the RFC 2369 post header.",
    "thread-list-2-reply": [Threading, ThreadingRules, MailingList, PlainText] =>
        "List discussion, reply linked by In-Reply-To and References: must thread by references although it has a List-Id.",
    "thread-list-3-reply": [Threading, ThreadingRules, MailingList, PlainText] =>
        "List discussion, grandchild with the full References chain: must thread by references although it has a List-Id.",
    "thread-newsletter-1-issue": [Threading, ThreadingRules, MailingList, PlainText] =>
        "Weekly newsletter, first issue: a List-Id and a subject shared by every issue.",
    "thread-newsletter-2-issue": [Threading, ThreadingRules, MailingList, PlainText] =>
        "Weekly newsletter, second issue a week later with the same subject: a List-Id message never joins by subject.",
    "thread-sent-1-received": [Threading, ThreadingRules, PlainText] =>
        "Sent-reply pair, the received message the account answers.",
    "thread-sent-2-reply": [Threading, ThreadingRules, PlainText] =>
        "A reply sent from the account's own address, linked by In-Reply-To and References; belongs in Sent.",
    "top-posted-reply-signature": [MultipartAlternative, PlainText, Html, Threading] =>
        "A reply above its quote in both parts: the answer, a signature, then the history as > lines and as a <blockquote>.",
    "transactional-shipping-notice": [MultipartAlternative, PlainText, Html, MailingList, Designed] =>
        "A shipping notice whose plain part carries a repeated label: value block \u{2014} the facts reader view lifts above the body copy.",
    "transfer-encoding-base64": [Base64, PlainText] =>
        "A plain-text body encoded base64, as export tools emit even when there is nothing to escape.",
    "transfer-encoding-quoted-printable": [QuotedPrintable, PlainText, EncodedWord] =>
        "Quoted-printable with soft line breaks, a literal =3D, encoded trailing whitespace and accented runs.",
    "untrusted-instructions": [PlainText, PromptInjection] =>
        "One honest question, then instructions and a tool call aimed at an assistant: none of it may become an action, a request or a marker.",
}

pub fn all() -> &'static [Fixture] {
    FIXTURES
}

pub fn count() -> usize {
    FIXTURES.len()
}

pub fn names() -> impl Iterator<Item = &'static str> {
    FIXTURES.iter().map(Fixture::name)
}

pub fn get(name: &str) -> Option<&'static Fixture> {
    let stem = name.strip_suffix(".eml").unwrap_or(name);
    FIXTURES.iter().find(|f| f.name == stem)
}

/// Panics listing the known names if there is no such fixture, so a typo in a test fails loudly.
pub fn load(name: &str) -> &'static Fixture {
    get(name).unwrap_or_else(|| {
        let known: Vec<&str> = names().collect();
        panic!(
            "no fixture named `{name}` in {CORPUS_DIR}; known fixtures: {}",
            known.join(", ")
        )
    })
}

pub fn by_category(category: Category) -> Vec<&'static Fixture> {
    FIXTURES
        .iter()
        .filter(|f| f.has_category(category))
        .collect()
}

/// With an empty slice, the whole corpus.
pub fn by_categories(categories: &[Category]) -> Vec<&'static Fixture> {
    FIXTURES
        .iter()
        .filter(|f| categories.iter().all(|c| f.has_category(*c)))
        .collect()
}
