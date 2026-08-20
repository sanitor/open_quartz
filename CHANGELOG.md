# Changelog

## Unreleased

## [0.20.0b] -- 2026-08-19

### Breaking Changes

- **Rust crate boundaries** — split the former monolithic runtime into substantive `open_quartz_schema`, `open_quartz_execution`, `open_quartz_host_api`, `open_quartz_sdk`, and `open_quartz_bindings` crates. `open_quartz` is now the thin native/WASM facade, and legacy internal module paths are no longer public.
- **Thin TypeScript cutover** — remove the legacy TypeScript graph/GPU runtimes, shader materialization, Math execution callbacks, duplicate ONNX registries and pre/postprocessors, and obsolete WebGL execution types. Public TypeScript objects now proxy Rust-owned aggregates and revisions.

### Features

- **Rust-backed public objects** — `OpenQuartz`, `Project`, `Graph`, `Node`, and `Port` now use Rust-owned snapshots, atomic edits, monotonic revisions, rollback/redo, validation, serialization, and structured stale/disposed errors through WASM and native bindings.
- **Rust-owned graph and project policy** — move node factories, connection/type invariants, cascade removal, Project normalization, screen saver transforms, catalog execution semantics, ONNX task planning, and host resource intents into directly tested Rust contracts.
- **Unified Renderer previews** — keep one Renderer output across graph-node, Side Panel, fullscreen, native TextureStream, bounded native readback, and browser PNG preview paths. The Side Panel now renders actual browser output data and a named native mirror target.
- **Measured browser path improvements** — use `FileReader` for preview Blob data URLs with an exact fallback and reuse per-frame typed arrays in the Worker; reproducible benchmark fixtures record the measured boundary without claiming unmeasured ORT/video gains.
- **Boundary enforcement** — add dependency checks, public-proxy parity checks, compile fixtures, language conformance tests, and CI enforcement for the new Rust/TypeScript boundaries.

### Fixes

- **Runtime playback state** — make Native live graph replacement restart the composition clock atomically under the runtime lock, remove the duplicate host `play` call, and recheck playback after lock acquisition so pause/stop races cannot advance a paused Runtime.
- **Renderer preview lifecycle** — switching In-place preview while playing no longer reports `Runtime can only play from the ready state`; Browser and Native Side Panel previews now receive the active Renderer output.
- **Graph undo/redo projection** — align Store actions with one Rust history entry so Renderer and ONNX defaults no longer create hidden extra revisions that leave nodes visible after Undo.
- **Web video playback** — keep one acknowledged video frame in flight per node, acknowledge Worker receipt before rendering, coalesce pending `ImageBitmap`s, and cap the Worker render loop at 60 FPS so message and GPU queues cannot run away. Browser previews no longer create duplicate media decoders; replacement sources are started before the old source is released, and stale Blob URLs are revoked.
- **Browser video file chooser** — stop the hidden file input's synthetic click from bubbling back into its clickable node container, which recursively reopened the chooser and emitted continuous errors; browser selection now stays synchronous while Tauri continues through the native dialog plugin.
- **Renderer fullscreen TextureStream lifecycle** — moving the canonical video consumer into fullscreen explicitly resumes playback, and closing fullscreen relocates it before React tears down the overlay so presentation does not stop.
- **Browser runtime lifecycle clock** — convert JavaScript lifecycle timestamps to `bigint` before crossing wasm-bindgen's `u64` boundary; browser playback now starts correctly so SYSTEM TIME can drive connected uniforms such as Hue Rotate `angle`.
- **Cross-WebView input reconnection** — remove the synthetic `mousedown` redispatch used for occupied input ports, use React Flow's supported edge reconnection API, and atomically replace an existing input edge when SYSTEM TIME is connected to Hue Rotate `angle`.

### Tests

- **Automated suites** — 704 Vitest tests across 39 files and 129 Rust tests across 43 suites pass, together with TypeScript diagnostics, production WASM/Vite builds, public-proxy parity, Rust dependency boundaries, and real Browser Renderer playback/preview smoke coverage.
- **Runtime lifecycle regressions** — cover Rust/Store revision parity, Native pause/stop lock races, live Native graph replacement, Browser play/pause/resume/stop cycles, and Renderer In-place-to-Side-Panel preview routing.

