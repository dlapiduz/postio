//! SQLCipher's crypto, in Rust instead of OpenSSL.
//!
//! **A spike, not a shipping decision.** It exists to answer one question
//! with code rather than argument: can Postio keep SQLCipher — the format,
//! the store on disk, every byte of it — and stop linking OpenSSL?
//!
//! # The seam
//!
//! OpenSSL is not wired into SQLCipher. It is the *default* provider, and
//! only because nothing else was named:
//!
//! ```c
//! #if !defined (SQLCIPHER_CRYPTO_CC) && !defined (SQLCIPHER_CRYPTO_LIBTOMCRYPT) \
//!  && !defined (SQLCIPHER_CRYPTO_OPENSSL) && !defined (SQLCIPHER_CRYPTO_CUSTOM)
//! #define SQLCIPHER_CRYPTO_OPENSSL
//! #endif
//! ```
//!
//! What a provider *is* is [`Provider`]: a table of eighteen function
//! pointers, of which five do real work — `hmac`, `kdf`, `cipher`, `random`
//! and `add_random` — and the rest report sizes and names. Every primitive
//! behind those five is already in this workspace's dependency graph, put
//! there by other crates: `aes`, `cbc`, `hmac`, `sha2`, `pbkdf2`,
//! `getrandom`.
//!
//! # What this does and does not change
//!
//! It changes **who computes the bytes, not what the bytes are.** SQLCipher
//! decides the format — AES-256-CBC over each page, a random per-page IV, an
//! HMAC over the ciphertext, and PBKDF2 over the passphrase and the file's
//! salt. All of that is in the amalgamation, above this table. So a store
//! written by the OpenSSL build opens here and the reverse, with no
//! migration and no re-encrypt, which is the property
//! [`tests/differential.rs`] is about.
//!
//! # Two ways in, and this crate uses the cheap one
//!
//! Shipping this means compiling SQLCipher with
//! `-DSQLCIPHER_CRYPTO_CUSTOM=postio_cipher_setup`, and `libsqlite3-sys`'s
//! build script has no way to ask for that — it hard-codes four branches and
//! emits `-DSQLCIPHER_CRYPTO_OPENSSL`, `_CC`, or a link line. That is a
//! patched build script, and the whole reason this is a spike.
//!
//! But SQLCipher also registers providers at *run time*
//! ([`install`]), and a provider registered that way is elevated to the
//! default for everything opened afterwards. So this can be proved against
//! the real amalgamation, on real stores, without touching anybody's build —
//! and what is left to settle is a build flag rather than whether the
//! cryptography is right.
//!
//! [`tests/differential.rs`]: https://github.com/dlapiduz/postio

use std::ffi::{CStr, c_char, c_int, c_void};

use aes::Aes256;
use aes::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit, block_padding::NoPadding};
use hmac::{Hmac, Mac};
use zeroize::Zeroize;

/// `SQLITE_OK`.
const OK: c_int = 0;
/// `SQLITE_ERROR`.
const ERROR: c_int = 1;

/// SQLCipher's algorithm selectors, for both `hmac` and `kdf`.
const HMAC_SHA1: c_int = 0;
const HMAC_SHA256: c_int = 1;
const HMAC_SHA512: c_int = 2;

/// AES-256: the only cipher SQLCipher 4 has.
const KEY_SZ: c_int = 32;
const IV_SZ: c_int = 16;
const BLOCK_SZ: c_int = 16;

/// What `PRAGMA cipher` reports. Spelled as OpenSSL's `OBJ_nid2sn` spells it,
/// because a person comparing two builds should not have to decide whether a
/// different string means a different cipher. It is reporting only —
/// `sqlcipher_codec_ctx_set_cipher` takes no name from here.
const CIPHER_NAME: &CStr = c"AES-256-CBC";
const PROVIDER_NAME: &CStr = c"rust";
const PROVIDER_VERSION: &CStr = c"postio-cipher 0.2.0";

type Aes256CbcEnc = cbc::Encryptor<Aes256>;
type Aes256CbcDec = cbc::Decryptor<Aes256>;

