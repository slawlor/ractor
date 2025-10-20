// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Benchmarks for specifically keeping large data on the stack. If the future
//! doesn't get boxed, this measures the relative performance impact
//!
//! Comparison of
//! `cargo bench --bench async_traits -p ractor --no-default-features -F tokio_runtime`
//! against
//! `cargo bench --bench async_traits -p ractor --no-default-features -F tokio_runtime,async-trait`

#[macro_use]
extern crate criterion;

use criterion::BatchSize;
use criterion::Criterion;
use ractor::Actor;
use ractor::ActorProcessingErr;
use ractor::ActorRef;
#[cfg(feature = "cluster")]
use ractor::Message;

#[allow(clippy::async_yields_async)]
fn big_stack_futures(c: &mut Criterion) {
    const NUM_MSGS: usize = 50;
    const NUM_BYTES: usize = 50_000;

    struct LargeFutureActor {
        num_msgs: usize,
    }

    struct LargeFutureActorState {
        cmsg: usize,
        data: [u64; NUM_BYTES],
    }

    struct LargeFutureActorMessage;
    #[cfg(feature = "cluster")]
    impl Message for LargeFutureActorMessage {}

    #[cfg_attr(feature = "async-trait", ractor::async_trait)]
    impl Actor for LargeFutureActor {
        type Msg = LargeFutureActorMessage;

        type State = LargeFutureActorState;

        type Arguments = ();

        async fn pre_start(
            &self,
            myself: ActorRef<Self::Msg>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            let _ = myself.cast(LargeFutureActorMessage);
            Ok(LargeFutureActorState {
                cmsg: 0usize,
                data: [0; NUM_BYTES],
            })
        }

        async fn handle(
            &self,
            myself: ActorRef<Self::Msg>,
            _message: Self::Msg,
            state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            state.cmsg += 1;
            state.data[state.cmsg] = state.cmsg as u64;
            if state.cmsg >= self.num_msgs {
                myself.stop(None);
            } else {
                let _ = myself.cast(LargeFutureActorMessage);
            }
            Ok(())
        }
    }

    let id =
        format!("Waiting on {NUM_MSGS} messages with large data in the Future to be processed");
    #[cfg(not(feature = "async-std"))]
    let runtime = tokio::runtime::Builder::new_multi_thread().build().unwrap();
    #[cfg(feature = "async-std")]
    async_std::task::block_on(async {});
    c.bench_function(&id, move |b| {
        b.iter_batched(
            || {
                #[cfg(not(feature = "async-std"))]
                {
                    runtime.block_on(async move {
                        let (_, handle) =
                            Actor::spawn(None, LargeFutureActor { num_msgs: NUM_MSGS }, ())
                                .await
                                .expect("Failed to create test actor");
                        handle
                    })
                }
                #[cfg(feature = "async-std")]
                {
                    async_std::task::block_on(async move {
                        let (_, handle) =
                            Actor::spawn(None, LargeFutureActor { num_msgs: NUM_MSGS }, ())
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

criterion_group! {
    name=async_traits;
    config = Criterion::default()
        .sample_size(100);
    targets=big_stack_futures
}
criterion_main!(async_traits);
