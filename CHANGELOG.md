# Changelog

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
