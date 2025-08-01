// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Macro helpers for remote procedure calls

/// `cast!` takes an actor and a message and emits a [crate::RactorErr] error
/// which can be pattern matched on in order to derive the output.
#[macro_export]
macro_rules! cast {
    ($actor:expr, $msg:expr) => {
        $actor.cast($msg).map_err($crate::RactorErr::from)
    };
}

/// `call!`: Perform an infinite-time remote procedure call to an [crate::Actor]
///
/// * `$actor` - The actor to call
/// * `$msg` - The message builder which takes in a [crate::port::RpcReplyPort] and emits a message which
///   the actor supports
/// * `$args` - (optional) Variable length arguments which will PRECEDE the reply channel when
///   constructing the message payload
///
/// Returns [Ok(_)] with the result on successful RPC or [Err(crate::RactorErr)] on failure
/// Example usage (without the `cluster` feature)
/// ```rust
/// use ractor::{call, Actor, RpcReplyPort, ActorRef, ActorProcessingErr};
/// struct TestActor;
/// enum MessageFormat {
///     TestRpc(String, RpcReplyPort<String>),
/// }
/// #[cfg(feature = "cluster")]
/// impl ractor::Message for MessageFormat {}
///
/// #[cfg_attr(feature = "async-trait", ractor::async_trait)]
/// impl Actor for TestActor {
///     type Msg = MessageFormat;
///     type Arguments = ();
///     type State = ();
///
///     async fn pre_start(&self, _this_actor: ActorRef<Self::Msg>, _: ()) -> Result<Self::State, ActorProcessingErr> {
///         Ok(())
///     }
///
///     async fn handle(
///         &self,
///         _this_actor: ActorRef<Self::Msg>,
///         message: Self::Msg,
///         _state: &mut Self::State,
///     ) -> Result<(), ActorProcessingErr> {
///         match message {
///             Self::Msg::TestRpc(arg, reply) => {
///                 // An error sending means no one is listening anymore (the receiver was dropped),
///                 // so we should shortcut the processing of this message probably!
///                 if !reply.is_closed() {
///                     let _ = reply.send(arg);
///                 }
///             }
///         }
///         Ok(())
///     }
/// }
///
/// #[tokio::main]
/// async fn main() {
///     let (actor, handle) = Actor::spawn(None, TestActor, ()).await.unwrap();
///     let result = call!(actor, MessageFormat::TestRpc, "Something".to_string()).unwrap();
///     assert_eq!(result, "Something".to_string());
///     actor.stop(None);
///     handle.await.unwrap();
/// }
/// ```
#[macro_export]
macro_rules! call {
    ($actor:expr, $msg:expr) => {{
        let err = $actor
            .call(|tx| $msg(tx), None)
            .await
            .map_err($crate::RactorErr::from);
        match err {
            Ok($crate::rpc::CallResult::Success(ok_value)) => Ok(ok_value),
            Ok(cr) => Err($crate::RactorErr::from(cr)),
            Err(e) => Err(e),
        }
    }};
    ($actor:expr, $msg:expr, $($args:expr),*) => {{
        let err = $actor
            .call(|tx| $msg($($args),*, tx), None)
            .await
            .map_err($crate::RactorErr::from);
        match err {
            Ok($crate::rpc::CallResult::Success(ok_value)) => Ok(ok_value),
            Ok(cr) => Err($crate::RactorErr::from(cr)),
            Err(e) => Err(e),
        }
    }};
}