## [0.19.0b] -- 2026-08-10

### Features

- **Self-contained Windows screen saver host** — File groups Load/Save/Save As and exports a selected Renderer into a version-3 native `.scr` that directly owns the shared Rust Runtime, wgpu executor, Win32 fullscreen/preview child window, and graph/resource manifest; it no longer launches or depends on an installed OpenQuartz/Tauri application.
- **Capability-closed screen saver packages** — Exports embed graph-required file video, FFmpeg, ONNX models, ORT, and DirectML payloads. The host restores image/video overrides, runs shared native media and ONNX task pipelines, and presents directly to a DX12 surface without per-frame GPU readback or GDI.

### Fixes

- **Native video source restoration** — project reload restores path-backed video thumbnails and source metadata without creating duplicate playback resources.
- **HEVC D3D12 surface ownership** — retain decoder-owned surfaces through direct GPU import so borrowed texture-array subresources remain valid.
- **Native video replacement and replay** — live H.265→H.264 changes detach the old decoder before attaching the replacement, ready graphs restart with PLAY, and retained TextureStream consumers resume after STOP or PAUSE.
- **Stable 8K D3D12VA conversion** — render NV12/P010 conversion directly into the persistent graph output texture, eliminating the observed per-frame temporary 8K RGBA allocation and copy.
- **Development build isolation** — `tauri dev` no longer builds the Windows screen saver host; SCR compilation runs only for Tauri production builds or the explicit `build:screensaver-stub` command, and shares `src-tauri/target` so common Rust dependencies reuse Cargo artifacts.
- **Screen saver predefined shaders** — exports now materialize catalog shader source into the self-contained graph instead of packaging the project-file placeholder `shaderCode: ""`; the native exporter rejects any remaining source-less shader before writing a `.scr`, preventing wgpu pipeline panics on launch.

### Tests

- **Automated suites** — 998 Vitest tests across 43 files, application/core Rust tests, and 4 native screen saver host tests pass; SCR package tests cover embedded resource offsets and extraction, while TypeScript type-check, browser production build, native build, and Rust/WASM compilation pass.
- **Native screen saver smoke** — the 5,725,184-byte host builds successfully and owns `/s`, `/p`, `/c`, Runtime, GPU surface, media, inference, and resource closure inside the `.scr` process.

## [0.18.0b] -- 2026-08-06

### Features

- **Direct TextureStream composition** — Renderer, SidePanel, and fullscreen views now share one canonical WebView2 media consumer instead of copying frames through intermediate canvases.
- **Accurate presentation telemetry** — renderer metrics distinguish graph execution, submitted, callback, presented, displayed, and dropped frames, including callback latency percentiles and burst counts.
- **Stable H.264 D3D12VA zero-copy input** — the Windows runtime uses a checksum-pinned FFmpeg master build with fixed reference-frame reuse, sustaining 61.31 FPS at 1920×1080 with zero CPU-copy bytes in the native benchmark.

### Fixes

- **TextureStream pacing and lifecycle** — corrected presentation timestamps, first-frame handshakes, stale readback rejection, stream-start retries, and repeated Zustand updates.
- **D3D12VA frame selection** — preserve FFmpeg-reported texture-array subresources so decoder surface reuse cannot reorder displayed frames.
- **TextureStream presentation scaling** — downscale oversized exported frames while retaining full-resolution native textures and screenshots.

### Tests

- **Native H.264 benchmark** — 10.01 seconds at 61.31 render/preview FPS, 391 decoded and uploaded frames, `cpu_copy_bytes=0`.
- **Automated suites** — 990 Vitest tests across 41 files and 76 Rust tests pass; TypeScript type-check passes.

## [0.17.0b] -- 2026-07-29

### Features

