// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Basic tests of errors, error conversions, etc

use crate::RactorErr;

#[test]
fn test_error_conversions() {
    let spawn = crate::SpawnErr::StartupCancelled;
    let ractor_err = RactorErr::from(crate::SpawnErr::StartupCancelled);
    assert_eq!(spawn.to_string(), ractor_err.to_string());

    let messaging = crate::MessagingErr::InvalidActorType;
    let ractor_err = RactorErr::from(crate::MessagingErr::InvalidActorType);
    assert_eq!(messaging.to_string(), ractor_err.to_string());

    let actor = crate::ActorErr::Cancelled;
    let ractor_err = RactorErr::from(crate::ActorErr::Cancelled);
    assert_eq!(actor.to_string(), ractor_err.to_string());

    let call_result = crate::rpc::CallResult::<()>::Timeout;
    let other = format!("{:?}", RactorErr::from(call_result));
    assert_eq!("Timeout".to_string(), other);

    let call_result = crate::rpc::CallResult::<()>::SenderError;
    let other = format!("{}", RactorErr::from(call_result));
    assert_eq!(
        RactorErr::from(crate::MessagingErr::ChannelClosed).to_string(),
        other
    );
}
