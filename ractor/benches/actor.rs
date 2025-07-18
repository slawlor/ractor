// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

#[macro_use]
extern crate criterion;

use criterion::BatchSize;
use criterion::Criterion;
use ractor::Actor;
use ractor::ActorProcessingErr;
use ractor::ActorRef;
#[cfg(feature = "cluster")]
use ractor::Message;
use ractor::OutputPort;

struct BenchActor;

struct BenchActorMessage;
#[cfg(feature = "cluster")]
impl Message for BenchActorMessage {}

#[cfg_attr(feature = "async-trait", ractor::async_trait)]
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
    #[cfg(not(feature = "async-std"))]
    let runtime = tokio::runtime::Builder::new_multi_thread().build().unwrap();
    #[cfg(feature = "async-std")]
    let _ = async_std::task::block_on(async {});
    c.bench_function(&id, move |b| {
        b.iter_batched(
            || {},
            |()| {
                #[cfg(not(feature = "async-std"))]
                {
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
                }
                #[cfg(feature = "async-std")]
                {
                    async_std::task::block_on(async move {
                        let mut handles = vec![];
                        for _ in 0..small {
                            let (_, handler) = Actor::spawn(None, BenchActor, ())
                                .await
                                .expect("Failed to create test agent");
                            handles.push(handler);
                        }
                        handles
                    })
                }
            },
            BatchSize::PerIteration,
        );
    });

    let id = format!("Creation of {large} actors");
    #[cfg(not(feature = "async-std"))]
    let runtime = tokio::runtime::Builder::new_multi_thread().build().unwrap();
    #[cfg(feature = "async-std")]
    let _ = async_std::task::block_on(async {});
    c.bench_function(&id, move |b| {
        b.iter_batched(
            || {},
            |()| {
                #[cfg(not(feature = "async-std"))]
                {
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
                }
                #[cfg(feature = "async-std")]
                {
                    async_std::task::block_on(async move {
                        let mut handles = vec![];
                        for _ in 0..large {
                            let (_, handler) = Actor::spawn(None, BenchActor, ())
                                .await
                                .expect("Failed to create test agent");
                            handles.push(handler);
                        }
                        handles
                    })
                }
            },
            BatchSize::PerIteration,
        );
    });
}

