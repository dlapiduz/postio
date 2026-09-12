//! Can the HMAC gap be closed, and with what?
//!
//! `examples/throughput.rs` puts the Rust provider at 1.86x OpenSSL on
//! HMAC-SHA256, which is the page MAC Postio actually uses and the dominant
//! per-page cost. This asks whether that is a fact about Rust or a fact about
//! one crate.
//!
//! ```sh
//! cargo run --release -p postio-cipher --example hmac_backends
//! cargo run --release -p postio-cipher --example hmac_backends --features sha2-asm
//! RUSTFLAGS="-C target-cpu=native" cargo run --release -p postio-cipher --example hmac_backends
//! ```
//!
//! # Why `sha2` is slow here and `ring` might not be
//!
//! `sha2` 0.10.9's SHA-256 backends are `soft`, `soft_compact`, `aarch64`,
//! `loongarch64_asm` and `x86` — and the x86 one is SHA-NI only
//! (`cpufeatures::new!(shani_cpuid, "sha", …)`). There is no AVX2 path. So on
//! a CPU with AVX2 and without SHA-NI, which is this one, OpenSSL runs hand
//! written AVX2 assembly and `sha2` runs portable Rust. That is the whole
//! gap, and it is not a statement about the language.
//!
//! `ring` embeds BoringSSL's assembly, which has the AVX2 path — and it is
//! already in this workspace's dependency graph, pulled in by rustls for
//! every TLS connection Postio makes. Using it would add no third-party code
//! at all, which is the same argument that made the RustCrypto choice easy.

#![allow(unsafe_code)]

use std::time::{Duration, Instant};

use hmac::{Hmac, Mac};
use postio_cipher::Provider;

const PAGE: usize = 4096;
const HMAC_SHA256: i32 = 1;
const OPS: usize = 20_000;
const ROUNDS: usize = 5;

fn floor_of(rounds: usize, mut body: impl FnMut() -> Duration) -> Duration {
    (0..rounds).map(|_| body()).min().expect("a round")
}

fn main() {
    let directory = tempfile::tempdir().expect("a directory");
    {
        let db = rusqlite::Connection::open(directory.path().join("warm.db")).expect("open");
        db.pragma_update(None, "key", "k").expect("key");
        db.execute_batch("CREATE TABLE t(x)").expect("a schema");
    }
    // SAFETY: the open above initialised SQLCipher, so there is a provider.
    let shipped = unsafe { postio_cipher::current() };
    let shipped: &Provider = shipped.as_provider();

    let key = [0x11u8; 32];
    let page: Vec<u8> = (0..PAGE).map(|n| n as u8).collect();
    let tail = b"\x01\x00\x00\x00";

    println!(
        "HMAC-SHA256 over a {PAGE}-byte page{}\n",
        if cfg!(feature = "sha2-asm") {
            "   [sha2/asm]"
        } else {
            ""
        }
    );

    // OpenSSL, through the vtable, exactly as SQLCipher calls it.
    let mut digest = [0u8; 64];
    let openssl = floor_of(ROUNDS, || {
        let started = Instant::now();
        for _ in 0..OPS {
            // SAFETY: a live table; `digest` is `get_hmac_sz` long.
            unsafe { shipped.sign(HMAC_SHA256, &key, &page, tail, &mut digest) };
        }
        started.elapsed()
    });
    report("openssl (the shipped provider)", openssl, openssl);

    // RustCrypto, which is what the provider uses today.
    let rustcrypto = floor_of(ROUNDS, || {
        let started = Instant::now();
        for _ in 0..OPS {
            let mut mac = Hmac::<sha2::Sha256>::new_from_slice(&key).expect("any key length");
            mac.update(&page);
            mac.update(tail);
            std::hint::black_box(mac.finalize().into_bytes());
        }
        started.elapsed()
    });
    report("rustcrypto sha2", rustcrypto, openssl);

    // ring, already in the graph.
    let ring_key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &key);
    let with_ring = floor_of(ROUNDS, || {
        let started = Instant::now();
        for _ in 0..OPS {
            let mut ctx = ring::hmac::Context::with_key(&ring_key);
            ctx.update(&page);
            ctx.update(tail);
            std::hint::black_box(ctx.sign());
        }
        started.elapsed()
    });
    report("ring (boringssl asm)", with_ring, openssl);

    // And that they agree, because a faster wrong answer is not an answer.
    let mut theirs = [0u8; 32];
    // SAFETY: a live table; 32 bytes is `get_hmac_sz(HMAC_SHA256)`.
    unsafe { shipped.sign(HMAC_SHA256, &key, &page, tail, &mut theirs) };
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(&key).expect("any key length");
    mac.update(&page);
    mac.update(tail);
    assert_eq!(
        &mac.finalize().into_bytes()[..],
        &theirs[..],
        "sha2 differs"
    );
    let mut ctx = ring::hmac::Context::with_key(&ring_key);
    ctx.update(&page);
    ctx.update(tail);
    assert_eq!(ctx.sign().as_ref(), &theirs[..], "ring differs");
    println!("\nall three agree byte for byte");
}

fn report(label: &str, took: Duration, against: Duration) {
    let ns = took.as_secs_f64() * 1e9 / OPS as f64;
    println!(
        "{label:<32} {ns:>9.1} ns   {:>7.0} MB/s   vs openssl {:>5.2}x",
        PAGE as f64 / ns * 1000.0,
        took.as_secs_f64() / against.as_secs_f64()
    );
}
