# Changelog

## [0.12.0b] -- 2026-07-18

### Features

- **Feedback/Accumulator engine** — ping-pong double-buffering with `previousFrame` uniform for shaders that accumulate state across frames. Auto-detected: if shader code references `previousFrame`, the engine creates two `rgba32f` ping-pong targets, binds the previous frame's texture on read, renders to the write target, and swaps per frame. No manual toggle needed.
- **Gray-Scott Reaction-Diffusion screensaver** — predefined FEEDBACK shader implementing the classic PDE system (`dA=0.16, dB=0.08, feedRate=0.040, killRate=0.060, timestep=0.2`) with 5-point Laplacian stencil, `uniform float iFrame` for periodic re-seeding, and configurable Clear Color RGBA in the side panel.
- **Field Color Map shader** — companion FEEDBACK shader that reads the G channel (chemical B concentration) from Gray-Scott output and maps it through a turbo colormap. Must remain split from the PDE node to avoid corrupting simulation state.
- **FEEDBACK shader category** — new shader category in the toolbar with Active/Inactive badge and Clear Color palette in the side panel.
- **Feedback badge on node cards** — "FB" indicator on shader nodes that use `previousFrame`.
- **Uniform default value extraction** — shader compiler now parses `uniform float name = value;` syntax, extracts the default value, and injects it into `selfUniforms` when the port is unconnected. All three stripping regexes handle the `= value` syntax.

### Fixes

- **WebGL renderer mock** — added `LinearSRGBColorSpace` export to three.js mock, fixing test failure.
- **ONNX introspection test** — fixed expected task for 4D output tensors.
- **Gray-Scott parameter scaling** — Laplacian kernel switched from 9-point (wrong anisotropy) to 5-point stencil; diffusion coefficients scaled for UV-space (no `1/h²` factor): `dA=0.16, dB=0.08`.
- **`uniform int` → `uniform float`** — `iFrame` changed to `float` type to avoid Three.js int uniform mismatch; comparison uses `iFrame < 0.5`.
- **Shader compile error line mapping** — preamble line offset corrected for shaders with `= default` syntax.

## [0.11.0b] -- 2026-07-17

### Features

- **Resample shader** -- passthrough identity shader (`texture → fragColor`) in the FILTER group. Leverages the node's output buffer size and format settings to perform rescaling and format conversion.
- **`npm run clean`** -- new script removes all build artifacts (`dist/`, `node_modules/.vite`, `src-tauri/target/`) for a clean rebuild.

### Fixes

- **ONNX introspection false detection** -- 4D image tensors (NCHW, e.g. `[1, 3, H, W]`) were misclassified as detection models because the width dimension triggered the `lastDim >= 5` heuristic. Added `outShape.length <= 3` guard so only 2D/3D outputs match detection. Custom ONNX models now correctly get `sampler2D` output ports.
- **Double-gamma on renderer output** -- Three.js r152+ defaults `outputColorSpace = SRGBColorSpace`, applying an extra linear→sRGB transfer on the final blit. Since all textures use `NoColorSpace` (no decode on read), this double-encoded the already-sRGB pixel values, visibly brightening the renderer output vs. the ONNX preview. Fixed by setting `outputColorSpace = LinearSRGBColorSpace`.
- **macOS icon oversized** -- regenerated `icon.icns` with Apple HIG-compliant ~80% inset on transparent canvas. macOS squircle mask now clips cleanly instead of cutting into edge-to-edge artwork.
- **Node header corner gap** -- inner header `rounded-t-xl` (12px) didn't nest inside the outer `rounded-xl` border (12px + 1px border). Changed to `rounded-t-[11px]` (outer radius minus border width) so selected border and header background align flush.
- **Buffer size input snaps to 512** -- width/height inputs in the side panel used `parseInt() || 512` on every keystroke, making it impossible to clear and retype a value. Now allows empty during editing and falls back to 512 only on blur.
- **Shader editor not scrollable** -- CodeMirror root lacked `height: 100%` and `.cm-scroller` lacked `overflow: auto`, preventing scroll on long shaders.
- **Custom ONNX node file picker** -- custom ONNX nodes now show an inline "Select .onnx file..." button instead of "Waiting to download...". Catalog nodes still show the download status. `portsVisible` simplified to `data.onnxStatus === 'ready'`.

## [0.10.0b] -- 2026-07-16

### Features

