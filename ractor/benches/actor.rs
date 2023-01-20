// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

#[macro_use]
extern crate criterion;

use criterion::{BatchSize, Criterion};
use ractor::{Actor, ActorRef};

struct BenchActor;

#[async_trait::async_trait]
impl Actor for BenchActor {
    type Msg = ();

    type State = ();

    async fn pre_start(&self, myself: ActorRef<Self>) -> Self::State {
        let _ = myself.cast(());
    }

    async fn handle(&self, myself: ActorRef<Self>, _message: Self::Msg, _state: &mut Self::State) {
        myself.stop(None);
    }
}

fn create_actors(c: &mut Criterion) {
    let small = 100;
    let large = 10000;

    let id = format!("Creation of {} actors", small);
    let runtime = tokio::runtime::Builder::new_multi_thread().build().unwrap();
    c.bench_function(&id, move |b| {
        b.iter_batched(
            || {},
            |()| {
                runtime.block_on(async move {
                    let mut handles = vec![];
                    for _ in 0..small {
                        let (_, handler) = Actor::spawn(None, BenchActor)
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

    let id = format!("Creation of {} actors", large);
    let runtime = tokio::runtime::Builder::new_multi_thread().build().unwrap();
    c.bench_function(&id, move |b| {
        b.iter_batched(
            || {},
            |()| {
                runtime.block_on(async move {
                    let mut handles = vec![];
                    for _ in 0..large {
                        let (_, handler) = Actor::spawn(None, BenchActor)
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

    let id = format!("Waiting on {} actors to process first message", small);
    let runtime = tokio::runtime::Builder::new_multi_thread().build().unwrap();
    c.bench_function(&id, move |b| {
        b.iter_batched(
            || {
                runtime.block_on(async move {
                    let mut join_set = tokio::task::JoinSet::new();

                    for _ in 0..small {
                        let (_, handler) = Actor::spawn(None, BenchActor)
                            .await
                            .expect("Failed to create test agent");
                        join_set.spawn(handler);
                    }
                    join_set
                })
            },
            |mut handles| {
                runtime.block_on(async move { while let Some(_) = handles.join_next().await {} })
            },
            BatchSize::PerIteration,
        );
    });

    let id = format!("Waiting on {} actors to process first message", large);
    let runtime = tokio::runtime::Builder::new_multi_thread().build().unwrap();
    c.bench_function(&id, move |b| {
        b.iter_batched(
            || {
                runtime.block_on(async move {
                    let mut join_set = tokio::task::JoinSet::new();
                    for _ in 0..large {
                        let (_, handler) = Actor::spawn(None, BenchActor)
                            .await
                            .expect("Failed to create test agent");
                        join_set.spawn(handler);
                    }
                    join_set
                })
            },
            |mut handles| {
                runtime.block_on(async move { while let Some(_) = handles.join_next().await {} })
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
        type Msg = ();

        type State = u64;

        async fn pre_start(&self, myself: ActorRef<Self>) -> Self::State {
            let _ = myself.cast(());
            0u64
        }

        async fn handle(
            &self,
            myself: ActorRef<Self>,
            _message: Self::Msg,
            state: &mut Self::State,
        ) {
            *state += 1;
            if *state >= self.num_msgs {
                myself.stop(None);
            } else {
                let _ = myself.cast(());
            }
        }
    }

    let id = format!("Waiting on {} messages to be processed", NUM_MSGS);
    let runtime = tokio::runtime::Builder::new_multi_thread().build().unwrap();
    c.bench_function(&id, move |b| {
        b.iter_batched(
            || {
                runtime.block_on(async move {
                    let (_, handle) = Actor::spawn(None, MessagingActor { num_msgs: NUM_MSGS })
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
