// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Output port implementations for actors

use super::MuxedValue;

/// A mutex'd port which represents up to 10 different types of messaged on the same port
pub struct MuxedOutputPort<T1, T2, T3, T4, T5, T6, T7, T8, T9, T10> {
    port: tokio::sync::mpsc::Receiver<MuxedValue<T1, T2, T3, T4, T5, T6, T7, T8, T9, T10>>,
    subscriptions: Vec<()>,
}

// TODO: setup subscriptions?

// impl<T1,T2,T3,T4,T5,T6,T7,T8,T9,T10> MuxedOutputPort<T1,T2,T3,T4,T5,T6,T7,T8,T9,T10> {
//     fn subscribe(&mut self, subscription: )
// }

/// An output-port (broadcast)
pub type OutputPort<T1, T2, T3, T4, T5, T6, T7, T8, T9, T10> =
    MuxedOutputPort<T1, T2, T3, T4, T5, T6, T7, T8, T9, T10>;

pub(crate) type OutputPortSender<T1, T2, T3, T4, T5, T6, T7, T8, T9, T10> =
    tokio::sync::mpsc::Sender<MuxedOutputPort<T1, T2, T3, T4, T5, T6, T7, T8, T9, T10>>;
