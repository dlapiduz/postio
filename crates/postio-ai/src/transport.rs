//! How a connection to the runtime is made: a loopback port or a local socket, nowhere else, within
//! a deadline and a size. `Transport` is the seam a test fills with a fake.

use std::fmt;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use postio_config::{ModelEndpoint, Reach};

pub trait Stream: Read + Write + Send {}

impl<T: Read + Write + Send> Stream for T {}

pub trait Transport: Send + Sync + fmt::Debug {
    /// A connection to `endpoint`, made before `deadline`, whose reads and
    /// writes also stop at it.
    fn connect(&self, endpoint: &ModelEndpoint, deadline: Instant) -> io::Result<Box<dyn Stream>>;
}

/// Nothing is looked up: the endpoint already holds addresses.
#[derive(Debug, Default, Clone, Copy)]
pub struct LocalTransport;

impl Transport for LocalTransport {
    fn connect(&self, endpoint: &ModelEndpoint, deadline: Instant) -> io::Result<Box<dyn Stream>> {
        let left = || {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                Err(io::Error::from(io::ErrorKind::TimedOut))
            } else {
                Ok(left)
            }
        };
        match endpoint.reach() {
            Reach::Loopback(addresses) => {
                let mut last = io::Error::from(io::ErrorKind::AddrNotAvailable);
                for address in addresses {
                    match TcpStream::connect_timeout(address, left()?) {
                        Ok(stream) => {
                            stream.set_read_timeout(Some(left()?))?;
                            stream.set_write_timeout(Some(left()?))?;
                            stream.set_nodelay(true)?;
                            return Ok(Box::new(stream));
                        }
                        Err(error) => last = error,
                    }
                }
                Err(last)
            }
            Reach::Socket(path) => {
                let stream = UnixStream::connect(path)?;
                stream.set_read_timeout(Some(left()?))?;
                stream.set_write_timeout(Some(left()?))?;
                Ok(Box::new(stream))
            }
        }
    }
}

/// A chat completion in a fixed schema is a few kilobytes; a runtime sending more is not answering.
pub(crate) const MAX_RESPONSE: usize = 1024 * 1024;

pub(crate) struct Bounded {
    inner: Box<dyn Stream>,
    deadline: Instant,
    read: usize,
}

impl Bounded {
    pub(crate) fn new(inner: Box<dyn Stream>, deadline: Instant) -> Self {
        Self {
            inner,
            deadline,
            read: 0,
        }
    }
}

impl Read for Bounded {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if Instant::now() > self.deadline {
            return Err(io::Error::from(io::ErrorKind::TimedOut));
        }
        let read = self.inner.read(buf)?;
        self.read += read;
        if self.read > MAX_RESPONSE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the runtime's answer is too large",
            ));
        }
        Ok(read)
    }
}

impl Write for Bounded {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if Instant::now() > self.deadline {
            return Err(io::Error::from(io::ErrorKind::TimedOut));
        }
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// A runtime slower than this is treated as not running, and the built-in answer stands.
pub const TIMEOUT: Duration = Duration::from_secs(30);
