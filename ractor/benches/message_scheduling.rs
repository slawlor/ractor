// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Steady-state local messaging with explicit Tokio placement and channel controls.
//! See docs/message-scheduling.md for methodology and profiling commands.

#[cfg(all(not(feature = "async-std"), not(target_arch = "wasm32")))]
mod scheduling {
    use std::hint::black_box;
    use std::thread;
    use std::time::Duration;

    use criterion::{Criterion, Throughput};
    use ractor::{Actor, ActorCell, ActorProcessingErr, ActorRef};
    use tokio::runtime::{Builder, Handle, Runtime};
    use tokio::sync::{mpsc, oneshot};
    use tokio::task::JoinHandle;

    const LANES: usize = 4;
    const MESSAGES: usize = 65_536;

    #[derive(Clone, Copy)]
    enum Placement {
        CurrentThread,
        Multi(usize),
        Shards { crossed: bool },
    }

    impl Placement {
        fn name(self) -> String {
            match self {
                Self::CurrentThread => "current_thread".into(),
                Self::Multi(workers) => format!("multi_{workers}"),
                Self::Shards { crossed: false } => "shards_4_colocated".into(),
                Self::Shards { crossed: true } => "shards_4_crossed".into(),
            }
        }
    }

    struct Executors {
        driver: Runtime,
        handles: Vec<Handle>,
        threads: Vec<(oneshot::Sender<()>, thread::JoinHandle<()>)>,
        crossed: bool,
    }

    impl Executors {
        fn new(placement: Placement) -> Self {
            let driver = match placement {
                Placement::Multi(workers) => Builder::new_multi_thread()
                    .worker_threads(workers)
                    .enable_all()
                    .build()
                    .unwrap(),
                _ => Builder::new_current_thread().enable_all().build().unwrap(),
            };
            let mut handles = Vec::new();
            let mut threads = Vec::new();
            if matches!(placement, Placement::Shards { .. }) {
                for lane in 0..LANES {
                    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
                    handles.push(runtime.handle().clone());
                    let (stop, stopped) = oneshot::channel();
                    let thread = thread::Builder::new()
                        .name(format!("ractor-shard-{lane}"))
                        .spawn(move || runtime.block_on(async { stopped.await.unwrap() }))
                        .unwrap();
                    threads.push((stop, thread));
                }
            } else {
                handles.resize(LANES, driver.handle().clone());
            }
            Self {
                driver,
                handles,
                threads,
                crossed: matches!(placement, Placement::Shards { crossed: true }),
            }
        }

        fn source(&self, lane: usize, independent: bool) -> &Handle {
            let destination = if independent { lane } else { 0 };
            &self.handles[if self.crossed {
                if independent {
                    (lane + 1) % LANES
                } else {
                    lane
                }
            } else {
                destination
            }]
        }
    }

    impl Drop for Executors {
        fn drop(&mut self) {
            for (stop, thread) in self.threads.drain(..) {
                stop.send(()).unwrap();
                thread.join().unwrap();
            }
        }
    }

    struct Work(u64);
    #[cfg(feature = "cluster")]
    impl ractor::Message for Work {}

    struct Counter {
        batch_size: usize,
        done: mpsc::UnboundedSender<u64>,
    }

    #[cfg_attr(feature = "async-trait", ractor::async_trait)]
    impl Actor for Counter {
        type Msg = Work;
        type State = (usize, u64);
        type Arguments = ();

        async fn pre_start(
            &self,
            _: ActorRef<Work>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok((0, 0))
        }

        async fn handle(
            &self,
            _: ActorRef<Work>,
            message: Work,
            state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            state.0 += 1;
            state.1 = state.1.wrapping_add(message.0);
            if state.0 == self.batch_size {
                self.done.send(black_box(state.1)).unwrap();
                *state = (0, 0);
            }
            Ok(())
        }
    }

    #[derive(Clone)]
    enum Sink {
        Actor(ActorRef<Work>),
        Channel(mpsc::UnboundedSender<u64>),
    }

