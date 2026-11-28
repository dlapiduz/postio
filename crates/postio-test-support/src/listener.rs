//! A loopback listener that counts connections, for egress tests. A test asserting zero connections
//! must first prove the listener would have seen one ([`Listener::control`]) (#1336).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const PIXEL: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae,
    0x42, 0x60, 0x82,
];

pub struct Listener {
    port: u16,
    accepted: Arc<AtomicUsize>,
    paths: Arc<std::sync::Mutex<Vec<String>>>,
}

impl Listener {
    pub fn start() -> Listener {
        let socket = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
        let port = socket.local_addr().expect("its address").port();
        let accepted = Arc::new(AtomicUsize::new(0));
        let paths = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (count, seen) = (accepted.clone(), paths.clone());
        std::thread::spawn(move || {
            for stream in socket.incoming().flatten() {
                count.fetch_add(1, Ordering::SeqCst);
                let seen = seen.clone();
                std::thread::spawn(move || answer(stream, &seen));
            }
        });
        Listener {
            port,
            accepted,
            paths,
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn count(&self) -> usize {
        self.accepted.load(Ordering::SeqCst)
    }

    pub fn paths(&self) -> Vec<String> {
        self.paths.lock().map(|p| p.clone()).unwrap_or_default()
    }

    /// Panics if it is not seen, because every zero measured after it would mean nothing.
    pub fn control(&self) {
        let before = self.count();
        let _ = TcpStream::connect(("127.0.0.1", self.port)).expect("the control connection");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while self.count() == before {
            assert!(
                std::time::Instant::now() < deadline,
                "the listener never saw its control"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    pub fn rewrite(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        loop {
            let at = ["http://", "https://"]
                .iter()
                .filter_map(|scheme| rest.find(scheme).map(|i| (i, scheme.len())))
                .min_by_key(|(i, _)| *i);
            let Some((start, scheme)) = at else {
                out.push_str(rest);
                return out;
            };
            out.push_str(&rest[..start]);
            let url = &rest[start + scheme..];
            let host_end = url
                .find(['/', '"', '\'', ')', ' ', '>'])
                .unwrap_or(url.len());
            out.push_str(&format!("http://127.0.0.1:{}", self.port));
            rest = &url[host_end..];
        }
    }
}

fn answer(mut stream: TcpStream, seen: &std::sync::Mutex<Vec<String>>) {
    let mut buffer = [0u8; 4096];
    let read = stream.read(&mut buffer).unwrap_or(0);
    let request = String::from_utf8_lossy(&buffer[..read]);
    if let Some(path) = request.split_whitespace().nth(1)
        && let Ok(mut seen) = seen.lock()
    {
        seen.push(path.to_owned());
    }
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        PIXEL.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(PIXEL);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_control_is_counted_and_urls_point_home() {
        let listener = Listener::start();
        assert_eq!(listener.count(), 0);
        listener.control();
        assert_eq!(listener.count(), 1);
        let rewritten = listener
            .rewrite(r#"<img src="https://beacon.example.com/x.png"> url(http://a.example.org/b)"#);
        let home = format!("http://127.0.0.1:{}", listener.port());
        assert_eq!(
            rewritten,
            format!(r#"<img src="{home}/x.png"> url({home}/b)"#)
        );
    }
}