- **Dual-target Rust SDK** — added the `open_quartz` crate for native and WASM with shared graph types, topology/dirty planning, WGSL parsing/compilation, typed frame planning, resource generations, structured lifecycle events, GPU resource primitives, and ONNX pre/postprocessing.
- **Rust-backed production WGSL parser** — `naga` now powers Header catalog parsing, Shader Editor diagnostics, SidePanel descriptions, graph updates, and node creation through the synchronous WASM SDK adapter; the legacy TypeScript parser and `wgsl_reflect` dependency were removed.
- **Native GPU production runtime** — added a Rust offscreen `GpuExecutor`, DX12/Metal/Vulkan selection, retained pipelines/targets/feedback resources, and a Rust-owned render thread. `PipelineService` now explicitly selects this runtime in Tauri and the browser adapter elsewhere.
- **GPU-only Windows Renderer output** — native DX12 output now flows through the three-slot shared-texture exporter, a cached D3D11On12/keyed-mutex bridge, NV12 VideoProcessor conversion, and WebView2 TextureStream into the existing Renderer canvases. The path carries no per-frame pixel IPC/readback; unsupported hosts retain coalesced RGBA readback.
- **Native ONNX graph execution** — connected ONNX execution commands to async GPU readback, CPU/DirectML ORT workers, six task-specific postprocessors, generation-safe completion, GPU output upload, cascade/static/video dirty propagation, and provider/data events.
- **Bundled native video and camera runtime** — Tauri packages FFmpeg and its notice; native threads decode file/camera sources with loop, rate, pause/resume, generation-tagged frame slots, and no decoded-frame WebView IPC. SidePanel camera discovery now uses browser MediaDevices or native DirectShow/AVFoundation/V4L2 devices by host.
- **Windows D3D12VA zero-copy video input** — native file video now decodes through libav D3D12VA into P010 `ID3D12Resource` surfaces, waits the decoder fence, imports the resource through wgpu-hal DX12, and performs GPU-only BT.709 P010→RGBA conversion. The runtime bundles a checksum-pinned LGPL shared FFmpeg build and reports `d3d12va-p010-zero-copy`; camera and non-Windows paths retain the explicit CPU-copy fallback.
- **Deterministic SDK startup and packaging** — the generated WASM SDK initializes before React, Vitest loads the real Node binding in global setup, and Windows bundles include ORT, DirectML, FFmpeg, and license resources.
- **Unified host runtime facade** — browser and Tauri adapters implement one `PipelineHostRuntime` lifecycle; App delegates orchestration to `PipelineService` without dual runtime startup or hidden fallback.

- **Runtime cutover hardening** — Rust Runtime now owns paced browser ticks, async completion acceptance, typed output contracts, presentation dispatch, resource-release retry semantics, and consuming work batches. Browser playback uses a Worker/OffscreenCanvas host; native media reports explicit CPU-copy counters and data-path capabilities.
- **Multi-presenter and DXGI export** — added retained `GpuPresentationFrame` handles, per-presenter latest-frame mailboxes and backpressure counters, a presenter registry, a three-slot asynchronous GPU readback ring, and a Windows `DxgiSharedTextureExporter` backed by shared `ID3D12Resource` textures, NT resource/fence handles, queue-ordered synchronization, and explicit consumer leases.
- **Native surface contracts** — added DXGI/IOSurface/DMA-BUF hardware-frame descriptors, synchronization metadata, native decoder/import traits, direct registration of imported GPU textures in `GpuExecutor`, and opt-in native runtime commands for shared-texture acquire/release. WebView2 capability distinguishes interface availability from stream readiness and selects TextureStream only after the renderer adapter is ready.
- **Lossless WebView policy** — rejected H.264 because it is lossy; WebView2 TextureStream is the Windows lossless realtime path, while bounded RGBA readback and explicit full-resolution capture remain compatibility paths.

### Fixes

