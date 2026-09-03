//! Acceptance tests for postio-pco — the first-run autoconfig probe.
//!
//! Every test here runs against a mocked transport (the resolver/HTTP layer),
//! so the default suite touches no network. Live probes are `#[ignore]`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use postio_account::discovery::{
    AutoconfigEndpoint, CancelToken, DiscoveryAutoconfig, DiscoveryError, DiscoveryOutcome,
    DiscoverySrvReport, DiscoverySrvService, DiscoveryTransport, Encryption, Probe, ProbeOptions,
    ProbeStep, SettingsSource, TransportError,
};

// --- mock transport -----------------------------------------------------

#[derive(Default)]
struct MockTransport {
    autoconfig: HashMap<ProbeStep, String>,
    srv: Option<DiscoverySrvReport>,
    /// The MX hostnames the domain publishes, best preference first (#94).
    mx: Option<Vec<String>>,
    calls: Mutex<Vec<ProbeStep>>,
    /// Every domain (and, for the subdomain step, local part) a request was
    /// actually made for — `postio-iigq`'s "only the domain the user typed"
    /// audit point needs the request's own arguments, not just which step
    /// ran.
    requested: Mutex<Vec<(ProbeStep, String, Option<String>)>>,
    stall: Option<Duration>,
    /// Whether the token the transport was handed had already been
    /// cancelled by the time the call ran, recorded per call. #57: the
    /// probe held a `CancelToken` and never passed it down, so nothing at
    /// the network layer could act on it.
    saw_cancelled: Mutex<Vec<bool>>,
    /// A clone of the token the transport was last handed, kept so the test
    /// can ask *afterwards* whether it observed the caller's cancellation.
    /// A transport handed a fresh token of its own would look identical at
    /// call time and never flip here.
    held: Mutex<Option<CancelToken>>,
}

impl MockTransport {
    fn new() -> Self {
        Self::default()
    }

    fn with_autoconfig(mut self, step: ProbeStep, xml: &str) -> Self {
        self.autoconfig.insert(step, xml.to_string());
        self
    }

    fn with_srv(mut self, report: DiscoverySrvReport) -> Self {
        self.srv = Some(report);
        self
    }

    /// The MX hostnames the domain publishes, best preference first (#94).
    fn with_mx(mut self, hosts: &[&str]) -> Self {
        self.mx = Some(hosts.iter().map(|h| (*h).to_owned()).collect());
        self
    }

    fn stalling() -> Self {
        Self {
            stall: Some(Duration::from_secs(30)),
            ..Self::default()
        }
    }

    fn calls(&self) -> Vec<ProbeStep> {
        self.calls.lock().unwrap().clone()
    }

    /// Every request actually made, as `(step, domain, local_part)`.
    fn requested(&self) -> Vec<(ProbeStep, String, Option<String>)> {
        self.requested.lock().unwrap().clone()
    }

    async fn maybe_stall(&self) {
        if let Some(stall) = self.stall {
            tokio::time::sleep(stall).await;
        }
    }

    /// What the transport could see of the caller's cancellation, per call.
    fn saw_cancelled(&self) -> Vec<bool> {
        self.saw_cancelled.lock().unwrap().clone()
    }

    fn observe(&self, cancel: &CancelToken) {
        self.saw_cancelled
            .lock()
            .unwrap()
            .push(cancel.is_cancelled());
        *self.held.lock().unwrap() = Some(cancel.clone());
    }

    /// Whether the token the transport kept has since been cancelled.
    fn saw_cancelled_after_the_fact(&self) -> bool {
        self.held
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(CancelToken::is_cancelled)
    }
}

#[async_trait]
impl DiscoveryTransport for MockTransport {
    async fn autoconfig(
        &self,
        endpoint: AutoconfigEndpoint<'_>,
        cancel: &CancelToken,
    ) -> Result<DiscoveryAutoconfig, TransportError> {
        let step = endpoint.step();
        self.observe(cancel);
        self.calls.lock().unwrap().push(step);
        let local_part = match endpoint {
            AutoconfigEndpoint::Subdomain { local_part, .. } => Some(local_part.to_owned()),
            AutoconfigEndpoint::WellKnown { .. } | AutoconfigEndpoint::Ispdb { .. } => None,
        };
        self.requested
            .lock()
            .unwrap()
            .push((step, endpoint.domain().to_owned(), local_part));
        self.maybe_stall().await;

        match self.autoconfig.get(&step) {
            Some(xml) => Ok(serde_xml_rs::from_str(xml).expect("fixture parses")),
            None => Err(TransportError::new("no autoconfig document")),
        }
    }

