// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! This module contains the remote procedure call's [CallResult] structure
//! and supported operations

/// The result from a [crate::rpc::call] operation
#[derive(Debug, Eq, PartialEq)]
pub enum CallResult<TResult> {
    /// Success, with the result
    Success(TResult),
    /// Timeout
    Timeout,
    /// The transmission channel was dropped without any message(s) being sent
    SenderError,
}

impl<T> CallResult<T> {
    /// Determine if the [CallResult] is a [CallResult::Success]
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success(_))
    }

    /// Determine if the [CallResult] is a [CallResult::Timeout]
    pub fn is_timeout(&self) -> bool {
        matches!(self, Self::Timeout)
    }

    /// Determine if the [CallResult] is a [CallResult::SenderError]
    pub fn is_send_error(&self) -> bool {
        matches!(self, Self::SenderError)
    }

    /// Unwrap a [CallResult], panicking on any non-success
    pub fn unwrap(self) -> T {
        match self {
            Self::Success(t) => t,
            Self::Timeout => panic!("called CallResult::<T>::unwrap()  on a `Timeout` value"),
            Self::SenderError => {
                panic!("called CallResult::<T>::unwrap() on a `SenderError` value")
            }
        }
    }

    /// Unwrap a [CallResult], panicking on non-succcess with the specified message
    pub fn expect(self, msg: &'static str) -> T {
        match self {
            Self::Success(t) => t,
            Self::Timeout => panic!(
                "{} - called CallResult::<T>::expect()  on a `Timeout` value",
                msg
            ),
            Self::SenderError => panic!(
                "{} - called CallResult::<T>::expect() on a `SenderError` value",
                msg
            ),
        }
    }

    /// Unwrap the [CallResult] or give a default value
    pub fn unwrap_or(self, default: T) -> T {
        if let Self::Success(t) = self {
            t
        } else {
            default
        }
    }

    /// Returns the [CallResult]'s success result or computes the closure
    pub fn unwrap_or_else<F>(self, f: F) -> T
    where
        F: FnOnce() -> T,
    {
        if let Self::Success(t) = self {
            t
        } else {
            f()
        }
    }

    /// Transforms the [CallResult] to a Result mapping `Success(t)` to `Ok(t)` and all else to `Err(err)`
    pub fn success_or<E>(self, err: E) -> Result<T, E> {
        if let Self::Success(t) = self {
            Ok(t)
        } else {
            Err(err)
        }
    }

    /// Transforms the [CallResult] to a Result mapping `Success(t)` to `Ok(t)` and all else to `Err(err())`
    pub fn success_or_else<E, F>(self, err: F) -> Result<T, E>
    where
        F: FnOnce() -> E,
    {
        if let Self::Success(t) = self {
            Ok(t)
        } else {
            Err(err())
        }
    }

    /// Maps the success value of the [CallResult] to another type
    pub fn map<O, F>(self, mapping: F) -> CallResult<O>
    where
        F: FnOnce(T) -> O,
    {
        match self {
            Self::Success(t) => CallResult::Success(mapping(t)),
            Self::Timeout => CallResult::Timeout,
            Self::SenderError => CallResult::SenderError,
        }
    }

    /// Maps the success value of the [CallResult] to another type
    /// or returns the default value
    pub fn map_or<O, F>(self, default: O, mapping: F) -> O
    where
        F: FnOnce(T) -> O,
    {
        match self {
            Self::Success(t) => mapping(t),
            Self::Timeout => default,
            Self::SenderError => default,
        }
    }

    /// Maps the success value of the [CallResult] to another type
    /// or returns the default function result
    pub fn map_or_else<D, O, F>(self, default: D, mapping: F) -> O
    where
        F: FnOnce(T) -> O,
        D: FnOnce() -> O,
    {
        match self {
            Self::Success(t) => mapping(t),
            Self::Timeout => default(),
            Self::SenderError => default(),
        }
    }
}
