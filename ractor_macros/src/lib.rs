// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Procedural macros for concise, explicit Ractor actor definitions.

extern crate proc_macro;

mod config;
mod expand;

use proc_macro::TokenStream;
use syn::parse_macro_input;

use crate::config::ActorConfig;

/// Generate a Ractor actor implementation from ordinary inherent methods.
///
/// Use `message = MessageType` for an existing message type, or
/// `message = [visibility] enum MessageType` to generate a local message enum from
/// the handler patterns and parameter types. `state` and `arguments` default
/// to `()`. Add the `thread_local` flag to implement `ThreadLocalActor` instead
/// of `Actor`.
///
/// Mark one-way handlers with `#[ractor::message(Message::Variant(...))]` and
/// request/reply handlers with `#[ractor::rpc(Message::Variant(...))]`. Flat
/// unit, tuple, and struct variant patterns are supported. Message bindings
/// must appear as method parameters in the same order. For an RPC, exactly one
/// binding is omitted from the method parameters and used as its reply port;
/// the method's return value is sent automatically. A handler may also take an
/// `ActorRef` first and `&State` or `&mut State` last.
///
/// Handlers can be sync or async. An explicit non-unit message-handler return
/// is propagated into the generated `handle` method with `?`. By default, an
/// RPC handler's full return type is its reply type, including `Result`.
/// `#[ractor::rpc(Pattern, reply = Type)]` instead treats the method return as
/// fallible actor processing, applies `?`, and sends the resulting `Type`.
///
/// When focused message or RPC handlers are present, the generated dispatch is
/// exhaustive, so adding a message variant without a handler is a compile
/// error. A canonical raw `handle` method remains supported, but cannot be
/// mixed with focused handlers. If neither form is present, the macro emits no
/// `handle` method and inherits the trait's no-op default. Other lifecycle
/// methods keep their normal async trait signatures.
///
/// Mark focused supervision handlers with
/// `#[ractor::supervision(SupervisionEvent::Variant(...))]`. Their parameters
/// follow the same actor-reference, pattern-binding, and state rules as message
/// handlers. Unmatched events retain Ractor's default supervision behavior.
///
/// ```ignore
/// #[ractor::actor(message = CounterMessage, state = u64)]
/// impl Counter {
///     async fn pre_start(
///         &self,
///         _myself: ractor::ActorRef<CounterMessage>,
///         _args: (),
///     ) -> Result<u64, ractor::ActorProcessingErr> {
///         Ok(0)
///     }
///
///     #[ractor::message(CounterMessage::Add(amount))]
///     fn add(&self, amount: u64, state: &mut u64) {
///         *state += amount;
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn actor(attributes: TokenStream, item: TokenStream) -> TokenStream {
    let config = parse_macro_input!(attributes as ActorConfig);
    let actor_impl = parse_macro_input!(item as syn::ItemImpl);

    match expand::expand_actor(config, actor_impl) {
        Ok(expanded) => expanded.into(),
        Err(error) => error.into_compile_error().into(),
    }
}
