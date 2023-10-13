// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Basic tests of errors, error conversions, etc

use crate::concurrency::Duration;
use crate::Actor;
use crate::ActorCell;
use crate::ActorProcessingErr;
use crate::ActorRef;
use crate::RactorErr;

#[test]
#[cfg_attr(not(target_arch = "wasm32"), tracing_test::traced_test)]
fn test_error_conversions() {
    crate::common_test::setup();

    let spawn = crate::SpawnErr::StartupCancelled;
    let ractor_err = RactorErr::<()>::from(crate::SpawnErr::StartupCancelled);
    assert_eq!(spawn.to_string(), ractor_err.to_string());

    let messaging = crate::MessagingErr::<()>::InvalidActorType;
    let ractor_err = RactorErr::<()>::from(crate::MessagingErr::InvalidActorType);
    assert_eq!(messaging.to_string(), ractor_err.to_string());

    let actor = crate::ActorErr::Cancelled;
    let ractor_err = RactorErr::<()>::from(crate::ActorErr::Cancelled);
    assert_eq!(actor.to_string(), ractor_err.to_string());

    let call_result = crate::rpc::CallResult::<()>::Timeout;
    let other = format!("{:?}", RactorErr::<()>::from(call_result));
    assert_eq!("Timeout".to_string(), other);

    let call_result = crate::rpc::CallResult::<()>::SenderError;
    let other = format!("{}", RactorErr::<()>::from(call_result));
    assert_eq!(
        RactorErr::<()>::from(crate::MessagingErr::ChannelClosed).to_string(),
        other
    );
}

#[crate::concurrency::test]
#[cfg_attr(not(target_arch = "wasm32"), tracing_test::traced_test)]
async fn test_error_message_extraction() {
    crate::common_test::setup();

    struct TestActor;

    #[async_trait::async_trait]
    impl Actor for TestActor {
        type Msg = ();
        type State = ();
        type Arguments = ();

        async fn pre_start(
            &self,
            _: ActorRef<Self::Msg>,
            _: Self::Arguments,
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(())
        }
    }

    let (actor, handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Failed to start test actor");
    // stop the actor, and wait for death which will free the message channels
    actor.stop(None);
    handle.await.unwrap();

    let err = crate::cast!(actor, ()).expect_err("Not an error!");
    assert!(err.has_message());
    assert!(err.try_get_message().is_some());

    let cell: ActorCell = actor.into();
    let bad_message_actor: ActorRef<u32> = cell.into();

    let err = crate::cast!(bad_message_actor, 0u32).expect_err("Not an error!");
    assert!(!err.has_message());
    assert!(err.try_get_message().is_none());
}

#[crate::concurrency::test]
async fn test_platform_sleep_works() {
    crate::common_test::setup();
    crate::concurrency::sleep(Duration::from_millis(100)).await;
    assert!(true);
}
