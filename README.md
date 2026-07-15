<p align="center">
  <img src="public/icon.svg" width="96" height="96" alt="Open Quartz icon">
</p>

<h1 align="center">Open Quartz</h1>

<p align="center">
  A hardware-accelerated visual graph editor for image, video, and neural network processing.
</p>

<p align="center">
  <img src="docs/screenshot.png" width="720" alt="Open Quartz screenshot">
</p>

Open Quartz is a node-based visual programming environment for real-time image, video, and ML inference processing. Build GPU-accelerated pipelines by connecting shader nodes, video/image inputs, ONNX neural network nodes, math operations, and renderer outputs on an infinite canvas. Inspired by Apple Quartz Composer and Shadertoy, with chaiNNer-style ML inference capabilities running entirely in the browser.

## Features

### Node Graph Editor
- **Drag, connect, and arrange** shader nodes on an infinite canvas (React Flow)
- **6 node types**: Shader (custom GLSL), Input (data sources), Math (CPU operations), Renderer (output viewer), ONNX (ML inference), Constant
- **Bezier curve edges** with type-safe connections — ports carry GLSL type metadata
- **MiniMap** for graph overview navigation
- **Box selection** for multi-node operations
- **Fit-to-view** on load

### Source System
- **Grouped SOURCE menu** — inputs organized into SYSTEM (Time/Mouse/Resolution), CONSTANTS (float/int/vec/mat), and EXTERNAL (Image/Framebuffer/Video) groups
- **Image input** — load images as sampler2D textures, with read-only width/height display
- **Framebuffer input** — load raw binary dump files as textures with configurable format (RGBA8 / RGBA32F / RG8 / RG32F / R8 / R32F / NV12), width, height, and stride
- **Video input** — camera and file video as sampler2D textures via HTMLVideoElement / THREE.VideoTexture; video dimensions propagate to downstream shader default size
- **Texture sampling config** — all sampler2D inputs support Filter (LINEAR / NEAREST) and Wrap (CLAMP / REPEAT / MIRROR) settings
- **Immediate preview** — Image, Framebuffer, and Video inputs show preview thumbnails as soon as data is loaded

### Math Nodes
- **29 CPU-based operations** across 6 categories: Arithmetic (add/subtract/multiply/divide/negate/modulo), Range (min/max/clamp/saturate/step/smoothstep/abs/sign), Trigonometry (sin/cos/tan/asin/acos/atan), Exponential (pow/sqrt/exp/log), Interpolation (mix), Rounding (floor/ceil/round/fract)
- **Auto type inference** — Math ports use `auto` type, resolved from connected peers. Output type promotes to widest input type
- **CPU-only evaluation** — no GPU shader compilation, pure JS computation in `runFrame()`. Results propagate to downstream shader uniforms
- **Compact visual nodes** — amber header with operation symbol (+ × sin √ etc.), 3-column layout
- **MATH dropdown menu** — 6 category sub-menus between SOURCE and SHADER in toolbar
- **Switchable operation** — change operation type from SidePanel dropdown without recreating the node

### Node Inspector & Editor (Side Panel)
- **Editable node label** and type badge
- **CodeMirror 6 shader editor** with GLSL syntax highlighting, error linting, and autocompletion
- **Port inspector** with color-coded data type indicators and inline uniform value editing
- **Per-component vector editing** (x/y/z/w) for vec2/vec3/vec4 uniforms
- **Image loading** for sampler2D input nodes (click or drag-and-drop)
- **Framebuffer config panel** — format dropdown, width/height inputs, stride input
- **Output preview** showing rendered results after graph execution
- **Output Auto Size** — checkbox to auto-infer width/height from inputs; manual override available (1–8192 px)

