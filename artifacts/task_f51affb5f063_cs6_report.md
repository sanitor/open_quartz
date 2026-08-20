# Phase 7 Change-set 6 Report

## Summary

Implemented a Rust-owned browser ONNX inference contract for model-family dispatch, tensor descriptors, tile planning, preprocess/encode, postprocess/decode, labels, thresholds, output mapping, and async completion envelope construction. Migrated `BrowserInferenceProvider` to use the Rust/proxy boundary while keeping TypeScript responsible for ORT-Web session lifecycle, raw tensor submission/readback, worker transport, and DOM/canvas overlays.

## Rust Contract Added

- Added `crates/open_quartz/src/onnx/task.rs`.
- Exposed typed JSON FFI helpers:
  - `planBrowserOnnxTask`
  - `encodeBrowserOnnxInput`
  - `decodeBrowserOnnxOutput`
  - `buildBrowserOnnxCompletion`
- Contract covers:
  - model families: `super-resolution-3x`, `realesrgan-x4`, `u2netp`, `modnet`, `midas-small`, `yolov8n`, `yolo26n-sem`, generic RGB/YCbCr
  - tensor descriptors: dtype/layout/shape for single and tiled execution
  - tile geometry: output dimensions, scale, fixed-size model inputs, min tile size, tile list
  - detection defaults: `targetSize=640`, `scoreThreshold=0.25`, `iouThreshold=0.45`
  - detection labels from Rust `COCO_CLASSES`
  - segmentation labels from Rust `CITYSCAPES_CLASSES`
  - Rust decode/NMS/segmentation mask boundaries
  - completion stamps and output payload policy

## TypeScript Retained

- `src/engine/onnx/inference.ts` now retains only:
  - ORT-Web script loading
  - `OnnxInferenceSession`
  - raw tensor `run`, `runRaw`, `runFull`
  - WebGPU/WASM fallback lifecycle
  - GPU allocation error classification
- `BrowserInferenceProvider` now retains:
  - model buffer/session lifecycle
  - calls to Rust plan/encode/decode/completion helpers
  - raw Float32Array marshalling into ORT-Web
  - tile loop execution over Rust-issued tile descriptors
  - browser canvas overlays for detection/segmentation visualization

## Deleted TypeScript Policy/Transforms

- Deleted `src/engine/onnx/yoloDetectionPostprocess.ts`.
- Deleted `src/engine/onnx/yoloSegmentationPostprocess.ts`.
- Removed TS implementations of:
  - RGB/YCbCr/MiDaS/background-removal codecs
  - letterbox preprocessing
  - YOLO decode/NMS
  - segmentation argmax, resize, palette colorization, class counts
  - super-resolution/background/depth/generic task dispatch functions
  - TS completion output payload construction in `BrowserInferenceProvider`

## Tests Run

- `cargo test -p open_quartz --test onnx_test --test onnx_ffi_test --no-default-features`
- `cargo test -p open_quartz --test runtime_contract_test --no-default-features runtime_rejects_stale_async_completions_and_preserves_launch_stamp`
- `npx vitest run tests/engine/onnxInference.test.ts tests/sdk/WasmSdkClient.test.ts tests/sdk/BrowserInferenceProvider.test.ts tests/components/OnnxPanel.test.tsx tests/store/onnxStore.test.ts`

All targeted tests passed.

## Duplicate Scan

Ran `rg` for deleted ONNX pre/postprocess symbols and old codec/task names across `src` and `tests`; no matches remained for:

- `letterboxPreprocess`
- `detectPostprocess`
- `decodeYolo`
- `segmentPostprocess`
- `decodeSegmentation`
- `maskToRgba`
- `CITYSCAPES_PALETTE`
- `COCO_CLASSES`
- `rgbCodec`
- `ycbcrCodec`
- `midasCodec`
- `TileCodec`
- `runSuperResolution`
- `runBackgroundRemoval`
- `runDepthEstimation`
- `runGenericImageToImage`
- `runDetection`
- `runSegmentation`

## Not Run

- No full workspace gates, npm test, formatters, linters, or performance tests, per task instructions.
- No separate browser ONNX smoke was available in the test tree beyond the added focused `BrowserInferenceProvider` bridge smoke; the existing `tests/functional/onnx.test.ts` is a Node/`onnxruntime-node` model smoke, not a browser ORT-Web smoke.
