// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Benchmarks for factory message dispatch with varying worker pool sizes.
//!
//! Measures raw, stable-state dispatch throughput — the factory and its
//! entire worker pool are spawned once and reused across all criterion
//! iterations so that only message routing + processing is timed.

#[macro_use]
extern crate criterion;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use criterion::{BenchmarkId, Criterion, Throughput};

use ractor::factory::queues::DefaultQueue;
use ractor::factory::routing::QueuerRouting;
use ractor::factory::*;
use ractor::Actor;
use ractor::ActorProcessingErr;
use ractor::ActorRef;

#[cfg(feature = "cluster")]
use ractor::Message;

// ============ Types ============

#[derive(Debug, Hash, Clone, Eq, PartialEq)]
struct BenchKey {
    id: u64,
}
#[cfg(feature = "cluster")]
impl ractor::BytesConvertable for BenchKey {
    fn from_bytes(bytes: Vec<u8>) -> Self {
        Self {
            id: u64::from_bytes(bytes),
        }
    }
    fn into_bytes(self) -> Vec<u8> {
        self.id.into_bytes()
    }
}

#[derive(Debug)]
struct BenchMessage;
#[cfg(feature = "cluster")]
impl Message for BenchMessage {}

type BenchQueue = DefaultQueue<BenchKey, BenchMessage>;

// ============ Worker ============

struct BenchWorker {
    counter: Arc<AtomicU64>,
}

#[cfg_attr(feature = "async-trait", ractor::async_trait)]
impl Worker for BenchWorker {
    type Key = BenchKey;
    type Message = BenchMessage;
    type State = ();
    type Arguments = ();

    async fn pre_start(
        &self,
        _wid: WorkerId,
        _factory: &ActorRef<FactoryMessage<BenchKey, BenchMessage>>,
        _args: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(())
    }

    async fn handle(
        &self,
        _wid: WorkerId,
        _factory: &ActorRef<FactoryMessage<BenchKey, BenchMessage>>,
        Job { key, .. }: Job<Self::Key, Self::Message>,
        _state: &mut Self::State,
    ) -> Result<BenchKey, ActorProcessingErr> {
        self.counter.fetch_add(1, Ordering::Relaxed);
        Ok(key)
    }
}

// ============ Worker builder ============

struct BenchWorkerBuilder {
    counter: Arc<AtomicU64>,
}

impl WorkerBuilder<BenchWorker, ()> for BenchWorkerBuilder {
    fn build(&mut self, _wid: usize) -> (BenchWorker, ()) {
        (
            BenchWorker {
                counter: self.counter.clone(),
            },
            (),
        )
    }
}

// ============ Helpers ============

/// Spawn a factory with `num_workers` workers, returning the factory ref and
/// a shared counter that tracks how many jobs have been processed.
async fn spawn_factory(
    num_workers: usize,
) -> (
    ActorRef<FactoryMessage<BenchKey, BenchMessage>>,
    ractor::concurrency::JoinHandle<()>,
    Arc<AtomicU64>,
) {
    let counter = Arc::new(AtomicU64::new(0));
    let worker_builder = BenchWorkerBuilder {
        counter: counter.clone(),
    };

    let factory_definition = Factory::<
        BenchKey,
        BenchMessage,
        (),
        BenchWorker,
        QueuerRouting<BenchKey, BenchMessage>,
        BenchQueue,
    >::default();

    let (factory, factory_handle) = Actor::spawn(
        None,
        factory_definition,
        FactoryArguments::builder()
            .num_initial_workers(num_workers)
            .queue(BenchQueue::default())
            .router(QueuerRouting::default())
            .worker_builder(Box::new(worker_builder) as Box<dyn WorkerBuilder<BenchWorker, ()>>)
            .build(),
    )
    .await
    .expect("Failed to spawn factory");

    (factory, factory_handle, counter)
}

// ============ Benchmark ============

fn factory_queuer_dispatch(c: &mut Criterion) {
    const NUM_MESSAGES: u64 = 50_000;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let mut group = c.benchmark_group("factory_queuer_dispatch");
    group.throughput(Throughput::Elements(NUM_MESSAGES));

    for num_workers in [100, 1_000, 5_000, 10_000] {
        // Spawn the factory ONCE for this pool size — outside the
        // measurement loop so worker creation cost is excluded.
        let (factory, _factory_handle, counter) =
            runtime.block_on(async { spawn_factory(num_workers).await });

        // Warm up: send one batch through so every worker has started
        // its processing loop and the runtime's thread pool is hot.
        runtime.block_on(async {
            for id in 0..NUM_MESSAGES {
                factory
                    .cast(FactoryMessage::Dispatch(Job {
                        key: BenchKey { id },
                        msg: BenchMessage,
                        options: JobOptions::default(),
                        accepted: None,
                    }))
                    .expect("Failed to dispatch job");
            }
            while counter.load(Ordering::Relaxed) < NUM_MESSAGES {
                tokio::task::yield_now().await;
            }
        });

        group.bench_with_input(
            BenchmarkId::new("workers", num_workers),
            &num_workers,
            |b, _| {
                b.iter(|| {
                    runtime.block_on(async {
                        // Reset counter for this iteration
                        let base = counter.load(Ordering::Relaxed);
                        let target = base + NUM_MESSAGES;

                        for id in 0..NUM_MESSAGES {
                            factory
                                .cast(FactoryMessage::Dispatch(Job {
                                    key: BenchKey { id },
                                    msg: BenchMessage,
                                    options: JobOptions::default(),
                                    accepted: None,
                                }))
                                .expect("Failed to dispatch job");
                        }

                        while counter.load(Ordering::Relaxed) < target {
                            tokio::task::yield_now().await;
                        }
                    })
                });
            },
        );

        // Tear down this factory before moving to the next pool size.
        runtime.block_on(async {
            factory.stop(None);
            let _ = _factory_handle.await;
        });
    }

    group.finish();
}

criterion_group!(factory_benches, factory_queuer_dispatch);
criterion_main!(factory_benches);
