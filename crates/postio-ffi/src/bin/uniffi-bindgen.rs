//! The bindings generator, built from this workspace rather than installed.
//!
//! `uniffi-bindgen` as a separately installed tool is a version that can skew
//! from the `uniffi` crate the scaffolding was generated with, and the failure
//! that produces is a Swift file that compiles against nothing. Building it
//! here means the generator and the runtime are the same version by
//! construction.
fn main() {
    uniffi::uniffi_bindgen_main()
}
