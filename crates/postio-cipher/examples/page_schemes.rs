//! What a page would cost if the *format* were ours to choose.
//!
//! ```sh
//! cargo run --release -p postio-cipher --example page_schemes
//! ```
//!
//! `examples/hmac_backends.rs` answers a narrow question — can the Rust
//! provider match OpenSSL at SQLCipher's own algorithms — and the answer is
//! yes, through `ring`, which is BoringSSL's assembly in a Rust wrapper. This
//! asks the wider one: forget what the format is today, is there a
//! *pure-Rust* scheme that is as fast or faster?
//!
//! # Why the comparison is per page and not per primitive
//!
//! SQLCipher's page path is **two passes**: AES-256-CBC over the page, then
//! HMAC over the ciphertext. An AEAD is one. So the honest unit is what it
//! costs to protect one 4 KiB page and to check and open it again, and a
//! scheme can win by doing less work rather than by doing the same work
//! faster.
//!
//! Everything here is measured both ways round, because reading is what
//! Postio does: a mailbox is written once by a sync and read on every
//! keystroke.

#![allow(unsafe_code)]

use std::time::{Duration, Instant};

use aes::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit, block_padding::NoPadding};
use chacha20poly1305::aead::{AeadInOut, KeyInit};
use hmac::{Hmac, Mac};
use postio_cipher::Provider;

const PAGE: usize = 4096;
const HMAC_SHA256: i32 = 1;
const OPS: usize = 20_000;
const ROUNDS: usize = 5;

type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;
type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;

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
    let iv = [0x22u8; 16];
    let page: Vec<u8> = (0..PAGE).map(|n| n as u8).collect();

    println!("one {PAGE}-byte page, sealed and opened again\n");

    // ── what Postio pays today ───────────────────────────────────────────
    //
    // Through the vtable, exactly as SQLCipher calls it: the cipher, then
    // the MAC over the ciphertext. Two passes over the page.
    {
        let mut out = vec![0u8; PAGE];
        let mut digest = [0u8; 64];
        let seal = floor_of(ROUNDS, || {
            let started = Instant::now();
            for _ in 0..OPS {
                // SAFETY: a live table; the buffers are the sizes the vtable
                // documents.
                unsafe {
                    shipped.transform(true, &key, &iv, &page, &mut out);
                    shipped.sign(HMAC_SHA256, &key, &out, b"\x01\x00\x00\x00", &mut digest);
                }
            }
            started.elapsed()
        });
        let open = floor_of(ROUNDS, || {
            // Allocated outside the timed loop, like every other scheme
            // here. A `vec![0u8; PAGE]` per iteration is only about half a
            // percent of this number, but it is half a percent charged to
            // the baseline and to nothing it is being compared against.
            let mut back = vec![0u8; PAGE];
            let started = Instant::now();
            for _ in 0..OPS {
                // SAFETY: as above.
                unsafe {
                    shipped.sign(HMAC_SHA256, &key, &out, b"\x01\x00\x00\x00", &mut digest);
                    shipped.transform(false, &key, &iv, &out, &mut back);
                }
            }
            started.elapsed()
        });
        report("aes-cbc + hmac-sha256 (openssl)", seal, open, None, None);
        BASELINE.with(|b| b.set((ns(seal), ns(open))));
    }

    let (seal_base, open_base) = BASELINE.with(|b| b.get());

    // ── the same scheme, pure Rust ───────────────────────────────────────
    {
        let mut out = vec![0u8; PAGE];
        let seal = floor_of(ROUNDS, || {
            let started = Instant::now();
            for _ in 0..OPS {
                Aes256CbcEnc::new_from_slices(&key, &iv)
                    .unwrap()
                    .encrypt_padded_b2b_mut::<NoPadding>(&page, &mut out)
                    .unwrap();
                let mut mac = Hmac::<sha2::Sha256>::new_from_slice(&key).unwrap();
                mac.update(&out);
                std::hint::black_box(mac.finalize().into_bytes());
            }
            started.elapsed()
        });
        let open = floor_of(ROUNDS, || {
            let mut back = vec![0u8; PAGE];
            let started = Instant::now();
            for _ in 0..OPS {
                let mut mac = Hmac::<sha2::Sha256>::new_from_slice(&key).unwrap();
                mac.update(&out);
                std::hint::black_box(mac.finalize().into_bytes());
                Aes256CbcDec::new_from_slices(&key, &iv)
                    .unwrap()
                    .decrypt_padded_b2b_mut::<NoPadding>(&out, &mut back)
                    .unwrap();
            }
            started.elapsed()
        });
        report(
            "aes-cbc + hmac-sha256 (rustcrypto)",
            seal,
            open,
            Some(seal_base),
            Some(open_base),
        );
    }

    // ── one pass, pure Rust: ChaCha20-Poly1305 ───────────────────────────
    //
    // What SQLite3 Multiple Ciphers uses by default, so it is a real
    // on-disk format rather than an invention. RustCrypto's ChaCha20 has
    // AVX2 behind it and Poly1305 is cheap, and neither wants an
    // instruction this machine lacks.
    aead_scheme::<chacha20poly1305::ChaCha20Poly1305>(
        "chacha20-poly1305 (rustcrypto)",
        &key,
        &page,
        12,
        seal_base,
        open_base,
    );

    // ── and with the 24-byte nonce Postio already uses for blobs ─────────
    aead_scheme::<chacha20poly1305::XChaCha20Poly1305>(
        "xchacha20-poly1305 (rustcrypto)",
        &key,
        &page,
        24,
        seal_base,
        open_base,
    );

    // ── AES-GCM, which this CPU has both instructions for ────────────────
    aead_scheme::<aes_gcm::Aes256Gcm>(
        "aes-256-gcm (rustcrypto)",
        &key,
        &page,
        12,
        seal_base,
        open_base,
    );

    // ── AES-CBC with BLAKE3 as the MAC instead of HMAC ───────────────────
    //
    // Keeps the cipher and replaces only the half that is slow. BLAKE3 is
    // pure Rust with SIMD, and is already in this workspace's graph.
    {
        let mut out = vec![0u8; PAGE];
        let seal = floor_of(ROUNDS, || {
            let started = Instant::now();
            for _ in 0..OPS {
                Aes256CbcEnc::new_from_slices(&key, &iv)
                    .unwrap()
                    .encrypt_padded_b2b_mut::<NoPadding>(&page, &mut out)
                    .unwrap();
                std::hint::black_box(blake3::keyed_hash(&key, &out));
            }
            started.elapsed()
        });
        let open = floor_of(ROUNDS, || {
            let mut back = vec![0u8; PAGE];
            let started = Instant::now();
            for _ in 0..OPS {
                std::hint::black_box(blake3::keyed_hash(&key, &out));
                Aes256CbcDec::new_from_slices(&key, &iv)
                    .unwrap()
                    .decrypt_padded_b2b_mut::<NoPadding>(&out, &mut back)
                    .unwrap();
            }
            started.elapsed()
        });
        report(
            "aes-cbc + blake3 keyed (rustcrypto)",
            seal,
            open,
            Some(seal_base),
            Some(open_base),
        );
    }
}