- **Vite generated SDK loading** — load the generated public WASM package through a fully qualified runtime URL so Vite does not treat it as a source dependency.
- **Native video frame reuse** — restore the fixed-size decoder buffer after every swap so later generations cannot publish zero-byte frames.
- **Video-to-image resource replacement** — detach stale native video before replacement image upload so cleanup cannot remove the new texture.
- **Browser screenshot readback** — `RealtimeHost.captureScreenshot()` now forwards the compositor's asynchronous WebGPU readback instead of returning a permanent `null` placeholder.
- **Native preview delivery** — native metadata now follows every render frame with one pending-readback backpressure slot, while Renderer and selected-node previews preserve the actual output dimensions.
- **Tauri content boundaries** — replaced the unrestricted CSP and wildcard asset scope with explicit script, worker, media, image, IPC, app-data, resource, and user-media policies.
- **Native stop/replay lifecycle** — starting playback after STOP now restarts retained FFmpeg video decoders and resets native playback timing instead of leaving the render loop on a frozen frame.
- **Runtime and Renderer FPS status** — the Header keeps graph execution FPS, while each Renderer panel reports its own 500 ms delivery FPS measured only after a received frame is drawn into a visible mirror canvas.
- **Native Renderer throughput** — live renderer events now use bounded 960px preview readback instead of full-resolution 8K IPC payloads; full-resolution `readOutput()` remains reserved for screenshots/capture.
- **H264 compatibility** — Windows D3D12VA P010 is selected only for eligible 10-bit sources; 8-bit H264 and unsupported formats use the existing FFmpeg CPU RGBA decoder instead of silently stopping after a D3D12 frame-format mismatch.
- **Native performance diagnostics** — add one-second aggregate timing logs for native render stages, GPU preview scale/readback, Tauri preview throughput/queueing, and browser `ImageData`/mirror-canvas drawing so the remaining 50→60 FPS bottleneck can be identified without per-frame log noise.
- **WebView2 TextureStream end-to-end** - register the required native `StartRequested` COM handler, keep video playback alive while waiting for its first decoder frame, create the first texture only after `StartRequested` (then reuse textures through `GetAvailableTexture`), defer the JS consumer request until the producer has output, retry failed handshakes, and wait for the renderer-adapter capability during native initialization.
- **Realtime UI responsiveness** — throttle transient FPS/time/frame store publication to 10 Hz in one Zustand update, isolate NodeGraph/SidePanel subscriptions from unrelated frame state, and coalesce native video mirror drawing onto browser `requestAnimationFrame`, preventing 60 Hz IPC/store churn from starving selection, side-panel, and window interactions during 8K playback.
- **D3D12VA loop stalls** — seek and flush the existing demuxer/decoder at file-video loop boundaries instead of reopening the media input and recreating the hardware device, retaining the decoder GPU context across loops.
- **Native live-graph recovery** — resume the native render worker after every graph update so a transient incomplete graph assembled while PLAY is active cannot leave the UI transport playing while the Rust worker remains stopped on a stale output-node error.
- **Native video resource lifecycle** — keep Tauri file videos on the native path when reselected, stop creating duplicate WebView video decoders for native thumbnails, use bounded 960px native preview readback instead of full-resolution 8K readback, and throttle selected-input previews to 5 Hz.
- **D3D12VA frame ordering** — sample the FFmpeg-reported D3D12 texture-array subresource instead of always reading array layer zero, preventing decoder surface reuse from making displayed video frames jump backward despite monotonic renderer timestamps.
- **Long-run 8K stability** — retain one RGBA conversion target per native video node instead of allocating a new 8K texture for every decoded D3D12VA frame, eliminating GPU allocation churn and the progressive 40→10 FPS collapse.
- **Visible Renderer FPS** — size node and SidePanel canvas backing stores to their visible preview bounds instead of 7680×3840, avoiding a full 8K `drawImage` plus browser canvas backing update on every TextureStream frame while preserving full-resolution native output and screenshot capture.
- **Direct TextureStream Renderer composition** — expose the native MediaStream through the UI store and render it directly in Renderer node, SidePanel, and fullscreen views; `requestVideoFrameCallback` now measures delivery without `video→canvas drawImage`, while canvas remains the fallback path.
- **SidePanel native stream path** — selected Renderer previews now use the same direct TextureStream `<video>` composition as the graph node and fullscreen view; the SidePanel no longer creates a hidden 960px canvas fallback while the native stream is active.
- **TextureStream single-consumer composition** — keep one hidden media consumer for the WebView2 stream and mirror its `srcObject` into the visible Renderer video when mounted, avoiding independent MediaStream consumers for the node, SidePanel, and fullscreen views.
- **Single DOM video consumer** — move the canonical TextureStream `<video>` element between the node, SidePanel, and fullscreen slots instead of assigning the same MediaStream to several video elements, ensuring WebView2 has exactly one media/compositor consumer.
- **TextureStream media clock** — use microsecond presentation timestamps for WebView2 TextureStream; consumer diagnostics showed `mediaDelta≈10s` per one-second window with the previous 100ns conversion, causing WebView2 to fast-forward/drop frames and oscillate between 10–45 FPS.
- **TextureStream UI state churn** — publish the native stream to Zustand only when the `MediaStream` identity changes; repeated native frame events no longer trigger redundant store updates or React reconciliation.
- **Renderer cadence accounting** — separate native graph execution, TextureStream frames submitted for composition, `requestVideoFrameCallback` cadence, and frames actually displayed versus dropped using `getVideoPlaybackQuality()`; expose callback P50/P95/max latency and presented-frame burst distribution instead of relabeling submitted frames as visible FPS.
- **TextureStream presentation scaling** — keep native graph/output textures at full resolution, but downscale oversized Windows TextureStream frames in a GPU render pass to a maximum 3840px dimension before DXGI export; screenshots and native output readback remain full-resolution.

