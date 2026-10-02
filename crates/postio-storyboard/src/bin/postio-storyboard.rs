//! The pure half's command line, which `scripts/storyboards.sh` drives.
//! Subcommands are in `specs/008-storyboards/contracts/runner.md`.

fn main() -> std::process::ExitCode {
    eprintln!("usage: postio-storyboard <lint|select|compare|parity|bundle|prompt|verdicts|page|key> ...");
    std::process::ExitCode::from(2)
}