thread_local! {
    static BASELINE: std::cell::Cell<(f64, f64)> = const { std::cell::Cell::new((0.0, 0.0)) };
}

/// One AEAD, sealed and opened in place, nonce included in the cost.
fn aead_scheme<A>(label: &str, key: &[u8; 32], page: &[u8], nonce_len: usize, sb: f64, ob: f64)
where
    A: KeyInit + AeadInOut,
{
    let cipher = A::new_from_slice(key).expect("a 256-bit key");
    let nonce = vec![0x33u8; nonce_len];
    let nonce = chacha20poly1305::aead::Nonce::<A>::try_from(&nonce[..]).expect("the nonce size");

    let mut buffer = page.to_vec();
    let tag = cipher
        .encrypt_inout_detached(&nonce, b"", (&mut buffer[..]).into())
        .expect("seal");

    let seal = floor_of(ROUNDS, || {
        let mut scratch = page.to_vec();
        let started = Instant::now();
        for _ in 0..OPS {
            scratch.copy_from_slice(page);
            std::hint::black_box(
                cipher
                    .encrypt_inout_detached(&nonce, b"", (&mut scratch[..]).into())
                    .expect("seal"),
            );
        }
        started.elapsed()
    });
    let open = floor_of(ROUNDS, || {
        let mut scratch = buffer.clone();
        let started = Instant::now();
        for _ in 0..OPS {
            scratch.copy_from_slice(&buffer);
            cipher
                .decrypt_inout_detached(&nonce, b"", (&mut scratch[..]).into(), &tag)
                .expect("open");
        }
        started.elapsed()
    });
    report(label, seal, open, Some(sb), Some(ob));
}

fn ns(took: Duration) -> f64 {
    took.as_secs_f64() * 1e9 / OPS as f64
}

fn report(label: &str, seal: Duration, open: Duration, sb: Option<f64>, ob: Option<f64>) {
    let (s, o) = (ns(seal), ns(open));
    let ratio = |value: f64, base: Option<f64>| match base {
        Some(base) => format!("{:>5.2}x", value / base),
        None => "  1.00x".to_string(),
    };
    println!(
        "{label:<36} seal {s:>8.1} ns {}   open {o:>8.1} ns {}   open {:>6.0} MB/s",
        ratio(s, sb),
        ratio(o, ob),
        PAGE as f64 / o * 1000.0,
    );
}
