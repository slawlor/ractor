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
                let prepare = prepare_args(fields);
                let packed = fields
                    .iter()
                    .enumerate()
                    .map(|(index, (field, ty))| pack_args(field, ty, index));
                let pattern = match &variant.field_style {
                    FieldStyle::Tuple => quote! { Self::#name(#(#field_names),*) },
                    FieldStyle::Named => quote! { Self::#name { #(#field_names),* } },
                };
                quote! {
                    #pattern => {
                        #prepare
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
                let prepare = prepare_args(fields);
                let packed = fields
                    .iter()
                    .enumerate()
                    .map(|(index, (field, ty))| pack_args(field, ty, index));
                quote! {
                    #pattern => {
                        #prepare
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
                if __args.is_empty() {
                    Ok(#construct)
                } else {
                    Err(ractor::message::BoxedDowncastErr)
                }
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
                if __ptr == __args.len() {
                    Ok(#construct)
                } else {
                    Err(ractor::message::BoxedDowncastErr)
                }
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
                if __args.is_empty() {
                    let __target_port = #target_port;
                    Ok(#construct)
                } else {
                    Err(ractor::message::BoxedDowncastErr)
                }
            }
        }
    } else {
        let unpacked = fields.iter().map(|(field, ty)| unpack_arg(field, ty));
        quote! {
            #variant_name => {
                let mut __ptr = 0usize;
                #(#unpacked;)*
                if __ptr == __args.len() {
                    let __target_port = #target_port;
                    Ok(#construct)
                } else {
                    Err(ractor::message::BoxedDowncastErr)
                }
            }
        }
    }
}

/// Generate field-length hints and reserve the complete frame when every field
/// can report its encoded size without serializing.
fn prepare_args(fields: &[(Ident, syn::Type)]) -> impl ToTokens {
    let hints = fields.iter().map(|(field, ty)| {
        quote! { <#ty as ractor::BytesConvertable>::serialized_len(&#field) }
    });

    quote! {
        let __serialized_lens = [#(#hints),*];
        let __total_len = __serialized_lens.iter().try_fold(0usize, |__total, __field_len| {
            __total
                .checked_add(::core::mem::size_of::<u64>())?
                .checked_add((*__field_len)?)
        });
        let mut __data = ::std::vec::Vec::new();
        if let Some(__total_len) = __total_len {
            __data
                .try_reserve_exact(__total_len)
                .map_err(|_| ractor::message::BoxedDowncastErr)?;
        }
    }
}

/// Generate per-field serialization code.
fn pack_args(field: &Ident, target_type: &syn::Type, index: usize) -> impl ToTokens {
    let index = syn::Index::from(index);
    quote! {
        {
            if let Some(__expected_len) = __serialized_lens[#index] {
                let __arg_len = <u64 as ::core::convert::TryFrom<usize>>::try_from(__expected_len)
                    .map_err(|_| ractor::message::BoxedDowncastErr)?
                    .to_be_bytes();
                let __additional = ::core::mem::size_of::<u64>()
                    .checked_add(__expected_len)
                    .ok_or(ractor::message::BoxedDowncastErr)?;
                __data
                    .try_reserve(__additional)
                    .map_err(|_| ractor::message::BoxedDowncastErr)?;
                __data.extend_from_slice(&__arg_len);
                let __data_offset = __data.len();
                <#target_type as ractor::BytesConvertable>::extend_bytes(#field, &mut __data);
                let __actual_len = __data
                    .len()
                    .checked_sub(__data_offset)
                    .ok_or(ractor::message::BoxedDowncastErr)?;
                if __actual_len != __expected_len {
                    return Err(ractor::message::BoxedDowncastErr);
                }
            } else {
                let __arg_data = <#target_type as ractor::BytesConvertable>::into_bytes(#field);
                let __arg_len = <u64 as ::core::convert::TryFrom<usize>>::try_from(__arg_data.len())
                    .map_err(|_| ractor::message::BoxedDowncastErr)?
                    .to_be_bytes();
                let __additional = ::core::mem::size_of::<u64>()
                    .checked_add(__arg_data.len())
                    .ok_or(ractor::message::BoxedDowncastErr)?;
                __data
                    .try_reserve(__additional)
                    .map_err(|_| ractor::message::BoxedDowncastErr)?;
                __data.extend_from_slice(&__arg_len);
                __data.extend(__arg_data);
            }
        }
    }
}

/// Generate per-field deserialization code.
fn unpack_arg(field: &Ident, target_type: &syn::Type) -> impl ToTokens {
    quote! {
        let #field = {
            let __len_end = __ptr
                .checked_add(::core::mem::size_of::<u64>())
                .ok_or(ractor::message::BoxedDowncastErr)?;
            let mut __len_bytes = [0u8; 8];
            let __encoded_len = __args
                .get(__ptr..__len_end)
                .ok_or(ractor::message::BoxedDowncastErr)?;
            __len_bytes.copy_from_slice(__encoded_len);
            let __len = <usize as ::core::convert::TryFrom<u64>>::try_from(
                u64::from_be_bytes(__len_bytes)
            )
                .map_err(|_| ractor::message::BoxedDowncastErr)?;

            let __data_end = __len_end
                .checked_add(__len)
                .ok_or(ractor::message::BoxedDowncastErr)?;
            let __data_bytes = __args
                .get(__len_end..__data_end)
                .ok_or(ractor::message::BoxedDowncastErr)?;
            let __t_result = ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                <#target_type as ractor::BytesConvertable>::from_bytes_ref(__data_bytes)
            }))
                .map_err(|_| ractor::message::BoxedDowncastErr)?;
            __ptr = __data_end;
            __t_result
        };
    }
}

/// Generate reply port bridge: typed → binary (for serialization).
fn gen_serialize_port(
    the_port: &Ident,
    target_type: &AngleBracketedGenericArguments,
) -> impl ToTokens {
    let generic_args = &target_type.args;
    quote! {
        {
            let (tx, rx) = ractor::concurrency::oneshot();
            let o_timeout = #the_port.get_timeout();
            ractor::concurrency::spawn(async move {
                if let Some(timeout) = o_timeout {
                    if let Ok(Ok(result)) = ractor::concurrency::timeout(timeout, rx).await {
                        if let Ok(typed_result) = ::std::panic::catch_unwind(
                            ::std::panic::AssertUnwindSafe(|| {
                                <#generic_args as ractor::BytesConvertable>::from_bytes(result)
                            })
                        ) {
                            let _ = #the_port.send(typed_result);
                        }
                    }
                } else {
                    if let Ok(result) = rx.await {
                        if let Ok(typed_result) = ::std::panic::catch_unwind(
                            ::std::panic::AssertUnwindSafe(|| {
                                <#generic_args as ractor::BytesConvertable>::from_bytes(result)
                            })
                        ) {
                            let _ = #the_port.send(typed_result);
                        }
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
    quote! {
        {
            let (tx, rx) = ractor::concurrency::oneshot::#port_type();
            let o_timeout = #the_port.get_timeout();
            ractor::concurrency::spawn(async move {
                if let Some(timeout) = o_timeout {
                    if let Ok(Ok(result)) = ractor::concurrency::timeout(timeout, rx).await {
                        if let Ok(bytes) = ::std::panic::catch_unwind(
                            ::std::panic::AssertUnwindSafe(|| {
                                <#generic_args as BytesConvertable>::into_bytes(result)
                            })
                        ) {
                            let _ = #the_port.send(bytes);
                        }
                    }
                } else {
                    if let Ok(result) = rx.await {
                        if let Ok(bytes) = ::std::panic::catch_unwind(
                            ::std::panic::AssertUnwindSafe(|| {
                                <#generic_args as BytesConvertable>::into_bytes(result)
                            })
                        ) {
                            let _ = #the_port.send(bytes);
                        }
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