/// The table SQLCipher calls its crypto through.
///
/// Field order is the C struct's, exactly, and that is the whole of the
/// contract: `add_random` before `random`, `get_cipher` after `cipher`. A
/// field out of place here is not a compile error, it is a function pointer
/// called with the wrong arguments — so [`crate::tests::the_table_is_the_size_c_expects`]
/// checks what can be checked from this side, and the differential test
/// checks the rest by getting the right answers out of it.
#[repr(C)]
pub struct Provider {
    init: Option<extern "C" fn() -> c_int>,
    shutdown: Option<extern "C" fn()>,
    get_provider_name: Option<extern "C" fn(*mut c_void) -> *const c_char>,
    add_random: Option<extern "C" fn(*mut c_void, *const c_void, c_int) -> c_int>,
    random: Option<extern "C" fn(*mut c_void, *mut c_void, c_int) -> c_int>,
    #[allow(clippy::type_complexity)]
    hmac: Option<
        extern "C" fn(
            *mut c_void,
            c_int,
            *const u8,
            c_int,
            *const u8,
            c_int,
            *const u8,
            c_int,
            *mut u8,
        ) -> c_int,
    >,
    #[allow(clippy::type_complexity)]
    kdf: Option<
        extern "C" fn(
            *mut c_void,
            c_int,
            *const u8,
            c_int,
            *const u8,
            c_int,
            c_int,
            c_int,
            *mut u8,
        ) -> c_int,
    >,
    #[allow(clippy::type_complexity)]
    cipher: Option<
        extern "C" fn(
            *mut c_void,
            c_int,
            *const u8,
            c_int,
            *const u8,
            *const u8,
            c_int,
            *mut u8,
        ) -> c_int,
    >,
    get_cipher: Option<extern "C" fn(*mut c_void) -> *const c_char>,
    get_key_sz: Option<extern "C" fn(*mut c_void) -> c_int>,
    get_iv_sz: Option<extern "C" fn(*mut c_void) -> c_int>,
    get_block_sz: Option<extern "C" fn(*mut c_void) -> c_int>,
    get_hmac_sz: Option<extern "C" fn(*mut c_void, c_int) -> c_int>,
    ctx_init: Option<extern "C" fn(*mut *mut c_void) -> c_int>,
    ctx_free: Option<extern "C" fn(*mut *mut c_void) -> c_int>,
    fips_status: Option<extern "C" fn(*mut c_void) -> c_int>,
    get_provider_version: Option<extern "C" fn(*mut c_void) -> *const c_char>,
    next: *mut Provider,
}

impl Provider {
    /// A table with nothing in it, for [`postio_cipher_setup`] to fill.
    const fn empty() -> Provider {
        Provider {
            init: None,
            shutdown: None,
            get_provider_name: None,
            add_random: None,
            random: None,
            hmac: None,
            kdf: None,
            cipher: None,
            get_cipher: None,
            get_key_sz: None,
            get_iv_sz: None,
            get_block_sz: None,
            get_hmac_sz: None,
            ctx_init: None,
            ctx_free: None,
            fips_status: None,
            get_provider_version: None,
            next: std::ptr::null_mut(),
        }
    }
}

#[allow(unsafe_code)]
unsafe extern "C" {
    /// SQLCipher's runtime provider registration. Declared rather than bound
    /// through `libsqlite3-sys`, which does not expose it: the symbol is in
    /// the amalgamation whichever provider that build compiled in.
    fn sqlcipher_register_provider(provider: *mut Provider) -> c_int;
    /// The provider in force. Used by the differential test to reach the
    /// OpenSSL one and ask it the same questions.
    fn sqlcipher_get_provider() -> *mut Provider;
    /// SQLCipher's own allocator. A provider it is going to free at shutdown
    /// has to have come from here — see [`install`].
    fn sqlcipher_malloc(size: u64) -> *mut c_void;
    /// The matching free, for a table that never reached the chain.
    fn sqlcipher_free(memory: *mut c_void, size: u64);
    /// SQLite's own initialiser: idempotent, thread-safe, and what runs
    /// SQLCipher's `sqlcipher_extra_init`. [`install`] calls it so that it
    /// has no precondition of its own.
    fn sqlite3_initialize() -> c_int;
}

