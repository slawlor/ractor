// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Parsing and validation for `RactorClusterMessage` derive input.

use quote::format_ident;
use syn::spanned::Spanned;
use syn::{Fields, TypePath, Variant};

use crate::ir::{FieldStyle, ParsedEnum, ParsedVariant, VariantKind};

/// Parse a `DeriveInput` into a fully validated `ParsedEnum`.
pub(crate) fn parse_cluster_message(ast: &syn::DeriveInput) -> syn::Result<ParsedEnum> {
    let name = &ast.ident;

    let enum_data = match &ast.data {
        syn::Data::Enum(data) => data,
        _ => {
            return Err(syn::Error::new(
                name.span(),
                "RactorClusterMessage can only be derived for enums, not structs or unions",
            ));
        }
    };

    let variants = enum_data
        .variants
        .iter()
        .map(parse_variant)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(ParsedEnum {
        ident: name.clone(),
        generics: ast.generics.clone(),
        variants,
    })
}

/// Parse a single enum variant into a `ParsedVariant`.
fn parse_variant(variant: &Variant) -> syn::Result<ParsedVariant> {
    let ident = variant.ident.clone();
    let variant_tag = ident.to_string();
    let is_rpc = check_rpc_attribute(variant)?;

    if is_rpc {
        parse_rpc_variant(variant, ident, variant_tag)
    } else {
        parse_cast_variant(variant, ident, variant_tag)
    }
}

/// Check the `#[rpc]` attribute on a variant: validate it has no arguments, no value,
/// and is not duplicated. Returns `true` if the variant has the `#[rpc]` attribute.
fn check_rpc_attribute(variant: &Variant) -> syn::Result<bool> {
    let rpc_attrs: Vec<_> = variant
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("rpc"))
        .collect();

    if rpc_attrs.is_empty() {
        return Ok(false);
    }

    if rpc_attrs.len() > 1 {
        return Err(syn::Error::new(
            rpc_attrs[1].span(),
            "Duplicate `#[rpc]` attribute on variant",
        ));
    }

    let attr = rpc_attrs[0];
    match &attr.meta {
        syn::Meta::Path(_) => {
            // #[rpc] — correct form
        }
        syn::Meta::List(_) => {
            return Err(syn::Error::new(
                attr.span(),
                "`#[rpc]` attribute does not accept arguments",
            ));
        }
        syn::Meta::NameValue(_) => {
            return Err(syn::Error::new(
                attr.span(),
                "`#[rpc]` attribute does not accept a value",
            ));
        }
    }

    Ok(true)
}

/// Check if a type's last path segment is `RpcReplyPort`.
fn is_reply_port_type(ty: &syn::Type) -> bool {
    if let syn::Type::Path(type_path) = ty {
        type_path
            .path
            .segments
            .last()
            .map(|seg| seg.ident == "RpcReplyPort")
            .unwrap_or(false)
    } else {
        false
    }
}

/// Parse a cast (non-RPC) variant.
fn parse_cast_variant(
    variant: &Variant,
    ident: syn::Ident,
    variant_tag: String,
) -> syn::Result<ParsedVariant> {
    let (data_fields, field_style) = match &variant.fields {
        Fields::Unit => (vec![], FieldStyle::Tuple),
        Fields::Unnamed(unnamed_fields) => {
            let fields: Vec<_> = unnamed_fields
                .unnamed
                .iter()
                .enumerate()
                .map(|(i, arg)| (format_ident!("field{}", i), arg.ty.clone()))
                .collect();
            (fields, FieldStyle::Tuple)
        }
        Fields::Named(named_fields) => {
            let fields: Vec<_> = named_fields
                .named
                .iter()
                .map(|f| (f.ident.clone().unwrap(), f.ty.clone()))
                .collect();
            (fields, FieldStyle::Named)
        }
    };

    // Check if any field is RpcReplyPort without #[rpc] — likely a missing attribute
    for (_, ty) in &data_fields {
        if is_reply_port_type(ty) {
            return Err(syn::Error::new(
                variant.span(),
                "Variant contains an `RpcReplyPort` field but is not marked with `#[rpc]`.\n\
                 \n\
                 Add the `#[rpc]` attribute to this variant:\n  \
                 #[rpc]\n  \
                 YourVariant(..., RpcReplyPort<ReturnType>),",
            ));
        }
    }

    Ok(ParsedVariant {
        ident,
        variant_tag,
        kind: VariantKind::Cast,
        field_style,
        data_fields,
    })
}

