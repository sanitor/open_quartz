# Phase 7 CS8 Boundary Closure Report

Task: `task_454a7bc64264`

## Final Crate DAG

- `open_quartz_schema`
  - Depends on `serde`, `serde_json`.
  - Owns domain/schema value contracts: graph/project/node/port IDs, `SdkError`, `SdkErrorCode`, and the shared `OnnxTask` enum.
- `open_quartz_execution`
  - Depends on `open_quartz_schema`.
  - Owns concrete implementation modules: `catalog`, `engine`, `event`, `gpu`, `graph`, `host::PlayerHost`, `media`, `native_video`, `onnx`, `runtime`, `wgsl`, and WASM browser GPU environment code.
- `open_quartz_host_api`
  - Depends on `open_quartz_schema` and `open_quartz_execution` for canonical catalog/runtime value contracts.
  - Owns host resource intent contracts and planning: attach/update/detach video, upload/remove image, load/unload ONNX, and stripped native graph payloads.
- `open_quartz_sdk`
  - Depends on `open_quartz_schema`, `open_quartz_host_api`, and `open_quartz_execution`.
  - Owns public aggregate objects: `Environment`, `OpenQuartz`, `Project`, `GraphLayout`, `Player`, `PlayerBuilder`, `Output`, `Subscription`, and resource catalog descriptors.
- `open_quartz`
  - Thin facade/binding crate.
  - Re-exports public schema and SDK aggregates, keeps `ffi` bindings, and composes execution/host API internally without exposing old internal modules.
- `open_quartz_bindings`
  - JNI binding crate depending on the thin `open_quartz` facade.

## Code Ownership Moved

- Moved execution implementation out of `crates/open_quartz/src` into `crates/open_quartz_execution/src`.
- Moved host intent planning out of `crates/open_quartz/src/host_intent.rs` into `crates/open_quartz_host_api/src`.
- Moved SDK aggregates out of `crates/open_quartz/src/sdk.rs` into `crates/open_quartz_sdk/src`.
- Removed the obsolete `host-internals` feature and removed the old `open_quartz::types` compatibility module.
- Updated Tauri and screensaver native hosts to import concrete implementation APIs from `open_quartz_execution` and schema values from `open_quartz_schema`.
- Updated facade FFI to call `open_quartz_execution` and `open_quartz_host_api` directly.
- Moved internal Rust tests to the owning crates and added `open_quartz_sdk/tests/public_aggregate_test.rs`.
- Updated CI boundary workflow to remove the `cargo test -p open_quartz --features host-internals` gate.
- Strengthened `scripts/check-rust-boundaries.mjs` to prove facade internals are forbidden, SDK consumers cannot import execution transitively, host API compiles independently as an intent API, and the obsolete feature is absent.

## Platform Limitation

- DXGI-specific test bodies are `cfg(windows)` and therefore compile as zero executed tests on this macOS worker. No other platform-only boundary limitation was observed.

## Verification

- `cargo test --workspace` passed.
- `npm run check:rust-boundaries` passed.
- `npm run check:public-proxy` passed.
- `npm exec -- tsc -b` passed.

Known warnings remain from pre-existing unused/dead-code diagnostics in `open_quartz_execution::native_video` and `src-tauri::native_runtime`; no formatter, linter, full npm suite, performance work, or commit was run.
