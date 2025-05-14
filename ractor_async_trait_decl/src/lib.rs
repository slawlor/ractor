use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use syn::{ItemImpl, ItemTrait};

#[proc_macro_attribute]
pub fn ractor_async_trait_decl(_: TokenStream, input: TokenStream) -> TokenStream {
    let input_trait = match syn::parse::<ItemTrait>(input.clone())
        .map(|x| x.into_token_stream())
        .or_else(|_| syn::parse::<ItemImpl>(input).map(|x| x.into_token_stream()))
    {
        Ok(o) => o,
        Err(_) => {
            return quote! {
                compile_error!("Expected trait or impl block");
            }
            .into();
        }
    };

    (quote! {
    #[cfg_attr(
        all(
            feature = "async-trait",
            not(all(target_arch = "wasm32", target_os = "unknown"))
        ),
        ractor::async_trait
    )]
    #[cfg_attr(
        all(
            feature = "async-trait",
           all(target_arch = "wasm32", target_os = "unknown")
        ),
        ractor::async_trait(?Send)
    )]
            #input_trait
        })
    .into()
}
