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
//!    c. For RPCs, the LAST argument **must** be the reply channel. Additionally the type of message the channel is expecting back must also implement `ractor::BytesConvertable`
//!    d. Lastly, for RPCs, they should additionally be decorated with `#[rpc]` on each variant's definition. This helps the macro identify that it
//!    is an RPC and will need port handler

extern crate proc_macro;
use proc_macro::TokenStream;
use quote::format_ident;
use quote::quote;
use quote::ToTokens;
use syn::AngleBracketedGenericArguments;
use syn::DeriveInput;
use syn::Fields;
use syn::Ident;
use syn::TypePath;
use syn::Variant;
use syn::{self};

/// Derive `ractor::Message` for messages that are local-only
#[proc_macro_derive(RactorMessage)]
pub fn ractor_message_derive_macro(input: TokenStream) -> TokenStream {
    // Construct a representation of Rust code as a syntax tree
    // that we can manipulate
    let ast: DeriveInput = syn::parse(input).unwrap();
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
/// 3. For RPCs, the LAST argument **must** be the reply channel. Additionally the type of message the channel is expecting back must also implement `ractor::BytesConvertable`
/// 4. Lastly, for RPCs, they should additionally be decorated with `#[rpc]` on each variant's definition. This helps the macro identify that it
///    is an RPC and will need port handler
/// 5. For backwards compatibility, you can add new variants as long as you don't rename variants until all nodes in the cluster are upgraded.
#[proc_macro_derive(RactorClusterMessage, attributes(rpc))]
pub fn ractor_cluster_message_derive_macro(input: TokenStream) -> TokenStream {
    // Construct a representation of Rust code as a syntax tree
    // that we can manipulate
    let ast: DeriveInput = syn::parse(input).unwrap();

    // Build the trait implementation
    impl_message_macro(&ast)
}

fn impl_message_macro(ast: &syn::DeriveInput) -> TokenStream {
    let name = &ast.ident;
    let (impl_generics, ty_generics, where_clause) = ast.generics.split_for_impl();

    // we don't support the derive macro on structs or unions
    if let syn::Data::Enum(enum_data) = &ast.data {
        // Build the "serialize()" handler for each variant
        let serialized_variants = enum_data
            .variants
            .iter()
            .map(impl_variant_serialize)
            .collect::<Vec<_>>();
        // Build the deserialize handlers for both casts and calls
        let casts = enum_data.variants.iter().filter_map(|variant| {
            let is_call = variant.attrs.iter().any(|attr| attr.path().is_ident("rpc"));
            if !is_call {
                Some(impl_cast_variant_deserialize(variant))
            } else {
                None
            }
        });
        let calls = enum_data.variants.iter().filter_map(|variant| {
            let is_call = variant.attrs.iter().any(|attr| attr.path().is_ident("rpc"));
            if is_call {
                Some(impl_call_variant_deserialize(variant))
            } else {
                None
            }
        });

        (quote! {
            impl #impl_generics ractor::Message for #name #ty_generics #where_clause {
                fn serializable() -> bool {
                    // Network serializable message
                    true
                }

                fn serialize(self) -> Result<ractor::message::SerializedMessage, ractor::message::BoxedDowncastErr> {
                    use ::ractor::BytesConvertable;
                    match self {
                        #( #serialized_variants ),*
                    }
                }

                fn deserialize(bytes: ractor::message::SerializedMessage) -> Result<Self, ractor::message::BoxedDowncastErr> {
                    use ::ractor::BytesConvertable;
                    match bytes {
                        ractor::message::SerializedMessage::Cast {variant, args, metadata} => {
                            match variant.as_str() {
                                #(#casts,)*
                                _ => {
                                    // unknown CAST type
                                    Err(ractor::message::BoxedDowncastErr)
                                }
                            }
                        }
                        ractor::message::SerializedMessage::Call {variant, args, reply, metadata} => {
                            match variant.as_str() {
                                #(#calls,)*
                                _ => {
                                    // unknown CALL type
                                    Err(ractor::message::BoxedDowncastErr)
                                }
                            }
                        }
                        _ => {
                            // call-reply isn't supported here
                            Err(ractor::message::BoxedDowncastErr)
                        }
                    }
                }
            }
        }).into()
    } else {
        TokenStream::new()
    }
}