### Documentation

- **Architecture cutover** — updated `docs/DESIGN.md` and README around the completed host selection, native ONNX dataflow, resource ownership, camera UI, security boundaries, observable provider fallback, and remaining cross-platform constraints.
- **Release automation** — GitHub releases are explicitly created with `--latest`, and the shared Rust SDK version is included in the synchronized version checklist.

### Tests

- **980 unit tests + 56 shader/pipeline tests**, all passing.
- **Rust/native contracts** — core and Tauri suites cover graph/FFI lifecycle, GPU execution, feedback preservation, native ONNX image tasks, multi-frame FFmpeg decode, resource replacement, and camera metadata parsing.
- **Presenter/media GPU contracts** — focused Rust tests exercise independent latest-frame replacement, shared-texture exporter dispatch, FFmpeg-style external hardware-frame import, graph-visible imported textures, DXGI resource/fence handle reopen, queue synchronization, pool saturation, reuse, and release.
- **Runtime smoke** — generated WASM loads in Node/Chromium; native DX12 image, video, async ONNX, shared-texture acquire/release, preview, and WebView2 TextureStream pipelines validate output; the TextureStream smoke observes the final DOM pixel after runtime shared-frame presentation. Bundled DirectML identity returns `7`.

## [0.16.0b] -- 2026-07-29

### Features

- **Zero-copy video input** — video nodes now use `importExternalTexture` for zero-copy GPU sampling. The video decoder's output buffer is referenced directly without any GPU-to-GPU copy. Compiler rewrites `textureSample` → `textureSampleBaseClampToEdge` + `texture_external` transparently. Image/framebuffer inputs unchanged.
- **Shader & port documentation** — WGSL `//` comments are parsed into shader descriptions and per-port tooltips. All 28 predefined shaders and 27 uniforms annotated with purpose and value ranges. SidePanel shows shader description in header; port labels show tooltip on hover; menu items show tooltip.
- **Edit-time GPU validation** — CodeMirror linter runs `createShaderModule` + `getCompilationInfo` (debounced 750ms) for red squiggly underlines on WGSL errors. Lazy-inits its own `GPUDevice` — works without pressing Play.
- **Correct uniform types** — all predefined shaders now declare `@group/@binding` with correct WGSL types (vec4f for colors, vec2f for sizes). Parser walks AST path instead of defaulting everything to float. 8 wrong uniform types fixed.
- **Node status tests** — 28 new tests across all 5 node types (ShaderNode, RendererNode, OnnxNode, InputNode, MathNode) covering every status factor and combination.

