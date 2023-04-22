// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

#[macro_use]
extern crate criterion;

use criterion::{BatchSize, Criterion};
#[cfg(feature = "cluster")]
use ractor::Message;
use ractor::{Actor, ActorProcessingErr, ActorRef};

struct BenchActor;

struct BenchActorMessage;
#[cfg(feature = "cluster")]
impl Message for BenchActorMessage {}

#[async_trait::async_trait]
impl Actor for BenchActor {
    type Msg = BenchActorMessage;

    type State = ();

    type Arguments = ();

    async fn pre_start(
        &self,
        myself: ActorRef<Self::Msg>,
        _: (),
    ) -> Result<Self::State, ActorProcessingErr> {
        let _ = myself.cast(BenchActorMessage);
        Ok(())
    }

    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        _message: Self::Msg,
        _state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        myself.stop(None);
        Ok(())
    }
}

fn create_actors(c: &mut Criterion) {
    let small = 100;
    let large = 10000;

    let id = format!("Creation of {small} actors");
    let runtime = tokio::runtime::Builder::new_multi_thread().build().unwrap();
    c.bench_function(&id, move |b| {
        b.iter_batched(
            || {},
            |()| {
                runtime.block_on(async move {
                    let mut handles = vec![];
                    for _ in 0..small {
                        let (_, handler) = Actor::spawn(None, BenchActor, ())
                            .await
                            .expect("Failed to create test agent");
                        handles.push(handler);
                    }
                    handles
                })
            },
            BatchSize::PerIteration,
        );
    });

    let id = format!("Creation of {large} actors");
    let runtime = tokio::runtime::Builder::new_multi_thread().build().unwrap();
    c.bench_function(&id, move |b| {
        b.iter_batched(
            || {},
            |()| {
                runtime.block_on(async move {
                    let mut handles = vec![];
                    for _ in 0..large {
                        let (_, handler) = Actor::spawn(None, BenchActor, ())
                            .await
                            .expect("Failed to create test agent");
                        handles.push(handler);
                    }
                    handles
                })
            },
            BatchSize::PerIteration,
        );
    });
}

fn schedule_work(c: &mut Criterion) {
    let small = 100;
    let large = 1000;

    let id = format!("Waiting on {small} actors to process first message");
    let runtime = tokio::runtime::Builder::new_multi_thread().build().unwrap();
    c.bench_function(&id, move |b| {
        b.iter_batched(
            || {
                runtime.block_on(async move {
                    let mut join_set = tokio::task::JoinSet::new();

                    for _ in 0..small {
                        let (_, handler) = Actor::spawn(None, BenchActor, ())
                            .await
                            .expect("Failed to create test agent");
                        join_set.spawn(handler);
                    }
                    join_set
                })
            },
            |mut handles| {
                runtime.block_on(async move { while handles.join_next().await.is_some() {} })
            },
            BatchSize::PerIteration,
        );
    });

    let id = format!("Waiting on {large} actors to process first message");
    let runtime = tokio::runtime::Builder::new_multi_thread().build().unwrap();
    c.bench_function(&id, move |b| {
        b.iter_batched(
            || {
                runtime.block_on(async move {
                    let mut join_set = tokio::task::JoinSet::new();
                    for _ in 0..large {
                        let (_, handler) = Actor::spawn(None, BenchActor, ())
                            .await
                            .expect("Failed to create test agent");
                        join_set.spawn(handler);
                    }
                    join_set
                })
            },
            |mut handles| {
                runtime.block_on(async move { while handles.join_next().await.is_some() {} })
            },
            BatchSize::PerIteration,
        );
    });
}

#[allow(clippy::async_yields_async)]
fn process_messages(c: &mut Criterion) {
    const NUM_MSGS: u64 = 100000;

    struct MessagingActor {
        num_msgs: u64,
    }

    #[async_trait::async_trait]
    impl Actor for MessagingActor {
        type Msg = BenchActorMessage;

        type State = u64;

        type Arguments = ();

        async fn pre_start(
            &self,
            myself: ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            let _ = myself.cast(BenchActorMessage);
            Ok(0u64)
        }

        async fn handle(
            &self,
            myself: ActorRef<Self::Msg>,
            _message: Self::Msg,
            state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            *state += 1;
            if *state >= self.num_msgs {
                myself.stop(None);
            } else {
                let _ = myself.cast(BenchActorMessage);
            }
            Ok(())
        }
    }

    let id = format!("Waiting on {NUM_MSGS} messages to be processed");
    let runtime = tokio::runtime::Builder::new_multi_thread().build().unwrap();
    c.bench_function(&id, move |b| {
        b.iter_batched(
            || {
                runtime.block_on(async move {
                    let (_, handle) = Actor::spawn(None, MessagingActor { num_msgs: NUM_MSGS }, ())
                        .await
                        .expect("Failed to create test actor");
                    handle
                })
            },
            |handle| {
                runtime.block_on(async move {
                    let _ = handle.await;
                })
            },
            BatchSize::PerIteration,
        );
    });
}

criterion_group!(actors, create_actors, schedule_work, process_messages);
criterion_main!(actors);
