// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Input port implementations for various sizes

/// An import-port
pub type InputPort<TMsg> = tokio::sync::mpsc::UnboundedSender<TMsg>;

pub(crate) type InputPortReceiver<TMsg> = tokio::sync::mpsc::UnboundedReceiver<TMsg>;
