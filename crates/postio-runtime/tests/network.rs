//! Following NetworkManager, against the real system bus.
//!
//! The mapping from `NMState` onto what the supervisor understands is unit
//! tested inside `postio_runtime::network`, with no bus involved. What cannot
//! be proven without a machine is that NetworkManager is where this build
//! thinks it is and says what this build thinks it says — so that is what is
//! here, `#[ignore]`d like every other test in this workspace that needs
//! something real:
//!
//! ```text
//! cargo test -p postio-runtime --test network -- --ignored --nocapture
//! ```

use postio_runtime::NetworkState;
use postio_runtime::network::follow;
use tokio::sync::watch;

#[tokio::test]
#[ignore = "needs a system bus running NetworkManager"]
async fn networkmanager_answers_with_a_state_this_build_understands() {
    let (sink, mut changed) = watch::channel(NetworkState::Unknown);
    tokio::spawn(follow(sink));

    // The first thing `follow` does after connecting is publish the *current*
    // state, so this does not wait on the network changing.
    tokio::time::timeout(std::time::Duration::from_secs(5), changed.changed())
        .await
        .expect("NetworkManager did not answer within five seconds")
        .expect("the listener stopped before saying anything");

    let reported = *changed.borrow_and_update();
    println!("NetworkManager reports: {reported:?}");
    assert_eq!(
        reported,
        NetworkState::Up,
        "this test assumes the machine running it is on the network; \
         anything else means the mapping or the property name is wrong"
    );
}

#[tokio::test]
#[ignore = "needs a system bus running NetworkManager"]
async fn the_listener_stops_when_nobody_is_listening_any_more() {
    // The engine holds the receiving end. When it stops, this task must not
    // keep a bus connection open for the life of the process.
    let (sink, changed) = watch::channel(NetworkState::Unknown);
    let listener = tokio::spawn(follow(sink));
    drop(changed);

    tokio::time::timeout(std::time::Duration::from_secs(10), listener)
        .await
        .expect("the listener outlived its receiver")
        .expect("the listener panicked");
}
