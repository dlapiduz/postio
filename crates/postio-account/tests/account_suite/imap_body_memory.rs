#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

fn rid(uid: u32) -> postio_model::RemoteId {
    postio_model::RemoteId::new(format!("100:{uid}"))
}

use postio_account::backend::{BodyPart, CountingSink};
use postio_account::cancel::CancelToken;
use postio_account::imap::{
    ConnectionPool, ConnectionSettings, IMAPS_PORT, ImapScript, PoolConfig, Priority,
    ScriptedConnector, fetch_part,
};
use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_model::TransportSecurity;

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

fn peak_bytes() -> usize {
    PEAK.load(Ordering::SeqCst)
}

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
    // `CountingSink`, not `VecSink`: a `VecSink`'s own accumulator would scale with `len` whatever
    // the fetch does.
    let mut sink = CountingSink::new();

    let before = peak_bytes();

    let result = fetch_part(
        &pool,
        "INBOX",
        &rid(101),
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

/// Peak memory for a message a hundred times larger stays within a constant multiple of the read
/// buffer.
#[tokio::test(flavor = "current_thread")]
async fn peak_memory_does_not_grow_with_message_size() {
    const SMALL: u32 = 64 * 1024; // one read buffer's worth
    const LARGE: u32 = 20 * 1024 * 1024; // 20 MiB: if this were buffered

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
