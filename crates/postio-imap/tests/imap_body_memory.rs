//! Proves `BodyPart::Whole` streams a large message rather than buffering
//! it: peak heap usage stays flat as the fetched message grows, not
//! proportional to it.
//!
//! A global allocator tracks live and peak bytes for the whole test binary.
//! [`ScriptedConnector`]'s canned reply is itself generated on demand via
//! [`postio_imap::imap::ImapScript::on_generated`] rather than held as one
//! big buffer — otherwise the *test double* would dominate the measurement
//! this test exists to take on the code under test.

#![allow(unsafe_code)]
// Installing a `GlobalAlloc` is `unsafe impl` by definition. That is the whole
// technique here: counting allocations is how this test proves a body fetch
// does not materialise the whole message. The crate's library code has no
// `unsafe` at all.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use postio_imap::backend::{BodyPart, CountingSink};
use postio_imap::cancel::CancelToken;
use postio_imap::imap::{
    ConnectionPool, ConnectionSettings, IMAPS_PORT, ImapScript, PoolConfig, Priority,
    ScriptedConnector, fetch_part,
};
use postio_imap::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_model::{TransportSecurity, Uid};

// ---------------------------------------------------------------------------
// A peak-tracking global allocator
// ---------------------------------------------------------------------------

struct TrackingAllocator;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let live = LIVE.fetch_add(layout.size(), Ordering::SeqCst) + layout.size();
            PEAK.fetch_max(live, Ordering::SeqCst);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::SeqCst);
    }
}

#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator;

/// The high-water mark of live bytes since the process started.
fn peak_bytes() -> usize {
    PEAK.load(Ordering::SeqCst)
}

// ---------------------------------------------------------------------------
// Fetching a message of `len` bytes
// ---------------------------------------------------------------------------

const ACCOUNT: &str = "someone@example.com";

async fn fetch_whole_message_of(len: u32) -> usize {
    let store = MemorySecretStore::new();
    let key = AccountKey::new(ACCOUNT);
    store
        .store(&key, &Password::new("app-specific-password"))
        .await
        .expect("seed the keyring");

    let connector = ScriptedConnector::new(
        ImapScript::extensions_hidden_until_login()
            .on(
                "SELECT",
                "* 5 EXISTS\n* 0 RECENT\n* OK [UIDVALIDITY 100] UIDs valid\n\
                 {tag} OK SELECT completed",
            )
            .on_generated(
                "FETCH",
                "* 1 FETCH (BODY[] {",
                len,
                ")\n{tag} OK FETCH completed",
            ),
    );

    let pool = ConnectionPool::new(
        ConnectionSettings::new(
            "imap.example.com",
            IMAPS_PORT,
            TransportSecurity::Tls,
            ACCOUNT,
        ),
        key,
        Arc::new(store),
        Arc::new(connector),
        PoolConfig::default(),
    );
    // `CountingSink`, not `VecSink`: a `VecSink` retains every byte by
    // design, so its own accumulator would scale with `len` regardless of
    // whether the fetch itself streams — exactly the confound this test
    // must avoid. `CountingSink` only ever holds a counter, which isolates
    // the fetch machinery's own behaviour.
    let mut sink = CountingSink::new();

    // The peak just before the operation under test: everything up to here
    // (the tokio runtime, TLS setup, the pool, the scripted transcript's own
    // small fixed strings) is setup, not what this test is measuring.
    let before = peak_bytes();

    let result = fetch_part(
        &pool,
        "INBOX",
        Uid::new(101),
        &BodyPart::Whole,
        &mut sink,
        Priority::Interactive,
        &CancelToken::new(),
    )
    .await
    .expect("the scripted whole-message fetch");

    assert_eq!(result.bytes_written, u64::from(len));
    assert_eq!(sink.bytes(), u64::from(len));

    peak_bytes().saturating_sub(before)
}

/// A message a hundred times the size of the small one costs no more peak
/// memory than a generous constant multiple of the read buffer — proving
/// `BodyPart::Whole` streams into the sink rather than buffering the whole
/// message, which would make this scale with `len` instead.
#[tokio::test(flavor = "current_thread")]
async fn peak_memory_does_not_grow_with_message_size() {
    const SMALL: u32 = 64 * 1024; // one read buffer's worth
    const LARGE: u32 = 20 * 1024 * 1024; // 20 MiB: if this were buffered
    // whole, it alone would blow past the cap below by two orders of
    // magnitude.

    // A single generous cap, not a ratio against the small run: a ratio
    // would itself scale with message size for anything less than a
    // perfectly flat implementation, defeating the point of the assertion.
    const CAP: usize = 4 * 1024 * 1024;

    let small_delta = fetch_whole_message_of(SMALL).await;
    assert!(
        small_delta < CAP,
        "fetching a {SMALL}-byte message peaked {small_delta} bytes above its \
         baseline, expected under {CAP}"
    );

    let large_delta = fetch_whole_message_of(LARGE).await;
    assert!(
        large_delta < CAP,
        "fetching a {LARGE}-byte message peaked {large_delta} bytes above its \
         baseline, expected under {CAP} — a whole-message fetch must not \
         buffer the message"
    );
}
