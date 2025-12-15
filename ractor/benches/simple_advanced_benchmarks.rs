// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Simplified advanced benchmarks for targeted performance analysis

#[macro_use]
extern crate criterion;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput};
use ractor::{Actor, ActorProcessingErr, ActorRef};

// ============================================================================
// 1. MESSAGE SIZE DISTRIBUTION BENCHMARKS
// ============================================================================

/// Small messages (1-32 bytes) - where SBO would theoretically help
fn small_message_workload(c: &mut Criterion) {
    const NUM_MSGS: u64 = 10000;

    // 8-byte message
    #[derive(Clone)]
    #[allow(unused)]
    struct Msg8B(u64);
    #[cfg(feature = "cluster")]
    impl ractor::Message for Msg8B {}

    struct Actor8B;

    #[cfg_attr(feature = "async-trait", ractor::async_trait)]
    impl Actor for Actor8B {
        type Msg = Msg8B;
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
            *state += 1;
            if *state >= NUM_MSGS {
                myself.stop(None);
            }
            Ok(())
        }
    }

    // 16-byte message
    #[derive(Clone)]
    #[allow(unused)]
    struct Msg16B(u64, u64);
    #[cfg(feature = "cluster")]
    impl ractor::Message for Msg16B {}

    struct Actor16B;

    #[cfg_attr(feature = "async-trait", ractor::async_trait)]
    impl Actor for Actor16B {
        type Msg = Msg16B;
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
            *state += 1;
            if *state >= NUM_MSGS {
                myself.stop(None);
            }
            Ok(())
        }
    }

    // 32-byte message
    #[derive(Clone)]
    #[allow(unused)]
    struct Msg32B(u64, u64, u64, u64);
    #[cfg(feature = "cluster")]
    impl ractor::Message for Msg32B {}

    struct Actor32B;

    #[cfg_attr(feature = "async-trait", ractor::async_trait)]
    impl Actor for Actor32B {
        type Msg = Msg32B;
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
            *state += 1;
            if *state >= NUM_MSGS {
                myself.stop(None);
            }
            Ok(())
        }
    }

    #[cfg(not(feature = "async-std"))]
    let runtime = tokio::runtime::Builder::new_multi_thread().build().unwrap();

    let mut group = c.benchmark_group("small_messages");
    group.throughput(Throughput::Elements(NUM_MSGS));

    group.bench_function("8_byte_messages", |b| {
        b.iter_batched(
            || {
                #[cfg(not(feature = "async-std"))]
                {
                    runtime.block_on(async {
                        Actor::spawn(None, Actor8B, ())
                            .await
                            .expect("Failed to spawn actor")
                    })
                }
                #[cfg(feature = "async-std")]
                {
                    async_std::task::block_on(async {
                        Actor::spawn(None, Actor8B, ())
                            .await
                            .expect("Failed to spawn actor")
                    })
                }
            },
            |(actor_ref, handle)| {
                #[cfg(not(feature = "async-std"))]
                {
                    runtime.block_on(async {
                        for i in 0..NUM_MSGS {
                            let _ = actor_ref.cast(Msg8B(i));
                        }
                        let _ = handle.await;
                    })
                }
                #[cfg(feature = "async-std")]
                {
                    async_std::task::block_on(async {
                        for i in 0..NUM_MSGS {
                            let _ = actor_ref.cast(Msg8B(i));
                        }
                        let _ = handle.await;
                    })
                }
            },
            BatchSize::PerIteration,
        );
    });

    group.bench_function("16_byte_messages", |b| {
        b.iter_batched(
            || {
                #[cfg(not(feature = "async-std"))]
                {
                    runtime.block_on(async {
                        Actor::spawn(None, Actor16B, ())
                            .await
                            .expect("Failed to spawn actor")
                    })
                }
                #[cfg(feature = "async-std")]
                {
                    async_std::task::block_on(async {
                        Actor::spawn(None, Actor16B, ())
                            .await
                            .expect("Failed to spawn actor")
                    })
                }
            },
            |(actor_ref, handle)| {
                #[cfg(not(feature = "async-std"))]
                {
                    runtime.block_on(async {
                        for i in 0..NUM_MSGS {
                            let _ = actor_ref.cast(Msg16B(i, i + 1));
                        }
                        let _ = handle.await;
                    })
                }
                #[cfg(feature = "async-std")]
                {
                    async_std::task::block_on(async {
                        for i in 0..NUM_MSGS {
                            let _ = actor_ref.cast(Msg16B(i, i + 1));
                        }
                        let _ = handle.await;
                    })
                }
            },
            BatchSize::PerIteration,
        );
    });

    group.bench_function("32_byte_messages", |b| {
        b.iter_batched(
            || {
                #[cfg(not(feature = "async-std"))]
                {
                    runtime.block_on(async {
                        Actor::spawn(None, Actor32B, ())
                            .await
                            .expect("Failed to spawn actor")
                    })
                }
                #[cfg(feature = "async-std")]
                {
                    async_std::task::block_on(async {
                        Actor::spawn(None, Actor32B, ())
                            .await
                            .expect("Failed to spawn actor")
                    })
                }
            },
            |(actor_ref, handle)| {
                #[cfg(not(feature = "async-std"))]
                {
                    runtime.block_on(async {
                        for i in 0..NUM_MSGS {
                            let _ = actor_ref.cast(Msg32B(i, i + 1, i + 2, i + 3));
                        }
                        let _ = handle.await;
                    })
                }
                #[cfg(feature = "async-std")]
                {
                    async_std::task::block_on(async {
                        for i in 0..NUM_MSGS {
                            let _ = actor_ref.cast(Msg32B(i, i + 1, i + 2, i + 3));
                        }
                        let _ = handle.await;
                    })
                }
            },
            BatchSize::PerIteration,
        );
    });

    group.finish();
}