/// Parse an RPC variant: validate fields and extract the `RpcReplyPort<T>` generic args.
fn parse_rpc_variant(
    variant: &Variant,
    ident: syn::Ident,
    variant_tag: String,
) -> syn::Result<ParsedVariant> {
    let (all_fields, field_style) = match &variant.fields {
        Fields::Unit => {
            return Err(syn::Error::new(
                variant.span(),
                "RPC calls must have at least one field: an `RpcReplyPort<T>`.\n\
                 \n\
                 Example:\n  #[rpc]\n  YourRpc(RpcReplyPort<YourReturnType>),",
            ));
        }
        Fields::Unnamed(unnamed_fields) => {
            let fields: Vec<_> = unnamed_fields
                .unnamed
                .iter()
                .enumerate()
                .map(|(i, arg)| (format_ident!("field{}", i), arg.ty.clone()))
                .collect();
            (fields, FieldStyle::Tuple)
        }
        Fields::Named(named_fields) => {
            let fields: Vec<_> = named_fields
                .named
                .iter()
                .map(|f| (f.ident.clone().unwrap(), f.ty.clone()))
                .collect();
            (fields, FieldStyle::Named)
        }
    };

    // Scan all fields for RpcReplyPort by type name, collecting owned data
    let port_matches: Vec<usize> = all_fields
        .iter()
        .enumerate()
        .filter(|(_, (_, ty))| is_reply_port_type(ty))
        .map(|(i, _)| i)
        .collect();

    if port_matches.is_empty() {
        return Err(syn::Error::new(
            variant.span(),
            "`#[rpc]` variant must contain exactly one `RpcReplyPort<T>` field.\n\
             \n\
             Example:\n  #[rpc]\n  YourRpc(YourArgs, RpcReplyPort<YourReturnType>),",
        ));
    }

    if port_matches.len() > 1 {
        return Err(syn::Error::new(
            variant.span(),
            "`#[rpc]` variant must contain exactly one `RpcReplyPort<T>` field, but found multiple",
        ));
    }

    let port_index = port_matches[0];
    let reply_port_field_name = all_fields[port_index].0.clone();
    let port_generic_args = if let syn::Type::Path(path_data) = &all_fields[port_index].1 {
        extract_reply_port_args(path_data)?
    } else {
        unreachable!("is_reply_port_type already verified this is a Type::Path")
    };

    // data_fields = all fields except the port
    let data_fields: Vec<_> = all_fields
        .into_iter()
        .enumerate()
        .filter(|(i, _)| *i != port_index)
        .map(|(_, f)| f)
        .collect();

    Ok(ParsedVariant {
        ident,
        variant_tag,
        kind: VariantKind::Call {
            reply_port_generic_args: port_generic_args,
            reply_port_field_name,
            reply_port_index: port_index,
        },
        field_style,
        data_fields,
    })
}

/// Extract the generic arguments from an `RpcReplyPort<T>` type path.
fn extract_reply_port_args(
    path_data: &TypePath,
) -> syn::Result<syn::AngleBracketedGenericArguments> {
    let last_segment = path_data.path.segments.last().ok_or_else(|| {
        syn::Error::new(
            path_data.span(),
            "Expected a type path with at least one segment (e.g., RpcReplyPort<T>)",
        )
    })?;

    if let syn::PathArguments::AngleBracketed(generic_args) = &last_segment.arguments {
        Ok(generic_args.clone())
    } else {
        Err(syn::Error::new(
            last_segment.span(),
            "RpcReplyPort must have generic type arguments.\n\
             \n\
             Expected: RpcReplyPort<YourReturnType>\n\
             \n\
             The generic argument specifies what type will be returned by the RPC call.",
        ))
    }
}
