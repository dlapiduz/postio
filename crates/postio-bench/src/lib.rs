//! The workspace's benchmarks live here, and nothing else does.
//!
//! # Why they are not in the crates they measure
//!
//! Cargo compiles a package's whole dev-dependency graph when it tests that
//! package, whichever target was asked for. `criterion` and the ~13 crates
//! behind it (plotters, clap, rayon, ciborium, regex, ...) were therefore
//! built to run a single integration test in `postio-app`, `postio-core`,
//! `postio-gtk`, `postio-index` and `postio-runtime` -- the five crates a
//! session is most likely to be iterating on, and the ones `issue-land.sh`
//! gates most often.
//!
//! Moving the bench targets into a crate of their own takes that cost out of
//! the inner loop entirely: benching still runs `cargo bench --workspace`,
//! which finds these the same way it always did, but testing `postio-core`
//! no longer builds a plotting library first.
//!
//! Nothing else belongs in this file. The benches reach their subjects
//! through ordinary public APIs -- there is no shared harness here to hide
//! behind, and a bench that needs one should say so out loud.
