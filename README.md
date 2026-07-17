<p align="center">
  <img src="public/icon.svg" width="96" height="96" alt="Open Quartz icon">
</p>

<h1 align="center">Open Quartz</h1>

<p align="center">
  A real-time heterogeneous video pipeline editor — GPU shaders, neural networks, and CPU math in one graph.
</p>

<p align="center">
  <img src="docs/screenshot.png" width="720" alt="Open Quartz screenshot">
</p>

Open Quartz is a node-based, hardware-accelerated framework for authoring real-time video processing pipelines. It fuses WebGPU/WebGL shader execution, ONNX neural-network inference, and CPU-side math into a single heterogeneous graph that runs at interactive frame rates. Connect source nodes (camera, video files, images, raw framebuffers), processing nodes (31 GLSL shader presets, 29 math ops, 7 ONNX models + custom), and renderer outputs on an infinite canvas. Inspired by Apple Quartz Composer, Shadertoy, and chaiNNer.

## Node Catalog

### Source Nodes

| Node | Type | Output | Description |
|------|------|--------|-------------|
| **Image** | Input | `sampler2D` | Load images as GPU textures. Drag-and-drop or file picker. |
| **Video** | Input | `sampler2D` | Camera or video file input via `VideoTexture`. Auto-updates each frame. |
| **Framebuffer** | Input | `sampler2D` | Raw binary dump files with configurable format (RGBA8/RGBA32F/RG8/RG32F/R8/R32F/NV12), width, height, stride. |
| **Time** | System | `float` | Elapsed time in seconds since Play. |
| **Time Delta** | System | `float` | Frame delta time. |
| **Frame** | System | `int` | Current frame number. |
| **Mouse** | System | `vec4` | Mouse position and click state (Shadertoy `iMouse` convention). |
| **Resolution** | System | `vec3` | Canvas resolution and pixel ratio. |
| **float / int / vec2-4 / mat2-4** | Constant | Various | Editable scalar, vector, and matrix values. |

### Shader Nodes (31 presets + custom)

| Category | Shaders |
|----------|---------|
| **Filter** | Resample, Sobel Edge Detection, Gaussian Blur 3×3, Box Blur, Sharpen, Emboss, Pixelate |
| **Color** | Grayscale, Brightness/Contrast, Hue Rotate, Threshold, Sepia |
| **Generator** | Solid Color, Gradient, Checkerboard, Noise, Circle |
| **Blend** | Add, Multiply, Screen, Overlay, Difference, Exclusion, Soft Light |
| **Distortion** | Twirl, Ripple, Displacement, Barrel, Pinch |
| **Custom** | Custom Shader (1 input), Custom 2-in-1 (2 inputs). Full GLSL 300 es editor with syntax highlighting, linting, and autocompletion. |

### Math Nodes (29 operations)

| Category | Operations |
|----------|------------|
| **Arithmetic** | add, subtract, multiply, divide, negate, modulo |
| **Range** | min, max, clamp, saturate, step, smoothstep, abs, sign |
| **Trigonometry** | sin, cos, tan, asin, acos, atan |
| **Exponential** | pow, sqrt, exp, log |
| **Interpolation** | mix |
| **Rounding** | floor, ceil, round, fract |

Auto type inference from connected peers. CPU-only evaluation, results propagate to downstream shader uniforms.

### ONNX Neural Network Nodes (7 models + custom)

| Category | Model | Size | Input | Output | Task |
|----------|-------|------|-------|--------|------|
| **Detection** | YOLOv8n | 12.8MB | 640×640 | `roi` + `sampler2D` overlay | 80-class COCO object detection |
| **Super-Resolution** | Sub-pixel CNN 3× | 0.2MB | 224×224 fixed | `sampler2D` 3× upscaled | Lightweight Y-channel SR |
| **Super-Resolution** | Real-ESRGAN 4× | 4.9MB | dynamic | `sampler2D` 4× upscaled | Photo-realistic upscaling |
| **Background Removal** | U²Net-P | 4.4MB | 320×320 fixed | `sampler2D` RGBA (alpha=mask) | General-purpose foreground extraction |
| **Background Removal** | MODNet | 24.7MB | 512×512 fixed | `sampler2D` RGBA (alpha=matte) | Portrait-focused matting |
| **Depth Estimation** | MiDaS v2.1 Small | 63MB | 256×256 fixed | `sampler2D` grayscale depth | Monocular relative depth |
| **Custom** | User `.onnx` file | any | auto-introspected | auto-introspected | Load any ONNX model, ports generated from I/O metadata |

All models auto-download on first use. Tiled inference engine handles arbitrary input sizes. Adaptive WebGPU→WASM fallback for incompatible GPUs. Backend probe at load time — user sees "CPU fallback" badge before pressing Play.

### Output Nodes

