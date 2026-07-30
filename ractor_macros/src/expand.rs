// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

use std::collections::HashSet;

use proc_macro2::{Span, TokenStream};
use proc_macro_crate::{crate_name, FoundCrate};
use quote::{quote, ToTokens};
use syn::parse::{Parse, ParseStream, Parser};
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{
    punctuated::Punctuated, Attribute, FieldPat, FnArg, ImplItem, ImplItemFn, ItemImpl, Member,
    Meta, Pat, PatIdent, Path, ReturnType, Signature, Token, Type, TypeReference, Visibility,
};

use crate::config::{ActorConfig, GeneratedMessageConfig, MessageConfig};

const LIFECYCLE_METHODS: &[&str] = &[
    "pre_start",
    "post_start",
    "post_stop",
    "handle",
    "handle_serialized",
    "handle_supervisor_evt",
];

pub(crate) fn expand_actor(
    config: ActorConfig,
    mut actor_impl: ItemImpl,
) -> syn::Result<TokenStream> {
    validate_impl(&actor_impl)?;
    if matches!(&config.message, MessageConfig::Generated(_))
        && !actor_impl.generics.params.is_empty()
    {
        return Err(syn::Error::new(
            actor_impl.generics.span(),
            "generated message enums do not yet support generic actor implementations; use `message = ExistingType`",
        ));
    }
    let generated_enum_cfg_attributes = match_arm_cfg_attributes(&actor_impl.attrs)?;

    let ractor = resolve_ractor_path(config.crate_path.as_ref());
    let message_type = config.message.ty();
    let mut inherent_items = Vec::new();
    let mut trait_methods = Vec::new();
    let mut dispatch_handlers = Vec::new();
    let mut supervision_handlers = Vec::new();
    let mut raw_handle_span = None;
    let mut raw_supervision_span = None;

    for item in actor_impl.items {
        let ImplItem::Fn(mut method) = item else {
            inherent_items.push(item);
            continue;
        };

        let message_attributes =
            take_handler_attributes(&mut method.attrs, &ractor, "message", true);
        let rpc_attributes = take_handler_attributes(&mut method.attrs, &ractor, "rpc", false);
        let supervision_attributes =
            take_handler_attributes(&mut method.attrs, &ractor, "supervision", true);
        if message_attributes.len() > 1 {
            return Err(syn::Error::new(
                message_attributes[1].span(),
                "message handler methods may only have one `#[ractor::message(...)]` attribute",
            ));
        }
        if supervision_attributes.len() > 1 {
            return Err(syn::Error::new(
                supervision_attributes[1].span(),
                "supervision handler methods may only have one `#[ractor::supervision(...)]` attribute",
            ));
        }
        if rpc_attributes.len() > 1 {
            return Err(syn::Error::new(
                rpc_attributes[1].span(),
                "RPC handler methods may only have one `#[ractor::rpc(...)]` attribute",
            ));
        }
        let handler_attribute_count = usize::from(!message_attributes.is_empty())
            + usize::from(!rpc_attributes.is_empty())
            + usize::from(!supervision_attributes.is_empty());
        if handler_attribute_count > 1 {
            return Err(syn::Error::new(
                method.sig.ident.span(),
                "a method can only be one of a message, RPC, or supervision handler",
            ));
        }

        if let Some(attribute) = message_attributes.first() {
            let mut pattern = attribute.parse_args_with(Pat::parse_single)?;
            if let MessageConfig::Generated(config) = &config.message {
                qualify_generated_message_pattern(&mut pattern, &config.ident)?;
            }
            dispatch_handlers.push(parse_handler(&method, pattern, HandlerKind::Message)?);
            inherent_items.push(ImplItem::Fn(method));
            continue;
        }

        if let Some(attribute) = rpc_attributes.first() {
            let RpcHandlerAttribute {
                mut pattern,
                reply_type,
            } = attribute.parse_args()?;
            if let MessageConfig::Generated(config) = &config.message {
                qualify_generated_message_pattern(&mut pattern, &config.ident)?;
            }
            inject_unit_rpc_reply(&mut pattern)?;
            dispatch_handlers.push(parse_rpc_handler(&method, pattern, reply_type, &ractor)?);
            inherent_items.push(ImplItem::Fn(method));
            continue;
        }

        if let Some(attribute) = supervision_attributes.first() {
            let pattern = attribute.parse_args_with(Pat::parse_single)?;
            supervision_handlers.push(parse_handler(&method, pattern, HandlerKind::Supervision)?);
            inherent_items.push(ImplItem::Fn(method));
            continue;
        }

        if is_lifecycle_method(&method.sig.ident) {
            validate_lifecycle_method(&method)?;
            if method.sig.ident == "handle" {
                raw_handle_span = Some(method.sig.ident.span());
            } else if method.sig.ident == "handle_supervisor_evt" {
                raw_supervision_span = Some(method.sig.ident.span());
            }
            trait_methods.push(method);
        } else {
            inherent_items.push(ImplItem::Fn(method));
        }
    }

    if let Some(span) = raw_handle_span {
        if !dispatch_handlers.is_empty() {
            return Err(syn::Error::new(
                span,
                "define either a raw `handle` method or focused `#[ractor::message(...)]`/`#[ractor::rpc(...)]` handlers, not both",
            ));
        }
    }
    if let Some(span) = raw_supervision_span {
        if !supervision_handlers.is_empty() {
            return Err(syn::Error::new(
                span,
                "define either a raw `handle_supervisor_evt` method or `#[ractor::supervision(...)]` handlers, not both",
            ));
        }
    }

    validate_unique_patterns(&dispatch_handlers, HandlerKind::Message)?;
    validate_unique_patterns(&supervision_handlers, HandlerKind::Supervision)?;

    let has_pre_start = trait_methods
        .iter()
        .any(|method| method.sig.ident == "pre_start");
    if !has_pre_start {
        if !is_unit_type(&config.state) || !is_unit_type(&config.arguments) {
            return Err(syn::Error::new(
                actor_impl.self_ty.span(),
                "`pre_start` is required unless both `state` and `arguments` are `()`",
            ));
        }
        trait_methods.push(default_pre_start(&ractor));
    }

    if raw_handle_span.is_none() && !dispatch_handlers.is_empty() {
        trait_methods.push(generated_handle(&ractor, &dispatch_handlers)?);
    }
    if raw_supervision_span.is_none() && !supervision_handlers.is_empty() {
        trait_methods.push(generated_supervision_handler(
            &ractor,
            &supervision_handlers,
        )?);
    }

    let ActorConfig {
        thread_local,
        message,
        state,
        arguments,
        crate_path: _,
    } = config;
    let generated_message = match &message {
        MessageConfig::Existing(_) => TokenStream::new(),
        MessageConfig::Generated(config) => {
            generate_message_enum(config, &dispatch_handlers, &generated_enum_cfg_attributes)?
        }
    };
    let trait_path = if thread_local {
        quote!(#ractor::thread_local::ThreadLocalActor)
    } else {
        quote!(#ractor::Actor)
    };
    let async_trait_attribute = if !thread_local && cfg!(feature = "async-trait") {
        quote!(#[#ractor::async_trait])
    } else {
        TokenStream::new()
    };

    let attrs = &actor_impl.attrs;
    let self_ty = &actor_impl.self_ty;
    let (impl_generics, _, where_clause) = actor_impl.generics.split_for_impl();
    let trait_impl = quote! {
        #(#attrs)*
        #async_trait_attribute
        impl #impl_generics #trait_path for #self_ty #where_clause {
            type Msg = #message_type;
            type State = #state;
            type Arguments = #arguments;

            #(#trait_methods)*
        }
    };

    let inherent_impl = if inherent_items.is_empty() {
        TokenStream::new()
    } else {
        actor_impl.items = inherent_items;
        quote!(#actor_impl)
    };

    Ok(quote! {
        #generated_message
        #inherent_impl
        #trait_impl
    })
}

fn validate_impl(actor_impl: &ItemImpl) -> syn::Result<()> {
    if actor_impl.trait_.is_some() {
        return Err(syn::Error::new(
            actor_impl.impl_token.span,
            "`#[ractor::actor]` must be applied to an inherent implementation block",
        ));
    }
    if actor_impl.unsafety.is_some() {
        return Err(syn::Error::new(
            actor_impl.impl_token.span,
            "unsafe actor implementation blocks are not supported",
        ));
    }
    if let Some(attribute) = actor_impl
        .attrs
        .iter()
        .find(|attribute| applies_async_trait(attribute))
    {
        return Err(syn::Error::new(
            attribute.span(),
            "remove `async_trait` from this implementation; `#[ractor::actor]` applies it when needed",
        ));
    }
    Ok(())
}

fn applies_async_trait(attribute: &Attribute) -> bool {
    if path_ends_with_async_trait(attribute.path()) {
        return true;
    }
    if !attribute.path().is_ident("cfg_attr") {
        return false;
    }

    attribute
        .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
        .map(|items| {
            items
                .iter()
                .skip(1)
                .any(|meta| path_ends_with_async_trait(meta.path()))
        })
        .unwrap_or(false)
}

fn path_ends_with_async_trait(path: &Path) -> bool {
    path.segments
        .last()
        .is_some_and(|segment| segment.ident == "async_trait")
}

fn resolve_ractor_path(explicit_path: Option<&Path>) -> Path {
    resolve_ractor_path_with(explicit_path, || crate_name("ractor"))
}

fn resolve_ractor_path_with<E>(
    explicit_path: Option<&Path>,
    find_crate: impl FnOnce() -> Result<FoundCrate, E>,
) -> Path {
    if let Some(path) = explicit_path {
        return path.clone();
    }

    match find_crate() {
        Ok(FoundCrate::Itself) => syn::parse_quote!(crate),
        Ok(FoundCrate::Name(name)) => {
            let crate_ident = syn::Ident::new(&name.replace('-', "_"), Span::call_site());
            syn::parse_quote!(::#crate_ident)
        }
        Err(_) => syn::parse_quote!(::ractor),
    }
}

#[cfg(test)]
mod resolve_ractor_path_tests {
    use super::*;

    fn path_string(path: &Path) -> String {
        path.to_token_stream().to_string()
    }

    #[test]
    fn lookup_failure_falls_back_to_ractor() {
        let path = resolve_ractor_path_with(None, || Err::<FoundCrate, _>("lookup failed"));

        assert_eq!(path_string(&path), ":: ractor");
    }

    #[test]
    fn explicit_path_takes_priority() {
        let explicit: Path = syn::parse_quote!(reexport::ractor);
        let path = resolve_ractor_path_with(Some(&explicit), || -> Result<FoundCrate, ()> {
            panic!("crate lookup should not run when crate_path is explicit")
        });

        assert_eq!(path_string(&path), path_string(&explicit));
    }

    #[test]
    fn renamed_dependency_uses_discovered_name() {
        let path = resolve_ractor_path_with(None, || {
            Ok::<_, ()>(FoundCrate::Name("renamed-ractor".to_owned()))
        });

        assert_eq!(path_string(&path), ":: renamed_ractor");
    }

    #[test]
    fn self_crate_uses_crate_path() {
        let path = resolve_ractor_path_with(None, || Ok::<_, ()>(FoundCrate::Itself));

        assert_eq!(path_string(&path), "crate");
    }
}

fn take_handler_attributes(
    attributes: &mut Vec<Attribute>,
    ractor: &Path,
    handler_attribute: &str,
    allow_bare: bool,
) -> Vec<Attribute> {
    let mut handler_attributes = Vec::new();
    attributes.retain(|attribute| {
        let is_handler = is_handler_attribute(attribute, ractor, handler_attribute, allow_bare);
        if is_handler {
            handler_attributes.push(attribute.clone());
        }
        !is_handler
    });
    handler_attributes
}

fn is_handler_attribute(
    attribute: &Attribute,
    ractor: &Path,
    handler_attribute: &str,
    allow_bare: bool,
) -> bool {
    let path = attribute.path();
    if allow_bare && path.is_ident(handler_attribute) {
        return true;
    }
    if path.segments.len() != ractor.segments.len() + 1 {
        return false;
    }

    path.segments
        .iter()
        .zip(&ractor.segments)
        .all(|(actual, expected)| actual.ident == expected.ident)
        && path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == handler_attribute)
}

#[cfg(test)]
mod handler_attribute_tests {
    use super::*;

    #[test]
    fn bare_rpc_is_not_consumed() {
        let ractor: Path = syn::parse_quote!(::renamed_ractor);
        let bare_rpc: Attribute = syn::parse_quote!(#[rpc(Message::Read(reply))]);
        let qualified_rpc: Attribute =
            syn::parse_quote!(#[renamed_ractor::rpc(Message::Read(reply))]);
        let mut attributes = vec![bare_rpc, qualified_rpc];
        let rpc_attributes = take_handler_attributes(&mut attributes, &ractor, "rpc", false);

        assert_eq!(rpc_attributes.len(), 1);
        assert_eq!(attributes.len(), 1);
        assert_eq!(rpc_attributes[0].path().segments[0].ident, "renamed_ractor");
        assert!(attributes[0].path().is_ident("rpc"));
    }

    #[test]
    fn existing_handler_attributes_keep_bare_compatibility() {
        let ractor: Path = syn::parse_quote!(::renamed_ractor);
        let bare_message: Attribute = syn::parse_quote!(#[message(Message::Go)]);
        let bare_supervision: Attribute =
            syn::parse_quote!(#[supervision(SupervisionEvent::ActorStarted(child))]);

        assert!(is_handler_attribute(
            &bare_message,
            &ractor,
            "message",
            true
        ));
        assert!(is_handler_attribute(
            &bare_supervision,
            &ractor,
            "supervision",
            true
        ));
    }
}

fn is_lifecycle_method(ident: &syn::Ident) -> bool {
    LIFECYCLE_METHODS
        .iter()
        .any(|method_name| ident == method_name)
}

fn validate_lifecycle_method(method: &ImplItemFn) -> syn::Result<()> {
    if !matches!(method.vis, Visibility::Inherited) {
        return Err(syn::Error::new(
            method.vis.span(),
            "actor lifecycle methods must be private",
        ));
    }
    validate_method_shape(&method.sig, "actor lifecycle methods")?;
    if method.sig.asyncness.is_none() {
        return Err(syn::Error::new(
            method.sig.fn_token.span,
            "actor lifecycle methods must be `async fn`",
        ));
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum StateAccess {
    Shared,
    Mutable,
}

#[derive(Clone, Copy)]
enum HandlerKind {
    Message,
    Rpc,
    Supervision,
}

impl HandlerKind {
    fn method_description(self) -> &'static str {
        match self {
            Self::Message => "message handler methods",
            Self::Rpc => "RPC handler methods",
            Self::Supervision => "supervision handler methods",
        }
    }

    fn parameter_label(self) -> &'static str {
        match self {
            Self::Message => "handler",
            Self::Rpc => "RPC handler",
            Self::Supervision => "supervision handler",
        }
    }

    fn pattern_label(self) -> &'static str {
        match self {
            Self::Message => "message pattern",
            Self::Rpc => "RPC pattern",
            Self::Supervision => "supervision pattern",
        }
    }

    fn event_label(self) -> &'static str {
        match self {
            Self::Message | Self::Rpc => "message",
            Self::Supervision => "supervision event",
        }
    }
}

struct RpcHandlerAttribute {
    pattern: Pat,
    reply_type: Option<Type>,
}

impl Parse for RpcHandlerAttribute {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let pattern = Pat::parse_single(input)?;
        let mut reply_type = None;

        if input.parse::<Option<Token![,]>>()?.is_some() && !input.is_empty() {
            let option: syn::Ident = input.parse()?;
            if option != "reply" {
                return Err(syn::Error::new(
                    option.span(),
                    "unknown RPC option; expected `reply = Type`",
                ));
            }
            input.parse::<Token![=]>()?;
            reply_type = Some(input.parse()?);
            input.parse::<Option<Token![,]>>()?;
        }

        if !input.is_empty() {
            return Err(input.error("unexpected tokens after RPC handler options"));
        }

        Ok(Self {
            pattern,
            reply_type,
        })
    }
}

struct RpcHandler {
    reply_binding: syn::Ident,
    reply_type: Type,
    propagates_processing_error: bool,
}

struct Handler {
    method_name: syn::Ident,
    pattern: Pat,
    pattern_key: String,
    binding_names: Vec<syn::Ident>,
    binding_types: Vec<Type>,
    wants_actor_ref: bool,
    state_access: Option<StateAccess>,
    is_async: bool,
    is_fallible: bool,
    rpc: Option<RpcHandler>,
    cfg_attributes: Vec<Attribute>,
}

fn parse_handler(
    method: &ImplItemFn,
    pattern: Pat,
    handler_kind: HandlerKind,
) -> syn::Result<Handler> {
    validate_method_shape(&method.sig, handler_kind.method_description())?;
    let (pattern_key, binding_names) = parse_handler_pattern(&pattern, handler_kind)?;
    let parameter_label = handler_kind.parameter_label();
    let parameters = typed_parameters(&method.sig, parameter_label)?;
    let (wants_actor_ref, state_access) = classify_handler_parameters(
        &parameters,
        &binding_names,
        method.sig.ident.span(),
        parameter_label,
        handler_kind.pattern_label(),
    )?;
    let payload_start = usize::from(wants_actor_ref);
    let payload_end = parameters.len() - usize::from(state_access.is_some());
    let binding_types = parameters[payload_start..payload_end]
        .iter()
        .map(|parameter| (*parameter.ty).clone())
        .collect();

    let cfg_attributes = match_arm_cfg_attributes(&method.attrs)?;

    Ok(Handler {
        method_name: method.sig.ident.clone(),
        pattern,
        pattern_key,
        binding_names,
        binding_types,
        wants_actor_ref,
        state_access,
        is_async: method.sig.asyncness.is_some(),
        is_fallible: !returns_unit(&method.sig.output),
        rpc: None,
        cfg_attributes,
    })
}

fn parse_rpc_handler(
    method: &ImplItemFn,
    pattern: Pat,
    explicit_reply_type: Option<Type>,
    ractor: &Path,
) -> syn::Result<Handler> {
    validate_method_shape(&method.sig, HandlerKind::Rpc.method_description())?;
    let (pattern_key, pattern_bindings) = parse_handler_pattern(&pattern, HandlerKind::Rpc)?;
    let parameters = typed_parameters(&method.sig, HandlerKind::Rpc.parameter_label())?;

    let propagates_processing_error = explicit_reply_type.is_some();
    let reply_type = match explicit_reply_type {
        Some(reply_type) => {
            if returns_unit(&method.sig.output) {
                return Err(syn::Error::new(
                    method.sig.output.span(),
                    "an RPC handler using `reply = Type` must return a fallible value; `Result<Type, ActorProcessingErr>` is the usual form",
                ));
            }
            reply_type
        }
        None => return_type(&method.sig.output),
    };
    if let Some(span) = impl_trait_span(&reply_type) {
        return Err(syn::Error::new(
            span,
            "RPC reply types must be concrete; `impl Trait` is not supported anywhere in a reply type",
        ));
    }

    let mut candidates = Vec::new();
    for reply_index in 0..pattern_bindings.len() {
        let payload_bindings = pattern_bindings
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != reply_index)
            .map(|(_, binding)| binding.clone())
            .collect::<Vec<_>>();
        if let Ok((wants_actor_ref, state_access)) = classify_handler_parameters(
            &parameters,
            &payload_bindings,
            method.sig.ident.span(),
            HandlerKind::Rpc.parameter_label(),
            HandlerKind::Rpc.pattern_label(),
        ) {
            candidates.push((reply_index, payload_bindings, wants_actor_ref, state_access));
        }
    }

    let (reply_index, binding_names, wants_actor_ref, state_access) = match candidates.len() {
        1 => candidates.pop().expect("one RPC reply candidate was found"),
        0 => {
            return Err(syn::Error::new(
                method.sig.ident.span(),
                "RPC handler parameters must match every RPC pattern binding except exactly one reply-port binding, with an optional leading `ActorRef` and optional trailing state reference",
            ));
        }
        _ => {
            return Err(syn::Error::new(
                method.sig.ident.span(),
                "RPC reply port is ambiguous; exactly one RPC pattern binding must be absent from the method parameters",
            ));
        }
    };

    let payload_start = usize::from(wants_actor_ref);
    let payload_end = parameters.len() - usize::from(state_access.is_some());
    let mut payload_types = parameters[payload_start..payload_end]
        .iter()
        .map(|parameter| (*parameter.ty).clone());
    let binding_types = pattern_bindings
        .iter()
        .enumerate()
        .map(|(index, _)| {
            if index == reply_index {
                syn::parse_quote!(#ractor::RpcReplyPort<#reply_type>)
            } else {
                payload_types
                    .next()
                    .expect("validated RPC payload binding is missing its parameter type")
            }
        })
        .collect();
    debug_assert!(payload_types.next().is_none());

    let cfg_attributes = match_arm_cfg_attributes(&method.attrs)?;
    Ok(Handler {
        method_name: method.sig.ident.clone(),
        pattern,
        pattern_key,
        binding_names,
        binding_types,
        wants_actor_ref,
        state_access,
        is_async: method.sig.asyncness.is_some(),
        is_fallible: false,
        rpc: Some(RpcHandler {
            reply_binding: pattern_bindings[reply_index].clone(),
            reply_type,
            propagates_processing_error,
        }),
        cfg_attributes,
    })
}

fn match_arm_cfg_attributes(attributes: &[Attribute]) -> syn::Result<Vec<Attribute>> {
    let mut filtered_attributes = Vec::new();
    for attribute in attributes {
        if let Some(meta) = filter_cfg_meta(&attribute.meta)? {
            let mut filtered_attribute = attribute.clone();
            filtered_attribute.meta = meta;
            filtered_attributes.push(filtered_attribute);
        }
    }
    Ok(filtered_attributes)
}

fn filter_cfg_meta(meta: &Meta) -> syn::Result<Option<Meta>> {
    if meta.path().is_ident("cfg") {
        return Ok(Some(meta.clone()));
    }
    if !meta.path().is_ident("cfg_attr") {
        return Ok(None);
    }

    let Meta::List(list) = meta else {
        return Err(syn::Error::new(
            meta.span(),
            "`cfg_attr` must use list syntax",
        ));
    };
    let items = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(list.tokens.clone())?;
    let mut items = items.into_iter();
    let Some(condition) = items.next() else {
        return Err(syn::Error::new(
            meta.span(),
            "`cfg_attr` requires a condition and at least one attribute",
        ));
    };
    if items.len() == 0 {
        return Err(syn::Error::new(
            meta.span(),
            "`cfg_attr` requires a condition and at least one attribute",
        ));
    }

    let mut filtered = Vec::new();
    for nested in items {
        if let Some(nested) = filter_cfg_meta(&nested)? {
            filtered.push(nested);
        }
    }
    if filtered.is_empty() {
        return Ok(None);
    }

    let mut filtered_list = list.clone();
    filtered_list.tokens = quote!(#condition, #(#filtered),*);
    Ok(Some(Meta::List(filtered_list)))
}

fn validate_method_shape(signature: &Signature, description: &str) -> syn::Result<()> {
    if signature.constness.is_some()
        || signature.unsafety.is_some()
        || signature.abi.is_some()
        || signature.variadic.is_some()
        || !signature.generics.params.is_empty()
        || signature.generics.where_clause.is_some()
    {
        return Err(syn::Error::new(
            signature.span(),
            format!("{description} cannot be const, unsafe, extern, variadic, or generic"),
        ));
    }

    let Some(FnArg::Receiver(receiver)) = signature.inputs.first() else {
        return Err(syn::Error::new(
            signature.span(),
            format!("{description} must begin with `&self`"),
        ));
    };
    if receiver.reference.is_none()
        || receiver.mutability.is_some()
        || receiver.colon_token.is_some()
    {
        return Err(syn::Error::new(
            receiver.span(),
            format!("{description} must use an immutable `&self` receiver"),
        ));
    }
    Ok(())
}

struct Parameter<'a> {
    ident: &'a syn::Ident,
    ty: &'a Type,
}

fn typed_parameters<'a>(
    signature: &'a Signature,
    handler_label: &str,
) -> syn::Result<Vec<Parameter<'a>>> {
    signature
        .inputs
        .iter()
        .skip(1)
        .map(|argument| match argument {
            FnArg::Typed(argument) => {
                let Pat::Ident(PatIdent {
                    by_ref: None,
                    ident,
                    subpat: None,
                    ..
                }) = argument.pat.as_ref()
                else {
                    return Err(syn::Error::new(
                        argument.pat.span(),
                        format!("{handler_label} parameters must be simple identifier bindings"),
                    ));
                };
                Ok(Parameter {
                    ident,
                    ty: argument.ty.as_ref(),
                })
            }
            FnArg::Receiver(receiver) => Err(syn::Error::new(
                receiver.span(),
                format!("{handler_label} methods may only have one receiver"),
            )),
        })
        .collect()
}

fn classify_handler_parameters(
    parameters: &[Parameter<'_>],
    bindings: &[syn::Ident],
    error_span: Span,
    handler_label: &str,
    pattern_label: &str,
) -> syn::Result<(bool, Option<StateAccess>)> {
    let without_state = match_handler_parameters(parameters, bindings, None);
    let with_state = parameters.last().and_then(|parameter| {
        state_reference_access(parameter.ty).and_then(|access| {
            match_handler_parameters(&parameters[..parameters.len() - 1], bindings, Some(access))
        })
    });

    match (without_state, with_state) {
        (Some(result), None) | (None, Some(result)) => Ok(result),
        (Some(_), Some(_)) => Err(syn::Error::new(
            error_span,
            format!("{handler_label} parameters are ambiguous; rename the actor reference or state parameter so payload bindings remain explicit"),
        )),
        (None, None) => Err(syn::Error::new(
            error_span,
            format!("{handler_label} parameters must match the {pattern_label} bindings, with an optional leading `ActorRef` and optional trailing state reference"),
        )),
    }
}

fn match_handler_parameters(
    parameters: &[Parameter<'_>],
    bindings: &[syn::Ident],
    state_access: Option<StateAccess>,
) -> Option<(bool, Option<StateAccess>)> {
    let (wants_actor_ref, payload_parameters) = if parameters.len() == bindings.len() {
        (false, parameters)
    } else if parameters.len() == bindings.len() + 1
        && parameters
            .first()
            .is_some_and(|parameter| is_actor_ref(parameter.ty))
    {
        (true, &parameters[1..])
    } else {
        return None;
    };

    payload_parameters
        .iter()
        .zip(bindings)
        .all(|(parameter, binding)| parameter.ident == binding)
        .then_some((wants_actor_ref, state_access))
}

fn state_reference_access(ty: &Type) -> Option<StateAccess> {
    let Type::Reference(TypeReference { mutability, .. }) = unparenthesized_type(ty) else {
        return None;
    };
    Some(if mutability.is_some() {
        StateAccess::Mutable
    } else {
        StateAccess::Shared
    })
}

fn unparenthesized_type(mut ty: &Type) -> &Type {
    loop {
        ty = match ty {
            Type::Group(group) => group.elem.as_ref(),
            Type::Paren(parenthesized) => parenthesized.elem.as_ref(),
            _ => return ty,
        };
    }
}

fn is_actor_ref(ty: &Type) -> bool {
    let Type::Path(path) = unparenthesized_type(ty) else {
        return false;
    };
    path.path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "ActorRef")
}

fn qualify_generated_message_pattern(
    pattern: &mut Pat,
    message_ident: &syn::Ident,
) -> syn::Result<()> {
    if let Pat::Ident(PatIdent {
        attrs,
        by_ref: None,
        mutability: None,
        ident,
        subpat: None,
    }) = pattern
    {
        if attrs.is_empty() {
            let variant = ident.clone();
            *pattern = syn::parse_quote!(#message_ident::#variant);
            return Ok(());
        }
    }

    let path = match pattern {
        Pat::Path(path) if path.qself.is_none() => &mut path.path,
        Pat::TupleStruct(tuple) if tuple.qself.is_none() => &mut tuple.path,
        Pat::Struct(structure) if structure.qself.is_none() => &mut structure.path,
        _ => {
            return Err(syn::Error::new(
                pattern.span(),
                "generated message patterns must name a unit, tuple, or struct enum variant",
            ));
        }
    };

    match path.segments.len() {
        1 => {
            let variant = path.segments[0].ident.clone();
            *path = syn::parse_quote!(#message_ident::#variant);
        }
        2 if path.segments[0].ident == *message_ident => {}
        _ => {
            return Err(syn::Error::new(
                path.span(),
                format!(
                    "generated message patterns must use `Variant` or `{message_ident}::Variant`"
                ),
            ));
        }
    }
    Ok(())
}

fn inject_unit_rpc_reply(pattern: &mut Pat) -> syn::Result<()> {
    let Pat::Path(unit_variant) = pattern else {
        return Ok(());
    };
    if unit_variant.qself.is_some() || !unit_variant.attrs.is_empty() {
        return Ok(());
    }

    let path = &unit_variant.path;
    let reply_binding = syn::Ident::new("__ractor_reply_port", Span::mixed_site());
    *pattern = Pat::parse_single.parse2(quote!(#path(#reply_binding)))?;
    Ok(())
}

fn parse_handler_pattern(
    pattern: &Pat,
    handler_kind: HandlerKind,
) -> syn::Result<(String, Vec<syn::Ident>)> {
    let pattern_description = match handler_kind {
        HandlerKind::Message => "message patterns",
        HandlerKind::Rpc => "RPC patterns",
        HandlerKind::Supervision => "supervision patterns",
    };
    let (path, fields): (&Path, Vec<&Pat>) = match pattern {
        Pat::Path(path) if path.qself.is_none() && path.attrs.is_empty() => {
            (&path.path, Vec::new())
        }
        Pat::TupleStruct(tuple) if tuple.qself.is_none() && tuple.attrs.is_empty() => {
            (&tuple.path, tuple.elems.iter().collect())
        }
        Pat::Struct(structure)
            if structure.qself.is_none()
                && structure.attrs.is_empty()
                && structure.rest.is_none() =>
        {
            (
                &structure.path,
                structure
                    .fields
                    .iter()
                    .map(|field| field.pat.as_ref())
                    .collect(),
            )
        }
        Pat::Struct(structure) if structure.rest.is_some() => {
            return Err(syn::Error::new(
                structure.span(),
                format!("{pattern_description} must list every struct variant field; `..` is not supported"),
            ));
        }
        _ => {
            return Err(syn::Error::new(
                pattern.span(),
                format!("{pattern_description} must be a unit, tuple, or struct enum variant with identifier or `_` fields"),
            ));
        }
    };

    let mut bindings = Vec::new();
    for field in fields {
        match field {
            Pat::Ident(PatIdent {
                attrs,
                by_ref: None,
                mutability: None,
                ident,
                subpat: None,
            }) if attrs.is_empty() => bindings.push(ident.clone()),
            Pat::Wild(wildcard) if wildcard.attrs.is_empty() => {}
            _ => {
                return Err(syn::Error::new(
                    field.span(),
                    format!(
                        "{pattern_description} fields must be plain identifier bindings or `_`"
                    ),
                ));
            }
        }
    }

    Ok((path.to_token_stream().to_string(), bindings))
}

fn validate_unique_patterns(handlers: &[Handler], handler_kind: HandlerKind) -> syn::Result<()> {
    let mut patterns = HashSet::new();
    for handler in handlers {
        if !patterns.insert(&handler.pattern_key) {
            return Err(syn::Error::new(
                handler.pattern.span(),
                format!(
                    "this {} variant already has a handler",
                    handler_kind.event_label()
                ),
            ));
        }
    }
    Ok(())
}

fn generate_message_enum(
    config: &GeneratedMessageConfig,
    handlers: &[Handler],
    cfg_attributes: &[Attribute],
) -> syn::Result<TokenStream> {
    let visibility = &config.visibility;
    let enum_ident = &config.ident;
    let variants = handlers
        .iter()
        .map(generate_message_variant)
        .collect::<syn::Result<Vec<_>>>()?;

    Ok(quote! {
        #(#cfg_attributes)*
        #[allow(missing_debug_implementations, missing_docs)]
        #visibility enum #enum_ident {
            #(#variants),*
        }
    })
}

fn generate_message_variant(handler: &Handler) -> syn::Result<TokenStream> {
    let cfg_attributes = &handler.cfg_attributes;
    let variant_ident = match &handler.pattern {
        Pat::Path(path) => path.path.segments.last().map(|segment| &segment.ident),
        Pat::TupleStruct(tuple) => tuple.path.segments.last().map(|segment| &segment.ident),
        Pat::Struct(structure) => structure.path.segments.last().map(|segment| &segment.ident),
        _ => None,
    }
    .ok_or_else(|| {
        syn::Error::new(
            handler.pattern.span(),
            "generated message pattern is missing a variant name",
        )
    })?;

    match &handler.pattern {
        Pat::Path(_) => Ok(quote! {
            #(#cfg_attributes)*
            #variant_ident
        }),
        Pat::TupleStruct(tuple) => {
            if tuple
                .elems
                .iter()
                .any(|element| matches!(element, Pat::Wild(_)))
            {
                return Err(syn::Error::new(
                    tuple.span(),
                    "generated message patterns cannot discard fields with `_` because every field needs a type",
                ));
            }
            let field_types = &handler.binding_types;
            Ok(quote! {
                #(#cfg_attributes)*
                #variant_ident(#(#field_types),*)
            })
        }
        Pat::Struct(structure) => {
            if structure
                .fields
                .iter()
                .any(|field| matches!(field.pat.as_ref(), Pat::Wild(_)))
            {
                return Err(syn::Error::new(
                    structure.span(),
                    "generated message patterns cannot discard fields with `_` because every field needs a type",
                ));
            }
            let fields = structure
                .fields
                .iter()
                .zip(&handler.binding_types)
                .map(|(field, ty)| generated_named_field(field, ty))
                .collect::<syn::Result<Vec<_>>>()?;
            Ok(quote! {
                #(#cfg_attributes)*
                #variant_ident { #(#fields),* }
            })
        }
        _ => unreachable!("handler patterns were validated before enum generation"),
    }
}

fn generated_named_field(field: &FieldPat, ty: &Type) -> syn::Result<TokenStream> {
    if matches!(field.pat.as_ref(), Pat::Wild(_)) {
        return Err(syn::Error::new(
            field.span(),
            "generated message patterns cannot discard fields with `_` because every field needs a type",
        ));
    }
    let Member::Named(field_ident) = &field.member else {
        return Err(syn::Error::new(
            field.member.span(),
            "generated struct message fields must be named",
        ));
    };
    Ok(quote!(#field_ident: #ty))
}

fn returns_unit(output: &ReturnType) -> bool {
    match output {
        ReturnType::Default => true,
        ReturnType::Type(_, ty) => is_unit_type(ty),
    }
}

fn return_type(output: &ReturnType) -> Type {
    match output {
        ReturnType::Default => syn::parse_quote!(()),
        ReturnType::Type(_, ty) => (**ty).clone(),
    }
}

fn impl_trait_span(ty: &Type) -> Option<Span> {
    #[derive(Default)]
    struct Finder {
        span: Option<Span>,
    }

    impl<'ast> Visit<'ast> for Finder {
        fn visit_type_impl_trait(&mut self, node: &'ast syn::TypeImplTrait) {
            if self.span.is_none() {
                self.span = Some(node.span());
            }
        }
    }

    let mut finder = Finder::default();
    finder.visit_type(ty);
    finder.span
}

fn is_unit_type(ty: &Type) -> bool {
    matches!(unparenthesized_type(ty), Type::Tuple(tuple) if tuple.elems.is_empty())
}

fn default_pre_start(ractor: &Path) -> ImplItemFn {
    syn::parse_quote! {
        async fn pre_start(
            &self,
            _myself: #ractor::ActorRef<Self::Msg>,
            _args: Self::Arguments,
        ) -> ::core::result::Result<Self::State, #ractor::ActorProcessingErr> {
            ::core::result::Result::Ok(())
        }
    }
}

fn generated_handle(ractor: &Path, handlers: &[Handler]) -> syn::Result<ImplItemFn> {
    let myself_ident = syn::Ident::new("__ractor_myself", Span::mixed_site());
    let message_ident = syn::Ident::new("__ractor_message", Span::mixed_site());
    let state_ident = syn::Ident::new("__ractor_state", Span::mixed_site());
    let arms = generated_handler_arms(handlers, &myself_ident, &state_ident);

    syn::parse2(quote! {
        #[allow(unused_variables)]
        async fn handle(
            &self,
            #myself_ident: #ractor::ActorRef<Self::Msg>,
            #message_ident: Self::Msg,
            #state_ident: &mut Self::State,
        ) -> ::core::result::Result<(), #ractor::ActorProcessingErr> {
            match #message_ident {
                #(#arms),*
            }
        }
    })
}

fn generated_supervision_handler(ractor: &Path, handlers: &[Handler]) -> syn::Result<ImplItemFn> {
    let myself_ident = syn::Ident::new("__ractor_myself", Span::mixed_site());
    let event_ident = syn::Ident::new("__ractor_supervision_event", Span::mixed_site());
    let state_ident = syn::Ident::new("__ractor_state", Span::mixed_site());
    let arms = generated_handler_arms(handlers, &myself_ident, &state_ident);

    syn::parse2(quote! {
        #[allow(unused_variables)]
        async fn handle_supervisor_evt(
            &self,
            #myself_ident: #ractor::ActorRef<Self::Msg>,
            #event_ident: #ractor::SupervisionEvent,
            #state_ident: &mut Self::State,
        ) -> ::core::result::Result<(), #ractor::ActorProcessingErr> {
            match #event_ident {
                #(#arms),*,
                __ractor_unhandled_event => {
                    match __ractor_unhandled_event {
                        #ractor::SupervisionEvent::ActorTerminated(_, _, _)
                        | #ractor::SupervisionEvent::ActorFailed(_, _) => {
                            #myself_ident.stop(::core::option::Option::None);
                        }
                        _ => {}
                    }
                    ::core::result::Result::Ok(())
                }
            }
        }
    })
}

fn generated_handler_arms(
    handlers: &[Handler],
    myself_ident: &syn::Ident,
    state_ident: &syn::Ident,
) -> Vec<TokenStream> {
    handlers
        .iter()
        .map(|handler| {
            let cfg_attributes = &handler.cfg_attributes;
            let pattern = &handler.pattern;
            let method_name = &handler.method_name;
            let mut arguments = Vec::new();
            if handler.wants_actor_ref {
                arguments.push(quote!(#myself_ident));
            }
            arguments.extend(handler.binding_names.iter().map(|binding| quote!(#binding)));
            if let Some(state_access) = handler.state_access {
                arguments.push(match state_access {
                    StateAccess::Shared => quote!(&*#state_ident),
                    StateAccess::Mutable => quote!(&mut *#state_ident),
                });
            }

            let await_suffix = handler.is_async.then(|| quote!(.await));
            if let Some(rpc) = &handler.rpc {
                let reply_binding = &rpc.reply_binding;
                let reply_type = &rpc.reply_type;
                let try_suffix = rpc.propagates_processing_error.then(|| quote!(?));
                let reply_value = syn::Ident::new("__ractor_reply_value", Span::mixed_site());
                return quote! {
                    #(#cfg_attributes)*
                    #pattern => {
                        let #reply_value: #reply_type =
                            self.#method_name(#(#arguments),*) #await_suffix #try_suffix;
                        let _ = #reply_binding.send(#reply_value);
                        ::core::result::Result::Ok(())
                    }
                };
            }
            let try_suffix = handler.is_fallible.then(|| quote!(?));
            quote! {
                #(#cfg_attributes)*
                #pattern => {
                    self.#method_name(#(#arguments),*) #await_suffix #try_suffix;
                    ::core::result::Result::Ok(())
                }
            }
        })
        .collect()
}