    impl Sink {
        fn send(&self, value: u64) {
            match self {
                Self::Actor(actor) => actor.cast(Work(value)).unwrap(),
                Self::Channel(sender) => sender.send(value).unwrap(),
            }
        }
    }

    struct Mailboxes {
        commands: Vec<mpsc::UnboundedSender<()>>,
        done: mpsc::UnboundedReceiver<u64>,
        actors: Vec<ActorCell>,
        tasks: Vec<JoinHandle<()>>,
        consumers: usize,
        checksum: u64,
    }

    impl Mailboxes {
        async fn new(
            executors: &Executors,
            producers: usize,
            independent: bool,
            raw: bool,
        ) -> Self {
            let per_producer = MESSAGES / producers;
            let consumers = if independent { producers } else { 1 };
            let per_consumer = MESSAGES / consumers;
            let (done_tx, done) = mpsc::unbounded_channel();
            let mut actors = Vec::new();
            let mut tasks = Vec::new();
            let mut sinks = Vec::new();
            for lane in 0..consumers {
                let done = done_tx.clone();
                if raw {
                    let (sender, mut receiver) = mpsc::unbounded_channel::<u64>();
                    tasks.push(executors.handles[lane].spawn(async move {
                        let mut seen = 0;
                        let mut checksum = 0u64;
                        while let Some(value) = receiver.recv().await {
                            seen += 1;
                            checksum = checksum.wrapping_add(value);
                            if seen == per_consumer {
                                done.send(black_box(checksum)).unwrap();
                                seen = 0;
                                checksum = 0;
                            }
                        }
                    }));
                    sinks.push(Sink::Channel(sender));
                } else {
                    let (actor, task) = executors.handles[lane]
                        .spawn(async move {
                            Actor::spawn(
                                None,
                                Counter {
                                    batch_size: per_consumer,
                                    done,
                                },
                                (),
                            )
                            .await
                            .unwrap()
                        })
                        .await
                        .unwrap();
                    actors.push(actor.get_cell());
                    tasks.push(task);
                    sinks.push(Sink::Actor(actor));
                }
            }
            let mut commands = Vec::new();
            for lane in 0..producers {
                let sink = sinks[if independent { lane } else { 0 }].clone();
                let (command, mut receive_command) = mpsc::unbounded_channel();
                commands.push(command);
                tasks.push(executors.source(lane, independent).spawn(async move {
                    while receive_command.recv().await.is_some() {
                        for value in 0..per_producer as u64 {
                            sink.send(black_box(value));
                        }
                    }
                }));
            }
            let checksum = (per_producer * (per_producer - 1) / 2 * (producers / consumers)) as u64;
            Self {
                commands,
                done,
                actors,
                tasks,
                consumers,
                checksum,
            }
        }

        async fn batch(&mut self) {
            for command in &self.commands {
                command.send(()).unwrap();
            }
            for _ in 0..self.consumers {
                assert_eq!(self.done.recv().await.unwrap(), self.checksum);
            }
        }

        async fn shutdown(self) {
            drop(self.commands);
            for actor in self.actors {
                actor.stop(None);
            }
            for task in self.tasks {
                task.await.unwrap();
            }
        }
    }

    enum Ping {
        Peer(ActorRef<Ping>),
        Token(u64),
    }
    #[cfg(feature = "cluster")]
    impl ractor::Message for Ping {}

    struct Peer {
        done: mpsc::UnboundedSender<()>,
    }

    #[cfg_attr(feature = "async-trait", ractor::async_trait)]
    impl Actor for Peer {
        type Msg = Ping;
        type State = Option<ActorRef<Ping>>;
        type Arguments = ();

        async fn pre_start(
            &self,
            _: ActorRef<Ping>,
            _: (),
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(None)
        }

        async fn handle(
            &self,
            _: ActorRef<Ping>,
            message: Ping,
            peer: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            match message {
                Ping::Peer(actor) => {
                    *peer = Some(actor);
                    self.done.send(()).unwrap();
                }
                Ping::Token(1) => self.done.send(()).unwrap(),
                Ping::Token(remaining) => {
                    peer.as_ref().unwrap().cast(Ping::Token(remaining - 1))?;
                }
            }
            Ok(())
        }
    }

