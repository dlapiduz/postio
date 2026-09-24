//! Facts about a host name that every protocol adapter asks.

/// Whether `host` names this machine -- the one case a connection may go in
/// the clear, because nothing crosses a network.
pub fn is_loopback(host: &str) -> bool {
    let host = host.trim().trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::is_loopback;

    #[test]
    fn this_machine_by_any_of_its_names() {
        for host in [
            "localhost",
            "LOCALHOST",
            "127.0.0.1",
            "127.8.9.1",
            "::1",
            "[::1]",
            " localhost ",
        ] {
            assert!(is_loopback(host), "{host:?} is this machine");
        }
    }

    #[test]
    fn a_host_name_that_only_looks_like_loopback_is_not() {
        // The prefix test this replaced let `127.example.com` -- a name that
        // resolves wherever its owner likes -- through as this machine, and
        // so through the one gate that allows a plaintext connection.
        for host in [
            "127.example.com",
            "localhost.example.com",
            "10.0.0.1",
            "::2",
            "example.com",
        ] {
            assert!(!is_loopback(host), "{host:?} is not this machine");
        }
    }
}
