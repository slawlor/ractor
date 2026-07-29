// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

use syn::parse::{Parse, ParseStream};
use syn::{Ident, Path, Result, Token, Type, Visibility};

/// An enum generated from the actor's message handlers.
pub(crate) struct GeneratedMessageConfig {
    pub(crate) visibility: Visibility,
    pub(crate) ident: Ident,
}

/// The message type supplied to `#[ractor::actor(...)]`.
pub(crate) enum MessageConfig {
    Existing(Type),
    Generated(GeneratedMessageConfig),
}

impl MessageConfig {
    pub(crate) fn ty(&self) -> Type {
        match self {
            Self::Existing(ty) => ty.clone(),
            Self::Generated(config) => {
                let ident = &config.ident;
                syn::parse_quote!(#ident)
            }
        }
    }
}

/// Configuration supplied to `#[ractor::actor(...)]`.
pub(crate) struct ActorConfig {
    pub(crate) thread_local: bool,
    pub(crate) message: MessageConfig,
    pub(crate) state: Type,
    pub(crate) arguments: Type,
    pub(crate) crate_path: Option<Path>,
}

impl Parse for ActorConfig {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut thread_local = false;
        let mut message = None;
        let mut state = None;
        let mut arguments = None;
        let mut crate_path = None;

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            let key_name = key.to_string();

            if key_name == "thread_local" {
                if thread_local {
                    return Err(syn::Error::new(key.span(), "duplicate `thread_local` flag"));
                }
                thread_local = true;
            } else {
                input.parse::<Token![=]>()?;
                match key_name.as_str() {
                    "message" => {
                        set_once(&mut message, parse_message_config(input)?, &key, "message")?;
                    }
                    "state" => {
                        set_once(&mut state, input.parse()?, &key, "state")?;
                    }
                    "arguments" => {
                        set_once(&mut arguments, input.parse()?, &key, "arguments")?;
                    }
                    "crate_path" => {
                        set_once(&mut crate_path, input.parse()?, &key, "crate_path")?;
                    }
                    _ => {
                        return Err(syn::Error::new(
                            key.span(),
                            "unknown actor option; expected `thread_local`, `message`, `state`, `arguments`, or `crate_path`",
                        ));
                    }
                }
            }

            if input.is_empty() {
                break;
            }
            input.parse::<Token![,]>()?;
        }

        let message = message.ok_or_else(|| {
            syn::Error::new(
                input.span(),
                "missing required actor option `message = ...`",
            )
        })?;

        Ok(Self {
            thread_local,
            message,
            state: state.unwrap_or_else(|| syn::parse_quote!(())),
            arguments: arguments.unwrap_or_else(|| syn::parse_quote!(())),
            crate_path,
        })
    }
}

fn parse_message_config(input: ParseStream<'_>) -> Result<MessageConfig> {
    if input.peek(Token![enum]) {
        input.parse::<Token![enum]>()?;
        return Ok(MessageConfig::Generated(GeneratedMessageConfig {
            visibility: Visibility::Inherited,
            ident: input.parse()?,
        }));
    }

    if input.peek(Token![pub]) {
        let visibility = input.parse()?;
        input.parse::<Token![enum]>()?;
        return Ok(MessageConfig::Generated(GeneratedMessageConfig {
            visibility,
            ident: input.parse()?,
        }));
    }

    Ok(MessageConfig::Existing(input.parse()?))
}

fn set_once<T>(slot: &mut Option<T>, value: T, key: &Ident, name: &str) -> Result<()> {
    if slot.is_some() {
        return Err(syn::Error::new(
            key.span(),
            format!("duplicate `{name}` option"),
        ));
    }
    *slot = Some(value);
    Ok(())
}
