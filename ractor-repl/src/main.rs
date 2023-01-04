// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

mod ping_pong;

// MAIN //
#[tokio::main]
async fn main() {
    let actor_handler = ping_pong::PingPong;
    let (agent, ports) = ractor::Actor::new(None, actor_handler);
    let (_actor_ref, actor_handle) = agent.start(ports, None).await.unwrap();
    actor_handle.await.unwrap();
}