### Preview Lightbox
- **Full-screen image viewer** — click any image or video preview to open with scroll-to-zoom, drag-to-pan, and double-click reset
- **Nearest-neighbor rendering** — pixelated display for accurate pixel inspection at zoom
- **Save as PNG** — toolbar button with native save dialog (File System Access API) and fallback download
- **Color Picker** — toggle crosshair mode to inspect pixel coordinates (x, y) and RGBA color values with floating tooltip and color swatch

### Realtime Rendering
- **rAF-driven rendering loop** with PLAY / PAUSE / STOP transport controls
- **Host/Compositor architecture** inspired by Quartz Composer's QCRenderer — host drives the frame clock, compositor walks the node graph
- **Shadertoy-compatible builtin uniforms**: `iTime`, `iTimeDelta`, `iFrame`, `iDate`, `iMouse`, `iResolution` — opt-in by declaring e.g. `uniform float iTime;` in your shader to receive auto-injected values
- **GPU-only output path** — no `readPixels` in the realtime loop; preview via mirror canvas blit
- **Clock with pause/resume/seek** — time freezes on PAUSE, resets on STOP, and can be seeked programmatically

### Renderer Node
- **Explicit output viewer** — the Quartz Composer QCView equivalent; green header distinguishes it from shader nodes
- **Input**: single `sampler2D` from an upstream shader
- **In-place preview** on the node canvas or **panel preview** in the side panel
- **Multi-renderer support** — each renderer has its own independent mirror canvas via GPU→GPU blit (`drawImage`)
- **No extra render pass** — reads the upstream FBO directly
- **Fullscreen live preview** with SAVE button for frame capture

### Shader Engine
- **FBO-based multi-pass rendering** via Three.js — each shader node renders to an offscreen framebuffer and passes results downstream
- **HalfFloat FBOs** for input/shader intermediates to preserve float precision through the pipeline
- **Topological sort** ensures correct execution order through the graph
- **Automatic uniform wiring** — connections map upstream output textures to downstream sampler uniforms
- **Scalar uniform injection** — unconnected inputs are editable inline in the inspector
- **Per-node iResolution** — each shader gets its own FBO dimensions, not a single global resolution
- **Builtin uniform AUTO badges** — PortInspector shows AUTO badges for builtin uniforms (iTime, iMouse, etc.)
- **GLSL 300 es** support

### Predefined Shader Templates (10)
- Custom Shader (intensity multiplier)
- Custom 2IN-1 (mix of two sampler2D inputs)
- Sobel Edge Detection
- Gaussian Blur 3×3
- Box Blur
- Sharpen
- Invert
- Grayscale
- Emboss
- Pixelate (with configurable block size)

### ONNX Neural Network Nodes
- **Catalog model system** — ONNX dropdown menu organized by category (Detection, Super-Resolution). Select a model to auto-download and add to the graph. No pre-bundled model files required.
- **3 built-in models**: YOLOv8n (object detection, 80 COCO classes), Sub-pixel CNN 3x (super-resolution), Real-ESRGAN 4x (super-resolution)
- **Custom ONNX nodes** — load any `.onnx` model file for inference with auto-introspected ports
- **Tiled inference** — generic `TileCodec` engine splits large images into overlapping tiles, runs inference per-tile, and stitches results. No input size restrictions.
- **Adaptive tile sizing** — starts at 64px tiles, automatically halves on WebGPU allocation failure. Proven tile size cached for subsequent frames.
- **WebGPU to WASM auto-fallback** — when WebGPU kernels are incompatible (e.g. AMD iGPU), session automatically rebuilds with WASM-only backend. Orange "CPU fallback" badge shown in the side panel.
- **Static pipeline optimization** — pipelines without time-varying inputs (no `iTime`/`iMouse`/video) render a single frame, avoiding unnecessary GPU work. ONNX completion triggers a follow-up re-render for downstream nodes.
- **Model download manager** — background download with progress tracking, in-memory buffer cache, Tauri disk persistence
- **Task-specific codecs**: RGB codec (ESRGAN, 3-channel), YCbCr codec (Sub-pixel CNN, 1-channel Y with nearest-neighbor Cb/Cr)
- **Score/IoU thresholds** editable in the side panel for detection models; live detection list with class name, confidence, and normalized bbox
- **Realtime path** — ONNX nodes work in the realtime rendering loop with async non-blocking inference. ONNX output cache survives graph recompiles.
- See `docs/NN_SUPPORT_DESIGN.md` for the NN roadmap and `docs/ONNX_NODE_DESIGN.md` for architecture details.


