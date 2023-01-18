// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Macro helpers for remote procedure calls

/// `cast!` takes an actor and a message and emits a [crate::RactorErr] error
/// which can be pattern matched on in order to derive the output
#[macro_export]
macro_rules! cast {
    ($actor:ident, $msg:expr) => {
        $actor.cast($msg).map_err($crate::RactorErr::from)
    };
}

/// `call!`: Perform an inifinite-time remote procedure call to an [crate::Actor]
///
/// * `$actor` - The actor to call
/// * `$msg` - The message builder which takes in a [crate::port::RpcReplyPort] and emits a message which
/// the actor supports
///
/// Returns [Ok(_)] with the result on successful RPC or [Err(crate::RactorErr)] on failure
#[macro_export]
macro_rules! call {
    ($actor:ident, $msg:expr) => {{
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
}

/// `call_t!`: Perform an finite-time remote procedure call to an [crate::Actor]
///
/// * `$actor` - The actor to call
/// * `$msg` - The message builder which takes in a [crate::port::RpcReplyPort] and emits a message which
/// the actor supports
/// * `$timeout` - The [tokio::time::Duration] timeout for how long the call can take before timing out
///
/// Returns [Ok(_)] with the result on successful RPC or [Err(crate::RactorErr)] on failure
#[macro_export]
macro_rules! call_t {
    ($actor:ident, $msg:expr, $timeout:expr) => {{
        let err = $actor
            .call(|tx| $msg(tx), Some($timeout))
            .await
            .map_err($crate::RactorErr::from);
        match err {
            Ok($crate::rpc::CallResult::Success(ok_value)) => Ok(ok_value),
            Ok(cr) => Err($crate::RactorErr::from(cr)),
            Err(e) => Err(e),
        }
    }};
}

/// `forward!`: Perform an infinite-time remote procedure call to a [crate::Actor]
/// and forwards the result to another actor if successful
///
/// * `$actor` - The actors to call
/// * `$msg` - The message builder, which takes in a [crate::port::RpcReplyPort] and emits a message which
/// the actor supports.
/// * `$forward` - The [crate::ActorRef] to forward the call to
/// * `$forward_mapping` - The message transformer from the RPC result to the forwarding actor's message format
///
/// Returns [Ok(())] on successful call forwarding, [Err(crate::RactorErr)] otherwies
#[macro_export]
macro_rules! forward {
    ($actor:ident, $msg:expr, $forward:ident, $forward_mapping:expr) => {{
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
            Err(e) => Err(e),
        }
    }};
}

/// `forward_t!`: Perform an finite-time remote procedure call to a [crate::Actor]
/// and forwards the result to another actor if successful
///
/// * `$actor` - The actors to call
/// * `$msg` - The message builder, which takes in a [crate::port::RpcReplyPort] and emits a message which
/// the actor supports.
/// * `$forward` - The [crate::ActorRef] to forward the call to
/// * `$forward_mapping` - The message transformer from the RPC result to the forwarding actor's message format
/// * `$timeout` - The [tokio::time::Duration] to allow the call to complete before timing out.
///
/// Returns [Ok(())] on successful call forwarding, [Err(crate::RactorErr)] otherwies
#[macro_export]
macro_rules! forward_t {
    ($actor:ident, $msg:expr, $forward:ident, $forward_mapping:expr, $timeout:expr) => {{
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
            Err(e) => Err(e),
        }
    }};
}

// TODO: subscribe to output port?
