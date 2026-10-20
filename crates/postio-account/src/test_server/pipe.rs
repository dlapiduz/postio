//! An in-memory way to reach the server: no socket, so no real I/O.
//!
//! Over loopback, tokio's paused clock misfires, because bytes arrive from
//! the OS while the runtime believes it is idle and jumps ahead. A pipe has
//! no such source: every wake-up is a task the runtime can see, so virtual
//! time advances exactly when everything is waiting on a deadline. That is
//! what lets a stall test assert "gave up after exactly the deadline" and
//! finish in milliseconds.
//!
//! The pipe's capacity is also a dial. A small one fills after a few
//! kilobytes, which is how a client is made to block in a write.

use std::sync::Arc;

use async_trait::async_trait;
use postio_transport::{Connector, Stream, TransportError, map_io};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

use super::Shared;
use super::session::Session;
use super::wire::Conn;

/// Connects a client to a [`TestServer`](super::TestServer) through memory.
///
/// Every connection gets a fresh pipe and a fresh server-side session, so the
/// server counts them the way it counts sockets.
#[derive(Debug)]
pub struct InMemoryConnector {
    shared: Arc<Shared>,
    capacity: usize,
}

impl InMemoryConnector {
    pub(super) fn new(shared: Arc<Shared>, capacity: usize) -> Self {
        Self { shared, capacity }
    }

    fn open(&self) -> Box<dyn Stream> {
        let (client, server) = tokio::io::duplex(self.capacity);
        let shared = Arc::clone(&self.shared);
        let stats = Arc::clone(&shared.stats);
        tokio::spawn(async move {
            let _ = Session::new(Conn::new(server, stats), shared).run().await;
        });
        Box::new(Pipe(client))
    }
}

#[async_trait]
impl Connector for InMemoryConnector {
    async fn connect_tcp(
        &self,
        _host: &str,
        _port: u16,
    ) -> Result<Box<dyn Stream>, TransportError> {
        Ok(self.open())
    }

    async fn connect_tls(
        &self,
        _host: &str,
        _port: u16,
    ) -> Result<Box<dyn Stream>, TransportError> {
        Err(TransportError::Unsupported(
            "the in-memory test server speaks cleartext only".to_owned(),
        ))
    }
}

#[derive(Debug)]
struct Pipe(DuplexStream);

#[async_trait]
impl Stream for Pipe {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        match self.0.read(buf).await {
            Ok(0) => Err(TransportError::Closed),
            Ok(count) => Ok(count),
            Err(error) => Err(map_io("reading from the server", error)),
        }
    }

    async fn write_all(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        self.0
            .write_all(bytes)
            .await
            .map_err(|error| map_io("writing to the server", error))
    }

    async fn upgrade_tls(self: Box<Self>, _host: &str) -> Result<Box<dyn Stream>, TransportError> {
        Err(TransportError::Unsupported(
            "the in-memory test server has no TLS".to_owned(),
        ))
    }

    fn is_encrypted(&self) -> bool {
        false
    }
}
