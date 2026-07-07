# Changelog

## [0.5.0b] — 2026-07-08

### Breaking Changes

- **Output node removed** — the standalone Output node type is eliminated. All shader nodes now serve as output points with built-in output configuration (format, resolution, sampling) and preview. This removes the redundant passthrough FBO copy that the Output node imposed.
- **Project file version 0.2.0** — project files are now versioned internally. Loading a file with an incompatible version (e.g. 0.1.0) will show an error. Re-save existing projects to upgrade.

### Features

- **Unit test suite** — 453 tests across 23 test files covering all modules (engine, utils, store, types, components). Coverage: 83% lines, 82% statements, 74% branches, 79% functions.
- **GitHub Actions CI** — push/PR to master triggers lint, type check, test + coverage. Coverage thresholds enforced (80% lines/statements, 70% branches, 75% functions).
- **Shader output config** — every shader node (not just leaves) has OUTPUT CONFIG in the side panel: format (RGBA8/RGBA32F/RG8/RG32F/R8/R32F), resolution (auto-size or manual 1–8192), and texture sampling (filter/wrap).
- **Shader preview thumbnails** — shader nodes show inline preview thumbnails with format and resolution badge after execution.
- **Zero-redundant-FBO pipeline** — FBO resolution propagates from shader output config upstream through the graph. No unnecessary intermediate FBOs; shaders execute at target resolution.

### Fixes

- **FBO resolution mismatch** — fixed pixelRatio=2 on Retina screens inflating the offscreen canvas to 2x the intended size, causing black regions when output dimensions exceeded the canvas. Offscreen pipeline now uses pixelRatio=1.
- **FBO size propagation** — shader/input FBOs now use the downstream output resolution instead of being hardcoded to input image dimensions.

## [0.4.2b] — 2026-07-07

### Features

- **Connection type validation** — dragging a wire between incompatible port types now shows a red line and rejects the connection on drop; compatible connections highlight in blue
  - sampler2D inputs accept: IMAGE/FRAMEBUFFER input nodes, shader/output/constant nodes (FBO texture)
  - Non-sampler inputs require exact dataType match (e.g. float↔float, vec3↔vec3)
- **Complete GLES 300 input types** — INPUT menu expanded with all GLSL ES 3.0 types: uint, uvec2-4, bvec2-4, ivec2-4, mat2-4

### Fixes

- **Shader editor selection highlight** — fixed invisible text selection caused by opaque `.cm-activeLine` background covering the selection layer
- **Linter error line numbers** — fixed off-by-one mapping and stripped-line compensation when user code contains `#version`/`precision` directives
- **Execution engine error line numbers** — shader compile errors now subtract the injected preamble (Three.js #version + precision + uniforms) to show correct user-code line numbers

## [0.4.1b] — 2026-07-07

### Features

- **Output node as framebuffer** — output node redesigned to match framebuffer input pattern:
  - Card shows single-row layout with input/output handles flanking a thumbnail preview
  - Configurable render target format: RGBA8, RGBA32F, RG8, RG32F, R8, R32F
  - Texture sampling config (Filter: LINEAR/NEAREST, Wrap: CLAMP/REPEAT/MIRROR)
  - Format and resolution badge on card thumbnail

## [0.4.0b] — 2026-07-06

### Features

- **Custom titlebar** — hide system title bar, use app Header as drag region; macOS traffic lights overlay, Windows custom minimize/maximize/close buttons; browser mode unaffected
- **App logo in header** — favicon displayed before OPENQUARTZ title
- **Lightbox toolbar** — semi-transparent centered toolbar in the preview lightbox with:
  - **Save as PNG** — native save dialog (File System Access API) with fallback download
  - **Color Picker** — toggle crosshair mode to inspect pixel coordinates (x, y) and RGBA color values with floating tooltip and color swatch
- **Nearest-neighbor rendering** — lightbox uses pixelated image rendering for accurate color picking at zoom

## [0.3.0b] — 2026-07-06

### Features

- **Grouped INPUT menu** — INPUT dropdown restructured into SCALAR / VECTOR / SAMPLER2D groups with hover-expand nested sub-menus
- **Framebuffer input** — new input type under SAMPLER2D for loading raw binary dump files as textures, with configurable format (RGBA8 / RGBA32F / RG8 / RG32F / R8 / R32F / NV12), width, height, and stride
- **Texture sampling config** — all sampler2D inputs (Image & Framebuffer) now have Filter (LINEAR / NEAREST) and Wrap (CLAMP / REPEAT / MIRROR) controls in the side panel
- **Immediate preview** — IMAGE and FRAMEBUFFER inputs show preview as soon as data is loaded, without pressing RUN
- **Output Auto Size** — output node has an Auto Size checkbox (default on); when off, width/height are editable with a default of 512

### Improvements

- IMAGE input node header now shows "IMAGE" instead of "SAMPLER2D"
- IMAGE input side panel shows read-only image dimensions
- Use HalfFloat FBOs for input/shader intermediates to preserve float precision through the pipeline
- Support float render target readback for preview generation

## [0.2.0b] — 2026-07-06

### Features

- **Preview lightbox** — click output preview image to open full-screen viewer with scroll-to-zoom, drag-to-pan, and double-click reset
- **Project save/load** — SAVE / SAVE AS / LOAD with `.quartz.json` project files, cross-browser compatible (no File System Access API dependency)
- **Auto-hide side panel** — SidePanel hides when no node is selected
- **Node error system** — shader compile errors with line info, unconnected input validation, auto-select errored node
- **Keyboard shortcuts** — Delete/Backspace to remove selected elements, Cmd+Z / Cmd+Shift+Z for undo/redo

### Fixes

- Fix shader editor stale closure when switching between nodes
- Fix MiniMap auto-hide behavior
- Fix port ID remapping when shader code changes (prevents false "unconnected input" errors)
- Preserve port IDs across shader re-parses to prevent run hangs
- Fix run hang on image load failure
- Fix GLSL validation to use correct `#version 300 es` prefix
- Fix project name tracking: use filename on LOAD, reset on CLEAR
- Fix SAVE AS to detect user-typed filename
- Replace File System Access API with download-based save (Safari compatibility)
- Fix file picker accept filter for `.quartz.json` files on Safari
- Auto-fit view after loading a project

## [0.1.0b] — 2025-05-25

### Features

- Initial release
- Visual GLSL shader node editor inspired by Apple Quartz Composer
- Three node types: Shader, Input, Output
- GLSL syntax highlighting, error linting, and autocompletion (CodeMirror 6)
- Automatic port generation from GLSL `uniform` / `out` declarations
- WebGL FBO render pipeline with topological sort execution
- Predefined shader templates (custom, 2-in-1, and built-in effects)
- Scalar and image (sampler2D) input nodes with type picker
- Output node with configurable width/height and per-node preview
- Undo/redo support
- macOS-style minimal UI with Tailwind CSS
