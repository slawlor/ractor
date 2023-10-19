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

// TODO #124 reenable once factories ready
#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
/// Setup a common test with proper tracing support (whether WASM or regular runtime)
pub fn setup() {
    extern crate console_error_panic_hook;

    // print pretty errors in wasm https://github.com/rustwasm/console_error_panic_hook
    // This is not needed for tracing_wasm to work, but it is a common tool for getting proper error line numbers for panics.
    console_error_panic_hook::set_once();

    // Add this line:
    let _ = tracing_wasm::try_set_as_global_default();

    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);
}

/// Setup a common test with proper tracing support (whether WASM or regular runtime)
#[cfg(not(target_arch = "wasm32"))]
pub fn setup() {}
