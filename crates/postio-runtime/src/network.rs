//! Feeds NetworkManager's opinion to the supervisor, only moving the link between `Up` and `Down`:
//! the attempt count stays the supervisor's, or every Wi-Fi re-association during an outage would
//! reset the backoff. Anything short of fully connected is `Unknown`; with no NetworkManager
//! `follow` returns quietly.

use postio_sync::NetworkState;
use tokio::sync::watch;

const SERVICE: &str = "org.freedesktop.NetworkManager";
const PATH: &str = "/org/freedesktop/NetworkManager";

/// `NMState` values, spelled out rather than taken from a binding crate: seven integers unchanged
/// since 2011.
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

/// Only the two ends of the scale are evidence; the middle is deliberately `Unknown`.
pub fn network_state(nm_state: u32) -> NetworkState {
    match nm_state {
        nm_state::CONNECTED_GLOBAL => NetworkState::Up,
        nm_state::ASLEEP | nm_state::DISCONNECTED | nm_state::DISCONNECTING => NetworkState::Down,
        nm_state::UNKNOWN
        | nm_state::CONNECTING
        | nm_state::CONNECTED_LOCAL
        | nm_state::CONNECTED_SITE => NetworkState::Unknown,
        // A newer NetworkManager's value: saying nothing is the only safe reading.
        _ => NetworkState::Unknown,
    }
}

/// Returns when the bus goes away, the engine stops, or there is no NetworkManager; none is a
/// failure worth reporting.
pub async fn follow(sink: watch::Sender<NetworkState>) {
    let Ok(connection) = zbus::Connection::system().await else {
        return;
    };
    let proxy = match zbus::Proxy::new(&connection, SERVICE, PATH, SERVICE).await {
        Ok(proxy) => proxy,
        Err(_) => return,
    };

    // Read the current state first: a client started while already offline would otherwise never
    // hear of it.
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
            return;
        }
    }
}

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
        // The supervisor is inert on `Unknown`: a link reaching only the local segment may still
        // reach a mail server.
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
