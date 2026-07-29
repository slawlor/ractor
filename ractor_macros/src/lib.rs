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
/// `message = MessageType` is required. `state` and `arguments` default to
/// `()`. Add the `thread_local` flag to implement `ThreadLocalActor` instead
/// of `Actor`.
///
/// Mark handlers with `#[ractor::message(Message::Variant(...))]`. Flat unit,
/// tuple, and struct variant patterns are supported, and their named bindings
/// must appear as method parameters in the same order. A handler may also take
/// an `ActorRef` first and `&State` or `&mut State` last. Handlers can be sync
/// or async; any explicit non-unit return must support `?` and is propagated
/// into the generated `handle` method.
///
/// The generated dispatch is exhaustive, so adding a message variant without
/// a handler is a compile error. A canonical raw `handle` method remains
/// supported, but cannot be mixed with generated message handlers. Other
/// lifecycle methods keep their normal async trait signatures.
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