    struct Pairs {
        starters: Vec<ActorRef<Ping>>,
        done: mpsc::UnboundedReceiver<()>,
        actors: Vec<ActorCell>,
        tasks: Vec<JoinHandle<()>>,
    }

    impl Pairs {
        async fn new(executors: &Executors) -> Self {
            let (done_tx, mut done) = mpsc::unbounded_channel();
            let mut actors = Vec::new();
            let mut tasks = Vec::new();
            let mut starters = Vec::new();
            for lane in 0..LANES {
                let mut pair = Vec::new();
                for side in 0..2 {
                    let destination = (lane + usize::from(executors.crossed) * side) % LANES;
                    let done = done_tx.clone();
                    let (actor, task) = executors.handles[destination]
                        .spawn(async move { Actor::spawn(None, Peer { done }, ()).await.unwrap() })
                        .await
                        .unwrap();
                    actors.push(actor.get_cell());
                    tasks.push(task);
                    pair.push(actor);
                }
                pair[0].cast(Ping::Peer(pair[1].clone())).unwrap();
                pair[1].cast(Ping::Peer(pair[0].clone())).unwrap();
                starters.push(pair.remove(0));
            }
            for _ in 0..LANES * 2 {
                done.recv().await.unwrap();
            }
            Self {
                starters,
                done,
                actors,
                tasks,
            }
        }

        async fn batch(&mut self) {
            for actor in &self.starters {
                actor.cast(Ping::Token((MESSAGES / LANES) as u64)).unwrap();
            }
            for _ in 0..LANES {
                self.done.recv().await.unwrap();
            }
        }

        async fn shutdown(self) {
            for actor in self.actors {
                actor.stop(None);
            }
            for task in self.tasks {
                task.await.unwrap();
            }
        }
    }

    pub fn benchmarks(c: &mut Criterion) {
        let placements = [
            Placement::CurrentThread,
            Placement::Multi(1),
            Placement::Multi(4),
            Placement::Multi(16),
            Placement::Shards { crossed: false },
            Placement::Shards { crossed: true },
        ];
        for (name, producers, independent) in [
            ("single", 1, false),
            ("fan_in_4", LANES, false),
            ("independent_4", LANES, true),
        ] {
            for raw in [false, true] {
                let implementation = if raw { "tokio_channel" } else { "actor" };
                let mut group = c.benchmark_group(format!("{implementation}/{name}"));
                group.throughput(Throughput::Elements(MESSAGES as u64));
                for placement in placements {
                    let executors = Executors::new(placement);
                    let mut mailboxes = executors.driver.block_on(Mailboxes::new(
                        &executors,
                        producers,
                        independent,
                        raw,
                    ));
                    group.bench_function(placement.name(), |b| {
                        b.iter(|| executors.driver.block_on(mailboxes.batch()));
                    });
                    executors.driver.block_on(mailboxes.shutdown());
                }
                group.finish();
            }
        }
        let mut group = c.benchmark_group("actor/ping_pong_4");
        group.throughput(Throughput::Elements(MESSAGES as u64));
        for placement in placements {
            let executors = Executors::new(placement);
            let mut pairs = executors.driver.block_on(Pairs::new(&executors));
            group.bench_function(placement.name(), |b| {
                b.iter(|| executors.driver.block_on(pairs.batch()));
            });
            executors.driver.block_on(pairs.shutdown());
        }
        group.finish();
    }

    pub fn config() -> Criterion {
        Criterion::default()
            .sample_size(30)
            .warm_up_time(Duration::from_secs(1))
            .measurement_time(Duration::from_secs(3))
    }
}

#[cfg(all(not(feature = "async-std"), not(target_arch = "wasm32")))]
criterion::criterion_group! {
    name = benches;
    config = scheduling::config();
    targets = scheduling::benchmarks
}
#[cfg(all(not(feature = "async-std"), not(target_arch = "wasm32")))]
criterion::criterion_main!(benches);

#[cfg(any(feature = "async-std", target_arch = "wasm32"))]
fn main() {}