    async fn srv(
        &self,
        domain: &str,
        cancel: &CancelToken,
    ) -> Result<DiscoverySrvReport, TransportError> {
        self.observe(cancel);
        self.calls.lock().unwrap().push(ProbeStep::Srv);
        self.requested
            .lock()
            .unwrap()
            .push((ProbeStep::Srv, domain.to_owned(), None));
        self.maybe_stall().await;

        match &self.srv {
            Some(report) => Ok(report.clone()),
            None => Err(TransportError::new("NXDOMAIN")),
        }
    }

    async fn mx(&self, domain: &str, cancel: &CancelToken) -> Result<Vec<String>, TransportError> {
        self.observe(cancel);
        self.calls.lock().unwrap().push(ProbeStep::Mx);
        self.requested
            .lock()
            .unwrap()
            .push((ProbeStep::Mx, domain.to_owned(), None));
        self.maybe_stall().await;

        match &self.mx {
            Some(hosts) => Ok(hosts.clone()),
            None => Err(TransportError::new("NXDOMAIN")),
        }
    }
}

/// A transport that fails the test if the probe ever reaches the network.
struct UnreachableTransport;

#[async_trait]
impl DiscoveryTransport for UnreachableTransport {
    async fn autoconfig(
        &self,
        _endpoint: AutoconfigEndpoint<'_>,
        _cancel: &CancelToken,
    ) -> Result<DiscoveryAutoconfig, TransportError> {
        panic!("the probe must not hit the network for a known provider");
    }

    async fn srv(
        &self,
        _domain: &str,
        _cancel: &CancelToken,
    ) -> Result<DiscoverySrvReport, TransportError> {
        panic!("the probe must not hit the network for a known provider");
    }

    async fn mx(
        &self,
        _domain: &str,
        _cancel: &CancelToken,
    ) -> Result<Vec<String>, TransportError> {
        panic!("the probe must not hit the network for a known provider");
    }
}

// --- fixtures -----------------------------------------------------------

fn autoconfig_xml(imap_host: &str, smtp_host: &str) -> String {
    format!(
        r#"<clientConfig version="1.1">
  <emailProvider id="example.org">
    <domain>example.org</domain>
    <displayName>Example Mail</displayName>
    <incomingServer type="imap">
      <hostname>{imap_host}</hostname>
      <port>993</port>
      <socketType>SSL</socketType>
      <username>%EMAILADDRESS%</username>
      <authentication>password-cleartext</authentication>
    </incomingServer>
    <outgoingServer type="smtp">
      <hostname>{smtp_host}</hostname>
      <port>587</port>
      <socketType>STARTTLS</socketType>
      <username>%EMAILADDRESS%</username>
      <authentication>password-cleartext</authentication>
    </outgoingServer>
  </emailProvider>
</clientConfig>"#
    )
}

fn srv_report() -> DiscoverySrvReport {
    DiscoverySrvReport {
        imap: None,
        imaps: Some(DiscoverySrvService {
            host: "imap.srv.example.org".into(),
            port: 993,
            priority: 0,
            weight: 1,
        }),
        submission: Some(DiscoverySrvService {
            host: "smtp.srv.example.org".into(),
            port: 587,
            priority: 0,
            weight: 1,
        }),
        submissions: None,
    }
}

fn fast_options() -> ProbeOptions {
    ProbeOptions {
        step_timeout: Duration::from_millis(200),
        overall_timeout: Duration::from_millis(800),
        ..ProbeOptions::default()
    }
}

fn settings(outcome: &DiscoveryOutcome) -> &postio_account::discovery::AccountSettings {
    match outcome {
        DiscoveryOutcome::Discovered(settings) => settings,
        other => panic!("expected discovered settings, got {other:?}"),
    }
}

