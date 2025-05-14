// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! An agent which communicates to some set of subscribers via an "Output port"
//!
//! Execute with
//!
//! ```text
//! cargo run --example output_port
//! ```

#![allow(clippy::incompatible_msrv)]

extern crate ractor;

use std::sync::Arc;

use ractor::Actor;
use ractor::ActorProcessingErr;
use ractor::ActorRef;
use ractor::OutputPort;
use tokio::time::timeout;
use tokio::time::Duration;

enum PublisherMessage {
    Publish(String),
}
#[cfg(feature = "cluster")]
impl ractor::Message for PublisherMessage {}

#[derive(Clone)]
struct Output(String);
#[cfg(feature = "cluster")]
impl ractor::Message for Output {}

struct Publisher;

#[cfg_attr(
    all(
        feature = "async-trait",
        not(all(target_arch = "wasm32", target_os = "unknown"))
    ),
    ractor::async_trait
)]
#[cfg_attr(
    all(
        feature = "async-trait",
       all(target_arch = "wasm32", target_os = "unknown")
    ),
    ractor::async_trait(?Send)
)]
impl Actor for Publisher {
    type Msg = PublisherMessage;

    type State = Arc<OutputPort<Output>>;
    type Arguments = Arc<OutputPort<Output>>;

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        port: Arc<OutputPort<Output>>,
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(port)
    }

    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            Self::Msg::Publish(msg) => {
                tracing::info!("Publishing {msg}");
                state.send(Output(format!("Published: {msg}")));
            }
        }
        Ok(())
    }
}

struct Subscriber;

enum SubscriberMessage {
    Published(String),
}
#[cfg(feature = "cluster")]
impl ractor::Message for SubscriberMessage {}

#[cfg_attr(
    all(
        feature = "async-trait",
        not(all(target_arch = "wasm32", target_os = "unknown"))
    ),
    ractor::async_trait
)]
#[cfg_attr(
    all(
        feature = "async-trait",
       all(target_arch = "wasm32", target_os = "unknown")
    ),
    ractor::async_trait(?Send)
)]
impl Actor for Subscriber {
    type Msg = SubscriberMessage;

    type State = ();
    type Arguments = ();

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        _: (),
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(())
    }

    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        _state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            Self::Msg::Published(msg) => {
                tracing::info!("Subscriber ({myself:?}) received published message '{msg}'");
            }
        }
        Ok(())
    }
}

fn init_logging() {
    let dir = tracing_subscriber::filter::Directive::from(tracing::Level::DEBUG);

    use std::io::stderr;
    use std::io::IsTerminal;

    use tracing_glog::Glog;
    use tracing_glog::GlogFields;
    use tracing_subscriber::filter::EnvFilter;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::Registry;

    let fmt = tracing_subscriber::fmt::Layer::default()
        .with_ansi(stderr().is_terminal())
        .with_writer(std::io::stderr)
        .event_format(Glog::default().with_timer(tracing_glog::LocalTime::default()))
        .fmt_fields(GlogFields::default().compact());

    let filter = vec![dir]
        .into_iter()
        .fold(EnvFilter::from_default_env(), |filter, directive| {
            filter.add_directive(directive)
        });

    let subscriber = Registry::default().with(filter).with(fmt);
    tracing::subscriber::set_global_default(subscriber).expect("to set global subscriber");
}

#[ractor_example_entry_proc::ractor_example_entry]
async fn main() {
    init_logging();

    let port = Arc::new(OutputPort::default());

    let (publisher_ref, publisher_handle) = Actor::spawn(None, Publisher, port.clone())
        .await
        .expect("Failed to start publisher");

    let mut subscriber_refs = vec![];
    let mut subscriber_handles = vec![];

    // spawn + setup the subscribers (NOT SUPERVISION LINKAGE)
    for _ in 0..10 {
        let (actor_ref, actor_handle) = Actor::spawn(None, Subscriber, ())
            .await
            .expect("Failed to start subscriber");

        // TODO: there has to be a better syntax than keeping an arc to the port?
        port.subscribe(actor_ref.clone(), |msg| {
            Some(SubscriberMessage::Published(msg.0))
        });

        subscriber_refs.push(actor_ref);
        subscriber_handles.push(actor_handle);
    }

    // send some messages (we should see the subscribers printout)
    for i in 0..3 {
        publisher_ref
            .cast(PublisherMessage::Publish(format!("Something {i}")))
            .expect("Send failed");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    // clean up everything
    publisher_ref.stop(None);
    for actor in subscriber_refs {
        actor.stop(None);
    }
    // wait for exits
    timeout(Duration::from_millis(50), publisher_handle)
        .await
        .expect("Actor failed to exit cleanly")
        .unwrap();
    for s in subscriber_handles.into_iter() {
        timeout(Duration::from_millis(50), s)
            .await
            .expect("Actor failed to exit cleanly")
            .unwrap();
    }
}
