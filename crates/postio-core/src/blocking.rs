//! Answers a synchronous callback (GTK, a WebKit `cid:` URI) that must read the store now.
//!
//! Only bounded indexed reads belong here: blocking the main loop past 16 ms drops a frame.

use std::cell::OnceCell;
use std::future::Future;

thread_local! {
    /// Multi-threaded with one worker: a `current_thread` runtime refuses `block_in_place`, so a
    /// nested `now` would abort the process.
    static BRIDGE: OnceCell<tokio::runtime::Runtime> = const { OnceCell::new() };
}

/// Panics if no runtime can be built (out of file descriptors).
pub fn now<T>(future: impl Future<Output = T>) -> T {
    // A nested `now` (a GTK callback inside another, as in sign-in then `install_autosave`) must
    // reuse the running runtime, since a second one panics. `block_in_place` makes that safe, but
    // only on a multi-threaded runtime.
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        // Asserted here because tokio's own message names no runtime to change and arrives through
        // a GTK trampoline that cannot unwind.
        assert!(
            handle.runtime_flavor() != tokio::runtime::RuntimeFlavor::CurrentThread,
            "a synchronous store read was reached from inside a current-thread \
             runtime, which can neither stand aside for `block_in_place` nor \
             answer a second `block_on`. Whichever runtime drives this \
             callback has to be built `new_multi_thread().worker_threads(1)` \
             -- see `postio_session::blocking`."
        );
        return tokio::task::block_in_place(|| handle.block_on(future));
    }

    BRIDGE.with(|cell| {
        cell.get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .expect("a runtime for synchronous store reads")
        })
        .block_on(future)
    })
}

#[cfg(test)]
mod tests {
    /// Nested synchronous store reads are the ordinary case (sign-in then `install_autosave`); a
    /// `current_thread` runtime aborted the process on them.
    #[test]
    fn a_store_read_inside_a_store_read_answers_rather_than_aborting() {
        let answer = super::now(async { super::now(async { 21 }) * 2 });
        assert_eq!(
            answer, 42,
            "the nested read came back wrong, which is a different bug from \
             the one this test is about"
        );
    }

    /// A `current_thread` runtime cannot serve a nested read; the panic must name the runtime to
    /// change.
    #[test]
    #[should_panic(expected = "worker_threads(1)")]
    fn a_current_thread_runtime_is_refused_by_name() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        runtime.block_on(async { super::now(async { 1 }) });
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn nesting_works_on_a_borrowed_runtime_too() {
        let answer = tokio::task::spawn_blocking(|| super::now(async { super::now(async { 7 }) }))
            .await
            .expect("the blocking task");
        assert_eq!(answer, 7);
    }
}