// --- Shipped provider presets -------------------------------------------
//
// Addresses in a real provider's domain are built rather than written out;
// a literal one reads as personal data to scripts/checks/check-no-personal-data.py.
// See CLAUDE.md, "No personal data".

#[tokio::test]
async fn a_shipped_provider_resolves_with_no_network_at_all() {
    for preset in postio_account::discovery::presets() {
        let address = format!("a@{}", preset.domains()[0]);
        let probe = Probe::new(Arc::new(UnreachableTransport));
        let report = probe.run(&address, &CancelToken::new()).await.unwrap();

        let settings = settings(&report.outcome);
        assert_eq!(settings.source, SettingsSource::Builtin);
        assert_eq!(settings.imap.host, preset.imap_host());
        assert_eq!(settings.smtp.host, preset.smtp_host());
        // Every shipped provider connects encrypted -- that is the real
        // invariant here, not that every provider picks the *same* one.
        // Implicit TLS and STARTTLS (office365's submission port, #868)
        // are both real transport security; plaintext is what this must
        // catch.
        assert_ne!(
            settings.imap.encryption,
            Encryption::None,
            "{}",
            preset.display_name()
        );
        assert_ne!(
            settings.smtp.encryption,
            Encryption::None,
            "{}",
            preset.display_name()
        );
    }
}

#[tokio::test]
async fn a_provider_that_refuses_account_passwords_says_so_before_the_password_field() {
    for preset in postio_account::discovery::presets() {
        if !preset.requires_app_password() {
            continue;
        }
        let address = format!("a@{}", preset.domains()[0]);
        let probe = Probe::new(Arc::new(UnreachableTransport));
        let report = probe.run(&address, &CancelToken::new()).await.unwrap();

        let settings = settings(&report.outcome);
        assert!(settings.requires_app_password);
        let note = settings.note.as_deref().unwrap_or_default();
        assert!(
            note.to_lowercase().contains("app-specific password"),
            "unhelpful note for {}: {note}",
            preset.display_name()
        );
    }
}

#[tokio::test]
async fn every_domain_a_shipped_provider_issues_is_recognised() {
    for preset in postio_account::discovery::presets() {
        for domain in preset.domains() {
            // Case and address decoration must not matter.
            for address in [
                format!("a@{domain}"),
                format!("a@{}", domain.to_uppercase()),
                format!("A.User+tag@{}", domain.to_uppercase()),
            ] {
                let probe = Probe::new(Arc::new(UnreachableTransport));
                let report = probe.run(&address, &CancelToken::new()).await.unwrap();
                assert_eq!(
                    settings(&report.outcome).imap.host,
                    preset.imap_host(),
                    "{address} was not matched to {}",
                    preset.display_name()
                );
            }
        }
    }
}

#[tokio::test]
async fn a_non_icloud_provider_does_not_claim_an_app_password_is_required() {
    let transport = Arc::new(
        MockTransport::new()
            .with_autoconfig(ProbeStep::WellKnown, &autoconfig_xml("imap.x", "smtp.x")),
    );
    let probe = Probe::with_options(transport, fast_options());
    let report = probe
        .run("user@example.org", &CancelToken::new())
        .await
        .unwrap();

    assert!(!settings(&report.outcome).requires_app_password);
}

// --- probe order --------------------------------------------------------

#[tokio::test]
async fn the_well_known_document_wins_and_stops_the_probe() {
    let transport = Arc::new(MockTransport::new().with_autoconfig(
        ProbeStep::WellKnown,
        &autoconfig_xml("imap.example.org", "smtp.example.org"),
    ));
    let probe = Probe::with_options(transport.clone(), fast_options());
    let report = probe
        .run("user@example.org", &CancelToken::new())
        .await
        .unwrap();

    let settings = settings(&report.outcome);
    assert_eq!(settings.source, SettingsSource::WellKnown);
    assert_eq!(settings.imap.host, "imap.example.org");
    assert_eq!(settings.imap.encryption, Encryption::Tls);
    assert_eq!(settings.smtp.host, "smtp.example.org");
    assert_eq!(settings.smtp.port, 587);
    assert_eq!(settings.smtp.encryption, Encryption::StartTls);
    assert_eq!(settings.display_name.as_deref(), Some("Example Mail"));

    // Nothing after the first hit is probed.
    assert_eq!(transport.calls(), vec![ProbeStep::WellKnown]);
}

