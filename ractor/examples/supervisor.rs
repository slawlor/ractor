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

use ractor::{Actor, ActorProcessingErr, ActorRef, RpcReplyPort, SupervisionEvent};

use tokio::time::Duration;

// ============================== Main ============================== //

#[tokio::main]
async fn main() {
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

#[async_trait::async_trait]
impl Actor for LeafActor {
    type Msg = LeafActorMessage;
    type State = LeafActorState;
    type Arguments = ();
    async fn pre_start(
        &self,
        _myself: ActorRef<Self>,
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
        _myself: ActorRef<Self>,
        message: Self::Msg,
        _state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            Self::Msg::Boom => {
                panic!("LeafActor: Oh crap!");
            }
            Self::Msg::NoOp => {
                println!("LeafActor: No-op!");
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
    leaf_actor: ActorRef<LeafActor>,
}

enum MidLevelActorMessage {
    GetLeaf(RpcReplyPort<ActorRef<LeafActor>>),
}
#[cfg(feature = "cluster")]
impl ractor::Message for MidLevelActorMessage {}

#[async_trait::async_trait]
impl Actor for MidLevelActor {
    type Msg = MidLevelActorMessage;
    type State = MidLevelActorState;
    type Arguments = ();

    async fn pre_start(
        &self,
        myself: ActorRef<Self>,
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
        _myself: ActorRef<Self>,
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
        _myself: ActorRef<Self>,
        message: SupervisionEvent,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            SupervisionEvent::ActorPanicked(dead_actor, panic_msg)
                if dead_actor.get_id() == state.leaf_actor.get_id() =>
            {
                println!("MidLevelActor: {dead_actor:?} panicked with '{panic_msg}'");

                panic!(
                    "MidLevelActor: Mid-level actor panicking because Leaf actor panicked with '{}'",
                    panic_msg
                );
            }
            other => {
                println!("MidLevelActor: recieved supervisor event '{other}'");
            }
        }
        Ok(())
    }
}

// ============================== Root Actor Definition ============================== //

struct RootActor;

#[derive(Clone)]
struct RootActorState {
    mid_level_actor: ActorRef<MidLevelActor>,
}

enum RootActorMessage {
    GetMidLevel(RpcReplyPort<ActorRef<MidLevelActor>>),
}
#[cfg(feature = "cluster")]
impl ractor::Message for RootActorMessage {}

#[async_trait::async_trait]
impl Actor for RootActor {
    type Msg = RootActorMessage;
    type State = RootActorState;
    type Arguments = ();

    async fn pre_start(
        &self,
        myself: ActorRef<Self>,
        _: (),
    ) -> Result<Self::State, ActorProcessingErr> {
        println!("RootActor: Started {myself:?}");
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
        _myself: ActorRef<Self>,
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
        myself: ActorRef<Self>,
        message: SupervisionEvent,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            SupervisionEvent::ActorPanicked(dead_actor, panic_msg)
                if dead_actor.get_id() == state.mid_level_actor.get_id() =>
            {
                println!("RootActor: {dead_actor:?} panicked with '{panic_msg}'");

                println!("RootActor: Terminating root actor, all my kids are dead!");
                myself.stop(Some("Everyone died :(".to_string()));
            }
            other => {
                println!("RootActor: recieved supervisor event '{other}'");
            }
        }
        Ok(())
    }
}
