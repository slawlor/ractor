// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, ItemFn};

/// Expands to `#[tokio::main(flavor = "current_thread")]` on `wasm32-unknown-unknown`, `#[tokio::main]` on other platforms
#[proc_macro_attribute]
pub fn ractor_example_entry(_: TokenStream, input: TokenStream) -> TokenStream {
    let input_fn = parse_macro_input!(input as ItemFn);

    (quote! {
        #[cfg_attr(
            all(target_arch = "wasm32", target_os = "unknown"),
            tokio::main(flavor = "current_thread")
        )]
        #[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), tokio::main)]

        #input_fn
    })
    .into()
}