fn impl_variant_serialize(variant: &Variant) -> impl ToTokens {
    let name = &variant.ident;
    let variant_name = name.to_string();
    let is_call = variant.attrs.iter().any(|attr| attr.path().is_ident("rpc"));
    if is_call {
        match &variant.fields {
            Fields::Unit => panic!("RPC Calls must have a `RpcReplyPort<T>`"),
            Fields::Unnamed(unnamed_fields) => {
                // we only support un-named fields, where the last field is the "reply" port
                let mut fields = unnamed_fields
                    .unnamed
                    .iter()
                    .enumerate()
                    .map(|(i, arg)| (format_ident!("field{}", i), &arg.ty))
                    .collect::<Vec<_>>();
                // the last field is the port
                let _ = fields.pop();
                let port_type = if let syn::Type::Path(path_data) =
                    unnamed_fields.unnamed.last().unwrap().ty.clone()
                {
                    get_generic_reply_port_type(&path_data)
                } else {
                    panic!("No generic arguments on path data!");
                };

                let port = format_ident!("reply");
                let target_port = convert_serialize_port(&port, &port_type);

                if fields.is_empty() {
                    // no arguments, just a port
                    quote! {
                        Self::#name(#port) => {
                            let target_port = #target_port;
                            Ok(ractor::message::SerializedMessage::Call {
                                variant: #variant_name.to_string(),
                                args: vec![],
                                reply: target_port,
                                metadata: None,
                            })
                        }
                    }
                } else {
                    let field_names = fields.iter().map(|(a, _)| a);
                    let packed = fields.iter().map(|(field, arg)| pack_args(field, arg));
                    quote! {
                        Self::#name(#(#field_names),*, #port) => {
                            let mut data = vec![];
                            #(#packed;)*
                            let target_port = #target_port;
                            Ok(ractor::message::SerializedMessage::Call {
                                variant: #variant_name.to_string(),
                                args: data,
                                reply: target_port,
                                metadata: None,
                            })
                        }
                    }
                }
            }
            _ => panic!("Named fields are not supported (today)"),
        }
    } else {
        match &variant.fields {
            Fields::Unit => {
                // empty, just use the index value
                quote! {
                    Self::#name => {
                        Ok(ractor::message::SerializedMessage::Cast {
                            variant: #variant_name.to_string(),
                            args: vec![],
                            metadata: None,
                        })
                    }
                }
            }
            Fields::Unnamed(unnamed_fields) => {
                // we only support un-named fields, where the last field is the "reply" port
                let fields = unnamed_fields
                    .unnamed
                    .iter()
                    .enumerate()
                    .map(|(i, arg)| (format_ident!("field{}", i), &arg.ty))
                    .collect::<Vec<_>>();

                let field_names = fields.iter().map(|(a, _)| a);
                let packed = fields.iter().map(|(field, arg)| pack_args(field, arg));
                quote! {
                    Self::#name(#(#field_names),*) => {
                        let mut data = vec![];

                        #(#packed ;)*

                        Ok(ractor::message::SerializedMessage::Cast {
                            variant: #variant_name.to_string(),
                            args: data,
                            metadata: None,
                        })
                    }
                }
            }
            _ => panic!("Named fields are not supported (today)"),
        }
    }
}

fn impl_cast_variant_deserialize(variant: &Variant) -> impl ToTokens {
    let name = &variant.ident;
    let variant_name = name.to_string();
    match &variant.fields {
        Fields::Unit => {
            // empty, just the index value
            quote! {
                #variant_name => {
                    Ok(Self::#name)
                }
            }
        }
        Fields::Unnamed(unnamed_fields) => {
            let fields = unnamed_fields
                .unnamed
                .iter()
                .enumerate()
                .map(|(i, arg)| (format_ident!("field{}", i), &arg.ty))
                .collect::<Vec<_>>();
            let field_names = fields.iter().map(|(a, _)| a);
            let unpacked = fields.iter().map(|(field, arg)| unpack_arg(field, arg));
            quote! {
                #variant_name => {
                    let mut ptr = 0usize;
                    #(#unpacked;)*
                    Ok(Self::#name(#(#field_names),*))
                }
            }
        }
        _ => panic!("Named fields are not supported (today)"),
    }
}

