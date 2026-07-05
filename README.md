<p align="center">
  <img src="public/icon.svg" width="96" height="96" alt="Open Quartz icon">
</p>

<h1 align="center">Open Quartz</h1>

<p align="center">
  A visual GLSL shader node editor inspired by Apple Quartz Composer.
</p>

<p align="center">
  <img src="docs/screenshot.png" width="720" alt="Open Quartz screenshot">
</p>

Build and connect GLSL shaders visually using a node graph. Each node is a shader processing unit — edit its GLSL code, and input/output ports are auto-generated from `uniform`/`out` declarations. Connect nodes to create shader pipelines and see real-time WebGL output.

## Features

### Node Graph Editor
- **Drag, connect, and arrange** shader nodes on an infinite canvas (React Flow)
- **4 node types**: Shader (custom GLSL), Input (data sources), Output (render targets), Constant (fixed values)
- **Bezier curve edges** with type-safe connections — ports carry GLSL type metadata
- **MiniMap** for graph overview navigation
- **Box selection** for multi-node operations
- **Fit-to-view** on load

### Node Inspector & Editor (Side Panel)
- **Editable node label** and type badge
- **CodeMirror 6 shader editor** with GLSL syntax highlighting, error linting, and autocompletion
- **Port inspector** with color-coded data type indicators and inline uniform value editing
- **Per-component vector editing** (x/y/z/w) for vec2/vec3/vec4 uniforms
- **Image loading** for sampler2D input nodes (click or drag-and-drop)
- **Output preview** showing rendered results after graph execution
- **Width/Height controls** for output nodes (1–8192 px, auto by default)

### Shader Engine
- **FBO-based multi-pass rendering** via Three.js — each shader node renders to an offscreen framebuffer and passes results downstream
- **Topological sort** ensures correct execution order through the graph
- **Automatic uniform wiring** — connections map upstream output textures to downstream sampler uniforms
- **Scalar uniform injection** — unconnected inputs are editable inline in the inspector
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

### Project Management
- **Save** — export your graph as a `.quartz.json` file
- **Load** — import a previously saved project
- **Editable project name** in the toolbar

### Undo / Redo
- **50-level history** with Ctrl+Z (undo) / Ctrl+Shift+Z or Ctrl+Y (redo)
- Snapshots taken before destructive operations

### GLSL Linting & Autocompletion
- **Real-time error checking** via WebGL2 shader compilation under the hood
- **Error markers** with precise line numbers in the editor gutter
- **Autocompletion** for GLSL keywords, types, built-in functions (52), built-in variables (16), and user-defined variables

### Desktop App (Tauri)
- Runs as a native desktop application via **Tauri 2**
- Same feature set as the web version with a native window

## Getting Started

```bash
npm install
npm run dev
```

Open http://localhost:5173 in your browser.

## Usage

1. Click **+ Shader**, **+ Input**, or **+ Output** in the toolbar to add nodes
2. Use the **Shader** dropdown to pick from 10 predefined shader templates or create a custom one
3. Select a shader node to edit its GLSL code in the right panel
4. Drag between port handles to connect nodes
5. Edit uniform values inline in the port inspector
6. Click **Run** to execute the graph and see output
7. Click **Save** to download a `.quartz.json` project file, or **Load** to restore one

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