/// Fill `provider` in — the shape `-DSQLCIPHER_CRYPTO_CUSTOM=postio_cipher_setup`
/// expects.
///
/// SQLCipher calls this once, with a table it allocated, and registers the
/// result itself. That is the shipping path, and it is the one with no
/// ownership question in it at all: SQLCipher allocates, this fills, SQLCipher
/// frees.
///
/// # Safety
///
/// `provider` must be non-null, aligned for `Provider`, and writable for
/// `size_of::<Provider>()` bytes. It does **not** have to be initialised.
///
/// That last sentence is load-bearing and was not true of the first version
/// of this function, which took `&mut *provider` to fill the fields. A
/// reference — even a `&mut` that is only written through — asserts that what
/// it points at is a valid `Provider`, and most of `Provider` is
/// `Option<fn(..)>`, a type with invalid bit patterns. Forming one over
/// uninitialised memory is undefined behaviour whatever you do with it
/// afterwards.
///
/// It happened to work because `sqlcipher_malloc` zeroes, so every `Option`
/// really was a valid `None` — which is the worst way for this kind of bug to
/// behave: correct by the allocator's habit rather than by anything the
/// signature says, and silent the day a caller hands over a plain `malloc`.
/// Writing through raw pointers asks nothing of the memory and costs nothing.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn postio_cipher_setup(provider: *mut Provider) -> c_int {
    if provider.is_null() {
        return ERROR;
    }
    /// Write one field without asserting anything about what is there now.
    macro_rules! put {
        ($field:ident, $value:expr) => {
            // SAFETY: the caller's contract — `provider` is writable and
            // aligned for `Provider`, so the field projection is in bounds
            // and `write` needs no valid value at the destination.
            unsafe { std::ptr::write(&raw mut (*provider).$field, $value) }
        };
    }
    put!(init, None);
    put!(shutdown, None);
    put!(get_provider_name, Some(get_provider_name as _));
    put!(add_random, Some(add_random as _));
    put!(random, Some(random as _));
    put!(hmac, Some(hmac as _));
    put!(kdf, Some(kdf as _));
    put!(cipher, Some(cipher as _));
    put!(get_cipher, Some(get_cipher as _));
    put!(get_key_sz, Some(get_key_sz as _));
    put!(get_iv_sz, Some(get_iv_sz as _));
    put!(get_block_sz, Some(get_block_sz as _));
    put!(get_hmac_sz, Some(get_hmac_sz as _));
    put!(ctx_init, Some(ctx_init as _));
    put!(ctx_free, Some(ctx_free as _));
    put!(fips_status, Some(fips_status as _));
    put!(get_provider_version, Some(get_provider_version as _));
    // SQLCipher sets this itself at registration; writing it keeps the table
    // wholly initialised rather than mostly.
    put!(next, std::ptr::null_mut());
    OK
}

/// Make this the provider SQLCipher uses from now on.
///
/// Everything opened afterwards encrypts and decrypts through Rust. Anything
/// already open goes on using whatever it was opened with, which is
/// SQLCipher's own rule rather than this crate's.
///
/// # Why this is a safe function and what it had to do to earn it
///
/// An earlier version of this was safe and should not have been. It
/// documented "call it after SQLite is initialised" and then did FFI that
/// depended on it — which is a safety precondition on a safe function, and
/// therefore a hole: `sqlcipher_malloc` reaches for a private heap and a
/// mutex that `sqlite3_initialize` is what creates, and nothing stopped a
/// caller from asking first.
///
/// It discharges that itself now. `sqlite3_initialize` is idempotent and is
/// what runs SQLCipher's own `sqlcipher_extra_init`, so after it returns
/// `SQLITE_OK` the heap and the mutexes exist and there is no precondition
/// left for a caller to get wrong.
///
/// # The table has to be SQLCipher's to free
///
/// This allocates through `sqlcipher_malloc` and hands the pointer over for
/// good. `sqlcipher_extra_shutdown` walks the provider chain and calls
/// `sqlcipher_free(provider, sizeof(sqlcipher_provider))` on every link, so a
/// table that was a `static` in this crate aborts the process at exit:
///
/// ```text
/// test ... ok
/// free(): invalid pointer
/// ```
///
/// The *shipping* path cannot make either mistake.
/// `-DSQLCIPHER_CRYPTO_CUSTOM` has SQLCipher allocate the table, call
/// [`postio_cipher_setup`] to fill it, and register it — this crate never
/// holds a pointer at all. This function is the runtime door, which exists so
/// the spike could be tested against the real amalgamation without patching
/// anybody's build script, and it is the half that needs the care.
///
/// Calling it more than once registers more than one table. That is not
/// unsound — SQLCipher owns and frees each of them — but the second is a
/// waste, and only the last is the default.
#[allow(unsafe_code)]
pub fn install() -> Result<(), &'static str> {
    // SAFETY: `sqlite3_initialize` takes no arguments, is documented as safe
    // to call any number of times and from any thread, and is what every
    // `sqlite3_open` calls first. It is the call that makes everything below
    // it defined.
    if unsafe { sqlite3_initialize() } != OK {
        return Err("sqlite would not initialise");
    }

    let size = std::mem::size_of::<Provider>() as u64;
    // SAFETY: SQLite is initialised, so SQLCipher's private heap and its
    // memory mutex exist. The allocation is exactly the size of the table it
    // is about to be handed.
    let provider = unsafe { sqlcipher_malloc(size).cast::<Provider>() };
    if provider.is_null() {
        return Err("sqlcipher would not allocate a provider table");
    }

    // SAFETY: the allocation above — non-null, aligned, and `size` bytes,
    // which is `postio_cipher_setup`'s whole contract.
    if unsafe { postio_cipher_setup(provider) } != OK {
        // SAFETY: nothing has been told about this table, so this crate is
        // still the only owner and freeing it is this crate's to do. The
        // size is the one it was allocated with.
        unsafe { sqlcipher_free(provider.cast(), size) };
        return Err("the provider table would not fill in");
    }

    // SAFETY: as above. This is the call that transfers ownership: after it
    // succeeds the table is on SQLCipher's chain and SQLCipher frees it at
    // shutdown, so nothing here may free it or keep the pointer.
    let rc = unsafe { sqlcipher_register_provider(provider) };
    if rc == OK {
        return Ok(());
    }
    // Refused, so it never reached the chain and nothing else will free it.
    // SAFETY: ownership never transferred; see above.
    unsafe { sqlcipher_free(provider.cast(), size) };
    Err("sqlcipher refused the provider")
}

