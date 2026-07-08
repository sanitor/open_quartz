# yolo-detector

OpenQuartz `onnx` node crate: YOLOv8n detector. Thin wasm-bindgen façade over
[`caozisheng/rimeflow-yolov8n`](https://github.com/caozisheng/rimeflow-yolov8n).

## Layout

Lives at `rust/crates/yolo-detector/` inside the OpenQuartz Cargo workspace at
`rust/Cargo.toml`. Sibling node crates land as `rust/crates/<name>/` and are
picked up by the workspace automatically.

## Why a wrapper

Upstream ships two pieces:

- `postprocess` — pure Rust YOLOv8 output decode + NMS.
- `ort_bridge` — `wasm_bindgen(inline_js)` bridge to `onnxruntime-web`
  (auto-loads `/ort/ort.min.js`, WebGPU EP with WASM fallback).

Upstream exposes a **free-function API** (`ort_init`, `ort_detect`, ...).
OpenQuartz's execution engine wants a **session handle** it can construct,
threshold, run, and release per node. This crate adds exactly that:
`YoloDetectorWasm` (session-shaped) + `classNames()` (COCO 80 table).

Nothing here duplicates upstream. `postprocess` and `ort_bridge` are `pub use`d
straight from the git dep — bump `Cargo.toml` `branch = "main"` (or pin to a
`rev`) to pick up upstream changes.

## Build

```bash
wasm-pack build rust/crates/yolo-detector --target web --release --out-dir pkg
# or
npm run build:wasm   # from repo root
```

Produces `pkg/yolo_detector.{js,d.ts}` + `yolo_detector_bg.wasm` for the Vite
alias `@nodes/yolo-detector`.

## Runtime prerequisites

Upstream `ort_bridge` **auto-injects** a `<script src="/ort/ort.min.js">` tag
on first `ort_init` if `globalThis.ort` is missing, so `index.html` does not
need to load ORT ahead of time. You still need the files:

```bash
npm run copy:ort   # copies node_modules/onnxruntime-web/dist/* to public/ort/
```

Model file: `public/models/yolov8n.onnx` — copied at `npm run predev` /
`prebuild` time by `scripts/copy-model.mjs` from the upstream Cargo git
checkout (`~/.cargo/git/checkouts/rimeflow-yolov8n-*/*/models/yolov8n.onnx`).
Not committed here.

## Public API (JS-facing)

```ts
import __wbg_init, { YoloDetectorWasm, classNames } from '@nodes/yolo-detector';

await __wbg_init();
const det = new YoloDetectorWasm(/* modelUrl? */, /* targetSize? */);
det.setThresholds(0.25, 0.45);
await det.init();
const { detections } = await det.detect(canvas, canvas.width, canvas.height);
det.release();

YoloDetectorWasm.captureWebgpuDevice();   // reserved for future WebGPURenderer
```

`detections[i]` is `{ bbox: [x1,y1,x2,y2] (normalized), score, class_id, class_name }`.

## Upgrading upstream

```bash
cd rust/crates/yolo-detector
cargo update -p rimeflow-yolov8n
wasm-pack build --target web --release --out-dir pkg
```
