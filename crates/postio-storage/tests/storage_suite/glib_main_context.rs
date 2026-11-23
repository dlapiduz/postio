//! Does a Turso future need a reactor? This is what the
//! eight `POSTIO-GLIB-SAFE` markers point at — a marker is a decision written
//! down, and it needs a measurement under it. Deliberately not `#[tokio::test]`, which supplies a
//! runtime.

#![cfg(target_os = "linux")]

#[test]
fn a_store_opens_and_reads_on_the_main_context_with_no_runtime() {
    assert!(
        tokio::runtime::Handle::try_current().is_err(),
        "this case is about a thread with no runtime on it, and there is one \
         -- so it would pass whatever the engine does"
    );

    let main_loop = glib::MainLoop::new(None, false);
    let quit = main_loop.clone();
    let outcome: std::rc::Rc<std::cell::RefCell<Option<Result<i64, String>>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));
    let wrote = std::rc::Rc::clone(&outcome);

    glib::spawn_future_local(async move {
        let answer = async {
            let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
            let key = postio_storage::key::StoreKey::generate()
                .derive(postio_storage::key::Purpose::Database);
            let store = postio_storage::Store::open(directory.path().join("probe.db"), &key)
                .await
                .map_err(|error| error.to_string())?;
            let connection = store.connect().await.map_err(|error| error.to_string())?;
            postio_storage::sql::scalar(&connection, "SELECT count(*) FROM sqlite_schema", ())
                .await
                .map_err(|error| error.to_string())
        }
        .await;
        *wrote.borrow_mut() = Some(answer);
        quit.quit();
    });

    main_loop.run();

    let answer = outcome
        .borrow_mut()
        .take()
        .expect("the future ran to completion");
    let objects = answer.unwrap_or_else(|error| {
        panic!(
            "a store read on the glib main context failed: {error}\n\
             If this is \"there is no reactor running\", the engine's futures \
             have stopped being self-contained and the eight \
             POSTIO-GLIB-SAFE markers this test backs are now wrong: each of \
             those awaits has to move onto the runtime and answer over a \
             channel, which is the shape `check-runtime-crossings.py` \
             documents."
        )
    });
    assert!(
        objects > 0,
        "the store opened but reported an empty schema, so the read did not \
         reach a real database and this proves nothing"
    );
}
