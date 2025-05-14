// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! This is an example of a parallel processing implementation of a Monte-Carlo simulation
//! The simulation is of a basic gambling game adapted from this page:
//! https://towardsdatascience.com/the-house-always-wins-monte-carlo-simulation-eb82787da2a3
//!
//! Demonstrates parallel work and supervision
//!
//! Run this example with
//!
//! ```bash
//! cargo run --example monte_carlo
//! ```

#![allow(clippy::incompatible_msrv)]

use std::collections::HashMap;

use ractor::cast;
use ractor::Actor;
use ractor::ActorId;
use ractor::ActorProcessingErr;
use ractor::ActorRef;
use rand::thread_rng;
use rand::Rng;

// ================== Player Actor ================== //

struct GameState {
    /// The player's current funds. Funds are allowed to be negative since the player
    /// can potentially lose more money than they started with.
    funds: i64,
    /// How much money the player wagers per turn.
    wager: u32,
    /// The total number of game rounds that will be played.
    total_rounds: u32,

    current_round: u32,
    results_vec: Vec<i64>,
}

impl Default for GameState {
    fn default() -> Self {
        Self {
            funds: 10_000,
            wager: 100,
            total_rounds: 100,
            current_round: 1,
            results_vec: vec![],
        }
    }
}

impl GameState {
    /// This function performs a dice roll according to the rules of the simple gambling game.
    /// On average, the player will win their roll 49 out of 100 times, resulting in a house edge
    /// of 2%
    fn roll_dice() -> bool {
        let mut rng = thread_rng();
        matches!(rng.gen_range(0..101), x if x > 51)
    }
}

struct Game;
struct GameMessage(ActorRef<GameManagerMessage>);
#[cfg(feature = "cluster")]
impl ractor::Message for GameMessage {}

#[cfg_attr(
    all(
        feature = "async-trait",
        not(all(target_arch = "wasm32", target_os = "unknown"))
    ),
    ractor::async_trait
)]
#[cfg_attr(
    all(
        feature = "async-trait",
       all(target_arch = "wasm32", target_os = "unknown")
    ),
    ractor::async_trait(?Send)
)]
impl Actor for Game {
    type Msg = GameMessage;

    type State = GameState;

    type Arguments = ();

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        _: (),
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(GameState::default())
    }

    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        if state.current_round <= state.total_rounds {
            state.current_round += 1;
            match Self::State::roll_dice() {
                true => state.funds += state.wager as i64,
                false => state.funds -= state.wager as i64,
            }
            state.results_vec.push(state.funds);
            cast!(myself, message).expect("Failed to send message");
        } else {
            // Now that the game is finished, the results of the game need to be reported
            // to the `GameManager`.
            cast!(
                message.0,
                GameManagerMessage {
                    id: myself.get_id(),
                    results: state.results_vec.clone(),
                }
            )?;
            // Because the `GameManager` is monitoring this actor we can stop this actor
            myself.stop(None);
        }
        Ok(())
    }
}

// ================== Manager Actor ================== //

struct GameManager;

struct GameManagerMessage {
    id: ActorId,
    results: Vec<i64>,
}
#[cfg(feature = "cluster")]
impl ractor::Message for GameManagerMessage {}

struct GameManagerState {
    /// The number of games that have been played so far.
    games_finished: u32,
    /// The total number of games that are to be played.
    total_games: u32,
    /// The results of each finished game, keyed by `Game` actor ID
    results: HashMap<ActorId, Vec<i64>>,
}

impl GameManagerState {
    fn new(total_games: u32) -> Self {
        Self {
            games_finished: 0,
            total_games,
            results: HashMap::new(),
        }
    }
}

#[cfg_attr(
    all(
        feature = "async-trait",
        not(all(target_arch = "wasm32", target_os = "unknown"))
    ),
    ractor::async_trait
)]
#[cfg_attr(
    all(
        feature = "async-trait",
       all(target_arch = "wasm32", target_os = "unknown")
    ),
    ractor::async_trait(?Send)
)]
impl Actor for GameManager {
    type Msg = GameManagerMessage;

    type State = GameManagerState;
    type Arguments = u32;

    async fn pre_start(
        &self,
        myself: ActorRef<Self::Msg>,
        num_games: u32,
    ) -> Result<Self::State, ActorProcessingErr> {
        // This is the first code that will run in the actor. It spawns the Game actors,
        // registers them to its monitoring list, then sends them a message indicating
        // that they should start their games.

        let game_conditions = GameState::default();
        tracing::info!("Starting funds: ${}", game_conditions.funds);
        tracing::info!("Wager per round: ${}", game_conditions.wager);
        tracing::info!("Rounds per game: {}", game_conditions.total_rounds);
        tracing::info!("Running simulations...");
        for _ in 0..num_games {
            let (actor, _) = Actor::spawn_linked(None, Game, (), myself.clone().into())
                .await
                .expect("Failed to start game");
            cast!(actor, GameMessage(myself.clone())).expect("Failed to send message");
        }

        Ok(GameManagerState::new(num_games))
    }

    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        state.results.insert(message.id, message.results);

        state.games_finished += 1;
        if state.games_finished >= state.total_games {
            // Each vec of results contains the entire history of a game for every time that
            // the dice was rolled. Instead of printing out all of that data, we will simply
            // print the average of the funds that the player had at the end of each game.
            let average_funds = state
                .results
                .values()
                .map(|v| v.last().unwrap())
                .sum::<i64>()
                / state.total_games as i64;

            tracing::info!("Simulations ran: {}", state.results.len());
            tracing::info!("Final average funds: ${average_funds}");

            myself.stop(None);
        }
        Ok(())
    }
}

const NUM_GAMES: u32 = 100;

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

#[ractor_example_entry_proc::ractor_example_entry]
async fn main() {
    init_logging();

    // create the supervisor
    let manager = GameManager;
    // spawn it off and wait for it to complete/exit
    let (_actor, handle) = Actor::spawn(None, manager, NUM_GAMES)
        .await
        .expect("Failed to start game manager");

    handle.await.unwrap();
}
