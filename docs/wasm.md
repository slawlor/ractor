# WASM support

Ractor supports the `wasm32-unknown-unknown` target for browser environments. This document covers what works, what doesn't, and how to test.

## Runtime

Ractor uses [`tokio-with-wasm`](https://docs.rs/tokio_with_wasm) to provide tokio-compatible APIs on WASM. Custom `Send`-safe timer implementations (`sleep`, `interval`, `timeout`) wrap JavaScript `setTimeout` through channels so that futures remain `Send` even though the underlying JS Promises are not.

Additional WASM-specific dependencies:

- `tokio_with_wasm` — tokio runtime shim for browsers
- `web-time` — `Instant` backed by `performance.now()`
- `js-sys` — access to JavaScript globals
- `wasm-bindgen` / `wasm-bindgen-futures` — Rust/JS interop

## What works

- `Actor` trait — spawn, cast, call, supervision (error-based)
- `ThreadLocalActor` — uses `spawn_local` rather than a dedicated thread
- `call!` / `cast!` macros
- Process groups (`ractor::pg`)
- Named registry (`ractor::registry`)
- Timers (`sleep`, `interval`, `timeout`)

## Limitations

- **Panic supervision is unavailable.** WASM targets use `panic = "abort"`, which means `catch_unwind` cannot capture panics — the entire program aborts instead. Supervision based on panic capture does not function; only error-based supervision works. Tests that rely on panic capture are gated with `#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]`.
- **`async-std` runtime is not supported on WASM.** Use the default tokio runtime.
- **`cluster` feature is not applicable.** Browser environments cannot host TCP listeners or establish the node-to-node connections that `ractor_cluster` requires.
- **`ThreadLocalActorSpawner` uses `spawn_local`.** Browsers are single-threaded for JS interop, so there is no dedicated OS thread — the spawner runs on the current task executor instead.

## Testing

WASM tests run in a real browser via `wasm-pack` and Puppeteer. To run locally:

```bash
# Install wasm-pack if needed
cargo install wasm-pack

# Run the default test suite
wasm-pack test --chrome ./ractor

# Run with a feature flag
wasm-pack test --chrome ./ractor --features async-trait
wasm-pack test --chrome ./ractor --features output-port-v2
```

CI uses a custom test runner (`.github/wasm-test-runner/`) that launches a headless Chrome via Puppeteer on Linux, controlled by the `wasm.yaml` workflow.
