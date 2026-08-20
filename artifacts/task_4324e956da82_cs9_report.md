# Phase 7 CS9 Report

Task: `task_4324e956da82`

## Changes

- Added `scripts/bench-browser-paths.mjs` for deterministic Web-path fixtures and JSON artifacts.
- Added `src/sdk/internal/browserPreviewEncoding.ts` and routed `BrowserRuntimeWorker.readOutputDataUrl` through it.
- Kept the existing preview pixel path intact: `readOutputRgba` -> `ImageData` -> `OffscreenCanvas.convertToBlob({ type: 'image/png' })`.
- Replaced only the PNG Blob-to-data-URL boundary when available: worker-native `FileReader.readAsDataURL(blob)`.
- Retained the previous chunked `String.fromCharCode`/`btoa` path as the fallback for platforms without `FileReader`.
- Reused `date` and `mouse` `Float32Array` frame inputs in `BrowserRuntimeWorker` instead of allocating new arrays each frame.
- Added `tests/sdk/browserPreviewEncoding.test.ts` for data URL contract parity, no-`FileReader` fallback behavior, and native `FileReader` routing.

## Measured Evidence

Artifacts:

- `artifacts/task_4324e956da82_browser_paths_baseline.json`
- `artifacts/task_4324e956da82_browser_paths_after.json`

Environment reported by the benchmark:

- Node `v26.0.0`
- macOS arm64
- `FileReader`: unavailable
- `OffscreenCanvas`: unavailable
- `ImageBitmap`: unavailable

Baseline fixture:

- Existing byte-string data URL path, 256 KiB payload: 5.601 ms/op.
- Existing byte-string data URL path, 2 MiB payload: 34.260 ms/op.
- Current frame input allocation fixture: 0.0001128 ms/op.
- Reused typed-array frame input fixture: 0.0000953 ms/op.

After fixture:

- Existing byte-string data URL fixture remained equivalent in Node fallback mode, 256 KiB payload: 4.332 ms/op.
- Existing byte-string data URL fixture remained equivalent in Node fallback mode, 2 MiB payload: 30.945 ms/op.
- Current allocation fixture: 0.0000998 ms/op.
- Reused typed-array fixture: 0.0000885 ms/op.

The data URL fixture validates byte-for-byte parity against a Node native base64 reference, but the Node native reference is not treated as a browser production speed claim. Because this worker environment lacks browser `FileReader`/`OffscreenCanvas`/`ImageBitmap`, no measured browser PNG readback speedup is claimed here.

## Deliberately Unchanged

- ORT tensor transfer/readback-upload remains unchanged in `BrowserInferenceProvider`; the current typed Rust request/result boundary still returns JS arrays, and changing that safely needs protocol-level conformance work.
- Video `createImageBitmap(video)` transfer remains unchanged; the existing bounded in-flight frame behavior is covered by `tests/sdk/BrowserHost.test.ts`.
- Worker timer scheduling remains `setTimeout` based; only per-frame typed-array allocation was reduced.
- Output keys, generations, completion stamps, and stale completion behavior were not changed.

## Verification

- `node scripts/bench-browser-paths.mjs --variant baseline --out artifacts/task_4324e956da82_browser_paths_baseline.json`
- `node scripts/bench-browser-paths.mjs --variant after --out artifacts/task_4324e956da82_browser_paths_after.json`
- `npx vitest run tests/sdk/browserPreviewEncoding.test.ts tests/sdk/BrowserHost.test.ts tests/sdk/BrowserInferenceProvider.test.ts`
- `npx tsc -p tsconfig.app.json --noEmit`

Focused tests passed: 3 test files, 10 tests.

## Platform Smoke Status

No existing targeted Browser ONNX smoke command was available. The repo has a browser-mode shader config, but no Browser ONNX smoke harness; this environment also lacks the browser APIs needed for an actual worker/preview/ImageBitmap runtime smoke from Node.
