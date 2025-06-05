use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use syn::{parse::Parser, punctuated::Punctuated, ItemImpl, ItemTrait, Path, Token};

#[proc_macro_attribute]
pub fn ractor_async_trait_decl(attr: TokenStream, input: TokenStream) -> TokenStream {
    let parser = Punctuated::<Path, Token![,]>::parse_terminated;
    let args = match parser.parse(attr) {
        Ok(parsed_args) => parsed_args,
        Err(_) => {
            return quote! {
                compile_error!("Expected a single path to the async_trait, e.g., ractor::async_trait");
            }
            .into();
        }
    };

    let default_trait = syn::parse_str("async_trait::async_trait").unwrap();

    let async_trait_path = args.first().unwrap_or(&default_trait);

    let input_trait = match syn::parse::<ItemTrait>(input.clone())
        .map(|x| x.into_token_stream())
        .or_else(|_| syn::parse::<ItemImpl>(input).map(|x| x.into_token_stream()))
    {
        Ok(o) => o,
        Err(_) => {
            return quote! {
                compile_error!("Expected a trait or impl block");
            }
            .into();
        }
    };

    // Generate the output code
    (quote! {
        #[cfg_attr(
            all(
                feature = "async-trait",
                not(all(target_arch = "wasm32", target_os = "unknown"))
            ),
            #async_trait_path
        )]
        #[cfg_attr(
            all(
                feature = "async-trait",
                all(target_arch = "wasm32", target_os = "unknown")
            ),
            #async_trait_path (?Send)
        )]
        #input_trait
    })
    .into()
}
