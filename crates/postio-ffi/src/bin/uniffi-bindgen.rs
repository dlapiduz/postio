//! The bindings generator, built here so it cannot skew from the `uniffi` runtime version.
fn main() {
    uniffi::uniffi_bindgen_main()
}