/// Put `provider` back in force.
///
/// The other half of [`install`], and the same call underneath: SQLCipher
/// elevates whatever is registered to the default, and a table already on its
/// list is moved rather than initialised again. For a test that has to ask
/// both providers the same question in one process.
///
/// # Safety
///
/// `provider` must be a live provider table — in practice one that
/// [`current`] handed out.
#[allow(unsafe_code)]
pub unsafe fn restore(provider: *mut Provider) -> Result<(), &'static str> {
    // SAFETY: the caller's contract.
    let rc = unsafe { sqlcipher_register_provider(provider) };
    if rc == OK {
        Ok(())
    } else {
        Err("sqlcipher refused the provider")
    }
}

/// The provider in force right now, for a caller that wants to compare.
///
/// # Safety
///
/// The returned pointer is SQLCipher's and must not be freed. It is valid for
/// the life of the process.
#[allow(unsafe_code)]
pub unsafe fn current() -> *mut Provider {
    // SAFETY: the caller's contract; SQLCipher owns the table.
    unsafe { sqlcipher_get_provider() }
}

/// Asking a provider the questions SQLCipher asks it.
///
/// The point of these is that they work on *any* provider, this crate's or
/// the one SQLCipher compiled in, so the two can be given identical inputs
/// and compared byte for byte. That comparison is the whole argument: a
/// provider that agrees with OpenSSL on every primitive produces a store
/// OpenSSL can read, and no amount of reading the code says so as plainly.
impl Provider {
    /// Derive a key, as `PRAGMA key` does.
    ///
    /// # Safety
    ///
    /// `self` must be a live provider table.
    #[allow(unsafe_code)]
    pub unsafe fn derive(
        &self,
        algorithm: c_int,
        pass: &[u8],
        salt: &[u8],
        workfactor: c_int,
        out: &mut [u8],
    ) -> c_int {
        let Some(kdf) = self.kdf else { return ERROR };
        kdf(
            std::ptr::null_mut(),
            algorithm,
            pass.as_ptr(),
            pass.len() as c_int,
            salt.as_ptr(),
            salt.len() as c_int,
            workfactor,
            out.len() as c_int,
            out.as_mut_ptr(),
        )
    }

    /// MAC `first || second`, as the page checksum does.
    ///
    /// # Safety
    ///
    /// `self` must be a live provider table, and `out` must have room for
    /// [`Provider::hmac_sz`] of `algorithm`.
    #[allow(unsafe_code)]
    pub unsafe fn sign(
        &self,
        algorithm: c_int,
        key: &[u8],
        first: &[u8],
        second: &[u8],
        out: &mut [u8],
    ) -> c_int {
        let Some(hmac) = self.hmac else { return ERROR };
        hmac(
            std::ptr::null_mut(),
            algorithm,
            key.as_ptr(),
            key.len() as c_int,
            first.as_ptr(),
            first.len() as c_int,
            second.as_ptr(),
            second.len() as c_int,
            out.as_mut_ptr(),
        )
    }

