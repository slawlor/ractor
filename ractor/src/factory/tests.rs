// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Factory functionality tests

mod basic;
mod draining_requests;
mod dynamic_discarding;
mod dynamic_pool;
mod dynamic_settings;
mod lifecycle;
mod priority_queueing;
mod ratelim;
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
mod worker_lifecycle;
