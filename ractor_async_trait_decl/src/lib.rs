use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Item};
#[proc_macro_attribute]
pub fn async_trait(args: TokenStream, input: TokenStream) -> TokenStream {
    let args_str = args.to_string();
    let input = parse_macro_input!(input as Item);

    if args_str.is_empty() {
        quote! {
            #[cfg_attr(
                not(all(target_arch = "wasm32", target_os = "unknown")),
                async_trait::async_trait
            )]
            #[cfg_attr(
                all(target_arch = "wasm32", target_os = "unknown"),
                async_trait::async_trait(?Send)
            )]
            #input
        }
        .into()
    } else {
        let args_ts = proc_macro2::TokenStream::from(args);
        quote! {
            #[async_trait::async_trait(#args_ts)]
            #input
        }
        .into()
    }
}
