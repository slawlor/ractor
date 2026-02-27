// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Code generation from parsed IR for `RactorClusterMessage`.
//!
//! Internal generated variables use `__` prefix to avoid collisions with
//! user-chosen field names in named (struct-style) variants.

use proc_macro2::TokenStream;
use quote::{format_ident, quote, ToTokens};
use syn::{AngleBracketedGenericArguments, Ident};

use crate::ir::{FieldStyle, ParsedEnum, ParsedVariant, VariantKind};

/// Generate the full `impl ractor::Message for ...` block from a parsed enum.
pub(crate) fn expand_cluster_message(parsed: &ParsedEnum) -> TokenStream {
    let name = &parsed.ident;
    let (impl_generics, ty_generics, where_clause) = parsed.generics.split_for_impl();

    let serialized_variants: Vec<_> = parsed.variants.iter().map(gen_serialize_arm).collect();

    let casts: Vec<_> = parsed
        .variants
        .iter()
        .filter(|v| matches!(v.kind, VariantKind::Cast))
        .map(gen_cast_deserialize_arm)
        .collect();

    let calls: Vec<_> = parsed
        .variants
        .iter()
        .filter(|v| matches!(v.kind, VariantKind::Call { .. }))
        .map(gen_call_deserialize_arm)
        .collect();

    quote! {
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
                    ractor::message::SerializedMessage::Cast {variant: __variant, args: __args, metadata: __metadata} => {
                        match __variant.as_str() {
                            #(#casts,)*
                            _ => {
                                // unknown CAST type
                                Err(ractor::message::BoxedDowncastErr)
                            }
                        }
                    }
                    ractor::message::SerializedMessage::Call {variant: __variant, args: __args, reply: __reply, metadata: __metadata} => {
                        match __variant.as_str() {
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
    }
}

/// Build a full ordered list of identifiers by inserting `port_ident` at `port_index`
/// among the data field identifiers.
fn build_ordered_bindings(
    data_fields: &[(Ident, syn::Type)],
    port_ident: &Ident,
    port_index: usize,
) -> Vec<Ident> {
    let total = data_fields.len() + 1;
    let mut result = Vec::with_capacity(total);
    let mut data_idx = 0;
    for i in 0..total {
        if i == port_index {
            result.push(port_ident.clone());
        } else {
            result.push(data_fields[data_idx].0.clone());
            data_idx += 1;
        }
    }
    result
}

/// Generate a serialization match arm for one variant.
fn gen_serialize_arm(variant: &ParsedVariant) -> impl ToTokens {
    let name = &variant.ident;
    let variant_name = &variant.variant_tag;
    let fields = &variant.data_fields;

    match &variant.kind {
        VariantKind::Cast => {
            if fields.is_empty() {
                let pattern = match &variant.field_style {
                    FieldStyle::Tuple => quote! { Self::#name },
                    FieldStyle::Named => quote! { Self::#name {} },
                };
                quote! {
                    #pattern => {
                        Ok(ractor::message::SerializedMessage::Cast {
                            variant: #variant_name.to_string(),
                            args: vec![],
                            metadata: None,
                        })
                    }
                }
            } else {
                let field_names: Vec<_> = fields.iter().map(|(a, _)| a).collect();
                let packed = fields.iter().map(|(field, ty)| pack_args(field, ty));
                let pattern = match &variant.field_style {
                    FieldStyle::Tuple => quote! { Self::#name(#(#field_names),*) },
                    FieldStyle::Named => quote! { Self::#name { #(#field_names),* } },
                };
                quote! {
                    #pattern => {
                        let mut __data = vec![];
                        #(#packed ;)*
                        Ok(ractor::message::SerializedMessage::Cast {
                            variant: #variant_name.to_string(),
                            args: __data,
                            metadata: None,
                        })
                    }
                }
            }
        }
        VariantKind::Call {
            reply_port_generic_args,
            reply_port_field_name,
            reply_port_index,
        } => {
            let port = reply_port_field_name;
            let target_port = gen_serialize_port(port, reply_port_generic_args);

            let pattern = match &variant.field_style {
                FieldStyle::Tuple => {
                    let all = build_ordered_bindings(fields, port, *reply_port_index);
                    quote! { Self::#name(#(#all),*) }
                }
                FieldStyle::Named => {
                    let data_names: Vec<_> = fields.iter().map(|(a, _)| a).collect();
                    quote! { Self::#name { #(#data_names,)* #port } }
                }
            };

            if fields.is_empty() {
                quote! {
                    #pattern => {
                        let __target_port = #target_port;
                        Ok(ractor::message::SerializedMessage::Call {
                            variant: #variant_name.to_string(),
                            args: vec![],
                            reply: __target_port,
                            metadata: None,
                        })
                    }
                }
            } else {
                let packed = fields.iter().map(|(field, ty)| pack_args(field, ty));
                quote! {
                    #pattern => {
                        let mut __data = vec![];
                        #(#packed;)*
                        let __target_port = #target_port;
                        Ok(ractor::message::SerializedMessage::Call {
                            variant: #variant_name.to_string(),
                            args: __data,
                            reply: __target_port,
                            metadata: None,
                        })
                    }
                }
            }
        }
    }
}

/// Generate a deserialization match arm for a cast variant.
fn gen_cast_deserialize_arm(variant: &ParsedVariant) -> impl ToTokens {
    let name = &variant.ident;
    let variant_name = &variant.variant_tag;
    let fields = &variant.data_fields;

    if fields.is_empty() {
        let construct = match &variant.field_style {
            FieldStyle::Tuple => quote! { Self::#name },
            FieldStyle::Named => quote! { Self::#name {} },
        };
        quote! {
            #variant_name => {
                Ok(#construct)
            }
        }
    } else {
        let field_names: Vec<_> = fields.iter().map(|(a, _)| a).collect();
        let unpacked = fields.iter().map(|(field, ty)| unpack_arg(field, ty));
        let construct = match &variant.field_style {
            FieldStyle::Tuple => quote! { Self::#name(#(#field_names),*) },
            FieldStyle::Named => quote! { Self::#name { #(#field_names),* } },
        };
        quote! {
            #variant_name => {
                let mut __ptr = 0usize;
                #(#unpacked;)*
                Ok(#construct)
            }
        }
    }
}

/// Generate a deserialization match arm for a call (RPC) variant.
fn gen_call_deserialize_arm(variant: &ParsedVariant) -> impl ToTokens {
    let name = &variant.ident;
    let variant_name = &variant.variant_tag;
    let fields = &variant.data_fields;

    let (reply_port_generic_args, reply_port_field_name, reply_port_index) = match &variant.kind {
        VariantKind::Call {
            reply_port_generic_args,
            reply_port_field_name,
            reply_port_index,
        } => (
            reply_port_generic_args,
            reply_port_field_name,
            *reply_port_index,
        ),
        VariantKind::Cast => unreachable!("gen_call_deserialize_arm called on cast variant"),
    };

    let target_port = gen_deserialize_port(&format_ident!("__reply"), reply_port_generic_args);

    let construct = match &variant.field_style {
        FieldStyle::Tuple => {
            let target_port_ident = format_ident!("__target_port");
            let all = build_ordered_bindings(fields, &target_port_ident, reply_port_index);
            quote! { Self::#name(#(#all),*) }
        }
        FieldStyle::Named => {
            let data_names: Vec<_> = fields.iter().map(|(a, _)| a).collect();
            let port_field = reply_port_field_name;
            quote! { Self::#name { #(#data_names,)* #port_field: __target_port } }
        }
    };

    if fields.is_empty() {
        quote! {
            #variant_name => {
                let __target_port = #target_port;
                Ok(#construct)
            }
        }
    } else {
        let unpacked = fields.iter().map(|(field, ty)| unpack_arg(field, ty));
        quote! {
            #variant_name => {
                let mut __ptr = 0usize;
                #(#unpacked;)*
                let __target_port = #target_port;
                Ok(#construct)
            }
        }
    }
}

/// Generate per-field serialization code.
fn pack_args(field: &Ident, target_type: &syn::Type) -> impl ToTokens {
    quote! {
        {
            let __arg_data = <#target_type as ractor::BytesConvertable>::into_bytes(#field);
            let __arg_len = (__arg_data.len() as u64).to_be_bytes();
            __data.extend(__arg_len);
            __data.extend(__arg_data);
        }
    }
}

/// Generate per-field deserialization code.
fn unpack_arg(field: &Ident, target_type: &syn::Type) -> impl ToTokens {
    quote! {
        let #field = {
            let mut __len_bytes = [0u8; 8];
            __len_bytes.copy_from_slice(&__args[__ptr..__ptr+8]);
            let __len = u64::from_be_bytes(__len_bytes) as usize;

            __ptr += 8;
            let __data_bytes = __args[__ptr..__ptr+__len].to_vec();
            let __t_result = <#target_type as ractor::BytesConvertable>::from_bytes(__data_bytes);
            __ptr += __len;
            __t_result
        };
    }
}

/// Generate reply port bridge: typed → binary (for serialization).
fn gen_serialize_port(
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

/// Generate reply port bridge: binary → typed (for deserialization).
fn gen_deserialize_port(
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
