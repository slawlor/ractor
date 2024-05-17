// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! An implementation of the Chandy & Misra solution to the classic finite state machine (FSM)
//! concurrency problem known as [Dining Philosophers]
//! (https://en.wikipedia.org/wiki/Dining_philosophers_problem) problem using `ractor`.
//!
//! Utilizes:
//! * Multiple actors, inter-communicating
//! * RPCs
//! * Mutable state transitions
//!
//! Run this example with
//!
//! ```bash
//! cargo run --example philosophers
//! ```

#![allow(clippy::incompatible_msrv)]

use std::collections::{HashMap, VecDeque};

use ractor::{cast, Actor, ActorId, ActorName, ActorProcessingErr, ActorRef, RpcReplyPort};
use tokio::time::{Duration, Instant};

// ============================ Fork Actor ============================ //

enum ForkMessage {
    /// Request the fork be sent to a philosopher
    RequestFork(ActorRef<PhilosopherMessage>),
    /// Mark the fork as currently being used
    UsingFork(ActorId),
    /// Sent to a fork to indicate that it was put down and no longer is in use. This will
    /// allow the fork to be sent to the next user.
    PutForkDown(ActorId),
}
#[cfg(feature = "cluster")]
impl ractor::Message for ForkMessage {}

struct ForkState {
    /// Flag to identify if the fork is clean or not
    clean: bool,
    /// The actor who currently owns the fork
    owned_by: Option<ActorRef<PhilosopherMessage>>,
    // A backlog of messages which get queue'd up in a state transition
    backlog: VecDeque<ForkMessage>,
}

struct Fork;

impl Fork {
    fn handle_internal(
        &self,
        myself: &ActorRef<ForkMessage>,
        message: ForkMessage,
        state: &mut ForkState,
    ) -> Option<ForkMessage> {
        match &message {
            ForkMessage::RequestFork(who) => {
                match &state.owned_by {
                    Some(owner) => {
                        if !state.clean {
                            let _ = cast!(owner, PhilosopherMessage::GiveUpFork(myself.get_id()));
                        }
                        // there's already an owner, backlog this message in priority
                        return Some(message);
                    }
                    None => {
                        // give the fork to the requester
                        let _ = cast!(who, PhilosopherMessage::ReceiveFork(myself.get_id()));
                        // set ownership
                        state.owned_by = Some(who.clone());
                    }
                }
            }
            ForkMessage::UsingFork(who) => match &state.owned_by {
                Some(owner) if owner.get_id() == *who => {
                    state.clean = false;
                }
                Some(other_owner) => {
                    tracing::info!(
                        "ERROR Received `UsingFork` from {:?}. Real owner is {:?}",
                        who,
                        other_owner.get_name().unwrap()
                    );
                }
                None => {
                    tracing::info!("ERROR Received `UsingFork` from {who:?}. Real owner is `None`");
                }
            },
            ForkMessage::PutForkDown(who) => match &state.owned_by {
                Some(owner) if owner.get_id() == *who => {
                    state.owned_by = None;
                    state.clean = true;
                }
                Some(other_owner) => {
                    tracing::info!(
                        "ERROR Received `PutForkDown` from {:?}. Real owner is {:?}",
                        who,
                        other_owner.get_name().unwrap()
                    );
                }
                None => {
                    tracing::info!(
                        "ERROR Received `PutForkDown` from {who:?}. Real owner is `None`"
                    );
                }
            },
        }
        None
    }
}

#[cfg_attr(feature = "async-trait", ractor::async_trait)]
impl Actor for Fork {
    type Msg = ForkMessage;
    type State = ForkState;
    type Arguments = ();
    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        _: (),
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(Self::State {
            clean: false,
            owned_by: None,
            backlog: VecDeque::new(),
        })
    }

    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        let mut maybe_unhandled = self.handle_internal(&myself, message, state);
        if let Some(message) = maybe_unhandled {
            state.backlog.push_back(message);
        } else {
            // we handled the message, check the queue for any work to dequeue and handle
            while !state.backlog.is_empty() && maybe_unhandled.is_none() {
                let head = state.backlog.pop_front().unwrap();
                maybe_unhandled = self.handle_internal(&myself, head, state);
            }
            // put the first unhandled msg back to the front of the queue
            if let Some(msg) = maybe_unhandled {
                state.backlog.push_front(msg);
            }
        }
        Ok(())
    }
}