fn schedule_work(c: &mut Criterion) {
    let small = 100;
    let large = 1000;

    let id = format!("Waiting on {small} actors to process first message");
    #[cfg(not(feature = "async-std"))]
    let runtime = tokio::runtime::Builder::new_multi_thread().build().unwrap();
    #[cfg(feature = "async-std")]
    let _ = async_std::task::block_on(async {});
    c.bench_function(&id, move |b| {
        b.iter_batched(
            || {
                #[cfg(not(feature = "async-std"))]
                {
                    runtime.block_on(async move {
                        let mut join_set = ractor::concurrency::JoinSet::new();

                        for _ in 0..small {
                            let (_, handler) = Actor::spawn(None, BenchActor, ())
                                .await
                                .expect("Failed to create test agent");
                            join_set.spawn(handler);
                        }
                        join_set
                    })
                }
                #[cfg(feature = "async-std")]
                {
                    async_std::task::block_on(async move {
                        let mut join_set = ractor::concurrency::JoinSet::new();

                        for _ in 0..small {
                            let (_, handler) = Actor::spawn(None, BenchActor, ())
                                .await
                                .expect("Failed to create test agent");
                            join_set.spawn(handler);
                        }
                        join_set
                    })
                }
            },
            |mut handles| {
                #[cfg(not(feature = "async-std"))]
                {
                    runtime.block_on(async move { while handles.join_next().await.is_some() {} })
                }
                #[cfg(feature = "async-std")]
                {
                    async_std::task::block_on(async move {
                        while handles.join_next().await.is_some() {}
                    })
                }
            },
            BatchSize::PerIteration,
        );
    });

    let id = format!("Waiting on {large} actors to process first message");
    #[cfg(not(feature = "async-std"))]
    let runtime = tokio::runtime::Builder::new_multi_thread().build().unwrap();
    #[cfg(feature = "async-std")]
    let _ = async_std::task::block_on(async {});
    c.bench_function(&id, move |b| {
        b.iter_batched(
            || {
                #[cfg(not(feature = "async-std"))]
                {
                    runtime.block_on(async move {
                        let mut join_set = ractor::concurrency::JoinSet::new();
                        for _ in 0..large {
                            let (_, handler) = Actor::spawn(None, BenchActor, ())
                                .await
                                .expect("Failed to create test agent");
                            join_set.spawn(handler);
                        }
                        join_set
                    })
                }
                #[cfg(feature = "async-std")]
                {
                    async_std::task::block_on(async move {
                        let mut join_set = ractor::concurrency::JoinSet::new();
                        for _ in 0..large {
                            let (_, handler) = Actor::spawn(None, BenchActor, ())
                                .await
                                .expect("Failed to create test agent");
                            join_set.spawn(handler);
                        }
                        join_set
                    })
                }
            },
            |mut handles| {
                #[cfg(not(feature = "async-std"))]
                {
                    runtime.block_on(async move { while handles.join_next().await.is_some() {} })
                }
                #[cfg(feature = "async-std")]
                {
                    async_std::task::block_on(async move {
                        while handles.join_next().await.is_some() {}
                    })
                }
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

    #[cfg_attr(feature = "async-trait", ractor::async_trait)]
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
            message: Self::Msg,
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
    #[cfg(not(feature = "async-std"))]
    let runtime = tokio::runtime::Builder::new_multi_thread().build().unwrap();
    #[cfg(feature = "async-std")]
    let _ = async_std::task::block_on(async {});
    c.bench_function(&id, move |b| {
        b.iter_batched(
            || {
                #[cfg(not(feature = "async-std"))]
                {
                    runtime.block_on(async move {
                        let (_, handle) =
                            Actor::spawn(None, MessagingActor { num_msgs: NUM_MSGS }, ())
                                .await
                                .expect("Failed to create test actor");
                        handle
                    })
                }
                #[cfg(feature = "async-std")]
                {
                    async_std::task::block_on(async move {
                        let (_, handle) =
                            Actor::spawn(None, MessagingActor { num_msgs: NUM_MSGS }, ())
                                .await
                                .expect("Failed to create test actor");
                        handle
                    })
                }
            },
            |handle| {
                #[cfg(not(feature = "async-std"))]
                {
                    runtime.block_on(async move {
                        let _ = handle.await;
                    })
                }
                #[cfg(feature = "async-std")]
                {
                    async_std::task::block_on(async move {
                        let _ = handle.await;
                    })
                }
            },
            BatchSize::PerIteration,
        );
    });
}

#[allow(clippy::async_yields_async)]
fn process_output_port_messages(c: &mut Criterion) {
    const NUM_MSGS: u64 = 1000;
    const NUM_RECEIVERS: u64 = 100;

    struct MessagingActor {
        num_msgs: u64,
    }

    #[cfg_attr(feature = "async-trait", ractor::async_trait)]
    impl Actor for MessagingActor {
        type Msg = BenchActorMessage;

        type State = (u64, OutputPort<u64>);

        type Arguments = OutputPort<u64>;

        async fn pre_start(
            &self,
            myself: ActorRef<Self::Msg>,
            arg: Self::Arguments,
        ) -> Result<Self::State, ActorProcessingErr> {
            let _ = myself.cast(BenchActorMessage);
            Ok((0u64, arg))
        }

        async fn handle(
            &self,
            myself: ActorRef<Self::Msg>,
            _message: Self::Msg,
            state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            state.0 += 1;
            if state.0 > self.num_msgs {
                myself.stop(None);
            } else {
                state.1.send(state.0);
                let _ = myself.cast(BenchActorMessage);
            }
            Ok(())
        }
    }
    struct ReceivingActor {
        num_msgs: u64,
    }

    #[cfg_attr(feature = "async-trait", ractor::async_trait)]
    impl Actor for ReceivingActor {
        type Msg = u64;

        type State = u64;

        type Arguments = ();

        async fn pre_start(
            &self,
            _myself: ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(0)
        }

        async fn handle(
            &self,
            myself: ActorRef<Self::Msg>,
            _message: Self::Msg,
            state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            *state += 1u64;
            if *state >= self.num_msgs {
                myself.stop(None);
            }
            Ok(())
        }
    }

    let id = format!(
        "Waiting on {NUM_MSGS} messages to be sent on output port to {NUM_RECEIVERS} actors"
    );
    #[cfg(not(feature = "async-std"))]
    let runtime = tokio::runtime::Builder::new_multi_thread().build().unwrap();
    #[cfg(feature = "async-std")]
    let _ = async_std::task::block_on(async {});
    c.bench_function(&id, move |b| {
        b.iter_batched(
            || {
                #[cfg(not(feature = "async-std"))]
                {
                    runtime.block_on(async move {
                        let output_port = OutputPort::default();
                        let mut handels = Vec::new();
                        for _ in 0..NUM_RECEIVERS {
                            let (r, h) =
                                Actor::spawn(None, ReceivingActor { num_msgs: NUM_MSGS }, ())
                                    .await
                                    .expect("Failed to create test actor");
                            output_port.subscribe(r, |v| Some(v));
                            handels.push(h);
                        }
                        let (_, handle) =
                            Actor::spawn(None, MessagingActor { num_msgs: NUM_MSGS }, output_port)
                                .await
                                .expect("Failed to create test actor");
                        handels.push(handle);
                        handels
                    })
                }
                #[cfg(feature = "async-std")]
                {
                    async_std::task::block_on(async move {
                        let output_port = OutputPort::default();
                        let mut handels = Vec::new();
                        for _ in 0..16 {
                            let (r, h) =
                                Actor::spawn(None, ReceivingActor { num_msgs: NUM_MSGS }, ())
                                    .await
                                    .expect("Failed to create test actor");
                            output_port.subscribe(r, |v| Some(v));
                            handels.push(h);
                        }
                        let (_, handle) =
                            Actor::spawn(None, MessagingActor { num_msgs: NUM_MSGS }, output_port)
                                .await
                                .expect("Failed to create test actor");
                        handels.push(handle);
                        handels
                    })
                }
            },
            |handles| {
                #[cfg(not(feature = "async-std"))]
                {
                    runtime.block_on(async move {
                        for handle in handles {
                            let _ = handle.await;
                        }
                    })
                }
                #[cfg(feature = "async-std")]
                {
                    async_std::task::block_on(async move {
                        for handle in handles {
                            let _ = handle.await;
                        }
                    })
                }
            },
            BatchSize::PerIteration,
        );
    });
}

criterion_group!(
    actors,
    create_actors,
    schedule_work,
    process_messages,
    process_output_port_messages
);
criterion_main!(actors);
