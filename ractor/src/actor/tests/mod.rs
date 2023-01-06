// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! General tests, more logic-specific tests are contained in sub-modules

use crate::{Actor, ActorHandler, SpawnErr};

mod supervisor;

#[tokio::test]
async fn test_panic_on_start_captured() {
    struct TestActor;

    #[async_trait::async_trait]
    impl ActorHandler for TestActor {
        type Msg = ();

        type State = ();

        async fn pre_start(&self, _this_actor: crate::ActorCell) -> Self::State {
            panic!("Boom!");
        }
    }

    let actor_output = Actor::spawn(None, TestActor).await;
    assert!(matches!(actor_output, Err(SpawnErr::StartupPanic(_))));
}