// ============================ Philosopher Actor ============================ //

#[derive(PartialEq, Eq)]
enum PhilosopherMode {
    /// The philosopher is thinking
    Thinking,
    /// The philosopher is hungry and waiting for one of the forks
    Hungry,
    /// The philosopher is eating
    Eating,
}

struct PhilosophersFork {
    /// The pointer to the fork actor
    fork: ActorRef<ForkMessage>,
    /// Does this philosopher currently have this fork?
    has: bool,
    /// Has the philosopher requested this fork?
    requested: bool,
}

#[derive(Clone, Debug)]
struct PhilosopherMetrics {
    /// The number of state changes that have occurred.
    state_change_count: u16,
    /// The number of times a Philosopher failed to eat because he didn't have both forks.
    failed_to_eat: u16,
    /// The time that the Philosopher spent thinking.
    time_thinking: Duration,
    /// The time that the Philosopher spent hungry.
    time_hungry: Duration,
    /// The time that the Philosopher spent eating.
    time_eating: Duration,
}

struct PhilosopherState {
    /// The current mode/state the philosopher is in
    mode: PhilosopherMode,
    /// The fork to the left of the Philosopher
    left: PhilosophersFork,
    /// The fork to the right of the Philosopher
    right: PhilosophersFork,
    /// The last time the philosopher's state changed. Tracking time eating, etc
    last_state_change: Instant,
    /// The metrics of this actor
    metrics: PhilosopherMetrics,
    /// time-slice
    time_slice: Duration,
}

impl PhilosopherState {
    fn new(
        left: ActorRef<ForkMessage>,
        right: ActorRef<ForkMessage>,
        time_slice: Duration,
    ) -> Self {
        Self {
            mode: PhilosopherMode::Thinking,
            left: PhilosophersFork {
                fork: left,
                has: false,
                requested: false,
            },
            right: PhilosophersFork {
                fork: right,
                has: false,
                requested: false,
            },
            last_state_change: Instant::now(),
            metrics: PhilosopherMetrics {
                state_change_count: 0,
                failed_to_eat: 0,
                time_thinking: Duration::from_micros(0),
                time_hungry: Duration::from_micros(0),
                time_eating: Duration::from_micros(0),
            },
            time_slice,
        }
    }
}

enum PhilosopherMessage {
    /// Command to stop eating. Note that since the `StopEating` message is sent as a scheduled message
    /// it may arrive after the philosopher has already changed state. For this reason we track
    /// the state change count and compare it with the number in the message.
    StopEating(u16),
    /// Command to stop eating. Note that since the `BecomeHungry` message is sent as a scheduled message
    /// it may arrive after the philosopher has already changed state. For this reason we track
    /// the state change count and compare it with the number in the message.
    BecomeHungry(u16),
    /// Instructs the philosopher to give up the fork
    GiveUpFork(ActorId),
    /// Instructs the philosopher they've received the specified fork
    ReceiveFork(ActorId),
    SendMetrics(RpcReplyPort<PhilosopherMetrics>),
}
#[cfg(feature = "cluster")]
impl ractor::Message for PhilosopherMessage {}

struct PhilosopherArguments {
    time_slice: Duration,
    left: ActorRef<ForkMessage>,
    right: ActorRef<ForkMessage>,
}

struct Philosopher;

impl Philosopher {
    /// Helper method to set the internal state to begin thinking
    fn begin_thinking(&self, myself: &ActorRef<PhilosopherMessage>, state: &mut PhilosopherState) {
        state.mode = PhilosopherMode::Thinking;
        state.metrics.state_change_count += 1;
        state.metrics.time_eating += Instant::elapsed(&state.last_state_change);
        state.last_state_change = Instant::now();

        // schedule become hungry after the thinking time has elapsed
        let metrics_count = state.metrics.state_change_count;
        #[allow(clippy::let_underscore_future)]
        let _ = myself.send_after(state.time_slice, move || {
            PhilosopherMessage::BecomeHungry(metrics_count)
        });
    }

