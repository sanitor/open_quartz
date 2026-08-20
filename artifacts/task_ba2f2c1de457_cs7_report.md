# Phase 7 Change-set 7 Report

## Implemented

- Added canonical Rust host resource intent planning in `crates/open_quartz/src/host_intent.rs`.
- Exposed the planner through wasm FFI as `planHostResourceIntents` and through Tauri as `native_host_resource_intents`.
- Migrated `BrowserHost` to consume Rust video intents while retaining only DOM video creation, playback, capture, and worker transport.
- Migrated `NativeHost` to consume Rust image/video/ONNX intents while retaining only Tauri calls, DOM image decode, raw byte fetch/decode, WebView2 texture-stream handling, and opaque native handles.
- Added focused Rust host-resource intent tests for browser video replacement/reuse, native video replacement/replay, video-to-image replacement ordering, ONNX catalog load defaults/integrity, and ONNX unload.
- Updated focused wasm/native host tests to assert the new planning boundary.

## Retained Metadata And Adapter Responsibilities

- `BrowserHost` retains DOM/platform metadata needed to create media elements from Rust intents: `nodeId`, `key`, video `kind`, source string, looping, and playback rate.
- `NativeHost` retains typed transport metadata from Rust intents: image source kind and dimensions, video source kind/source/playback settings, ONNX model id/task/path/thresholds/download descriptor, and opaque Tauri/native handles.
- TS adapters still perform platform work only: DOM video element lifecycle, `createImageBitmap`, image decode via canvas, fetch of raw image bytes, Tauri command invocation, WebView2 texture stream presentation, and worker message transport.

## Deleted TypeScript Policy

- Removed `NativeHost` import/use of `getOnnxModelDescriptor`.
- Removed local `syncImageResources`, `syncVideoResources`, `syncOnnxResources`, `stripGraphResourcePayloads`, and native image/video/ONNX resource maps.
- Removed `BrowserHost` local `videoSourceKey`/node-scan reconcile policy; video attach/update/detach now comes from Rust intent output.
- Verification scan found no host-layer matches for `getOnnxModelDescriptor`, `syncImageResources`, `syncVideoResources`, `syncOnnxResources`, `videoSourceKey`, `stripGraphResourcePayloads`, ONNX threshold/default derivation fields, or image/video resource source fields in `src/sdk/internal/BrowserHost.ts` and `src/sdk/internal/NativeHost.ts`.

## Verification

- `cargo test -p open_quartz --test host_resource_intent_test`
- `cargo test -p open_quartz --test runtime_contract_test --test output_registry_test --test player_native_test --test host_resource_intent_test`
- `npx vitest run tests/sdk/WasmSdkClient.test.ts tests/sdk/BrowserHost.test.ts tests/sdk/NativeHost.test.ts`
- `cargo check -p app`

All commands passed. Existing warnings were unchanged unused/dead-code warnings in native video/runtime and app targets.

## Platform Smoke Status

- Native smoke is wired behind `OPEN_QUARTZ_NATIVE_SMOKE` in the Tauri app startup path, but this worker did not launch a GUI/Tauri app instance or provision platform GPU/WebView2 smoke prerequisites.
- Browser ONNX smoke was not part of this CS7 host-resource task and no focused browser-host platform smoke script was available beyond the targeted Vitest host tests above.