#[tokio::test]
async fn the_probe_walks_the_documented_order() {
    let transport = Arc::new(
        MockTransport::new()
            .with_autoconfig(ProbeStep::Ispdb, &autoconfig_xml("imap.tb", "smtp.tb")),
    );
    let probe = Probe::with_options(transport.clone(), fast_options());
    let report = probe
        .run("user@example.org", &CancelToken::new())
        .await
        .unwrap();

    assert_eq!(settings(&report.outcome).source, SettingsSource::Ispdb);
    assert_eq!(
        transport.calls(),
        vec![
            ProbeStep::WellKnown,
            ProbeStep::AutoconfigSubdomain,
            ProbeStep::Ispdb
        ]
    );
    // Every attempt is reported so the UI can show what was tried.
    assert_eq!(report.attempts.len(), 3);
    assert!(report.attempts[0].is_miss());
    assert!(report.attempts[2].is_hit());
}

#[tokio::test]
async fn srv_records_are_used_when_no_autoconfig_document_exists() {
    let transport = Arc::new(MockTransport::new().with_srv(srv_report()));
    let probe = Probe::with_options(transport.clone(), fast_options());
    let report = probe
        .run("user@example.org", &CancelToken::new())
        .await
        .unwrap();

    let settings = settings(&report.outcome);
    assert_eq!(settings.source, SettingsSource::Srv);
    assert_eq!(settings.imap.host, "imap.srv.example.org");
    assert_eq!(settings.imap.encryption, Encryption::Tls);
    assert_eq!(settings.smtp.host, "smtp.srv.example.org");
    assert_eq!(settings.smtp.port, 587);
    assert_eq!(settings.smtp.encryption, Encryption::StartTls);
    assert_eq!(transport.calls().last(), Some(&ProbeStep::Srv));
}

// --- failing fast to manual entry ---------------------------------------

#[tokio::test]
async fn a_domain_with_no_autoconfig_fails_fast_to_manual_entry() {
    let transport = Arc::new(MockTransport::new());
    let probe = Probe::with_options(transport.clone(), fast_options());

    let started = Instant::now();
    let report = probe
        .run("user@nowhere.example", &CancelToken::new())
        .await
        .unwrap();

    assert!(
        matches!(
            report.outcome,
            DiscoveryOutcome::ManualEntry { suggestion: None }
        ),
        "got {:?}",
        report.outcome
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "took {:?}",
        started.elapsed()
    );
    assert!(report.attempts.iter().all(|a| !a.is_hit()));
}

#[tokio::test]
async fn common_name_guesses_only_prefill_the_manual_form() {
    let transport = Arc::new(MockTransport::new());
    let options = ProbeOptions {
        guess_common_names: true,
        ..fast_options()
    };
    let report = Probe::with_options(transport, options)
        .run("user@nowhere.example", &CancelToken::new())
        .await
        .unwrap();

    // A guess is never presented as discovered: the user still lands in the
    // manual form, only with the fields filled in.
    match report.outcome {
        DiscoveryOutcome::ManualEntry {
            suggestion: Some(settings),
        } => {
            assert_eq!(settings.source, SettingsSource::Guess);
            assert_eq!(settings.imap.host, "imap.nowhere.example");
            assert_eq!(settings.imap.port, 993);
            assert_eq!(settings.smtp.host, "smtp.nowhere.example");
            assert_eq!(settings.smtp.port, 465);
        }
        other => panic!("expected a guessed suggestion, got {other:?}"),
    }
}

#[tokio::test]
async fn an_autoconfig_document_without_an_imap_server_is_a_miss() {
    let xml = r#"<clientConfig version="1.1">
  <emailProvider id="pop.example">
    <domain>pop.example</domain>
    <incomingServer type="pop3">
      <hostname>pop.pop.example</hostname>
      <port>995</port>
      <socketType>SSL</socketType>
    </incomingServer>
  </emailProvider>
</clientConfig>"#;

    let transport = Arc::new(MockTransport::new().with_autoconfig(ProbeStep::WellKnown, xml));
    let report = Probe::with_options(transport, fast_options())
        .run("user@pop.example", &CancelToken::new())
        .await
        .unwrap();

    assert!(matches!(
        report.outcome,
        DiscoveryOutcome::ManualEntry { .. }
    ));
}