### Fixes

- **Uniform values never uploaded to GPU** — `runFrame()` had a `// TODO` where scalar uniform buffers were never created. All uniforms (self-owned, upstream, builtins like `iTime`) now correctly written to `GPUBuffer` and bound. This was the root cause of "always black output" with `intensity` slider.
- **`@doraemon` leaks as a port** — regex fallback scanner now skips identifiers preceded by `@` (attribute names). Regression test added.
- **Preamble line offset** — extra `\n` between preamble and user code caused GPU error line numbers to be off by 1. Fixed in both `compileWgslShader` and `validateWgslEdit`.
- **Debounced shader editing** — port reparse debounced to 400ms (was every keystroke). No more UI thrashing while typing.

### UI

- **SidePanel cleanup** — header follows node card format (icon + type + label + delete icon). Output config moved inline under output ports with grid layout. Feedback badge in header instead of separate section.
- **MathNode** — symbol enlarged (16→22px), color muted to match caption, port handles aligned to card edges.

### Tests

- **WebGPU bittrue tests** — upgraded from WebGL2/GLSL to WebGPU/WGSL pipeline. 7 shader-level tests + 7 integration pipeline tests (passthrough, uniform, cascade, generator, builtins, video zero-copy). All run on real GPU in browser mode.
- **WGSL parser** — 21 tests covering AST path, regex fallback, error handling, port ID preservation, type mapping, comment extraction, `@doraemon` regression.
- **Predefined shaders** — exact input/output spec for all 28 shaders + 2 custom templates.
- **959 unit tests + 56 shader/pipeline tests**, all passing.

## [0.15.0b] -- 2026-07-23

### Features

- **Full WebGPU pipeline (Phases 2–4)** — replaced the dual WebGL+WebGPU architecture with a single-GPUDevice zero-copy datapath. Pure WebGPU 2D shader rendering layer, WGSL shader compiler, all 34 shader presets migrated to WGSL, ORT inference sharing the GPUDevice with `preferredOutputLocation: 'gpu-buffer'`.
- **WGSL shader parser + compiler** — new `wgslParser` extracts uniform/sampler bindings from user WGSL; `wgslCompiler` injects system preamble (bindings, fullscreen vertex shader) and compiles to `GPURenderPipeline`. CodeMirror switched to WGSL syntax highlighting.
- **Video texture upload** — `WebGPUBackend.uploadVideoFrame` uploads `HTMLVideoElement` frames to GPU textures per-frame via `copyExternalImageToTexture`, with zero-allocation texture reuse.
- **ONNX nodes wired into render pipeline** — `WebGPUExecutionEngine.runOnnxInference` was dead code (TODO); now fully implemented for all 6 tasks (super-resolution, background-removal, depth-estimation, detection, segmentation, generic). Async per-frame inference with result caching; video inputs trigger per-frame re-inference.
- **GPU I/O binding** — `OnnxInferenceSession.loadFromBuffer(buffer, gpuDevice)` shares the render pipeline's GPUDevice with ORT's WebGPU EP. `preferredOutputLocation: 'gpu-buffer'` keeps ORT outputs on GPU. Compute shader converts planar float32 `[1,C,H,W]` GPUBuffer → `rgba8unorm` texture without CPU readback.

### Fixes

- **`isUpstreamVideo` key/value bug** — `for (const [sourceId] of bindings.entries())` destructured the Map key (uniform name) instead of value (node id), causing video upstream detection to always fail. ONNX cached the first frame's black result and never re-inferred. Fixed to iterate `.values()`.
- **`drawDetectionOverlay` argument order** — was called as `(detections, width, height, classes)` but signature expects `(sourceCanvas, width, height, detections)`.
- **Detection field mapping** — `Detection.classId` → `OnnxDetection.class_id` + `class_name` mapping was missing.

### Tests

