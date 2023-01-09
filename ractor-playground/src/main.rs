// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

mod ping_pong;

use ractor::Actor;

// MAIN //
#[tokio::main]
async fn main() {
    let (_, actor_handle) = Actor::spawn(None, ping_pong::PingPong)
        .await
        .expect("Failed to start actor");
    actor_handle.await.expect("Actor failed to exit cleanly");
}
