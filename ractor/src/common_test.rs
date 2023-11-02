// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

// TODO #124 (slawlor): Redesign this without usage of core time primatives (i.e.
// use concurrency instants)
#[cfg(not(target_arch = "wasm32"))]
use std::future::Future;

use crate::concurrency::sleep;
use crate::concurrency::Duration;
use crate::concurrency::Instant;

pub async fn periodic_check<F>(check: F, timeout: Duration)
where
    F: Fn() -> bool,
{
    let start = Instant::now();
    while start.elapsed() < timeout {
        if check() {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }

    let backtrace = backtrace::Backtrace::new();
    assert!(check(), "Periodic check failed.\n{:?}", backtrace);
}

pub async fn periodic_async_check<F, Fut>(check: F, timeout: Duration)
where
    F: Fn() -> Fut,
    Fut: Future<Output = bool>,
{
    let start = Instant::now();
    while start.elapsed() < timeout {
        if check().await {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }

    let backtrace = backtrace::Backtrace::new();
    assert!(
        check().await,
        "Async periodic check failed.\n{:?}",
        backtrace
    );
}