/// `call_t!`: Perform an finite-time remote procedure call to an [crate::Actor]
///
/// * `$actor` - The actor to call
/// * `$msg` - The message builder variant
/// * `$timeout_ms` - the timeout in milliseconds for the remote procedure call
/// * `$args` - (optional) Variable length arguments which will PRECEDE the reply channel when
///   constructing the message payload
///
/// Returns [Ok(_)] with the result on successful RPC or [Err(crate::RactorErr)] on failure
///
/// Example usage
/// ```rust
/// use ractor::{call_t, Actor, ActorRef, RpcReplyPort, ActorProcessingErr};
/// struct TestActor;
/// enum MessageFormat {
///     TestRpc(String, RpcReplyPort<String>),
/// }
/// #[cfg(feature = "cluster")]
/// impl ractor::Message for MessageFormat {}
///
/// #[cfg_attr(feature = "async-trait", ractor::async_trait)]
/// impl Actor for TestActor {
///     type Msg = MessageFormat;
///     type Arguments = ();
///     type State = ();
///
///     async fn pre_start(&self, _this_actor: ActorRef<Self::Msg>, _: ()) -> Result<Self::State, ActorProcessingErr> {
///         Ok(())
///     }
///
///     async fn handle(
///         &self,
///         _this_actor: ActorRef<Self::Msg>,
///         message: Self::Msg,
///         _state: &mut Self::State,
///     ) -> Result<(), ActorProcessingErr> {
///         match message {
///             Self::Msg::TestRpc(arg, reply) => {
///                 // An error sending means no one is listening anymore (the receiver was dropped),
///                 // so we should shortcut the processing of this message probably!
///                 if !reply.is_closed() {
///                     let _ = reply.send(arg);
///                 }
///             }
///         }
///         Ok(())
///     }
/// }
///
/// #[tokio::main]
/// async fn main() {
///     let (actor, handle) = Actor::spawn(None, TestActor, ()).await.unwrap();
///     let result = call_t!(actor, MessageFormat::TestRpc, 50, "Something".to_string()).unwrap();
///     assert_eq!(result, "Something".to_string());
///     actor.stop(None);
///     handle.await.unwrap();
/// }
/// ```
#[macro_export]
macro_rules! call_t {
    ($actor:expr, $msg:expr, $timeout_ms:expr) => {{
        let err = $actor
            .call(|tx| $msg(tx), Some($crate::concurrency::Duration::from_millis($timeout_ms)))
            .await
            .map_err($crate::RactorErr::from);
        match err {
            Ok($crate::rpc::CallResult::Success(ok_value)) => Ok(ok_value),
            Ok(cr) => Err($crate::RactorErr::from(cr)),
            Err(e) => Err(e),
        }
    }};
    ($actor:expr, $msg:expr, $timeout_ms:expr, $($args:expr),*) => {{
        let err = $actor
            .call(|tx| $msg($($args),*, tx), Some($crate::concurrency::Duration::from_millis($timeout_ms)))
            .await
            .map_err($crate::RactorErr::from);
        match err {
            Ok($crate::rpc::CallResult::Success(ok_value)) => Ok(ok_value),
            Ok(cr) => Err($crate::RactorErr::from(cr)),
            Err(e) => Err(e),
        }
    }};
}

/// `forward!`: Perform a remote procedure call to a [crate::Actor]
/// and forwards the result to another actor if successful
///
/// * `$actor` - The actors to call
/// * `$msg` - The message builder, which takes in a [crate::port::RpcReplyPort] and emits a message which
///   the actor supports.
/// * `$forward` - The [crate::ActorRef] to forward the call to
/// * `$forward_mapping` - The message transformer from the RPC result to the forwarding actor's message format
/// * `$timeout` - The [crate::concurrency::Duration] to allow the call to complete before timing out.
///
/// Returns [Ok(())] on successful call forwarding, [Err(crate::RactorErr)] otherwies
#[macro_export]
macro_rules! forward {
    ($actor:expr, $msg:expr, $forward:ident, $forward_mapping:expr) => {{
        let future_or_err = $actor
            .call_and_forward(|tx| $msg(tx), &$forward, $forward_mapping, None)
            .map_err($crate::RactorErr::from);
        match future_or_err {
            Ok(future) => {
                let err = future.await;
                match err {
                    Ok($crate::rpc::CallResult::Success(Ok(()))) => Ok(()),
                    Ok($crate::rpc::CallResult::Success(Err(e))) => Err($crate::RactorErr::from(e)),
                    Ok(cr) => Err($crate::RactorErr::from(cr)),
                    Err(_join_err) => Err($crate::RactorErr::Messaging(
                        $crate::MessagingErr::ChannelClosed,
                    )),
                }
            }
            Err(_) => Err($crate::RactorErr::Messaging(
                $crate::MessagingErr::ChannelClosed,
            )),
        }
    }};
    ($actor:expr, $msg:expr, $forward:ident, $forward_mapping:expr, $timeout:expr) => {{
        let future_or_err = $actor
            .call_and_forward(|tx| $msg(tx), &$forward, $forward_mapping, Some($timeout))
            .map_err($crate::RactorErr::from);
        match future_or_err {
            Ok(future) => {
                let err = future.await;
                match err {
                    Ok($crate::rpc::CallResult::Success(Ok(()))) => Ok(()),
                    Ok($crate::rpc::CallResult::Success(Err(e))) => Err($crate::RactorErr::from(e)),
                    Ok(cr) => Err($crate::RactorErr::from(cr)),
                    Err(_join_err) => Err($crate::RactorErr::Messaging(
                        $crate::MessagingErr::ChannelClosed,
                    )),
                }
            }
            Err(_) => Err($crate::RactorErr::Messaging(
                $crate::MessagingErr::ChannelClosed,
            )),
        }
    }};
}

#[cfg(test)]
mod tests;