- **Integration tests for ONNX data flow** — `executionEngineOnnx.test.ts` (fake GPU backend + mock ORT): verifies upstream texture RGBA is read and passed to inference; verifies video inputs trigger per-frame re-inference (no black cache). `detectionOverlayIntegration.test.ts`: field mapping and overlay call contracts.
- **YOLOv8n functional test** — real model download + inference + `detectPostprocess` decode validation. 3 tests added to `tests/functional/onnx.test.ts`.
- **884 unit tests**, all passing.

## [0.14.0b] -- 2026-07-22

### Architecture Refactoring

- **Store slicing** — split the 756-line `useGraphStore` monolith into 4 Zustand slices (`graphSlice`, `transportSlice`, `projectSlice`, `uiSlice`) plus a shared `helpers` module. External API unchanged — all existing `useGraphStore` selectors continue working.
- **Catalog extraction** — moved shader presets, ONNX catalog/registry, and math ops from `engine/` to `catalog/`. Components and store no longer depend on engine for static data. 30 import paths updated.
- **Executor extraction** — extracted shader, math, and input execution logic from the 1,266-line `ExecutionEngine` into dedicated modules under `engine/executors/` (ShaderExecutor, MathExecutor, InputExecutor). Engine delegates to executors; public API unchanged.
- **Service layer** — introduced `PipelineService` (`services/PipelineService.ts`) as the sole bridge between store and engine. Eliminated the engine→store backflow (`useGraphStore` import removed from `executionEngine.ts`). ONNX backend detection now uses a callback chain instead of direct store access.
- **App.tsx simplified** — 107 → 33 lines. No direct engine or store imports; just mounts `PipelineService` on the hidden canvas.

### Tests

- **79 regression tests** for architecture safety: pipeline integration (23), executor contracts (25), lifecycle/bridge (31). Total: 1124 tests across 44 files.

### Docs

- **DESIGN.md §10 — Software Architecture** — new section covering layered architecture, dependency rules, reactive pipeline design, store slicing, service layer, and 4-PR implementation roadmap.

## [0.13.0b] -- 2026-07-21

### Features

- **NodeShell base component** — all 5 node types (Shader, Math, ONNX, Input, Renderer) now share a single `NodeShell` wrapper for header, caption, status LED, and border styling. Header layout: SVG icon (matching toolbar) + type name (UPPERCASE) + instance label (lowercase) + status LED (green/gray/red).
- **Prebuilt shader code sharing** — predefined catalog shaders (Resample, Blur, etc.) store a `shaderTemplateId` reference instead of per-instance code. All instances share the same source; project JSON omits shader code for prebuilt nodes. Side panel shows read-only shader editor with gray background for prebuilt shaders.
- **Instance labels** — each node gets a unique auto-generated instance name (`resample_1`, `add_2`) separate from the type/template name. Users can rename instances in the side panel. Multiple instances of the same shader type are now distinguishable.
- **Per-node preview readback** — side panel preview reads back only the selected node's FBO each frame via `readNodeOutput()`. No readback when no node is selected (zero GPU overhead). Works for both static and dynamic pipelines.
- **Project file format v0.4.0** — new fields: `shaderTemplateId`, `templateName`. Prebuilt shader code stripped on save, restored from catalog on load.

### Fixes

- **Static pipeline async texture race** — image input textures load asynchronously (`loadImageTexture` returns Promise), but static pipelines rendered immediately before the texture was ready, producing black output. Now awaits all pending texture loads before the first render frame.
- **Gray-Scott reaction-diffusion tuning** — corrected Laplacian kernel and diffusion coefficient scaling.
- **Feedback buffer reset on drag** — dragging nodes during playback no longer resets ping-pong feedback buffers; plan rebuilds only trigger on topology/data changes, not position changes.
- **Builtin uniforms in ports** — `iTime`, `iMouse`, etc. no longer appear as editable input ports on node cards.
- **Field Color Map category** — moved from FEEDBACK to COLOR category where it belongs.
- **Node graph interaction** — fixed edge detach, reconnect, and delete behaviors.

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
