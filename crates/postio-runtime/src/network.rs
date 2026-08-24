//! What the operating system says about the network.
//!
//! [`Supervisor::set_network`](postio_sync::Supervisor::set_network) is the
//! seam that lets a reconnect stop being a guess. Without anything feeding it,
//! the supervisor is correct but not prompt: a laptop whose lid has just opened
//! waits out a backoff that was measured against a network which no longer
//! exists — up to two minutes of a mail client that looks broken.
//!
//! # Why this is not in `postio-sync`
//!
//! That crate has no async runtime and no bus, and keeping it that way is what
//! lets the whole sync engine be tested with nothing running. The listener
//! belongs to the layer that already owns a loop, which is [`crate::engine`].
//!
//! # What is trusted, and what is not
//!
//! NetworkManager knows about the *interface*, not about whether a mail server
//! answers. So its opinion is only ever used to move the link between
//! [`NetworkState::Up`] and [`NetworkState::Down`]; the attempt count is the
//! supervisor's and stays where it is, or every Wi-Fi re-association during a
//! server outage would reset the backoff.
//!
//! Anything short of a fully connected link maps to [`NetworkState::Unknown`]
//! rather than `Down`, which the supervisor treats as no evidence at all. A
//! link that only reaches the local segment may still reach a mail server on
//! it, and being told "no network" when there is one costs the user their mail;
//! being told nothing costs them the promptness this module exists to add.
//!
//! # Absent NetworkManager
//!
//! Not every machine has it, and nothing here requires it. [`follow`] returns
//! quietly when the system bus or the service is not there, which leaves the
//! engine on [`NetworkState::Unknown`] — exactly how it behaved before this
//! module existed.

use postio_sync::NetworkState;
use tokio::sync::watch;

/// The bus name, object and interface NetworkManager publishes its state on.
const SERVICE: &str = "org.freedesktop.NetworkManager";
const PATH: &str = "/org/freedesktop/NetworkManager";

/// `NMState` — the values of NetworkManager's `State` property and of the
/// argument to its `StateChanged` signal.
///
/// Spelled out rather than taken from a binding crate: it is seven integers
/// that have not changed since 2011, and naming them here is the whole of what
/// this module needs to know about NetworkManager.
mod nm_state {
    pub const UNKNOWN: u32 = 0;
    pub const ASLEEP: u32 = 10;
    pub const DISCONNECTED: u32 = 20;
    pub const DISCONNECTING: u32 = 30;
    pub const CONNECTING: u32 = 40;
    pub const CONNECTED_LOCAL: u32 = 50;
    pub const CONNECTED_SITE: u32 = 60;
    pub const CONNECTED_GLOBAL: u32 = 70;
}

/// What one `NMState` value means to the reconnect supervisor.
///
/// Only the two ends of the scale are evidence. See the module docs for why
/// the middle — connecting, or connected to something short of the internet —
/// is deliberately [`NetworkState::Unknown`] rather than a guess in either
/// direction.
pub fn network_state(nm_state: u32) -> NetworkState {
    match nm_state {
        nm_state::CONNECTED_GLOBAL => NetworkState::Up,
        nm_state::ASLEEP | nm_state::DISCONNECTED | nm_state::DISCONNECTING => NetworkState::Down,
        nm_state::UNKNOWN
        | nm_state::CONNECTING
        | nm_state::CONNECTED_LOCAL
        | nm_state::CONNECTED_SITE => NetworkState::Unknown,
        // A value from a NetworkManager newer than this build. Saying nothing
        // is the only safe reading of a number whose meaning is not known.
        _ => NetworkState::Unknown,
    }
}

/// Follow NetworkManager, publishing what it says into `sink`.
///
/// Returns when the bus goes away, when the engine holding the other end of
/// `sink` stops, or immediately when there is no NetworkManager to follow —
/// none of which is a failure worth reporting to the user, because the
/// supervisor works without this and only becomes slower.
pub async fn follow(sink: watch::Sender<NetworkState>) {
    let Ok(connection) = zbus::Connection::system().await else {
        return;
    };
    let proxy = match zbus::Proxy::new(&connection, SERVICE, PATH, SERVICE).await {
        Ok(proxy) => proxy,
        Err(_) => return,
    };

    // The current state before any signal: a client that started while the
    // network was already down would otherwise not hear about it until it
    // changed, which on a machine that stays offline is never.
    if let Ok(state) = proxy.get_property::<u32>("State").await
        && sink.send(network_state(state)).is_err()
    {
        return;
    }

    let Ok(signals) = proxy.receive_signal("StateChanged").await else {
        return;
    };
    let mut signals = std::pin::pin!(signals);

    while let Some(signal) = next(signals.as_mut()).await {
        let Ok(state) = signal.body().deserialize::<u32>() else {
            continue;
        };
        if sink.send(network_state(state)).is_err() {
            // Nobody is listening any more: the engine has stopped.
            return;
        }
    }
}

/// One item from a stream, without pulling in a combinator crate for it.
async fn next<S: zbus::export::futures_core::Stream>(
    mut stream: std::pin::Pin<&mut S>,
) -> Option<S::Item> {
    std::future::poll_fn(|context| stream.as_mut().poll_next(context)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_fully_connected_link_counts_as_up() {
        assert_eq!(network_state(nm_state::CONNECTED_GLOBAL), NetworkState::Up);
    }

    #[test]
    fn a_link_that_is_down_or_asleep_counts_as_down() {
        for state in [
            nm_state::ASLEEP,
            nm_state::DISCONNECTED,
            nm_state::DISCONNECTING,
        ] {
            assert_eq!(
                network_state(state),
                NetworkState::Down,
                "NMState {state} should park the link"
            );
        }
    }

    #[test]
    fn a_half_connected_link_is_not_evidence_either_way() {
        // The supervisor is inert on Unknown, which is the point: a link that
        // reaches only the local segment may still reach a mail server on it,
        // and a client mid-association is about to know better in a moment.
        for state in [
            nm_state::UNKNOWN,
            nm_state::CONNECTING,
            nm_state::CONNECTED_LOCAL,
            nm_state::CONNECTED_SITE,
        ] {
            assert_eq!(
                network_state(state),
                NetworkState::Unknown,
                "NMState {state} should leave the supervisor's own judgement alone"
            );
        }
    }

    #[test]
    fn a_value_from_a_newer_networkmanager_says_nothing() {
        assert_eq!(network_state(80), NetworkState::Unknown);
        assert_eq!(network_state(u32::MAX), NetworkState::Unknown);
    }
}