    /// Encrypt (`encrypting`) or decrypt one page body.
    ///
    /// # Safety
    ///
    /// `self` must be a live provider table, and `out` must be as long as
    /// `input`.
    #[allow(unsafe_code)]
    pub unsafe fn transform(
        &self,
        encrypting: bool,
        key: &[u8],
        iv: &[u8],
        input: &[u8],
        out: &mut [u8],
    ) -> c_int {
        let Some(cipher) = self.cipher else {
            return ERROR;
        };
        cipher(
            std::ptr::null_mut(),
            if encrypting { 1 } else { 0 },
            key.as_ptr(),
            key.len() as c_int,
            iv.as_ptr(),
            input.as_ptr(),
            input.len() as c_int,
            out.as_mut_ptr(),
        )
    }

    /// What this provider calls itself — `"openssl"`, or this crate's
    /// `"rust"`.
    #[allow(unsafe_code)]
    pub fn name(&self) -> String {
        let Some(get) = self.get_provider_name else {
            return String::new();
        };
        let name = get(std::ptr::null_mut());
        if name.is_null() {
            return String::new();
        }
        // SAFETY: a provider's name is a `'static` C string in its own
        // translation unit; OpenSSL's is a literal and so is this crate's.
        unsafe { CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned()
    }

    /// The digest size for `algorithm`, in bytes.
    pub fn hmac_sz(&self, algorithm: c_int) -> c_int {
        self.get_hmac_sz
            .map_or(0, |get| get(std::ptr::null_mut(), algorithm))
    }

    /// The key, IV and block sizes, in that order.
    pub fn sizes(&self) -> (c_int, c_int, c_int) {
        (
            self.get_key_sz.map_or(0, |g| g(std::ptr::null_mut())),
            self.get_iv_sz.map_or(0, |g| g(std::ptr::null_mut())),
            self.get_block_sz.map_or(0, |g| g(std::ptr::null_mut())),
        )
    }
}

/// This crate's table, for a caller that wants to compare it with another
/// without registering it first.
///
/// Leaked on purpose, and **never registered**: a table SQLCipher has not
/// been told about must not end up on the chain it frees at shutdown, and one
/// it has been told about is already reachable through [`current`].
///
/// # Safety
///
/// The pointer must not be freed and must not be passed to
/// `sqlcipher_register_provider` — use [`install`] for that, which allocates
/// a table SQLCipher may keep.
#[allow(unsafe_code)]
pub unsafe fn table() -> *mut Provider {
    static COMPARISON: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *COMPARISON.get_or_init(|| {
        let provider = Box::into_raw(Box::new(Provider::empty()));
        // SAFETY: a fresh, writable allocation of exactly this type.
        unsafe { postio_cipher_setup(provider) };
        provider as usize
    }) as *mut Provider
}

extern "C" fn get_provider_name(_ctx: *mut c_void) -> *const c_char {
    PROVIDER_NAME.as_ptr()
}

extern "C" fn get_provider_version(_ctx: *mut c_void) -> *const c_char {
    PROVIDER_VERSION.as_ptr()
}

extern "C" fn get_cipher(_ctx: *mut c_void) -> *const c_char {
    CIPHER_NAME.as_ptr()
}

extern "C" fn get_key_sz(_ctx: *mut c_void) -> c_int {
    KEY_SZ
}

extern "C" fn get_iv_sz(_ctx: *mut c_void) -> c_int {
    IV_SZ
}

extern "C" fn get_block_sz(_ctx: *mut c_void) -> c_int {
    BLOCK_SZ
}

extern "C" fn get_hmac_sz(_ctx: *mut c_void, algorithm: c_int) -> c_int {
    match algorithm {
        HMAC_SHA1 => 20,
        HMAC_SHA256 => 32,
        HMAC_SHA512 => 64,
        _ => 0,
    }
}

/// No per-context state. OpenSSL's provider does the same — the context is a
/// hook for a provider that needs one, and this one does not.
#[allow(unsafe_code)]
extern "C" fn ctx_init(ctx: *mut *mut c_void) -> c_int {
    if ctx.is_null() {
        return ERROR;
    }
    // SAFETY: SQLCipher passes the address of a pointer it owns.
    unsafe { *ctx = std::ptr::null_mut() };
    OK
}

extern "C" fn ctx_free(_ctx: *mut *mut c_void) -> c_int {
    OK
}

/// Not a FIPS build, and saying so is the honest answer rather than a
/// limitation: OpenSSL's own provider returns whatever libcrypto reports, and
/// what this returns is read only by `PRAGMA cipher_fips_status`.
extern "C" fn fips_status(_ctx: *mut c_void) -> c_int {
    0
}

#[allow(unsafe_code)]
extern "C" fn random(_ctx: *mut c_void, buffer: *mut c_void, length: c_int) -> c_int {
    if buffer.is_null() || length < 0 {
        return ERROR;
    }
    // SAFETY: SQLCipher's contract is that `buffer` has room for `length`
    // bytes; it is the same contract `RAND_bytes` is called under next door.
    let out = unsafe { std::slice::from_raw_parts_mut(buffer.cast::<u8>(), length as usize) };
    match getrandom::getrandom(out) {
        Ok(()) => OK,
        Err(_) => ERROR,
    }
}

/// Entropy the caller wants mixed in.
///
/// Accepted and dropped, deliberately. `getrandom` is the kernel's pool and
/// has no "stir this in" door — and does not need one: what SQLCipher passes
/// here is material it already has, so discarding it cannot lower the
/// entropy of anything [`random`] goes on to produce. OpenSSL's provider
/// forwards it to `RAND_add`, which is a statement about OpenSSL's userspace
/// generator rather than about the pool underneath it.
extern "C" fn add_random(_ctx: *mut c_void, _buffer: *const c_void, _length: c_int) -> c_int {
    OK
}

/// HMAC over `in1 || in2`, into a buffer of exactly the digest's size.
///
/// Two inputs rather than one because SQLCipher MACs the page ciphertext and
/// then the page number, and will not allocate to join them.
#[allow(clippy::too_many_arguments)]
#[allow(unsafe_code)]
extern "C" fn hmac(
    _ctx: *mut c_void,
    algorithm: c_int,
    key: *const u8,
    key_sz: c_int,
    in1: *const u8,
    in1_sz: c_int,
    in2: *const u8,
    in2_sz: c_int,
    out: *mut u8,
) -> c_int {
    // The OpenSSL provider's own first check: a null first input is an
    // error, a null second one is simply "there is no second part".
    if in1.is_null() || key.is_null() || out.is_null() || key_sz < 0 || in1_sz < 0 {
        return ERROR;
    }
    // SAFETY: SQLCipher's contract on every one of these: `key` holds
    // `key_sz` bytes, `in1` holds `in1_sz`, `in2` holds `in2_sz` when it is
    // not null, and `out` has room for `get_hmac_sz(algorithm)`.
    let (key, first, second) = unsafe {
        (
            std::slice::from_raw_parts(key, key_sz as usize),
            std::slice::from_raw_parts(in1, in1_sz as usize),
            if in2.is_null() || in2_sz <= 0 {
                &[][..]
            } else {
                std::slice::from_raw_parts(in2, in2_sz as usize)
            },
        )
    };

    /// One arm per digest, spelled out rather than written generically.
    ///
    /// `Hmac<D>`'s bounds are a thicket of `CoreProxy` associated types, and
    /// a `where` clause long enough to satisfy them would be three times the
    /// size of the three lines it replaced — for a match with exactly three
    /// arms that will not grow: SQLCipher has three algorithm constants.
    macro_rules! sign {
        ($digest:ty) => {{
            let Ok(mut mac) = Hmac::<$digest>::new_from_slice(key) else {
                return ERROR;
            };
            mac.update(first);
            mac.update(second);
            let tag = mac.finalize().into_bytes();
            // SAFETY: `out` has room for the digest — `get_hmac_sz` is what
            // SQLCipher sized it from, and it answers for the same
            // `algorithm` this match arm was chosen by.
            unsafe { std::ptr::copy_nonoverlapping(tag.as_ptr(), out, tag.len()) };
            OK
        }};
    }

    match algorithm {
        HMAC_SHA1 => sign!(sha1::Sha1),
        HMAC_SHA256 => sign!(sha2::Sha256),
        HMAC_SHA512 => sign!(sha2::Sha512),
        _ => ERROR,
    }
}

/// PBKDF2 over the passphrase and the file's salt.
#[allow(clippy::too_many_arguments)]
#[allow(unsafe_code)]
extern "C" fn kdf(
    _ctx: *mut c_void,
    algorithm: c_int,
    pass: *const u8,
    pass_sz: c_int,
    salt: *const u8,
    salt_sz: c_int,
    workfactor: c_int,
    key_sz: c_int,
    key: *mut u8,
) -> c_int {
    if pass.is_null() || salt.is_null() || key.is_null() {
        return ERROR;
    }
    if pass_sz < 0 || salt_sz < 0 || key_sz <= 0 || workfactor <= 0 {
        return ERROR;
    }
    // SAFETY: SQLCipher's contract, as for `hmac` above.
    let (pass, salt, out) = unsafe {
        (
            std::slice::from_raw_parts(pass, pass_sz as usize),
            std::slice::from_raw_parts(salt, salt_sz as usize),
            std::slice::from_raw_parts_mut(key, key_sz as usize),
        )
    };

    let rounds = workfactor as u32;
    let derived = match algorithm {
        HMAC_SHA1 => pbkdf2::pbkdf2::<Hmac<sha1::Sha1>>(pass, salt, rounds, out),
        HMAC_SHA256 => pbkdf2::pbkdf2::<Hmac<sha2::Sha256>>(pass, salt, rounds, out),
        HMAC_SHA512 => pbkdf2::pbkdf2::<Hmac<sha2::Sha512>>(pass, salt, rounds, out),
        _ => return ERROR,
    };
    if derived.is_err() {
        out.zeroize();
        return ERROR;
    }
    OK
}

/// One page, AES-256-CBC, no padding.
///
/// `mode` is OpenSSL's `enc` flag, which is what SQLCipher passes through: 1
/// encrypts, 0 decrypts. `in_sz` is always a whole number of blocks — the
/// page body — and `out` is a buffer of the same size.
#[allow(clippy::too_many_arguments)]
#[allow(unsafe_code)]
extern "C" fn cipher(
    _ctx: *mut c_void,
    mode: c_int,
    key: *const u8,
    key_sz: c_int,
    iv: *const u8,
    input: *const u8,
    in_sz: c_int,
    out: *mut u8,
) -> c_int {
    if key.is_null() || iv.is_null() || input.is_null() || out.is_null() {
        return ERROR;
    }
    if key_sz != KEY_SZ || in_sz <= 0 || in_sz % BLOCK_SZ != 0 {
        return ERROR;
    }
    // SAFETY: SQLCipher's contract: `key` is `key_sz` bytes, `iv` is
    // `get_iv_sz()`, `input` is `in_sz`, and `out` has room for `in_sz` —
    // the amalgamation asserts `in_sz == csz` on the way out of the OpenSSL
    // provider, which is the same promise from the other side.
    let (key, iv, input, out) = unsafe {
        (
            std::slice::from_raw_parts(key, key_sz as usize),
            std::slice::from_raw_parts(iv, IV_SZ as usize),
            std::slice::from_raw_parts(input, in_sz as usize),
            std::slice::from_raw_parts_mut(out, in_sz as usize),
        )
    };

    let done = if mode == 1 {
        Aes256CbcEnc::new_from_slices(key, iv)
            .ok()
            .and_then(|enc| enc.encrypt_padded_b2b_mut::<NoPadding>(input, out).ok())
            .is_some()
    } else {
        Aes256CbcDec::new_from_slices(key, iv)
            .ok()
            .and_then(|dec| dec.decrypt_padded_b2b_mut::<NoPadding>(input, out).ok())
            .is_some()
    };
    if done { OK } else { ERROR }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_the_size_c_expects() {
        // Eighteen pointers. This cannot catch a field in the *wrong order*
        // -- every one of them is pointer-sized, so a transposition is the
        // same number of bytes -- which is why the differential test asks
        // the table questions rather than measuring it.
        assert_eq!(
            std::mem::size_of::<Provider>(),
            18 * std::mem::size_of::<*const c_void>()
        );
    }

    /// The contract [`postio_cipher_setup`] states, exercised over memory
    /// that really is uninitialised.
    ///
    /// The first version of that function took `&mut *provider` to fill the
    /// fields, which asserts the memory already holds a valid `Provider` —
    /// and most of a `Provider` is `Option<fn(..)>`, which has invalid bit
    /// patterns. It passed everything, because `sqlcipher_malloc` zeroes and
    /// every `Option` really was a valid `None`.
    ///
    /// This is deliberately `alloc`, not `alloc_zeroed`: the point is memory
    /// with no promise attached, which is what the doc comment now says is
    /// enough. On its own it proves the function *works* there; run under
    /// Miri it would also have failed on the version that did not.
    #[test]
    #[allow(unsafe_code)]
    fn the_table_fills_in_over_memory_that_was_never_initialised() {
        let layout = std::alloc::Layout::new::<Provider>();
        // SAFETY: a non-zero-sized layout, and the allocation is freed below
        // with the same one.
        let raw = unsafe { std::alloc::alloc(layout) }.cast::<Provider>();
        assert!(!raw.is_null(), "the test allocator gave up");

        // SAFETY: non-null, aligned and `size_of::<Provider>()` bytes, which
        // is the whole of what `postio_cipher_setup` asks for. It is *not*
        // initialised, which is the point.
        assert_eq!(unsafe { postio_cipher_setup(raw) }, OK);

        // SAFETY: initialised by the call above, so a reference is now
        // sound — which it was not a line earlier.
        let filled: &Provider = unsafe { &*raw };
        assert_eq!(filled.name(), "rust");
        assert_eq!(filled.sizes(), (32, 16, 16));
        assert!(
            filled.next.is_null(),
            "a table handed over with a stale `next` would splice whatever \
             was in that memory into SQLCipher's provider chain"
        );

        // SAFETY: the allocation above, with the layout it was made with,
        // and nothing else holds it.
        unsafe { std::alloc::dealloc(raw.cast(), layout) };
    }

    #[test]
    fn the_sizes_are_sqlcipher_4s() {
        assert_eq!(get_key_sz(std::ptr::null_mut()), 32, "AES-256");
        assert_eq!(get_iv_sz(std::ptr::null_mut()), 16);
        assert_eq!(get_block_sz(std::ptr::null_mut()), 16);
        assert_eq!(get_hmac_sz(std::ptr::null_mut(), HMAC_SHA512), 64);
        assert_eq!(get_hmac_sz(std::ptr::null_mut(), HMAC_SHA256), 32);
        assert_eq!(get_hmac_sz(std::ptr::null_mut(), HMAC_SHA1), 20);
        assert_eq!(
            get_hmac_sz(std::ptr::null_mut(), 99),
            0,
            "an algorithm this provider does not have is nought bytes, not a \
             guess"
        );
    }

    /// RFC 6070's PBKDF2-HMAC-SHA1 vector, which is the only one of the three
    /// with published test vectors — and enough to prove the arguments are
    /// in the order this provider thinks they are.
    #[test]
    fn the_kdf_matches_a_published_vector() {
        let mut key = [0u8; 20];
        let rc = kdf(
            std::ptr::null_mut(),
            HMAC_SHA1,
            c"password".as_ptr().cast(),
            8,
            c"salt".as_ptr().cast(),
            4,
            4096,
            key.len() as c_int,
            key.as_mut_ptr(),
        );
        assert_eq!(rc, OK);
        assert_eq!(
            key,
            [
                0x4b, 0x00, 0x79, 0x01, 0xb7, 0x65, 0x48, 0x9a, 0xbe, 0xad, 0x49, 0xd9, 0x26, 0xf7,
                0x21, 0xd0, 0x65, 0xa4, 0x29, 0xc1,
            ]
        );
    }

    #[test]
    fn a_page_round_trips_through_itself() {
        let key = [7u8; 32];
        let iv = [3u8; 16];
        let plain = [9u8; 64];
        let mut encrypted = [0u8; 64];
        let mut back = [0u8; 64];

        assert_eq!(
            cipher(
                std::ptr::null_mut(),
                1,
                key.as_ptr(),
                32,
                iv.as_ptr(),
                plain.as_ptr(),
                64,
                encrypted.as_mut_ptr()
            ),
            OK
        );
        assert_ne!(encrypted, plain, "that would not be encryption");
        assert_eq!(
            cipher(
                std::ptr::null_mut(),
                0,
                key.as_ptr(),
                32,
                iv.as_ptr(),
                encrypted.as_ptr(),
                64,
                back.as_mut_ptr()
            ),
            OK
        );
        assert_eq!(back, plain);
    }

    #[test]
    fn a_short_block_is_refused_rather_than_padded() {
        // SQLCipher hands whole pages and does its own framing, so a length
        // that is not a whole number of blocks is a bug upstream of here.
        // Padding it would turn that bug into a store that reads back wrong.
        let key = [7u8; 32];
        let iv = [3u8; 16];
        let plain = [9u8; 17];
        let mut out = [0u8; 32];
        assert_eq!(
            cipher(
                std::ptr::null_mut(),
                1,
                key.as_ptr(),
                32,
                iv.as_ptr(),
                plain.as_ptr(),
                17,
                out.as_mut_ptr()
            ),
            ERROR
        );
    }
}
