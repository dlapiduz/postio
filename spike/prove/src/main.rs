//! SQLCipher with a Rust crypto provider, proved two ways.
//!
//!   prove write <path>   create an encrypted store and put a row in it
//!   prove read  <path>   open one somebody else wrote and read it back
//!
//! Built twice — once with `LIBSQLITE3_SYS_CRYPTO_CUSTOM=postio_cipher_setup`
//! and once without — the two binaries have to be able to read each other's
//! stores, or an existing install could not be upgraded into this.
const KEY: &str = "correct horse battery staple";

fn open(path: &str) -> rusqlite::Connection {
    let db = rusqlite::Connection::open(path).expect("open");
    db.pragma_update(None, "key", KEY).expect("key");
    db
}

fn main() {
    // Keeps `postio_cipher_setup` in the link even when nothing calls it:
    // with `-DSQLCIPHER_CRYPTO_CUSTOM` the *C* calls it, and the linker has
    // no other reason to keep a symbol from an rlib.
    let _keep: unsafe extern "C" fn(_) -> _ = postio_cipher::postio_cipher_setup;

    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_default();
    let path = args.next().expect("a path");

    let db = open(&path);
    let provider: String = db
        .query_row("PRAGMA cipher_provider", [], |r| r.get(0))
        .expect("cipher_provider");
    let cipher: String = db
        .query_row("PRAGMA cipher", [], |r| r.get(0))
        .expect("cipher");

    match mode.as_str() {
        "write" => {
            db.execute_batch(
                "CREATE TABLE mail(subject TEXT); INSERT INTO mail VALUES ('it works')",
            )
            .expect("write");
            drop(db);
            let head = std::fs::read(&path).expect("the file")[..16].to_vec();
            assert!(
                !head.starts_with(b"SQLite format 3"),
                "the store is not encrypted at all"
            );
            println!("wrote  {path}  provider={provider}  cipher={cipher}  (encrypted)");
        }
        "read" => {
            let subject: String = db
                .query_row("SELECT subject FROM mail", [], |r| r.get(0))
                .expect("the row the other provider wrote");
            assert_eq!(subject, "it works");
            println!("read   {path}  provider={provider}  cipher={cipher}  -> {subject:?}");
        }
        other => panic!("unknown mode {other}"),
    }
}