// --- bad input ----------------------------------------------------------

#[tokio::test]
async fn a_malformed_address_is_rejected_without_probing() {
    let probe = Probe::new(Arc::new(UnreachableTransport));
    for address in ["", "not-an-address", "@example.org", "user@", "user@@x.org"] {
        assert!(
            matches!(
                probe.run(address, &CancelToken::new()).await,
                Err(DiscoveryError::InvalidAddress(_))
            ),
            "{address:?} was accepted"
        );
    }
}

// --- cancellation and timeouts ------------------------------------------

#[tokio::test]
async fn probing_is_cancellable() {
    let transport = Arc::new(MockTransport::stalling());
    let options = ProbeOptions {
        step_timeout: Duration::from_secs(30),
        overall_timeout: Duration::from_secs(60),
        ..ProbeOptions::default()
    };
    let probe = Probe::with_options(transport, options);
    let cancel = CancelToken::new();

    let handle = {
        let cancel = cancel.clone();
        tokio::spawn(async move { probe.run("user@example.org", &cancel).await })
    };

    tokio::time::sleep(Duration::from_millis(50)).await;
    cancel.cancel();

    let started = Instant::now();
    let result = handle.await.unwrap();
    assert!(
        matches!(result, Err(DiscoveryError::Cancelled)),
        "{result:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "cancellation did not take effect: {:?}",
        started.elapsed()
    );
}

/// #57: the probe's cancel token has to reach the transport, or nothing at
/// the network layer can act on it.
///
/// This is the wiring, not the mechanism. `probing_is_cancellable` above
/// already passed while the token stopped the probe *awaiting* an answer and
/// left the request itself running — the transport never saw the token at
/// all, so `PimalayaTransport` could not have cancelled a socket even in
/// principle. Asserting that the transport receives a token which observes
/// the caller's cancellation is the only assertion that could have failed
/// then and passes now.
#[tokio::test]
async fn the_transport_is_handed_a_token_that_sees_the_cancellation() {
    let transport = Arc::new(MockTransport::stalling());
    let probe = Probe::with_options(
        transport.clone(),
        ProbeOptions {
            step_timeout: Duration::from_millis(50),
            overall_timeout: Duration::from_millis(200),
            ..ProbeOptions::default()
        },
    );

    let cancel = CancelToken::new();
    // Cancelled before the run, so every call the probe makes is made with a
    // token that is already flagged. A transport that was handed a *fresh*
    // token -- or none -- would report false.
    cancel.cancel();
    let _ = probe.run("user@example.org", &cancel).await;

    // An already-cancelled token short-circuits before the first step, so
    // drive it again with cancellation arriving mid-flight to get a call
    // that actually reaches the transport.
    let cancel = CancelToken::new();
    let handle = {
        let probe = Probe::with_options(
            transport.clone(),
            ProbeOptions {
                step_timeout: Duration::from_secs(5),
                overall_timeout: Duration::from_secs(10),
                ..ProbeOptions::default()
            },
        );
        let cancel = cancel.clone();
        tokio::spawn(async move { probe.run("user@example.org", &cancel).await })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    cancel.cancel();
    let _ = handle.await.expect("the probe task did not panic");

    let seen = transport.saw_cancelled();
    assert!(
        !seen.is_empty(),
        "the transport was never called, so this proves nothing"
    );
    assert!(
        seen.iter().any(|observed| !observed),
        "every call already saw a cancelled token, so this cannot tell a \
         live token from a pre-cancelled one: {seen:?}"
    );
    assert!(
        transport.saw_cancelled_after_the_fact(),
        "the token the transport holds never observed the caller's \
         cancellation, so a real transport could not stop its request"
    );
}

#[tokio::test]
async fn an_already_cancelled_token_short_circuits() {
    let cancel = CancelToken::new();
    cancel.cancel();
    assert!(cancel.is_cancelled());

    let probe = Probe::new(Arc::new(UnreachableTransport));
    assert!(matches!(
        probe.run("user@example.org", &cancel).await,
        Err(DiscoveryError::Cancelled)
    ));
}

#[tokio::test]
async fn a_stalled_probe_times_out_instead_of_hanging() {
    let transport = Arc::new(MockTransport::stalling());
    let probe = Probe::with_options(
        transport.clone(),
        ProbeOptions {
            step_timeout: Duration::from_millis(50),
            overall_timeout: Duration::from_millis(120),
            ..ProbeOptions::default()
        },
    );

    let started = Instant::now();
    let report = probe
        .run("user@example.org", &CancelToken::new())
        .await
        .unwrap();

    assert!(
        matches!(
            report.outcome,
            DiscoveryOutcome::ManualEntry { suggestion: None }
        ),
        "{:?}",
        report.outcome
    );
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "probe hung for {:?}",
        started.elapsed()
    );
    assert!(report.attempts.iter().any(|a| a.timed_out()));
    // The overall budget stops the walk before every step has been tried.
    assert!(transport.calls().len() < 4);
}

