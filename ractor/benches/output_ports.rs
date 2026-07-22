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

struct BenchActorMessage;
#[cfg(feature = "cluster")]
impl Message for BenchActorMessage {}

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

criterion_group!(output_ports, process_output_port_messages);
criterion_main!(output_ports);