/// Large messages (64-1024 bytes) - where heap allocation is expected
fn large_message_workload(c: &mut Criterion) {
    const NUM_MSGS: u64 = 10000;

    // 64-byte message
    #[derive(Clone)]
    #[allow(unused)]
    struct Msg64B([u64; 8]);
    #[cfg(feature = "cluster")]
    impl ractor::Message for Msg64B {}

    struct Actor64B;

    #[cfg_attr(feature = "async-trait", ractor::async_trait)]
    impl Actor for Actor64B {
        type Msg = Msg64B;
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
            *state += 1;
            if *state >= NUM_MSGS {
                myself.stop(None);
            }
            Ok(())
        }
    }

    // 256-byte message
    #[derive(Clone)]
    #[allow(unused)]
    struct Msg256B([u64; 32]);
    #[cfg(feature = "cluster")]
    impl ractor::Message for Msg256B {}

    struct Actor256B;

    #[cfg_attr(feature = "async-trait", ractor::async_trait)]
    impl Actor for Actor256B {
        type Msg = Msg256B;
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
            *state += 1;
            if *state >= NUM_MSGS {
                myself.stop(None);
            }
            Ok(())
        }
    }

    #[cfg(not(feature = "async-std"))]
    let runtime = tokio::runtime::Builder::new_multi_thread().build().unwrap();

    let mut group = c.benchmark_group("large_messages");
    group.throughput(Throughput::Elements(NUM_MSGS));

    group.bench_function("64_byte_messages", |b| {
        b.iter_batched(
            || {
                #[cfg(not(feature = "async-std"))]
                {
                    runtime.block_on(async {
                        Actor::spawn(None, Actor64B, ())
                            .await
                            .expect("Failed to spawn actor")
                    })
                }
                #[cfg(feature = "async-std")]
                {
                    async_std::task::block_on(async {
                        Actor::spawn(None, Actor64B, ())
                            .await
                            .expect("Failed to spawn actor")
                    })
                }
            },
            |(actor_ref, handle)| {
                #[cfg(not(feature = "async-std"))]
                {
                    runtime.block_on(async {
                        for i in 0..NUM_MSGS {
                            let _ = actor_ref.cast(Msg64B([i; 8]));
                        }
                        let _ = handle.await;
                    })
                }
                #[cfg(feature = "async-std")]
                {
                    async_std::task::block_on(async {
                        for i in 0..NUM_MSGS {
                            let _ = actor_ref.cast(Msg64B([i; 8]));
                        }
                        let _ = handle.await;
                    })
                }
            },
            BatchSize::PerIteration,
        );
    });

    group.bench_function("256_byte_messages", |b| {
        b.iter_batched(
            || {
                #[cfg(not(feature = "async-std"))]
                {
                    runtime.block_on(async {
                        Actor::spawn(None, Actor256B, ())
                            .await
                            .expect("Failed to spawn actor")
                    })
                }
                #[cfg(feature = "async-std")]
                {
                    async_std::task::block_on(async {
                        Actor::spawn(None, Actor256B, ())
                            .await
                            .expect("Failed to spawn actor")
                    })
                }
            },
            |(actor_ref, handle)| {
                #[cfg(not(feature = "async-std"))]
                {
                    runtime.block_on(async {
                        for i in 0..NUM_MSGS {
                            let _ = actor_ref.cast(Msg256B([i; 32]));
                        }
                        let _ = handle.await;
                    })
                }
                #[cfg(feature = "async-std")]
                {
                    async_std::task::block_on(async {
                        for i in 0..NUM_MSGS {
                            let _ = actor_ref.cast(Msg256B([i; 32]));
                        }
                        let _ = handle.await;
                    })
                }
            },
            BatchSize::PerIteration,
        );
    });

    group.finish();
}

// ============================================================================
// 2. SPAWN THROUGHPUT BENCHMARK
// ============================================================================

fn spawn_throughput(c: &mut Criterion) {
    struct MinimalActor;

    #[cfg_attr(feature = "async-trait", ractor::async_trait)]
    impl Actor for MinimalActor {
        type Msg = ();
        type State = ();
        type Arguments = ();

        async fn pre_start(
            &self,
            myself: ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            myself.stop(None);
            Ok(())
        }

        async fn handle(
            &self,
            _myself: ActorRef<Self::Msg>,
            _message: Self::Msg,
            _state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            Ok(())
        }
    }

    #[cfg(not(feature = "async-std"))]
    let runtime = tokio::runtime::Builder::new_multi_thread().build().unwrap();

    let mut group = c.benchmark_group("spawn_throughput");

    for count in [10, 100, 1000] {
        group.throughput(Throughput::Elements(count));
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            b.iter_batched(
                || {},
                |_| {
                    #[cfg(not(feature = "async-std"))]
                    {
                        runtime.block_on(async {
                            let mut handles = Vec::with_capacity(count as usize);
                            for _ in 0..count {
                                let (_, handle) = Actor::spawn(None, MinimalActor, ())
                                    .await
                                    .expect("Failed to spawn");
                                handles.push(handle);
                            }
                            for handle in handles {
                                let _ = handle.await;
                            }
                        })
                    }
                    #[cfg(feature = "async-std")]
                    {
                        async_std::task::block_on(async {
                            let mut handles = Vec::with_capacity(count as usize);
                            for _ in 0..count {
                                let (_, handle) = Actor::spawn(None, MinimalActor, ())
                                    .await
                                    .expect("Failed to spawn");
                                handles.push(handle);
                            }
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

    group.finish();
}

criterion_group!(
    advanced_benches,
    small_message_workload,
    large_message_workload,
    spawn_throughput
);
criterion_main!(advanced_benches);