- **Background Removal nodes** (Phase 3) -- U²Net-P (4.4MB, 320×320 fixed) and MODNet (24.7MB, 512×512 fixed). Output is RGBA with alpha = foreground mask, directly compositable by downstream shaders.
- **Depth Estimation node** (Phase 4) -- MiDaS v2.1 Small (63MB, 256×256 fixed, BGR + ImageNet normalization). Outputs grayscale depth map for DOF/parallax/fog shader effects.
- **Custom ONNX model loading** (Phase 5) -- "Select Model File..." button loads any `.onnx` file. Auto-introspects model I/O → generates ports. Generic image→image execution via rgbCodec passthrough.
- **WebGPU probe at model load time** -- after download, runs a tiny dummy inference to detect GPU compatibility. User sees "CPU fallback" badge on the node immediately, before pressing Play. Results cached in localStorage by model + GPU vendor.
- **Execution engine refactor** -- `runTsOrtInference` generic method replaces per-task duplicated methods. New tasks require one routing line + one codec. Net -92 lines for the same functionality.
- **ONNX catalog expanded** -- 4 categories, 7 built-in models: Detection (YOLOv8n), Super-Resolution (Sub-pixel CNN 3×, Real-ESRGAN 4×), Background Removal (U²Net-P, MODNet), Depth Estimation (MiDaS v2.1 Small). Plus Custom ONNX.
- **Functional test suite** (`npm run test:models`) -- 15 tests with real model download, real `onnxruntime-node` inference, output verification for all 6 catalog models + custom. Weekly CI workflow + manual trigger.
- **Shader bit-true tests** (`npm run test:shaders`) -- 6 WebGL2 pixel-exact tests (identity, invert, grayscale, constant, alpha, channel swap) via vitest browser mode with system browser.
- **NN roadmap refocused on real-time** -- design principle: only models <30MB, <100ms on WebGPU. Large models (u2net 176MB, RIFE, LaMa) explicitly excluded. Phase 4b (JSON output tasks) split for later.

### Fixes

- **Fixed-size models missing WASM fallback** -- u2netp and sub-pixel CNN's `fixedSize` path bypassed the adaptive retry, causing hangs on incompatible GPUs.
- **MODNet Concat dimension mismatch** -- encoder needs input divisible by 32; set `fixedSize: 512` instead of dynamic tiling.
- **9 tsc build errors** -- TS 6 `Uint8ClampedArray<ArrayBufferLike>` vs DOM `ImageData` constructor; unused imports; `globalThis.ort` typing via global augmentation.
- **Wasm-pack snippet not in git** -- `yolo_detector.js` regenerated with new hash but `inline0.js` was gitignored. Force-added for CI.

## [0.9.0b] -- 2026-07-15

### Features

- **ONNX Catalog system** -- model dropdown organized by category (Detection, Super-Resolution). Three built-in models: YOLOv8n, Sub-pixel CNN 3x, Real-ESRGAN 4x. Models auto-download on first use, no pre-bundled files required. "Custom ONNX Model..." option for user-supplied `.onnx` files.
- **Tiled super-resolution inference** -- generic `TileCodec` engine splits images into overlapping tiles (64px + 8px padding), runs per-tile inference, stitches results with padding cropping. No input size restrictions -- full-resolution output at any scale.
- **Adaptive tile sizing** -- starts at 64px, automatically halves on WebGPU buffer allocation failure, caches proven size for subsequent frames. Zero retry cost after first convergence.
- **WebGPU to WASM auto-fallback** -- when WebGPU kernels are incompatible (e.g. AMD Radeon iGPU), session automatically rebuilds with WASM-only backend. Orange "CPU fallback" badge in side panel.
- **Static pipeline detection** -- pipelines without time-varying inputs (`iTime`/`iMouse`/`iTimeDelta`/`iFrame`/video) render a single frame then stop the rAF loop. ONNX completion triggers a follow-up re-render with frozen inputs (no clock advance). Cascaded ONNX nodes naturally converge.
- **ONNX output cache** -- inference results survive plan rebuilds (graph recompile), preventing redundant re-inference after unrelated node data changes.
- **Model download manager** -- `OnnxModelManager` with background download, progress events, in-memory buffer cache, Tauri disk persistence.
- **Model introspection** -- `inferTaskFromMeta` and `metaToDefaultPorts` auto-detect model task (detection/SR/generic) from I/O shape and generate appropriate port signatures.
- **198 new tests** -- onnxInference (34), onnxCatalog (56), onnxIntrospect (40), onnxModelManager (13), realtimeHost/isStaticPipeline (25), onnxStore (10), MathNode (20). Total: 954 tests across 40 files.