    /// Helper command to set the internal state to begin eating
    fn begin_eating(&self, myself: &ActorRef<PhilosopherMessage>, state: &mut PhilosopherState) {
        state.metrics.time_hungry += Instant::elapsed(&state.last_state_change);
        state.last_state_change = Instant::now();
        state.mode = PhilosopherMode::Eating;
        state.metrics.state_change_count += 1;

        // Now that we are eating we will tell the fork that we are using it,
        // thus marking the fork as dirty.
        let _ = state
            .left
            .fork
            .cast(ForkMessage::UsingFork(myself.get_id()));
        let _ = state
            .right
            .fork
            .cast(ForkMessage::UsingFork(myself.get_id()));

        // schedule stop eating after the eating time has elapsed
        let metrics_count = state.metrics.state_change_count;
        #[allow(clippy::let_underscore_future)]
        let _ = myself.send_after(state.time_slice, move || {
            PhilosopherMessage::StopEating(metrics_count)
        });
    }

    /// Helper command to request any forks which are missing
    fn request_missing_forks(
        &self,
        myself: &ActorRef<PhilosopherMessage>,
        state: &mut PhilosopherState,
    ) {
        if !state.left.has && !state.left.requested {
            state.left.requested = true;
            let _ = state
                .left
                .fork
                .cast(ForkMessage::RequestFork(myself.clone()));
        }
        if !state.right.has && !state.right.requested {
            state.right.requested = true;
            let _ = state
                .right
                .fork
                .cast(ForkMessage::RequestFork(myself.clone()));
        }
    }
}

#[cfg_attr(feature = "async-trait", ractor::async_trait)]
impl Actor for Philosopher {
    type Msg = PhilosopherMessage;
    type State = PhilosopherState;
    type Arguments = PhilosopherArguments;
    async fn pre_start(
        &self,
        myself: ActorRef<Self::Msg>,
        args: PhilosopherArguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        // initialize the simulation by making the philosopher's hungry
        let _ = cast!(myself, Self::Msg::BecomeHungry(0));
        Ok(Self::State::new(args.left, args.right, args.time_slice))
    }

    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            PhilosopherMessage::SendMetrics(reply) => {
                let _ = reply.send(state.metrics.clone());
            }
            PhilosopherMessage::StopEating(state_id) => {
                // Processes a command to stop eating.
                if state.metrics.state_change_count == state_id
                    && state.mode == PhilosopherMode::Eating
                {
                    self.begin_thinking(&myself, state);
                }
            }
            PhilosopherMessage::BecomeHungry(state_id) => {
                // The philosopher is being instructed to get hungry which will cause them to ask for the
                // forks to eat.
                if state.metrics.state_change_count == state_id {
                    if state.left.has && state.right.has {
                        // we have both forks, starting eating
                        self.begin_eating(&myself, state);
                    } else {
                        // we're missing some forks, maybe request the forks we need?
                        match state.mode {
                            PhilosopherMode::Thinking => {
                                state.metrics.time_thinking +=
                                    Instant::elapsed(&state.last_state_change);
                                state.last_state_change = Instant::now();
                                state.mode = PhilosopherMode::Hungry;
                                state.metrics.state_change_count += 1;
                                self.request_missing_forks(&myself, state);
                            }
                            PhilosopherMode::Hungry => {
                                tracing::info!(
                                    "ERROR: {} Got `BecomeHungry` while hungry!",
                                    myself.get_name().unwrap()
                                );
                            }
                            PhilosopherMode::Eating => {
                                tracing::info!(
                                    "ERROR: {} Got `BecomeHungry` while eating!",
                                    myself.get_name().unwrap()
                                );
                            }
                        }
                    }
                }
            }
            PhilosopherMessage::GiveUpFork(fork) => {
                // Processes a command to a philosopher to give up a fork. Note that this can be received
                // when the philosopher is in any state since the philosopher will not put down a fork
                // unless he is asked to. A philosopher can be eating, stop eating and start thinking
                // and then start eating again if no one asked for his forks. The fork actor is the only
                // actor sending this message and it will only do so if the fork is dirty.
                if state.left.fork.get_id() == fork {
                    if state.left.has {
                        state.left.has = false;
                        let _ = state
                            .left
                            .fork
                            .cast(ForkMessage::PutForkDown(myself.get_id()));
                    }
                } else if state.right.fork.get_id() == fork {
                    if state.right.has {
                        state.right.has = false;
                        let _ = state
                            .right
                            .fork
                            .cast(ForkMessage::PutForkDown(myself.get_id()));
                    }
                } else {
                    tracing::info!(
                        "ERROR: {} received a `GiveUpFork` from an unknown fork!",
                        myself.get_name().unwrap()
                    );
                }
                match state.mode {
                    PhilosopherMode::Hungry => {
                        state.metrics.failed_to_eat += 1;
                        self.begin_thinking(&myself, state);
                    }
                    PhilosopherMode::Eating => {
                        self.begin_thinking(&myself, state);
                    }
                    _ => {
                        // already thinking
                    }
                }
            }
            PhilosopherMessage::ReceiveFork(fork) => {
                // The philosopher received a fork. Once they have both forks they can start eating.
                // Otherwise they have to wait for the other fork to begin eating.
                if state.left.fork.get_id() == fork {
                    state.left.has = true;
                    state.left.requested = false;
                } else if state.right.fork.get_id() == fork {
                    state.right.has = true;
                    state.right.requested = false;
                } else {
                    tracing::info!(
                        "ERROR: {} received a `ReceiveFork` from an unknown fork!",
                        myself.get_name().unwrap()
                    );
                }

                // if we have both forks, we can start eating
                if state.left.has && state.right.has {
                    self.begin_eating(&myself, state);
                }
            }
        }
        Ok(())
    }
}

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

