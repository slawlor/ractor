// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Intermediate representation types for parsed `RactorClusterMessage` enums.

use syn::AngleBracketedGenericArguments;
use syn::Generics;
use syn::Ident;

/// Whether the variant uses tuple or struct-style fields.
pub(crate) enum FieldStyle {
    /// Tuple variant: `Variant(A, B)`
    Tuple,
    /// Struct variant: `Variant { a: A, b: B }`
    Named,
}

/// A fully parsed and validated enum derive input.
pub(crate) struct ParsedEnum {
    pub ident: Ident,
    pub generics: Generics,
    pub variants: Vec<ParsedVariant>,
}

/// A single parsed variant with fields already split (reply port separated for RPC variants).
pub(crate) struct ParsedVariant {
    pub ident: Ident,
    pub variant_tag: String,
    pub kind: VariantKind,
    /// Whether the variant uses tuple or struct-style fields.
    pub field_style: FieldStyle,
    /// Data fields (excludes the `RpcReplyPort` for RPC variants).
    pub data_fields: Vec<(Ident, syn::Type)>,
}

/// Distinguishes cast (fire-and-forget) from call (RPC with reply port) variants.
pub(crate) enum VariantKind {
    /// A fire-and-forget message variant (no `#[rpc]`).
    Cast,
    /// An RPC variant (`#[rpc]`), storing the generic arguments from `RpcReplyPort<T>`.
    Call {
        reply_port_generic_args: AngleBracketedGenericArguments,
        /// The field name/ident for the reply port (synthetic for tuple, original for named).
        reply_port_field_name: Ident,
        /// The 0-based index of the reply port in the original field list.
        reply_port_index: usize,
    },
}
