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

    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let data_dir = manifest.join("data");
    // The app icon is postio-gtk's file, not a copy of it.
    let icons_dir = manifest.join("../postio-gtk/data/icons");
    for name in ["dev.postio.Postio.svg", "dev.postio.Postio-symbolic.svg"] {
        println!(
            "cargo:rerun-if-changed={}",
            icons_dir.join("scalable/apps").join(name).display()
        );
    }
    glib_build_tools::compile_resources(
        &[
            data_dir.to_str().expect("data dir path is not UTF-8"),
            icons_dir.to_str().expect("icons dir path is not UTF-8"),
        ],
        data_dir
            .join("widgets.gresource.xml")
            .to_str()
            .expect("gresource path is not UTF-8"),
        "postio-widgets.gresource",
    );
}