### Fixes

- **Dropdown menus not dismissible in Tauri** -- `startDragging()` consumed click events on dismiss overlays. Fixed by using `onMouseDown` instead of `onClick`.
- **Video cross-origin error** -- `crossOrigin='anonymous'` added to all video elements, fixing WebGL `texImage2D` SecurityError with Tauri asset protocol URLs.
- **ONNX re-inference loop** -- `updateNodeData` for backend status triggered graph recompile, clearing ONNX output and restarting inference. Fixed with output cache + conditional backend writes.
- **Renderer black after ONNX** -- async ONNX inference completed after static pipeline's single frame. Fixed with `scheduleRerender` callback + `renderer-remount` event listener for fullscreen.
- **Fullscreen renderer blank** -- fullscreen canvas mounted after the render pass in static mode. `renderer-remount` event triggers `renderToScreen()` repaint.

## [0.8.0b] — 2026-07-08

### Features

- **Math nodes** — 29 CPU-based math operations across 6 categories (Arithmetic, Range, Trigonometry, Exponential, Interpolation, Rounding). Pure JS computation in `runFrame()`, no GPU shader compilation. Amber-colored compact nodes with operation symbol display (+, ×, sin, √, etc.).
- **Auto type system** — new `'auto'` DataType for Math node ports. Actual type inferred from connected peers. Output type promotes to widest input type (`int → float`, `float < vec2 < vec3 < vec4`). Port colors update in real-time to reflect inferred type.
- **Relaxed connection rules for Math** — `auto` ports accept any scalar/vector type connection. `sampler2D`/`samplerCube` connections to auto ports are rejected. Both `isConnectionValid` (drag preview) and `onConnect` (commit) enforce the rule.
- **System source nodes** — TIME, TIME DELTA, FRAME, MOUSE, RESOLUTION as dedicated input nodes under SOURCE → SYSTEM menu. Green header, read-only live value display during playback (e.g. `2.345s`, `42`). Pure CPU value providers — no shader compilation.
- **SOURCE menu** — INPUT menu renamed to SOURCE and reorganized into three groups: SYSTEM (time/mouse/resolution), CONSTANTS (float/int/vec/mat), EXTERNAL (image/framebuffer/video). Moved before SHADER in toolbar order.
- **MATH menu** — new toolbar dropdown between SOURCE and SHADER with 6 category sub-menus matching QC-style Math/Logic patch organization.
- **Math SidePanel** — operation selector dropdown (switchable at any time), port type inference display, editable default values for unconnected inputs.
- **Engine math pipeline** — `scalarUpstream` map tracks all upstream connections (not just sampler2D). Math results propagate to downstream shader uniforms via `mathValues` map. Math→Math chaining supported.
- **113 new tests** — mathOps (76 tests, 100% coverage), store math/system (24 tests), engine math pipeline (13 tests). Total: 756 tests across 33 files.

### Fixes

- **System source shader error** — system source nodes (Time, etc.) no longer compile shader, eliminating `EXT_blend_func_extended` dual-output GLSL error.
- **Video thumbnail blank** — video preview shows first frame via `#t=0.1` URL fragment instead of blank `preload="metadata"`.
- **Video auto-play** — video thumbnails no longer auto-play/loop in non-play state on node and SidePanel previews.
- **Math→shader propagation** — fixed `upstreamSamplerBindings` only tracking sampler2D connections; added `scalarUpstream` map for scalar/math value injection into downstream shaders.
- **System source inputMode** — `makeNode` now correctly passes `inputMode` parameter; system nodes properly set `inputMode='system'`.
- **isConnectionValid for auto** — React Flow drag-preview validation now allows `auto` ↔ scalar/vector connections instead of rejecting on type mismatch.

## [0.7.1b] — 2026-07-09

### Features

- **Test coverage boost** — 642 tests across 30 files (up from 550/29). Coverage: 80% lines, 79% statements, 69% branches, 65% functions.
- **Coverage thresholds restored** — CI enforces 78% lines/statements, 55% branches, 64% functions.

### Fixes