fn impl_call_variant_deserialize(variant: &Variant) -> impl ToTokens {
    let name = &variant.ident;
    let variant_name = name.to_string();
    match &variant.fields {
        Fields::Unit => panic!("RPC Calls must have a `RpcReplyPort<T>`"),
        Fields::Unnamed(unnamed_fields) => {
            let mut fields = unnamed_fields
                .unnamed
                .iter()
                .enumerate()
                .map(|(i, arg)| (format_ident!("field{}", i), &arg.ty))
                .collect::<Vec<_>>();

            let port = format_ident!("reply");
            let port_type = if let syn::Type::Path(path_data) =
                unnamed_fields.unnamed.last().unwrap().ty.clone()
            {
                get_generic_reply_port_type(&path_data)
            } else {
                panic!("No generic arguments on path data!");
            };

            let target_port = convert_deserialize_port(&port, &port_type);

            // the last field is the port, pop it off
            let _ = fields.pop();
            if fields.is_empty() {
                quote! {
                    #variant_name => {
                        let target_port = #target_port;
                        Ok(Self::#name(target_port))
                    }
                }
            } else {
                let field_names = fields.iter().map(|(a, _)| a);
                let unpacked = fields.iter().map(|(field, arg)| unpack_arg(field, arg));
                quote! {
                    #variant_name => {
                        let mut ptr = 0usize;
                        #(#unpacked;)*
                        let target_port = #target_port;
                        Ok(Self::#name(#(#field_names),*, target_port))
                    }
                }
            }
        }
        _ => panic!("Named fields are not supported (today)"),
    }
}

fn pack_args(field: &Ident, target_type: &syn::Type) -> impl ToTokens {
    quote! {
        {
            let arg_data = <#target_type as ractor::BytesConvertable>::into_bytes(#field);
            let arg_len = (arg_data.len() as u64).to_be_bytes();
            data.extend(arg_len);
            data.extend(arg_data);
        }
    }
}

fn unpack_arg(field: &Ident, target_type: &syn::Type) -> impl ToTokens {
    quote! {
        let #field = {
            let mut len_bytes = [0u8; 8];
            len_bytes.copy_from_slice(&args[ptr..ptr+8]);
            let len = u64::from_be_bytes(len_bytes) as usize;

            ptr += 8;
            let data_bytes = args[ptr..ptr+len].to_vec();
            let t_result = <#target_type as ractor::BytesConvertable>::from_bytes(data_bytes);
            ptr += len;
            t_result
        };
    }
}

fn convert_deserialize_port(
    the_port: &Ident,
    port_type: &AngleBracketedGenericArguments,
) -> impl ToTokens {
    let generic_args = &port_type.args;
    // TODO: catch unwind for the conversion? returning Err(BoxedDowncastErr)
    quote! {
        {
            let (tx, rx) = ractor::concurrency::oneshot::#port_type();
            let o_timeout = #the_port.get_timeout();
            ractor::concurrency::spawn(async move {
                if let Some(timeout) = o_timeout {
                    if let Ok(Ok(result)) = ractor::concurrency::timeout(timeout, rx).await {
                        let _ = #the_port.send(<#generic_args as BytesConvertable>::into_bytes(result));
                    }
                } else {
                    if let Ok(result) = rx.await {
                        let _ = #the_port.send(<#generic_args as BytesConvertable>::into_bytes(result));
                    }
                }
            });
            if let Some(timeout) = o_timeout {
                ractor::RpcReplyPort::<_>::from((tx, timeout))
            } else {
                ractor::RpcReplyPort::<_>::from(tx)
            }
        }
    }
}

fn convert_serialize_port(
    the_port: &Ident,
    target_type: &AngleBracketedGenericArguments,
) -> impl ToTokens {
    // TODO: catch unwind for the conversion? returning Err(BoxedDowncastErr)
    let generic_args = &target_type.args;
    quote! {
        {
            let (tx, rx) = ractor::concurrency::oneshot();
            let o_timeout = #the_port.get_timeout();
            ractor::concurrency::spawn(async move {
                if let Some(timeout) = o_timeout {
                    if let Ok(Ok(result)) = ractor::concurrency::timeout(timeout, rx).await {
                        let typed_result = <#generic_args as ractor::BytesConvertable>::from_bytes(result);
                        let _ = #the_port.send(typed_result);
                    }
                } else {
                    if let Ok(result) = rx.await {
                        let typed_result = <#generic_args as ractor::BytesConvertable>::from_bytes(result);
                        let _ = #the_port.send(typed_result);
                    }
                }
            });
            if let Some(timeout) = o_timeout {
                ractor::RpcReplyPort::<_>::from((tx, timeout))
            } else {
                ractor::RpcReplyPort::<_>::from(tx)
            }
        }
    }
}

fn get_generic_reply_port_type(path_data: &TypePath) -> AngleBracketedGenericArguments {
    if let syn::PathArguments::AngleBracketed(generic_args) =
        &path_data.path.segments.last().unwrap().arguments
    {
        generic_args.clone()
    } else {
        panic!("RpcReplyPort failed to get generic args");
    }
}
