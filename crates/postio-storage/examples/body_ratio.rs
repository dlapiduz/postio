//! Measure what body compression actually achieves on this project's corpus.
//!
//! ADR 0020 plans against 1.57x per-value and 2.19x with a trained dictionary,
//! both taken from real mail. This reports what the 38-message `.eml` corpus
//! in `postio-model` gives, which is a *different and much smaller* thing:
//!
//! **A corpus figure, not a forecast.** Thirty-eight fixtures chosen to cover
//! parser edge cases are not a mailbox, and they are far more self-similar
//! than real mail — which is exactly the trap ADR 0020 records. A scratch
//! benchmark over generated mail reaches 6-7x and means nothing. Do not put a
//! number from this into an ADR or a PR body as a prediction.
//!
//! ```text
//! cargo run -p postio-storage --example body_ratio --features test-support
//! ```

fn main() {
    let bodies: Vec<String> = postio_model::test_corpus::all()
        .iter()
        .flat_map(|fixture| {
            let parsed = postio_model::mime::parse(fixture.bytes());
            [parsed.body.text.clone(), parsed.body.html.clone()]
        })
        .flatten()
        .filter(|body| !body.is_empty())
        .collect();

    let plain: usize = bodies.iter().map(String::len).sum();
    let per_value: usize = bodies
        .iter()
        .map(|body| {
            zstd::bulk::compress(body.as_bytes(), 3)
                .expect("compress")
                .len()
        })
        .sum();

    let dictionary = zstd::dict::from_samples(&bodies, 110 * 1024).expect("train a dictionary");
    let with_dictionary: usize = bodies
        .iter()
        .map(|body| {
            zstd::bulk::Compressor::with_dictionary(3, &dictionary)
                .expect("a compressor")
                .compress(body.as_bytes())
                .expect("compress")
                .len()
        })
        .sum();

    println!("bodies:                {}", bodies.len());
    println!("plain:                 {plain} B");
    println!(
        "per-value zstd:        {per_value} B  ({:.2}x)",
        plain as f64 / per_value as f64
    );
    println!(
        "with a dictionary:     {with_dictionary} B  ({:.2}x)",
        plain as f64 / with_dictionary as f64
    );
    println!("dictionary itself:     {} B", dictionary.len());
    println!(
        "with the dictionary counted once: {:.2}x",
        plain as f64 / (with_dictionary + dictionary.len()) as f64
    );
    println!();
    println!(
        "The per-value figure is the one to believe. The dictionary column is\n\
         trained on the same 40 bodies it then compresses, so it is measuring\n\
         memorisation; and at this size the dictionary outweighs the corpus, so\n\
         counting it once turns the whole exercise into a net loss. That is what\n\
         `body::should_train` refusing a corpus under 32 samples and 64 KiB is\n\
         for. ADR 0020's 2.19x ceiling comes from real mail, not from here."
    );
}