- **ONNX overlay Y-flip** — `CanvasTexture.flipY` set to `true` to match the pipeline's OpenGL texture coordinate convention. Fixes inverted detection boxes in renderer output.
- **Menu interaction** — submenu gap bridge prevents accidental dismiss when sliding from primary to secondary menu; backdrop click closes both menu levels; `onMouseLeave` moved to menu container.
- **Renderer icon** — replaced emoji `🖥` with outline SVG matching other toolbar button icons.
- **CI wasm snippets** — committed `rust/crates/yolo-detector/pkg/snippets/` to git so CI can resolve `inline0.js` import without `build:wasm`.

## [0.7.0b] — 2026-07-09

### Features

- **Realtime rendering loop** — rAF-driven Host/Compositor architecture inspired by QC's `QCRenderer`. `PLAY / PAUSE / STOP` transport replaces legacy single-shot `RUN`.
- **Time system** — Shadertoy-compatible builtin uniforms (`iTime`, `iTimeDelta`, `iFrame`, `iDate`, `iMouse`, `iResolution`) auto-injected when declared in shader. Per-node `iResolution` matches each shader's FBO dimensions.
- **Renderer node** — explicit output viewer (QC's `QCView` equivalent). Green header, accepts upstream shader output via `sampler2D`. In-place preview on node or panel preview in side panel. No extra render pass — reads upstream FBO directly.
- **Multi-renderer support** — each renderer node has its own mirror canvas; output via GPU→GPU `drawImage` blit. Multiple renderers can display simultaneously.
- **Fullscreen live preview** — click FULLSCREEN on renderer panel preview to open live canvas overlay with SAVE button for frame capture as PNG.
- **Video input** — new `video` input mode under SAMPLER2D. Supports camera (`getUserMedia`) and file upload. `HTMLVideoElement` → `THREE.VideoTexture`, auto-updates each frame. Video dimensions propagate to downstream shader default size.
- **Video file persistence** — Tauri: stores absolute file path, restores via `convertFileSrc` on project load. Web: blob URL with reload prompt.
- **GPU-only output path** — realtime renderer preview uses no `readPixels` / `toDataURL`. All output stays on GPU via mirror canvas blit.
- **ONNX realtime support** — ONNX inference nodes now work in the realtime path with async non-blocking execution (1–N frame latency).
- **Builtin uniform badges** — PortInspector shows `AUTO` badge on builtin uniforms (`iTime`, `iMouse`, etc.) indicating they are auto-injected by the engine.
- **Clock** — `pause()` / `resume()` / `seek()` support. FPS calculated via sliding window average.
- **MouseState** — Shadertoy `iMouse` convention (origin bottom-left, z/w for click state).

### Breaking Changes

- **RUN button removed** — single-shot execution UI eliminated. All rendering goes through `PLAY` which drives the realtime Host. Future single-frame needs will use `STEP` or `ScrubHost`.
- **`isRunning` / `setRunning` removed from store** — replaced by `loopState: 'stopped' | 'playing' | 'paused'` with `play()` / `pause()` / `resume()` / `stop()` actions.

### Fixes

- **Stop/play lifecycle** — WebGL context preserved across stop/play cycles (`clearResources` instead of `dispose`). Canvas properly unmounted on stop and remounted on play.
- **Video pause** — pausing the host now also pauses `<video>` elements; resume restarts them.
- **Video source reconciliation** — async video init triggers plan recompile so textures appear without manual graph interaction.
- **Shader `v_uv` redefinition** — `shaderCompiler` now strips user `in vec2 v_uv;` declarations to avoid GLSL redefinition errors.
- **WebGL feedback loop guard** — `renderWithMaterial` checks for self-referencing texture/target before draw.
- **Per-node iResolution** — each shader receives its own FBO dimensions instead of a global value, fixing UV scaling bugs on mixed-resolution graphs.
- **autoSize respected** — shader nodes with `autoSize !== false` now correctly use upstream-derived default size instead of hardcoded 512×512.

## [0.6.0b] — 2026-07-08

### Fixes

- **Image input no longer wastes an FBO** — image input nodes now pass their `THREE.Texture` directly to downstream shaders instead of blitting through an intermediate FBO. Eliminates one full-screen copy per image input per execution.
- **Shader output size is self-contained** — a shader node's configured width/height now only determines its own render target resolution. Removed the reverse propagation that leaked downstream shader dimensions into upstream input nodes.
- **Framebuffer input uses its own dimensions** — raw/framebuffer input nodes now create their FBO at the declared `fbWidth × fbHeight` instead of a propagated size.

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
