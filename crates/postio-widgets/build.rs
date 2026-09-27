//! Build step for `postio-widgets`: compile `data/widgets.gresource.xml`
//! into the GResource bundle that carries the shared widgets' stylesheet, so
//! both desktop apps resolve it from the binary, with nothing read from disk
//! at run time and nothing fetched.

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=data/widgets.gresource.xml");
    println!("cargo:rerun-if-changed=data/widgets.css");
    println!("cargo:rerun-if-changed=data/metrics.css");

    let data_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("data");
    glib_build_tools::compile_resources(
        &[data_dir.to_str().expect("data dir path is not UTF-8")],
        data_dir
            .join("widgets.gresource.xml")
            .to_str()
            .expect("gresource path is not UTF-8"),
        "postio-widgets.gresource",
    );
}