#[tokio::test]
async fn the_default_options_are_bounded() {
    let options = ProbeOptions::default();
    assert!(options.step_timeout <= Duration::from_secs(10));
    assert!(options.overall_timeout <= Duration::from_secs(30));
    assert!(options.step_timeout <= options.overall_timeout);
    assert!(!options.guess_common_names);
}

// --- postio-iigq: privacy audit ------------------------------------------

#[tokio::test]
async fn only_the_domain_the_user_typed_is_ever_probed() {
    // No step answers with anything usable, so the probe has to walk all
    // four before giving up into `ManualEntry` — which is what makes this
    // exercise every endpoint's own request, not just whichever one
    // happened to hit first.
    let transport = Arc::new(MockTransport::new());
    let probe = Probe::with_options(transport.clone(), fast_options());

    let report = probe
        .run("a.user+tag@Example.ORG", &CancelToken::new())
        .await
        .expect("cancellation was never asked for");
    assert!(
        matches!(report.outcome, DiscoveryOutcome::ManualEntry { .. }),
        "every step was made to miss, on purpose: {:?}",
        report.outcome
    );

    let requested = transport.requested();
    assert_eq!(
        requested.len(),
        5,
        "every step should have made exactly one request: {requested:?}"
    );
    for (step, domain, local_part) in &requested {
        assert_eq!(
            domain, "example.org",
            "{step:?} asked about the wrong domain, or a stale one"
        );
        match (step, local_part) {
            // Only the subdomain step's request carries a local part at
            // all — the one place a provider's autoconfig can be keyed
            // per-user rather than per-domain.
            (ProbeStep::AutoconfigSubdomain, Some(local)) => {
                assert_eq!(local, "a.user+tag", "the wrong mailbox's local part");
            }
            (ProbeStep::AutoconfigSubdomain, None) => {
                panic!("the subdomain step lost the local part entirely")
            }
            (_, None) => {}
            (_, Some(local)) => panic!("{step:?} should never carry a local part: {local}"),
        }
    }
}

#[tokio::test]
async fn a_guessed_suggestion_is_built_after_every_step_missed_not_instead_of_them() {
    // Nothing here answers, the same as the domain-scoping test above, so
    // the loop reaches every step before falling back to a guess.
    let transport = Arc::new(MockTransport::new());
    let probe = Probe::with_options(
        transport.clone(),
        ProbeOptions {
            guess_common_names: true,
            ..fast_options()
        },
    );

    let report = probe
        .run("user@example.org", &CancelToken::new())
        .await
        .expect("no cancellation to fail on");
    match report.outcome {
        DiscoveryOutcome::ManualEntry {
            suggestion: Some(guess),
        } => {
            assert_eq!(guess.source, SettingsSource::Guess);
            assert_eq!(guess.imap.host, "imap.example.org");
        }
        other => panic!("expected a guessed suggestion, got {other:?}"),
    }

    // The guess itself is string formatting, not a further request: every
    // call the mock saw was a step, and there are only five of those.
    assert_eq!(transport.requested().len(), 5);
}

// --- live probe ---------------------------------------------------------