### Project Management
- **Save / Save As** — export your graph as a `.quartz.json` file with native save dialog
- **Load** — import a previously saved project with auto-fit view
- **Editable project name** in the toolbar (double-click to rename)
- **Project file tracking** — SAVE silently overwrites last saved file

### Undo / Redo
- **50-level history** with Cmd/Ctrl+Z (undo) / Cmd/Ctrl+Shift+Z or Cmd/Ctrl+Y (redo)
- Snapshots taken before destructive operations

### GLSL Linting & Autocompletion
- **Real-time error checking** via WebGL2 shader compilation under the hood
- **Error markers** with precise line numbers in the editor gutter
- **Autocompletion** for GLSL keywords, types, built-in functions (52), built-in variables (16), and user-defined variables

### Desktop App (Tauri)
- Runs as a native desktop application via **Tauri 2**
- **Custom titlebar** — no system title bar; app header serves as the drag region
- **macOS**: overlay title bar style with native traffic light controls
- **Windows**: custom minimize/maximize/close buttons
- **Video file persistence** — Tauri asset protocol (`convertFileSrc`) preserves absolute video file paths across sessions
- Same feature set as the web version

## Getting Started

```bash
npm install
npm run build:wasm     # builds ONNX wasm bridge (requires wasm-pack)
npm run dev
```

> **Note:** `build:wasm` is required before first run. The wasm bridge JS/TS bindings are checked into git, but the wasm binary is not. Skipping this step will cause build errors.
>
> `wasm-pack` install: `cargo install wasm-pack`

Open http://localhost:5173 in your browser. See `docs/` for architecture and design documents.

## Usage

1. Click **SHADER** dropdown to pick from predefined templates or create a custom shader
2. Click **SOURCE** dropdown and hover a group (SYSTEM / CONSTANTS / EXTERNAL) to add source nodes
3. Click **MATH** dropdown to add CPU-based math operation nodes for signal processing
4. Add a **RENDERER** node to view shader output — each renderer provides an independent preview
5. Select a shader node to edit its GLSL code in the right panel
6. Drag between port handles to connect nodes
7. Edit uniform values inline in the port inspector
8. Click **PLAY** to start the realtime rendering loop; **PAUSE** to freeze, **STOP** to reset
9. Use `uniform float iTime;` / `uniform vec4 iMouse;` / `uniform vec3 iResolution;` in shaders for time, mouse, and resolution — declared uniforms are auto-injected
10. Click the renderer preview or **FULLSCREEN** to open a live fullscreen view; **SAVE** to export a frame as PNG
11. Click **SAVE** to download a `.quartz.json` project file, or **LOAD** to restore one

## Desktop app (Tauri)

Open Quartz also runs as a native desktop application via [Tauri](https://v2.tauri.app).

### Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (install via `winget install Rustlang.Rustup` on Windows)

### Development

```bash
npm run tauri dev
```

This starts the Vite dev server and opens the Tauri native window.

### Build

```bash
npm run tauri build
```

Produces a platform-specific installer in `src-tauri/target/release/bundle/`.

## Build (web)

```bash
npm run build
```

Output goes to `dist/`.

## Tech Stack

React 19 · TypeScript 6 · Vite 8 · React Flow 12 · Three.js · Zustand 5 · Immer · CodeMirror 6 · Tailwind CSS 4 · Tauri 2

## License

MIT — see [LICENSE](LICENSE).
