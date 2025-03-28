// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! An example supervisory tree of actors
//!
//! Execute with
//!
//! ```text
//! cargo run --example supervisor
//! ```

#![allow(clippy::incompatible_msrv)]

use ractor::{Actor, ActorProcessingErr, ActorRef, RpcReplyPort, SupervisionEvent};

use tokio::time::Duration;

// ============================== Main ============================== //

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

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    tokio::main(flavor = "current_thread")
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), tokio::main)]
async fn main() {
    init_logging();

    let (root, handle) = Actor::spawn(Some("root".to_string()), RootActor, ())
        .await
        .expect("Failed to start root actor");
    let mid_level = root
        .call(
            RootActorMessage::GetMidLevel,
            Some(Duration::from_millis(100)),
        )
        .await
        .expect("Failed to send message to root actor")
        .expect("Failed to get mid level actor");

    let leaf = mid_level
        .call(
            MidLevelActorMessage::GetLeaf,
            Some(Duration::from_millis(100)),
        )
        .await
        .expect("Failed to send message to mid-level actor")
        .expect("Failed to get leaf actor");

    // send some no-op's to the leaf
    for _ in 1..10 {
        leaf.cast(LeafActorMessage::NoOp)
            .expect("Failed to send message to leaf actor");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    // send the "Boom" message and follow the supervision flow
    leaf.cast(LeafActorMessage::Boom)
        .expect("Failed to send message to leaf actor");

    handle.await.expect("Failed waiting for root actor to die");
}

// ============================== Leaf actor ============================== //

struct LeafActor;

#[derive(Clone)]
struct LeafActorState {}

enum LeafActorMessage {
    Boom,
    NoOp,
}
#[cfg(feature = "cluster")]
impl ractor::Message for LeafActorMessage {}

#[cfg_attr(feature = "async-trait", ractor::async_trait)]
impl Actor for LeafActor {
    type Msg = LeafActorMessage;
    type State = LeafActorState;
    type Arguments = ();
    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        _: (),
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(LeafActorState {})
    }

    // async fn post_start(&self, myself: ActorCell, state: &Self::State) -> Option<Self::State> {
    //     None
    // }

    // async fn post_stop(&self, myself: ActorCell, state: Self::State) -> Self::State {
    //     state
    // }

    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        _state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            Self::Msg::Boom => {
                panic!("LeafActor: Oh crap!");
            }
            Self::Msg::NoOp => {
                tracing::info!("LeafActor: No-op!");
            }
        }
        Ok(())
    }

    // async fn handle_supervisor_evt(
    //     &self,
    //     myself: ActorCell,
    //     message: SupervisionEvent,
    //     state: &Self::State,
    // ) -> Option<Self::State> {
    //     None
    // }
}

// ============================== Mid-level supervisor ============================== //

struct MidLevelActor;

#[derive(Clone)]
struct MidLevelActorState {
    leaf_actor: ActorRef<LeafActorMessage>,
}

enum MidLevelActorMessage {
    GetLeaf(RpcReplyPort<ActorRef<LeafActorMessage>>),
}
#[cfg(feature = "cluster")]
impl ractor::Message for MidLevelActorMessage {}

#[cfg_attr(feature = "async-trait", ractor::async_trait)]
impl Actor for MidLevelActor {
    type Msg = MidLevelActorMessage;
    type State = MidLevelActorState;
    type Arguments = ();

    async fn pre_start(
        &self,
        myself: ActorRef<Self::Msg>,
        _: (),
    ) -> Result<Self::State, ActorProcessingErr> {
        let (leaf_actor, _) =
            Actor::spawn_linked(Some("leaf".to_string()), LeafActor, (), myself.into()).await?;
        Ok(MidLevelActorState { leaf_actor })
    }

    // async fn post_start(&self, myself: ActorCell, state: &Self::State) -> Option<Self::State> {
    //     None
    // }

    // async fn post_stop(&self, myself: ActorCell, state: Self::State) -> Self::State {
    //     state
    // }

    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            MidLevelActorMessage::GetLeaf(reply) => {
                if !reply.is_closed() {
                    reply.send(state.leaf_actor.clone())?;
                }
            }
        }
        Ok(())
    }

    async fn handle_supervisor_evt(
        &self,
        _myself: ActorRef<Self::Msg>,
        message: SupervisionEvent,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            SupervisionEvent::ActorFailed(dead_actor, panic_msg)
                if dead_actor.get_id() == state.leaf_actor.get_id() =>
            {
                tracing::info!("MidLevelActor: {dead_actor:?} panicked with '{panic_msg}'");

                panic!(
                    "MidLevelActor: Mid-level actor panicking because Leaf actor panicked with '{}'",
                    panic_msg
                );
            }
            other => {
                tracing::info!("MidLevelActor: received supervisor event '{other}'");
            }
        }
        Ok(())
    }
}

// ============================== Root Actor Definition ============================== //

struct RootActor;

#[derive(Clone)]
struct RootActorState {
    mid_level_actor: ActorRef<MidLevelActorMessage>,
}

enum RootActorMessage {
    GetMidLevel(RpcReplyPort<ActorRef<MidLevelActorMessage>>),
}
#[cfg(feature = "cluster")]
impl ractor::Message for RootActorMessage {}

#[cfg_attr(feature = "async-trait", ractor::async_trait)]
impl Actor for RootActor {
    type Msg = RootActorMessage;
    type State = RootActorState;
    type Arguments = ();

    async fn pre_start(
        &self,
        myself: ActorRef<Self::Msg>,
        _: (),
    ) -> Result<Self::State, ActorProcessingErr> {
        tracing::info!("RootActor: Started {myself:?}");
        let (mid_level_actor, _) = Actor::spawn_linked(
            Some("mid-level".to_string()),
            MidLevelActor,
            (),
            myself.into(),
        )
        .await?;
        Ok(RootActorState { mid_level_actor })
    }

    // async fn post_start(&self, myself: ActorCell, state: &Self::State) -> Option<Self::State> {
    //     None
    // }

    // async fn post_stop(&self, myself: ActorCell, state: Self::State) -> Self::State {
    //     state
    // }

    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            RootActorMessage::GetMidLevel(reply) => {
                if !reply.is_closed() {
                    reply.send(state.mid_level_actor.clone())?;
                }
            }
        }
        Ok(())
    }

    async fn handle_supervisor_evt(
        &self,
        myself: ActorRef<Self::Msg>,
        message: SupervisionEvent,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            SupervisionEvent::ActorFailed(dead_actor, panic_msg)
                if dead_actor.get_id() == state.mid_level_actor.get_id() =>
            {
                tracing::info!("RootActor: {dead_actor:?} panicked with '{panic_msg}'");

                tracing::info!("RootActor: Terminating root actor, all my kids are dead!");
                myself.stop(Some("Everyone died :(".to_string()));
            }
            other => {
                tracing::info!("RootActor: received supervisor event '{other}'");
            }
        }
        Ok(())
    }
}
