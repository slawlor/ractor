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

extern crate ractor;

use std::sync::Arc;

use ractor::{Actor, ActorRef, OutputPort};
use tokio::time::{timeout, Duration};

enum PublisherMessage {
    Publish(String),
}

struct Publisher {
    output: Arc<OutputPort<String>>,
}

#[async_trait::async_trait]
impl Actor for Publisher {
    type Msg = PublisherMessage;

    type State = ();

    async fn pre_start(&self, _myself: ActorRef<Self>) -> Self::State {}

    async fn handle(&self, _myself: ActorRef<Self>, message: Self::Msg, _state: &mut Self::State) {
        match message {
            Self::Msg::Publish(msg) => {
                println!("Publishing {}", msg);
                self.output.send(format!("Published: {}", msg));
            }
        }
    }
}

struct Subscriber;

enum SubscriberMessage {
    Published(String),
}

#[async_trait::async_trait]
impl Actor for Subscriber {
    type Msg = SubscriberMessage;

    type State = ();

    async fn pre_start(&self, _myself: ActorRef<Self>) -> Self::State {}

    async fn handle(&self, myself: ActorRef<Self>, message: Self::Msg, _state: &mut Self::State) {
        match message {
            Self::Msg::Published(msg) => {
                println!(
                    "Subscriber ({:?}) received published message '{}'",
                    myself, msg
                );
            }
        }
    }
}

#[tokio::main]
async fn main() {
    let port = Arc::new(OutputPort::default());

    let (publisher_ref, publisher_handle) = Actor::spawn(
        None,
        Publisher {
            output: port.clone(),
        },
    )
    .await
    .expect("Failed to start publisher");

    let mut subscriber_refs = vec![];
    let mut subscriber_handles = vec![];

    // spawn + setup the subscribers (NOT SUPERVISION LINKAGE)
    for _ in 0..10 {
        let (actor_ref, actor_handle) = Actor::spawn(None, Subscriber)
            .await
            .expect("Failed to start subscriber");

        // TODO: there has to be a better syntax than keeping an arc to the port?
        port.subscribe(actor_ref.clone(), |msg| {
            Some(SubscriberMessage::Published(msg))
        });

        subscriber_refs.push(actor_ref);
        subscriber_handles.push(actor_handle);
    }

    // send some messages (we should see the subscribers printout)
    for i in 0..3 {
        publisher_ref
            .cast(PublisherMessage::Publish(format!("Something {}", i)))
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