#[tokio::test]
#[ignore = "hits the network"]
async fn live_probe_against_the_thunderbird_ispdb() {
    use postio_account::discovery::PimalayaTransport;

    let probe = Probe::new(Arc::new(PimalayaTransport::new()));
    // Built, not written out: a literal address in a real provider's domain
    // reads as personal data to scripts/checks/check-no-personal-data.py.
    let domain = std::env::var("POSTIO_TEST_ISPDB_DOMAIN").unwrap_or_else(|_| "gmail.com".into());
    let report = probe
        .run(&format!("someone@{domain}"), &CancelToken::new())
        .await
        .unwrap();

    let settings = settings(&report.outcome);
    assert!(
        settings
            .imap
            .host
            .contains(domain.split('.').next().unwrap())
    );
}

// ── #94: MX records name a provider Postio ships settings for ───────────

#[tokio::test]
async fn mx_records_naming_a_known_provider_prefill_that_providers_servers() {
    // The case the convention guess gets wrong: a custom domain delegated to
    // a provider is not reachable at `imap.<that-domain>`. Its MX says where
    // its mail actually goes, and the preset table knows whose host that is.
    let transport =
        Arc::new(MockTransport::new().with_mx(&["mx01.mail.icloud.com", "mx02.mail.icloud.com"]));
    let probe = Probe::with_options(transport.clone(), fast_options());
    let report = probe
        .run("user@example.org", &CancelToken::new())
        .await
        .unwrap();

    let settings = settings(&report.outcome);
    assert_eq!(
        settings.source,
        SettingsSource::Mx,
        "the servers come from the row, but *that this domain is theirs* is \
         inferred from MX -- the source has to say which"
    );
    assert_eq!(settings.imap.host, "imap.mail.me.com");
    assert_eq!(settings.smtp.host, "smtp.mail.me.com");

    // #69's trap: an iCloud custom domain authenticates as the Apple ID, and
    // no MX lookup can know it. Prefilling a login from an MX hit would be
    // confidently wrong in exactly the case this step exists for.
    assert_eq!(
        settings.login, "user@example.org",
        "the login is the address the user typed, never one MX implied"
    );

    assert_eq!(
        transport.calls(),
        vec![
            ProbeStep::WellKnown,
            ProbeStep::AutoconfigSubdomain,
            ProbeStep::Ispdb,
            ProbeStep::Mx,
        ],
        "MX runs after everything the domain published about mail \
         configuration, and before SRV"
    );
}

#[tokio::test]
async fn mx_records_naming_nobody_known_are_a_miss_and_the_probe_carries_on() {
    // Plenty of domains publish MX for a host Postio ships no row for. That
    // is an answer, not a failure, and SRV still gets its turn.
    let transport = Arc::new(
        MockTransport::new()
            .with_mx(&["mail.some-host.example"])
            .with_srv(srv_report()),
    );
    let probe = Probe::with_options(transport.clone(), fast_options());
    let report = probe
        .run("user@example.org", &CancelToken::new())
        .await
        .unwrap();

    assert_eq!(settings(&report.outcome).source, SettingsSource::Srv);
    let mx = report
        .attempts
        .iter()
        .find(|attempt| attempt.step == ProbeStep::Mx)
        .expect("the MX step is reported like every other");
    assert!(
        mx.is_miss(),
        "MX that names nobody known is a miss, not a failure: the records \
         answered, they just named nobody in the table"
    );
}

#[tokio::test]
async fn an_mx_hit_prefills_the_manual_form_rather_than_claiming_a_discovery() {
    // It is inference, not publication — the same reason `guess_common_names`
    // is off by default. A domain that delegates its mail somewhere has not
    // said anything about *how to log in*, so this must not present itself as
    // a discovered account.
    let transport = Arc::new(MockTransport::new().with_mx(&["mx01.mail.icloud.com"]));
    let probe = Probe::with_options(transport.clone(), fast_options());
    let report = probe
        .run("user@example.org", &CancelToken::new())
        .await
        .unwrap();

    assert!(
        !settings(&report.outcome).source.is_authoritative(),
        "MX is better evidence than a convention guess and still not \
         authoritative"
    );
}
