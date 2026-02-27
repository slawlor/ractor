// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Procedure macro for Message formatting in `ractor_cluster`. This implements
//! the `ractor::Message` trait for non-serializable and serializable messages automatically.
//!
//! note: rampant use of `cargo expand` was used in the making of this macro. E.g.
//!
//! ```text
//! cargo expand -p ractor_playground 2>&1 > expand.tmp.rs
//! ```
//!
//! Caveats:
//!
//! 1. Non-serializable macros are simply getting `impl ractor::Message for MyStructOrEnum` added onto their struct
//! 2. Serializable messages have to have a few formatting requirements.
//!    a. All variants of the enum will be numbered based on their lexicographical ordering, which is sent over-the-wire in order to decode which
//!    variant was called. This is the `index` field on any variant of `ractor::message::SerializedMessage`
//!    b. All properties of the message **MUST** implement the `ractor::BytesConvertable` trait which means they supply a `to_bytes` and `from_bytes` method. Many
//!    types are pre-done for you in `ractor`'s definition of the trait
//!    c. For RPCs, exactly one field must be an `RpcReplyPort<T>` (at any position). Additionally the type of message the channel is expecting back must also implement `ractor::BytesConvertable`
//!    d. Lastly, for RPCs, they should additionally be decorated with `#[rpc]` on each variant's definition. This helps the macro identify that it
//!    is an RPC and will need port handler
//!    e. Both tuple-style (`Variant(A, B)`) and struct-style (`Variant { a: A, b: B }`) fields are supported

extern crate proc_macro;

mod codegen;
mod ir;
mod parse;

use proc_macro::TokenStream;
use quote::quote;
use syn::DeriveInput;

/// Derive `ractor::Message` for messages that are local-only
#[proc_macro_derive(RactorMessage)]
pub fn ractor_message_derive_macro(input: TokenStream) -> TokenStream {
    let ast: DeriveInput = match syn::parse(input) {
        Ok(ast) => ast,
        Err(err) => return err.to_compile_error().into(),
    };

    let name = &ast.ident;
    let (impl_generics, ty_generics, where_clause) = ast.generics.split_for_impl();
    let gen = quote! {
        impl #impl_generics ractor::Message for #name #ty_generics #where_clause {}
    };
    gen.into()
}

/// Derive `ractor::Message` for messages that can be sent over the network
///
/// Serializable messages have to have a few formatting requirements.
/// 1. All variants of the enum will be tagged based on their variant name, which is sent over-the-wire in order to decode which
///    variant was called. This is the `variant` field on `ractor::message::SerializedMessage::Cast` and `Call`.
/// 2. All properties of the message **MUST** implement the `ractor::BytesConvertable` trait which means they supply a `to_bytes` and `from_bytes` method. Many
///    types are pre-done for you in `ractor`'s definition of the trait
/// 3. For RPCs, exactly one field must be an `RpcReplyPort<T>` (at any position). Additionally the type of message the channel is expecting back must also implement `ractor::BytesConvertable`
/// 4. Lastly, for RPCs, they should additionally be decorated with `#[rpc]` on each variant's definition. This helps the macro identify that it
///    is an RPC and will need port handler
/// 5. Both tuple-style (`Variant(A, B)`) and struct-style (`Variant { a: A, b: B }`) fields are supported
/// 6. For backwards compatibility, you can add new variants as long as you don't rename variants until all nodes in the cluster are upgraded.
#[proc_macro_derive(RactorClusterMessage, attributes(rpc))]
pub fn ractor_cluster_message_derive_macro(input: TokenStream) -> TokenStream {
    let ast: DeriveInput = match syn::parse(input) {
        Ok(ast) => ast,
        Err(err) => return err.to_compile_error().into(),
    };

    match parse::parse_cluster_message(&ast) {
        Ok(parsed) => codegen::expand_cluster_message(&parsed).into(),
        Err(err) => err.to_compile_error().into(),
    }
}
