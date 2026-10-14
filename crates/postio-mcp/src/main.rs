//! `postio-mcp`: Postio over MCP, on stdin and stdout (ADR 0010).
//!
//! An MCP client launches it; it answers until its stdin closes. stdout is
//! the protocol and nothing else writes to it: the log goes to stderr and
//! the journal.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use postio_mcp::{Audit, Installed, Server, serve};

const USAGE: &str = "\
postio-mcp: Postio's mail, for an AI agent, over the Model Context Protocol.

An MCP client starts it and speaks JSON-RPC on its stdin and stdout; it
listens on no socket. It reads the accounts and folders config.toml's
[mcp] section shares, and nothing else. It can save drafts, and archive,
move and label where [mcp] allows it, but never send or delete mail.

Usage: postio-mcp [--config <file>] [--audit <file>]

  --config <file>  the config.toml to read [mcp] from
                   (default: $POSTIO_CONFIG, else Postio's own)
  --audit <file>   where to log each tool call, by id
                   (default: mcp-access.log beside Postio's store)
  --version        say which Postio this is
";

#[cfg(feature = "demo")]
const DEMO_USAGE: &str = "  --demo <seed>    serve one of Focus's demo stores, in memory
                   (small, empty, long-thread, two-accounts, ...)
";

struct Options {
    config: Option<PathBuf>,
    audit: Option<PathBuf>,
    demo: Option<String>,
}

fn options() -> Result<Option<Options>, String> {
    let mut options = Options {
        config: None,
        audit: None,
        demo: None,
    };
    let mut arguments = std::env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        let mut value = |name: &str| {
            arguments
                .next()
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match argument.to_str() {
            Some("--config") => options.config = Some(value("--config")?.into()),
            Some("--audit") => options.audit = Some(value("--audit")?.into()),
            Some("--demo") if cfg!(feature = "demo") => {
                options.demo = Some(value("--demo")?.to_string_lossy().into_owned());
            }
            Some("--help" | "-h") => {
                print!("{USAGE}");
                #[cfg(feature = "demo")]
                print!("{DEMO_USAGE}");
                return Ok(None);
            }
            Some("--version" | "-V") => {
                println!("postio-mcp {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            _ => return Err(format!("unknown argument {}", argument.to_string_lossy())),
        }
    }
    Ok(Some(options))
}

fn main() -> ExitCode {
    let options = match options() {
        Ok(Some(options)) => options,
        Ok(None) => return ExitCode::SUCCESS,
        Err(problem) => {
            eprintln!("postio-mcp: {problem}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let config = options
        .config
        .or_else(|| postio_config::paths::config_path().ok());
    let _logging = postio_session::logging::init(
        &config
            .as_deref()
            .map(postio_session::logging::config_at)
            .unwrap_or_default(),
    );
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "postio-mcp starting");

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let served = match options.demo {
        #[cfg(feature = "demo")]
        Some(seed) => {
            let (host, _blobs) = match demo::start(&seed) {
                Ok(started) => started,
                Err(problem) => {
                    eprintln!("postio-mcp: {problem}");
                    return ExitCode::FAILURE;
                }
            };
            let audit = options.audit.map_or_else(Audit::none, Audit::to_file);
            let mut server = Server::new(postio_mcp::Held::new(host), config, audit);
            serve(stdin.lock(), stdout.lock(), &mut server)
        }
        #[cfg(not(feature = "demo"))]
        Some(_) => unreachable!("--demo is refused without the demo feature"),
        None => {
            let audit = options.audit.map_or_else(Audit::installed, Audit::to_file);
            let backend = Installed::new(
                config.clone(),
                Arc::new(postio_account::secret::KeyringSecretStore::default()),
            );
            let mut server = Server::new(backend, config, audit);
            serve(stdin.lock(), stdout.lock(), &mut server)
        }
    };
    match served {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "stdin or stdout failed: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(feature = "demo")]
mod demo {
    use std::sync::Arc;

    use postio_host::Host;

    /// A host over the demo store `seed` names, and the folder its blobs
    /// are in, kept for as long as it serves.
    pub fn start(seed: &str) -> Result<(Arc<Host>, tempfile::TempDir), String> {
        let seed = postio_demo::Seed::from_id(seed)
            .ok_or_else(|| format!("there is no demo store called {seed}"))?;
        let (database, _account) = postio_core::blocking::now(postio_demo::seeded(seed));
        let blobs_dir = tempfile::tempdir().map_err(|error| format!("no scratch: {error}"))?;
        let blobs = postio_storage::BlobStore::open(
            blobs_dir.path().to_path_buf(),
            &postio_storage::test_support::blob_keys(),
        )
        .map_err(|error| format!("no blob store: {error}"))?;
        let host = Host::start(database, blobs, |wiring| {
            wiring.with_secrets(Arc::new(
                postio_account::secret::MemorySecretStore::default(),
            ))
        })?;
        Ok((Arc::new(host), blobs_dir))
    }
}
