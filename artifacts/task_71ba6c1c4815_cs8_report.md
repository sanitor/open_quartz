# Phase 7 Change-set 8 Report

## Implemented

- Added a non-default Rust `host-internals` feature on `open_quartz` and made execution/host modules private for default consumers: `catalog`, `engine`, `event`, `gpu`, `graph`, `host`, `host_intent`, `media`, `native_video`, `onnx`, `runtime`, `types`, and `wgsl`.
- Enabled `host-internals` only where platform/runtime code needs it: `src-tauri` and `open-quartz-screensaver-stub`.
- Kept the public Rust API available through root object/schema exports such as `OpenQuartz`, `Project`, `Graph`, `Player`, `DataType`, `ProjectFile`, and `SdkError`.
- Removed GUI/runtime projection fields from typed Rust public schema: `collapsed`, `expanded`, `resolvedWidth`, `resolvedHeight`, `onnxStatus`, `onnxProgress`, `onnxError`, `onnxBackend`, `onnxCustomFileName`, `imageFileName`, `rawFileName`, and `videoFileName`.
- Preserved project compatibility by letting those legacy/UI fields round-trip through `NodeData.extra`; `types_test` now fixtures those fields explicitly.
- Moved renderer expansion and initial ONNX status defaults into the TS store adapter projection so GUI state remains in TS.
- Tightened TS public/internal indexes: public `src/sdk/index.ts` exposes proxies/catalog/error types only, and `src/sdk/internal/index.ts` exposes host/runtime adapter contracts.
- Added `scripts/check-rust-boundaries.mjs` and wired it into `package.json` and `.github/workflows/dependency-boundaries.yml`.
- Updated dependency-boundary expectations for the intentional public `Catalog` proxy.

## Boundary Checks Added

- Default-feature external Rust consumers can compile public root/schema types.
- Default-feature external Rust consumers cannot import `open_quartz::{engine,event,gpu,graph,host,onnx,runtime,types,wgsl}` internals.
- Default external consumers can import `open_quartz_schema` domain values but cannot import removed schema projection symbols like `OnnxStatus` or construct removed fields like `NodeData.collapsed`.

## Verification

- `cargo test --workspace`
- `cargo test -p open_quartz --features host-internals`
- `cargo test -p open_quartz --features host-internals --test player_native_test`
- `npm run check:rust-boundaries`
- `npm exec -- tsc -b`
- `npm exec -- vitest run tests/sdk/dependencyBoundary.test.ts tests/sdk/languageConformance.test.ts tests/store/useGraphStore.test.ts`
- `npm run check:public-proxy`

All verification commands passed. Existing non-blocking warnings remain in native video/runtime/app code.

## Platform Limitation

The crate boundary is feature-gated rather than split into new execution/host crates because Tauri, the screensaver stub, wasm FFI, and internal integration tests still share concrete runtime modules in this worktree. A physical crate split would be a larger ownership move and was not necessary to enforce the default public dependency boundary.
