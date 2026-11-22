//! A model endpoint on this computer and nothing else: `ModelEndpoint::parse` is the one reading of
//! the
//! address and refuses anything else with a sentence, so a client cannot be pointed at another
//! host.
//! Nothing is looked up: `localhost` is read as the loopback addresses, since a resolver could
//! answer with another machine.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Extras;

/// Absent means off: with no section there is nothing to connect to and Postio looks for no
/// runtime. A
/// section that cannot be used (no endpoint, no model, or an endpoint on another computer) is
/// reported
/// by validation and used by no feature, and the file still loads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FocusModel {
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub model: String,
    #[serde(default = "crate::yes")]
    pub needs_action: bool,
    #[serde(default = "crate::yes")]
    pub digest_summary: bool,
    #[serde(default = "crate::yes")]
    pub like_this: bool,
    #[serde(flatten)]
    pub extras: Extras,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelFeature {
    NeedsAction,
    DigestSummary,
    LikeThis,
}

impl ModelFeature {
    pub const ALL: [ModelFeature; 3] = [
        ModelFeature::NeedsAction,
        ModelFeature::DigestSummary,
        ModelFeature::LikeThis,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelProblem {
    NoEndpoint,
    NoModel,
    Refused(EndpointRefused),
}

impl FocusModel {
    pub fn switched_on(&self, feature: ModelFeature) -> bool {
        match feature {
            ModelFeature::NeedsAction => self.needs_action,
            ModelFeature::DigestSummary => self.digest_summary,
            ModelFeature::LikeThis => self.like_this,
        }
    }

    pub fn usable(&self) -> Result<(ModelEndpoint, &str), ModelProblem> {
        if self.endpoint.trim().is_empty() {
            return Err(ModelProblem::NoEndpoint);
        }
        let endpoint = ModelEndpoint::parse(&self.endpoint).map_err(ModelProblem::Refused)?;
        let model = self.model.trim();
        if model.is_empty() {
            return Err(ModelProblem::NoModel);
        }
        Ok((endpoint, model))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelEndpoint {
    target: Target,
    /// The API path such as `/v1`, with no trailing slash.
    base: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    Loopback {
        addresses: Vec<SocketAddr>,
        authority: String,
    },
    Socket(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach<'a> {
    Loopback(&'a [SocketAddr]),
    Socket(&'a Path),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointRefused {
    why: String,
}

impl EndpointRefused {
    fn new(why: impl Into<String>) -> Self {
        Self { why: why.into() }
    }
}

impl fmt::Display for EndpointRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the model must run on this computer: {}", self.why)
    }
}

impl std::error::Error for EndpointRefused {}

const WRITE_IT_AS: &str = "write it as http://127.0.0.1:<port>/v1, http://[::1]:<port>/v1, \
                           http://localhost:<port>/v1 or unix:<absolute socket path>";

impl ModelEndpoint {
    /// `http://` to a loopback address (`127.0.0.0/8`, `::1`) or `localhost`, with a port and a
    /// base path
    /// such as `/v1`; or `unix:` and an absolute socket path. Anything else is refused, with the
    /// reason.
    pub fn parse(text: &str) -> Result<Self, EndpointRefused> {
        let text = text.trim();
        if let Some(path) = text.strip_prefix("unix:") {
            let path = Path::new(path);
            if !path.is_absolute() {
                return Err(EndpointRefused::new(format!(
                    "a local socket is named by its absolute path; {WRITE_IT_AS}"
                )));
            }
            return Ok(Self {
                target: Target::Socket(path.to_path_buf()),
                base: "/v1".to_owned(),
            });
        }
        let url = url::Url::parse(text).map_err(|_| {
            EndpointRefused::new(format!("`{text}` is not an address; {WRITE_IT_AS}"))
        })?;
        if url.scheme() != "http" {
            return Err(EndpointRefused::new(format!(
                "Postio speaks to a model on this computer over plain http:// or a local \
                 socket, and `{}:` is for reaching other machines",
                url.scheme()
            )));
        }
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(EndpointRefused::new(format!(
                "an endpoint is a base address with no user, query or fragment; {WRITE_IT_AS}"
            )));
        }
        let port = url.port().unwrap_or(80);
        let addresses: Vec<IpAddr> = match url.host() {
            Some(url::Host::Ipv4(address)) if address.is_loopback() => vec![address.into()],
            Some(url::Host::Ipv6(address)) if address.is_loopback() => vec![address.into()],
            // Read as the loopback addresses themselves: nothing is looked up.
            Some(url::Host::Domain(name)) if name.eq_ignore_ascii_case("localhost") => {
                vec![Ipv4Addr::LOCALHOST.into(), Ipv6Addr::LOCALHOST.into()]
            }
            Some(host) => {
                return Err(EndpointRefused::new(format!(
                    "`{host}` is not this computer; use a loopback address (127.0.0.1 or \
                     [::1]), localhost, or a local socket"
                )));
            }
            None => {
                return Err(EndpointRefused::new(format!(
                    "`{text}` names no host; {WRITE_IT_AS}"
                )));
            }
        };
        let authority = match url.host() {
            Some(host) => match url.port() {
                Some(port) => format!("{host}:{port}"),
                None => host.to_string(),
            },
            None => unreachable!("a host was required above"),
        };
        Ok(Self {
            target: Target::Loopback {
                addresses: addresses
                    .into_iter()
                    .map(|address| SocketAddr::new(address, port))
                    .collect(),
                authority,
            },
            base: url.path().trim_end_matches('/').to_owned(),
        })
    }

    pub fn reach(&self) -> Reach<'_> {
        match &self.target {
            Target::Loopback { addresses, .. } => Reach::Loopback(addresses),
            Target::Socket(path) => Reach::Socket(path),
        }
    }

    /// `/v1` and `/models` make `/v1/models`.
    pub fn path(&self, route: &str) -> String {
        format!("{}/{}", self.base, route.trim_start_matches('/'))
    }

    pub fn authority(&self) -> &str {
        match &self.target {
            Target::Loopback { authority, .. } => authority,
            Target::Socket(_) => "localhost",
        }
    }

    /// The socket's path and no port, over a socket.
    pub fn log_target(&self) -> (String, u16) {
        match &self.target {
            Target::Loopback { addresses, .. } => addresses
                .first()
                .map(|address| (address.ip().to_string(), address.port()))
                .unwrap_or_default(),
            Target::Socket(path) => (format!("unix:{}", path.display()), 0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loopback(endpoint: &ModelEndpoint) -> Vec<SocketAddr> {
        match endpoint.reach() {
            Reach::Loopback(addresses) => addresses.to_vec(),
            Reach::Socket(path) => panic!("a socket, {path:?}"),
        }
    }

    #[test]
    fn a_loopback_address_is_this_computer() {
        let v4 = ModelEndpoint::parse("http://127.0.0.1:11434/v1").expect("loopback");
        assert_eq!(loopback(&v4), vec!["127.0.0.1:11434".parse().unwrap()]);
        assert_eq!(v4.path("models"), "/v1/models");
        assert_eq!(v4.authority(), "127.0.0.1:11434");

        let v6 = ModelEndpoint::parse("http://[::1]:8080/v1/").expect("loopback");
        assert_eq!(loopback(&v6), vec!["[::1]:8080".parse().unwrap()]);
        assert_eq!(v6.path("/models"), "/v1/models");

        assert!(ModelEndpoint::parse("http://127.1.2.3:9000/v1").is_ok());
    }

    #[test]
    fn localhost_is_read_as_the_loopback_addresses_and_never_looked_up() {
        let endpoint = ModelEndpoint::parse("http://LOCALHOST:11434/v1").expect("localhost");
        assert_eq!(
            loopback(&endpoint),
            vec![
                "127.0.0.1:11434".parse().unwrap(),
                "[::1]:11434".parse().unwrap()
            ]
        );
        assert_eq!(endpoint.authority(), "localhost:11434");
    }

    #[test]
    fn a_local_socket_is_named_by_its_absolute_path() {
        let endpoint = ModelEndpoint::parse("unix:/run/user/1000/model.sock").expect("a socket");
        assert_eq!(
            endpoint.reach(),
            Reach::Socket(Path::new("/run/user/1000/model.sock"))
        );
        assert_eq!(endpoint.path("models"), "/v1/models");
        assert!(ModelEndpoint::parse("unix:model.sock").is_err());
    }

    #[test]
    fn another_host_is_refused_with_a_sentence_saying_why() {
        for text in [
            "http://192.0.2.10:11434/v1",
            "http://model.example.com/v1",
            "http://0.0.0.0:11434/v1",
            "http://[2001:db8::1]:11434/v1",
            "http://localhost.example.com:11434/v1",
        ] {
            let refused = ModelEndpoint::parse(text).expect_err(text).to_string();
            assert!(
                refused.starts_with("the model must run on this computer: "),
                "{refused}"
            );
            assert!(refused.contains("is not this computer"), "{refused}");
        }
    }

    #[test]
    fn a_scheme_for_other_machines_and_what_is_not_an_address_are_refused() {
        let https = ModelEndpoint::parse("https://127.0.0.1:11434/v1").expect_err("https");
        assert!(https.to_string().contains("`https:`"), "{https}");
        for text in [
            "",
            "127.0.0.1:11434",
            "http://user:pw@127.0.0.1:11434/v1",
            "http://127.0.0.1:11434/v1?key=x",
            "ftp://127.0.0.1/v1",
        ] {
            let refused = ModelEndpoint::parse(text).expect_err(text).to_string();
            assert!(
                refused.starts_with("the model must run on this computer: "),
                "{text:?}: {refused}"
            );
        }
    }

    #[test]
    fn the_egress_log_is_told_where_the_connection_went() {
        let endpoint = ModelEndpoint::parse("http://localhost:11434/v1").expect("localhost");
        assert_eq!(endpoint.log_target(), ("127.0.0.1".to_owned(), 11434));
        let socket = ModelEndpoint::parse("unix:/tmp/model.sock").expect("a socket");
        assert_eq!(socket.log_target(), ("unix:/tmp/model.sock".to_owned(), 0));
    }
}