| Node | Input | Description |
|------|-------|-------------|
| **Renderer** | `sampler2D` | Explicit output viewer (Quartz Composer QCView equivalent). In-place preview, fullscreen live view, frame capture as PNG. |

## Features

### Realtime Rendering
- **rAF-driven rendering loop** with PLAY / PAUSE / STOP transport controls
- **Host/Compositor architecture** inspired by Quartz Composer's QCRenderer
- **Shadertoy-compatible builtin uniforms**: `iTime`, `iTimeDelta`, `iFrame`, `iDate`, `iMouse`, `iResolution`
- **Static pipeline optimization** — graphs without time-varying inputs render one frame then stop
- **GPU-only output path** — no `readPixels` in the realtime loop; preview via mirror canvas blit

### Node Graph Editor
- Drag, connect, and arrange nodes on an infinite canvas (React Flow)
- Bezier curve edges with type-safe connections — ports carry GLSL type metadata
- MiniMap, box selection, fit-to-view

### Node Inspector (Side Panel)
- CodeMirror 6 shader editor with GLSL syntax highlighting, error linting, and autocompletion
- Port inspector with color-coded type indicators and inline uniform editing
- Per-component vector editing (x/y/z/w) for vec2-4 uniforms
- Output preview, Auto Size, sampling config (filter/wrap)

### Preview Lightbox
- Full-screen viewer with scroll-to-zoom, drag-to-pan, double-click reset
- Nearest-neighbor rendering for pixel inspection
- Save as PNG, color picker with coordinate display

### Project Management
- Save / Save As / Load (`.quartz.json` files)
- 50-level undo/redo with Cmd/Ctrl+Z / Cmd/Ctrl+Shift+Z

### Desktop App (Tauri)
- Native desktop application via Tauri 2
- Custom titlebar (macOS traffic lights, Windows min/max/close)
- Video file persistence via asset protocol

## Getting Started

```bash
npm install
npm run build:wasm     # builds ONNX wasm bridge (requires wasm-pack)
npm run dev
```

> **Note:** `build:wasm` is required before first run. `wasm-pack` install: `cargo install wasm-pack`

Open http://localhost:5173 in your browser. See `docs/` for architecture and design documents.

## Testing

```bash
npm test               # 990 unit tests (fast, CI gate)
npm run test:models    # 15 ONNX functional tests (real models, real inference)
npm run test:shaders   # 6 WebGL2 bit-true tests (system browser, real GPU)
```

## Desktop app (Tauri)

```bash
npm run tauri dev      # development
npm run tauri build    # production installer
```

## Build (web)

```bash
npm run build          # output to dist/
```

## Tech Stack

React 19 · TypeScript 6 · Vite 8 · React Flow 12 · Three.js · Zustand 5 · Immer · CodeMirror 6 · Tailwind CSS 4 · Tauri 2 · onnxruntime-web/node

## Roadmap

### Feedback / Accumulator (next milestone)

The engine currently treats every node as a pure function — inputs in, output out, no state across frames. This blocks an entire class of temporal effects that Quartz Composer's **Accumulator** patch enabled.

**Goal:** Add a per-node **feedback buffer** (ping-pong render targets) so a shader can read its own previous frame output via an implicit `previousFrame` uniform.

Unlocked effects:
- Motion blur / trails (blend current frame onto decaying previous)
- Reaction-diffusion (Gray-Scott, Belousov-Zhabotinsky)
- Flow fields / particle advection
- Fluid simulation (Navier-Stokes, Euler)
- Temporal anti-aliasing (TAA)
- Recursive feedback art / video feedback loops

Engine changes required:
1. **Ping-pong targets** — each feedback-enabled node gets two render targets; swap read/write each frame
2. **`previousFrame` uniform** — auto-injected sampler2D bound to the node's last-frame output
3. **Clear / reset** — initial state on first frame or on Stop→Play transition
4. **UI** — toggle on the node to enable feedback; side panel shows buffer state

### Other planned Quartz Composer parity patches

| QC Patch | Description | Complexity |
|----------|-------------|------------|
| **Delay (1-frame)** | Read another node's previous frame output | Shares ping-pong infra with Accumulator |
| **Image Transition** | Animated wipe/dissolve/push between two images | Shader preset + iTime |
| **Iterator / Replicate** | Execute a sub-graph N times per frame with varying params | Graph engine loop construct |
| **Macro Patch** | Collapse a sub-graph into a reusable compound node | Graph serialization + UI |
| **Bloom** | Multi-pass blur + additive blend (CIFilter equivalent) | Multi-pass rendering |
| **Motion Blur** | Directional / radial blur driven by velocity | Feedback or multi-sample |
| **Sample & Hold** | Latch a value and hold until triggered | Stateful node type |

## License

MIT — see [LICENSE](LICENSE).