#[tokio::main]
async fn main() {
    init_logging();

    // TODO: move configuration to CLAP args
    let time_slice = Duration::from_millis(10);
    let run_time = Duration::from_secs(5);

    let philosopher_names = [
        "Confucius",
        "Descartes",
        "Benjamin Franklin",
        "Socrates",
        "Aristotle",
        "Plato",
        "John Locke",
        "Nietzsche",
        "Karl Marx",
        "Pythagoras",
        "Montesquieu",
    ];
    let mut forks = Vec::with_capacity(philosopher_names.len());
    let mut philosophers = Vec::with_capacity(philosopher_names.len());
    let mut all_handles = tokio::task::JoinSet::new();

    let mut results: HashMap<ActorName, Option<PhilosopherMetrics>> =
        HashMap::with_capacity(philosopher_names.len());

    // create the forks
    for _i in 0..philosopher_names.len() {
        let (fork, handle) = Actor::spawn(None, Fork, ())
            .await
            .expect("Failed to create fork!");
        forks.push(fork);
        all_handles.spawn(handle);
    }

    // Spawn the philosopher actors clockwise from top of the table
    for left in 0..philosopher_names.len() {
        let right = if left == 0 {
            philosopher_names.len() - 1
        } else {
            left - 1
        };
        let p = PhilosopherArguments {
            time_slice,
            left: forks[left].clone(),
            right: forks[right].clone(),
        };
        let (philosopher, handle) =
            Actor::spawn(Some(philosopher_names[left].to_string()), Philosopher, p)
                .await
                .expect("Failed to create philosopher!");
        results.insert(philosopher_names[left].to_string(), None);
        philosophers.push(philosopher);
        all_handles.spawn(handle);
    }

    // wait for the simulation to end
    tokio::time::sleep(run_time).await;
    // collect the metrics from the philosophers, and they'll stop after reporting metrics
    for philosopher in philosophers.iter() {
        let metrics = ractor::call_t!(philosopher, PhilosopherMessage::SendMetrics, 50)
            .expect("Failed to perform RPC");
        results.insert(philosopher.get_name().unwrap(), Some(metrics));
    }

    // cleanup forks & philosophers
    for fork in forks {
        fork.stop(None);
    }
    for philosopher in philosophers {
        philosopher.stop(None);
    }

    // wait for everything to shut down
    while all_handles.join_next().await.is_some() {}

    // print metrics
    tracing::info!("Simulation results");
    for (who, metric) in results {
        tracing::info!("{who}: {metric:?}");
    }
}
