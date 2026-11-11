# Changelog

## Unreleased

### Seven-model production and native-window E2E regression

- Add `npm run test:desktop:inference`: SHA-pinned original model preparation from the Rust catalog, real RenderBridge/model-worker/NativePlayer image and hardware-video execution with ORT unavailable, a separate independent CPU oracle, then all seven models through native Slint menus, typed connections, Load/Play/Pause/Resume/Unload/Reload and Save/reopen. Missing assets or hardware fail instead of silently skipping; actual renderer pixels and diagnostic artifacts remain in per-case reports.
- Commit public-domain NASA portrait fixtures and a deterministic H.264 two-frame-group clip with source attribution and hashes. Preserve production artifacts separately from opt-in same-request GPU source/input/raw/logit snapshots, and keep performance sampling in a separate scheduled/manual lane without UI FPS gates.
- Adopt explicitly approved numerically stable reference semantics: every diagnostic InstanceNormalization uses centered FP64 population/affine arithmetic before FP32 export; production graphs/weights stay unchanged. Semantic labels must equal their own same-run logits ArgMax, all logits retain the existing FP32 tolerance, and only overlapping predeclared error intervals may explain cross-backend near-tie label differences. Exact-ID equality, differences and accepted-boundary evidence remain separately reported; no blanket pixel exemption.
- Correct CPU YOLO overlay conversion from normalized coordinates to source pixels and align GPU inclusive/clamped two-pixel strokes; preserve normalized detection data. Add real low-variance normalization, mixed typed-output lifetime, high-margin/wrong-ArgMax rejection and bounding-box location regressions.
- Verify locally on Apple M4 Pro: all seven production models and all 37 same-frame stable-reference cases passed; native-window suite 15/15 (preflight plus seven image/video pairs), core 211, inference backend 37 (1 ignored), desktop 79 (2 opt-in ignored), and report/reference JavaScript 13 passed. Separate serial seven-model Release performance task completed with 3 warmups and 30 samples/path. YOLO26 preserves four reported near-tie class differences across three fixtures; semantic acceptance is not exact-ID equality. Native reports: `test-results/inference-e2e/production.json`, `ui-2026-11-10T13-47-52-135Z/report.html`, `performance/summary.json`. Hosted CI execution has not been observed locally.

### Video → YOLO → Renderer output fix

- Select an inference image port by its sampler type for preview subscriptions instead of assuming the first output is an image. YOLOv8n declares ROI detections before its overlay; subscribing that ROI as Preview rejected graph commit and left the model-loading plan unset.
- Accept both host JSON and resident device-buffer Resource payloads on ROI ports, while keeping Preview restricted to texture ports and Mesh/Json resource rejection unchanged. Fix the first-frame payload-contract failure without CPU readback or fake detection JSON.
- Verify the original 1920×1080 VideoToolbox video through YOLOv8n and Renderer: live image and detection boxes, paused output retention, five accepted core completions and zero video CPU-copy bytes. Core tests 210 passed; desktop tests 79 passed, 2 opt-in ignored. UI screenshots: `test-results/yolo-video-renderer-fixed*.png`.

### Opt-in Metal catalog image inference (2026-11-09)

- Add an independent GPU execution path for the supported image-catalog graphs, correct the ESR raw CPU diagnostic to supply its task-owned source-scaled output shape, and add a reproducible `catalog_benchmark` against the existing ORT CPU pipeline. The six checked models are U²Net-P, MiDaS, MODNet, YOLO26n, YOLOv8n and RealESRGAN-x4; this opt-in path does not replace or widen the production fixed-profile resident SR3 contract.
- Validate on Apple M4 Pro / Metal at 1280×720 with 30 alternating CPU/GPU samples and 2–3 warmups per path: every selected raw FP32 output value met 1e-3 absolute/relative tolerance, every final RGBA image met max error ≤1 byte, and source texture parity reported zero host upload bytes. YOLOv8n's deterministic gradient fixture yielded zero detections; non-empty detection decoding remains unverified.
- Record the per-model CPU/GPU P50 comparison, acceleration, benefits, correctness checks and measurement limits in [Metal catalog 收益与结论](docs/INFERENCE_ARCHITECTURE.md#metal-catalog-收益与结论2026-11-09-实测). Keep single-frame completion-wall latency distinct from UI latency, GPU execution timestamps, cross-device performance and production acceptance.
- Exercise resource lifetime separately from timing for every catalog model: retain three consumer output leases, verify capacity rejects a fourth submit, release/reuse one slot, then destroy the session and wait with a consumer output still leased. RealESRGAN also recompiles at 640×360 and restores 1280×720 with exact 4× output extents. Per-model three-sample diagnostics are in `test-results/output-metrics/catalog-<model>-1280x720-lifecycle.json`; SR3 resource and pool tests remain in the native Metal suite.
### Resident SR3 convolution performance (2026-11-09)

- Replace independent per-pixel scalar convolution with preparation-specialized FP32 32-channel × 32-pixel workgroup tiles, 4×4 per-thread register accumulation and K=32 shared input/weight tiles (8KB workgroup storage). Keep the full original network, NCHW semantics, signed weights, padding, bias/ReLU, channel tail, same Metal queue and zero per-frame host image/Tensor transfers; no FP16 or approximation.
- Add the reproducible `resident_benchmark` example comparing existing ORT CPU readback/preprocess/infer/postprocess/upload with actual resident GPU submission-completion latency. Same model/input/output, 10 warmups and 60 samples/path/size in three alternating blocks; all-byte verification outside timing. Reject unreliable external Metal timestamp diagnostics instead of inventing GPU execution time.
- Observe Apple M4 Pro Release P50 completion latency: resident 4.34/4.42/4.74 ms for 672×672/1920×1080/3840×2160 final outputs, versus CPU 13.92/15.66/22.57 ms (3.20×/3.54×/4.76× faster) and baseline GPU 44.39/44.53/44.83 ms (10.22×/10.07×/9.46× faster). Every output-byte comparison max error 1; resident host image bytes 0. Fixed 224→672 network; not full-resolution inference, pipelined throughput or UI FPS. Raw samples/P95 and command: [inference performance](docs/INFERENCE_ARCHITECTURE.md#sr3-稳态性能验证2026-11-09).
- Verify optimized code: workspace 270, full-native/backend 373 (1 ignored), desktop 79 (2 opt-in ignored), cached-model/reference 2 and release Metal dense-numeric/resource-lifecycle 2 passed. Rebuilt native Slint window displayed optimized shared SR3 output through Play/Pause with a nonexistent ORT path and zero host transfer counters. Full UI/Shader suites, coverage and release packaging were not rerun for this optimization; earlier cutover results below remain baseline evidence.

### Hardware-only resident inference (2026-11-09)

- Replace production host-image inference with the same Slint wgpu30 Metal Device/Queue: GPU texture → BT.601/resize224 → full four-layer FP32 Conv/ReLU → PixelShuffle3/RGB reconstruction → resident RGBA → Slint/downstream Shader. Independent `oq-inference/gpu` compiles original ONNX topology, weights and reshape layout without approximation; GpuImageSession scratch is STORAGE-only, weights upload at initialization only, source pixels never enter CPU.
- Enforce MetalGpu / Metal, GPU placement / Device path, RequireResident + AcceleratorOnly + Forbid CPU. UI exposes only Metal GPU / Auto Hardware. ORT/CPU/CoreML EP host Tensor APIs/examples remain explicit reference/diagnostics, never production fallback. Unsupported models/platforms fail before frame access. Source introspection and complete typed task contracts remain intact.
- Support resident SR3 only, fixed 224→672 network profile. Arbitrary-size sources still resize to 224 and output source dimensions ×3: no window tiling, full-resolution large-image quality or speedup claim. Windows `.scr` inference export/stub execution is explicitly unavailable with CPU forbidden; non-inference Shader/hardware-video export remains.
- Require hardware video graph inputs; remove FFmpeg RGBA software fallback. Unsupported cameras/Linux/Windows without D3D12 refuse. One-time file image/framebuffer CPU decode-ingress and initial upload remain explicit preparation boundaries; capture/reference/test readback is opt-in, never a production step.
- Retain output owners through TextureHandle/GpuOutputHandle/RenderUpdate/Slint current handles, preventing pool reuse while consumers hold leases. Three-slot maximum; saturation coalesces a retry without failure or a fourth GPU submission, preserving published textures/events until released-lease reuse. Completion metrics mean publication-ready, not physical GPU completion; no fabricated GPU timing.
- Observe resident_compare on Apple M4 Pro Metal: GPU-generated 8×8 source texture without COPY_DST, full-network 24×24 output versus explicit ORT CPU reference, max/mean byte error 0/0, host image bytes 0, six dispatches, scratch 26,292,224 B and weights 238,628 B. Wrong-device rejection occurs before submit; pool-three saturation and released-lease reuse were observed. First completion wall 69.494292 ms includes diagnostic copy and initial pipeline compilation, not throughput or GPU time. Durable reproduction: `ORT_DYLIB_PATH=... cargo run --release -p oq-inference --features gpu,ort --example resident_compare -- MODEL.onnx REPORT_DIR`.
- Validate this cutover: workspace 270, full-native/backend 372 (1 ignored), desktop 79 (2 opt-in ignored), real cached-model/CPU-reference tests 2, and production cached-model execution with a nonexistent ORT library path 1 passed. Real Metal numeric/device/three-lease regression passed; native SR3 window displayed GPU 8×8 → 24×24 shared textures with zero host transfer counters. UI 18/18, Shader pixels 34/34, dependency boundaries and macOS portable packaging passed. Both unchanged 100% coverage gates failed: default lines/functions/regions 96.96%/96.07%/97.00%; full-native 95.97%/91.55%/94.84%. Reports and exact scope: [DESIGN validation](docs/DESIGN.md#validation).


### Complete model interfaces and macOS Core ML (2026-11-07 historical)

- Add runtime-independent `inspect_onnx` with source IR/opset/producer/model versions, recursive Tensor/SparseTensor/Sequence/Map/Optional/Opaque types, exact dtype codes, scalar versus unknown rank, symbolic dimensions/denotations, metadata and required/optional/initializer inputs. Inspect external-weight references without loading or copying tensor payloads. Preserve source declarations separately from optimized ORT session metadata.
- Add typed image-task contracts for exact named inputs and selected outputs, preprocessing/resize, decoder and declared/concrete shape constraints. Validate before readback and before output decoding; remove tensor-length-based channel guesses. U²Net-P retains its seven outputs in introspection and explicitly binds primary mask `1959`, verified against the real artifact and direct named invocation.
- Expose complete model/task interface JSON in the native inspector and add independent `inspect_model` and `coreml_compare` examples. Add opt-in runtime profiling to distinguish actual provider execution from registration.
- At this historical stage, add ORT CoreML EP with MLComputeUnits=ALL and explicit CoreML/CPU fallback. SR3 max/mean absolute error versus CPU was 0.000496149/0.0000568557; mixed execution was slower. The then-CPU default and host-image path are superseded by hardware-only production above, not retained as fallback.
- Historical validation: workspace 270, native/backend 362, desktop 78 plus SR graph-worker, real U²Net named-mask parity, UI 18/18, Shader 34/34, boundaries and macOS packaging. Both overall 100% coverage gates failed. These are not resident-cutover results.

### Generic inference and independent runtime (earlier historical stage)

- Rename the user-facing group/menu/node to **Inference / 推理** and organize model choices by task. ONNX remains the current artifact/file format and ONNX Runtime remains the actual backend; native runtime files and `ORT_DYLIB_PATH` keep their real names.
- Add independent `oq-inference` with a usable ORT CPU/Windows DirectML adapter, reusable sessions, named multiple-input/output FP32 invocation, model metadata/hash, checked shapes/names/capacities, request/session identity and real invocation metrics. It has no OpenQuartz/NodeId/revision/UI/wgpu dependency; OpenQuartz retains graph image tasks, pre/postprocessing and lifecycle adaptation.
- Add effective backend/fallback/residency/placement policy. Preserve the CPU-compatible default; report configured providers, fallback reasons, Unknown internal placement and CPU transfer path. Reject unsupported resident/hardware-placement requirements rather than silently weakening them. Direct Core ML bindings, CUDA, device Tensor I/O and hardware residency are not implemented.
- Save canonical `inference` / `inference*` node records while accepting old `onnx` / `onnx*` wire aliases. Preserve old node/port IDs, edges, task spellings and project version; migrate Rust/core/desktop/screen saver callers without legacy Rust class aliases. Rename desktop runtime/inspector sources to `inference_runtime.rs` / `inspector_inference.rs`.
- Keep media upload → completed-result validation by NativePlayer through PlaybackSession → graph advance → ordered GPU work/inference dispatch → presentation. Image inference still uses CPU readback → ORT → GPU upload; completed inference telemetry is distinct from Render FPS. Independent `.scr` keeps its own synchronous loop.
- Verify workspace 257, full-native/backend 342 and independent feature tests; Slint 78 plus the real cached super-resolution render-worker test; native UI 18/18, catalog Shader pixels 34/34, independent contracts/backend dependency boundaries and macOS release packaging. Preserve legacy customPath/params values and canonical precedence through typed save, and mark rejected strict policies invalid in the inspector. Fix unrendered Shader allocations appearing as stopped previews without weakening pixel assertions. Both 100% coverage gates still fail; exact scope and reports are in [DESIGN 验证基线](docs/DESIGN.md#validation).

### Native ownership extraction (2026-11-06 earlier baseline)

- Extract real state and resource owners within `open_quartz`: compiler shader/topology/plan; GraphEvaluator dirty/Math/feedback and GraphInstance identity/generation/state; PlaybackSession containing only graph+clock+OutputHub; NativePlayer as the thin production coordinator; OutputHub contract/registry/presentation/resources/backend/capabilities; RenderDevice GPU executor/readback/Windows presenter; VideoSources graph-source map/lifecycle/upload callback; InferenceTasks sessions/pending/task IDs/channels/workers/accepted completions. Move production calls and owning behavior with the state; splitting impl blocks across files is insufficient. Preserve root types and existing native feature names, change advanced paths without old execution/runtime namespaces or class aliases, and add no placeholder crates.
- Preserve the same Slint render thread/Device/Queue; standalone .scr drives PlaybackSession/evaluation/GpuExecutor directly. NativePlayer selects RenderDevice output; OutputHub supports an independent presentation backend. Preflight candidate graph/clock and scalar batch before commit, share static Arc data, and validate completions before GPU mutation. PreparedNativeGraph uses fresh textures/owned video handles; open new video successfully before clearing candidate textures, clear detached/obsolete-video textures before optional image upload, and preserve old source/pixels on abort. UI request generation differs from accepted graph revision. Close invalidates pending/relinquishes descriptors and outputs; ORT workers retain Arc sessions until completion, and InferenceTasks Drop joins them. Public headless Player driver and graph_mut history limitations remain unchanged.
- Historical results before the generic inference refactor: workspace 234/34 suites, full-native 315/30 suites, Slint 73, UI 18/18, Shader 34/34, editor 10 and native boundaries passed. Independent ONNX-only 247/30 suites and runtime-only 279/28 suites passed. npm run build passed runtime copy/sign/--help and ZIP dist-desktop/OpenQuartz-0.20.0b-macos-arm64.zip. Reports and lifecycle evidence are preserved in [DESIGN 验证基线](docs/DESIGN.md#validation), not reused as current cutover evidence.
- Both overall 100% coverage gates failed: default 134 tests passed with lines/functions/regions 98.53%/97.88%/98.34%; native 201 passed with 97.70%/95.96%/96.38%. Thresholds were not relaxed; RenderDevice/InferenceTasks/VideoSources remain measured, NativePlayer only retains the existing facade exclusion. git diff --check and no-old-alias scan passed; old execution directories are removed. Camera/Windows/Linux/.scr/mobile remain untested.
- Web/Tauri/WASM/bindings/browser stay retired; Slint and independent Windows `.scr` use typed Rust APIs. Backend FFI remains necessary. DESIGN now describes the current architecture and its latest verification baseline; earlier migration evidence remains in the historical entries below and the Slint migration document. App icons/runtime remain in `apps/open_quartz_slint/{icons,runtime}` and the Shader source remains `crates/open_quartz/src/project/shader_catalog.json`.

### Earlier Unreleased work (archival evidence)

The sections below record earlier stages and their then-current products, paths and results. Claims about retained Web/Tauri/WASM or earlier checks are historical, superseded by the native-only retirement above, and do not validate the current product.

### Fixes

- **Core responsibility groups and optional Web bindings** — keep only `lib.rs` at the core source root; group implementations by project/player/resources/execution/inference/media/bindings and migrate advanced paths without aliases. Rust desktop uses typed APIs; retain Web through optional `bindings::wasm` (`web-bindings`, implied by `browser`) and Tauri JSON IPC. Remove eight dead wrappers and the Engine JS class export; preserve native Engine, live exports, typed semantics and independent products. Cleanup checks pass; three overall 100% coverage gates still fail on execution/device gaps. Current evidence and historical consolidation results are separate in [DESIGN 验证基线](docs/DESIGN.md#validation).
- **Single core crate (historical consolidation)** — consolidate domain/schema, execution and SDK implementations and tests into `open_quartz`; remove the old packages and migrate consumers without aliases or forwarding shims. At that stage, preserve Rust root SDK and JS names, keep domain/SDK implementation modules private, and expose advanced host modules with explicit device capabilities. Default builds retain graph/project/WGSL/runtime state without device dependencies. Keep independent media/editor/exporter/stub and application products separate; preserve behavior, with public real Player driver, graph_mut tightening and browser coordinator extraction left as follow-up. The responsibility cleanup above supersedes that stage's module layout and JS export surface.
- **Core consolidation regression (historical evidence)** — prior to the responsibility/binding cleanup, verify default core 230 tests, retained workspace 308 tests plus targeted native-runtime disabled-capability/state/output retention and native-onnx offline model transfer/actual scalar inference regressions, Slint 72 tests, real UI 18/18, TS 704 tests, type checking, default/browser WASM, standalone editor and Tauri app library checks. Independent default/four-feature boundary checks pass after fixing a dependency leak; actual Chrome 157 browser contracts pass 5/5 with pixel assertions preserved. Correct old fixtures to choose valid surface formats (prefer sRGB only when supported) and initialize OffscreenCanvas 2d before transfer. Separate default/full-native 100% coverage gates both fail: lines/functions/regions 99.97%/99.69%/99.92% and 98.54%/98.20%/97.27%; the then-named domain/SDK/FFI/planner remain 100%. No ignore expansion; newly measured Prepared code is 96.55%/100%/97.06% with a provider platform branch gap. Windows/Linux runtime and release packaging were not rerun. These results do not validate the current cleanup.
- **Resource planner consolidation (historical path)** — move resource-intent planning into the then-existing `open_quartz_execution::resources::plan` and remove the independent host API crate; the SDK, facade and Slint use the execution-owned planner. Current path: `open_quartz::resources::plan`.
- **Remove Android glue** — delete the unused Java SDK, JNI/C ABI handle-table crate and tests, Android launcher assets and Tauri Android entry/packaging configuration. Remove Java-source proxy parity checks from scripts and CI; retain Rust/WASM/TypeScript contracts, desktop execution and active resource-intent planning. Verify 303 workspace tests, 704 TypeScript tests, type checking, Rust dependency boundaries and 18 real-window desktop interactions on macOS.
- **Unified native execution (historical paths)** — move GPU/video/ONNX coordination into the then-existing `open_quartz_execution::runtime::ExecutionRuntime` and model I/O into `execution::resources::models`. Remove the separate native host crate and PlayerHost forwarding layer; Slint/Tauri used the execution runtime directly, while SDK/WASM/screen saver retained shared Runtime rules and device adapters. These paths and products describe that stage only; current ownership and native paths are in DESIGN 模块与所有权 and 源码索引, model services remain `open_quartz::resources::models`.
- **Execution cutover regression** — 305 workspace tests, 72 Slint tests, 18 real-window interaction cases and 34 catalog Shader pixel cases pass on macOS; dependency boundaries, WASM compilation and native release packaging/startup checks pass. Verify real ORT CPU inference and VideoToolbox hardware video through the unified runtime. Read shader-test node metadata from one UI tree snapshot to avoid querying handles destroyed by asynchronous image-row replacement; retain all pixel assertions.
- **Native path cleanup** — remove the unused external-frame media scaffold, unimplemented lifecycle facades, `FramePacer`, and uncompiled Tauri video decoder copy. Route native graph updates through typed Graph APIs while preserving JSON host/WASM boundaries; share one resource-intent plan between Slint video and ONNX synchronization, and use one production/test ONNX task dispatcher. Compile D3D12-only metadata and helpers only where consumed; retain platform decoding and async generation/ticket protections.
- **Filter input sizing and playback indicators** — Auto size follows the first connected sampler in declared port order and actual uploaded Image/Video dimensions through filter cascades; explicit sizes remain fixed, and builtin `iResolution` uses the allocated target. Restore toolbar composition time from the native runtime and show graph Render FPS on Renderer cards/preview/fullscreen, with pause/resume/stop handling. Add real GPU source-replacement/manual-cascade regressions and a real-window Auto→manual→Auto, clock and FPS acceptance case; the user confirms the UI behavior.
- **Output-size/metrics UI regression** — add `auto-size-playback-time-renderer-fps` to the normal desktop UI suite: real non-square PNG input plus unrelated-size source, Filter/Renderer Auto→manual→Auto dimensions, actual blue preview pixels, advancing/frozen/resumed/reset composition time, and positive/zero render FPS transitions. Full macOS real-window suite passes 18/18; report status totals follow the case catalog rather than a fixed count.
- **Real MP4 selection and standalone Video playback** — preserve BT.470BG/PAL primaries from decoded H.264 frames, including files whose MP4 container claims BT.709, and convert linear RGB to BT.709 before sRGB GPU output. Include primaries in the conversion-pipeline cache key. Recognize terminal external `sampler2D` inputs as texture outputs so Play without a Renderer no longer fails and clears previews; explicit Renderer precedence remains unchanged. Verify the user's original 1080p MP4 through `npm run dev` and native Choose File, thumbnail/Inspector updates and Play/Pause/Resume; the user confirms the fix. Add real GPU color/conflicting-tag/cache-isolation and source-only advancing-frame regressions.
- **VideoToolbox NV12 throughput** — request Metal-compatible macOS decoder surfaces instead of FFmpeg's unconditional OpenGL-compatible surfaces. Rebuild the pinned SDK as `oq-vt-hw2-metal`, invalidate the old cache and reject old runtime builds; preserve required hardware decoding and the iOS OpenGLES branch. Document 8K experiments in `crates/oq-media/README.md`: formal-SDK full-file decode reaches 135.14 FPS for the tested HEVC8 source versus 53.69 FPS before, while HEVC10 remains comparable at 135.29 FPS. Verify GPU colors for H.264/HEVC NV12 and HEVC P010; these decode numbers are not screen-present FPS.
- **Native port dragging** — connect ports by dragging from their full visible circles, in either direction, with a live wire preview and zoom-aware release hit testing. Keep click-to-connect, SDK type/cycle validation, occupied-input replacement and single-step Undo; blank/same-direction drops do not mutate the graph. Real-window regressions now start at circle centers rather than row labels.
- **Stopped video thumbnails** — decode and upload one real source frame after Choose File while stopped or paused, then stop polling. Update node and Inspector previews for replacement files, including identical dimensions, without starting graph playback or advancing the video's start position. Cover source replacement/races with GPU pixel assertions and verify red/white/gray file changes through the macOS system chooser.
- **Slint node dragging** — replace per-mark grid rectangles with a repeated image, reuse shared GPU preview images by texture identity, and publish only changed node rows. Handle Slint 1.18's non-reflexive GPU/empty `Image` equality explicitly. Optimize dependencies in the dev profile while preserving application debugging; native Metal drag probes reduced UI-thread frame encoding from about 33ms to 4–5ms, with typical 16.6ms frame intervals. Dynamic pixels and single-step drag Undo remain verified; 68 application tests pass.
- **Inspector pointer hit testing** — position the sidebar resize handle at its left edge instead of leaving an invisible drag strip across the middle of Inspector controls, restoring ordinary pointer clicks on centered Apply actions.
- **First Play after stopped graph edits** — preserve pending dirty nodes across graph replacements and propagate them through the new connections. A static Shader edited before connecting a Renderer now executes on the first Play instead of exposing an uninitialized transparent texture; engine command-order and real GPU blue-pixel regressions cover the sequence.
- **Catalog feedback initialization** — inject referenced implicit system uniforms into native execution plans and editor validation, and expose zero-based `iFrame` to shaders while keeping runtime/System Frame counters one-based. The original Gray-Scott catalog source now seeds on the first real frame and evolves through the feedback buffers.
- **Empty startup and stable node placement** — start the native app with an empty Untitled graph and stopped playback; New/Open reset transport and graph edits preserve the explicit Play state. Distinguish a prepared graph from a stopped player so the first Play retains its pending work. Preserve the manual world-to-screen transform during projection updates, fixing single-node drops snapping back to center while keeping Fit explicit. Native pixel-position smoke and all 70 application tests pass.
- **Slint ONNX state projection** — keep graph model identities stable across SDK project replacement and publish terminal load failures and unloads to both the inspector and Renderer status. Late model completions remain ticket-scoped and cannot attach to a replaced or cancelled node.
- **Independent screen saver runtime** — embed shared FFmpeg codec and ORT VC++ DLLs; delay-load FFmpeg imports and statically link the MSVC startup runtime so an exported `.scr` can extract its dependencies before first use. Bake raw framebuffer sources into portable PNG assets without modifying the editor project.
- **Native file-dialog reentrancy** — release the Slint Controller's `RefCell` borrow before entering AppKit's modal loop, and keep project edits busy-guarded until the panel returns. Apply the same boundary to Video/Image/Framebuffer selection, PNG capture and Windows screen saver export; nested render timers no longer abort on an already-borrowed Controller.
- **Screen saver target isolation** — build the static-CRT stub with explicit `--target x86_64-pc-windows-msvc --locked`, keeping target flags off host build scripts/procedural macros and copying the executable from the target-specific directory.
- **LaunchServices startup verification** — explicitly package Slint as a regular foreground macOS application (`LSUIElement=false`, `LSBackgroundOnly=false`, minimum macOS 13). After rebuilding, `open -n -F` cold launch and a separate cold launch with `--args --project …` both produced the native 1180×792 window and rendered 96×48 bundled-video frames. Code signing verifies. The earlier winit abort has no recoverable panic payload and was not reproduced in these runs; its cause remains unconfirmed, so metadata is not claimed as a proven crash fix.

### Tests

- **Repeatable Slint UI automation** — add an optional MCP build, a Node runner for real-window startup/graph editing/transport/persistence regressions, screenshot pixel assertions, and retained failure diagnostics. Add `test:desktop:ui` and `dev:desktop:ui` commands and Linux/macOS CI artifact collection.
- **All Shader image E2E coverage** — add real PNG fixtures, explicit non-degenerate parameters, independent pixel references and a native-window case for every catalog Shader. Retain input/reference/output images, phase-level results and errors in the HTML report; run through `test:desktop:shaders` locally and in Linux/macOS CI. Local macOS/Metal acceptance passes all 34 shaders and 17 interaction cases. Model physical preview interpolation, RGBA8 feedback quantization and platform-dependent noise statistics explicitly; retain wrong-input, wrong-frequency and shuffled-noise rejection tests.
- **Slint ONNX acceptance** — 52 Slint tests pass; release build succeeds. Native AppKit/Metal acceptance covers a real local `image_identity.onnx` session with `2×2` CPU output and shared texture preview, saved-project separation, custom catalog cancellation through a blocked FIFO read, late-completion invalidation, unload cleanup, SDK Open/Save, and the 800×540 narrow-window layout.
- **Desktop integration acceptance** — 62 Slint tests, 292 workspace test results and 704 Web tests pass; independent Web production build and macOS release packaging succeed. A temporary production Controller/NativeHost/FemtoVG-wgpu harness uses the packaged FFmpeg/ORT libraries to verify advancing 96×48 video frames, exact blue/red Renderer PNGs, selection while paused, native fullscreen and Escape, Stop/Play, SDK Save/Open, ONNX cancel/load/inference/unload, real UI pixel snapshots and clean exit. Native dialog acceptance is recorded below; real camera permission/capture and Windows/Linux runtime acceptance remain unverified.
- **Unlocked native-dialog acceptance** — reproduce the Video chooser's timer-reentry abort, then verify the rebuilt macOS package with real system dialogs: Video cancel and successful 80×40 replacement, Image loading, raw 2×1 red/green framebuffer loading, and PNG save/cancel. The saved PNG is 80×40; 62 Slint tests and default release packaging pass again. The user separately confirms the fullscreen button responds correctly. Windows cross-check reaches bindgen with checksum-verified FFmpeg headers, but stops at missing Windows SDK `errno.h`; it is not a passing Windows build.
- **Phase 7 macOS cutover acceptance** — verify the default native ZIP after extraction outside the workspace: no ORT/FFmpeg environment overrides, real Metal 96×48 video, FFmpeg resolved inside the moved bundle, signature/CLI checks and ordinary exit code 0. `npm run package:tauri -- --bundles app` also passes through the independent Web/WASM build; correct optional artifact upload to the root workspace's `target/release/bundle/`, ignore generated `dist-desktop/`, and match macOS Intel/ARM CI coverage to release configuration. Phase 7 is complete for the agreed macOS scope; actual camera/Windows/Linux/mobile validation is deferred and remote CI/publication are not claimed.

### Features

- **macOS VideoToolbox media core** — add framework-independent `oq-media` with retained hardware-frame leases, PTS/epochs, reset, cancellation and drain; route macOS H.264/HEVC files through in-process libav and CoreVideo/Metal planes into the existing shared wgpu graph. `oq` denotes OpenQuartz origin, not a dependency; the crate lives under `crates/oq-media` and is imported as `oq_media`. Bundle a checksum-pinned LGPL-only FFmpeg 8.1.1 SDK, corresponding source and hardware-required patch. Verify NV12/P010 GPU color/range, unsupported HDR/linear transfer rejection, real Slint transport/Renderer output and relocated native-package playback on Apple Silicon. Preserve explicit software fallback for backend failures, reject unsupported presentation semantics, and retain existing Windows/Linux/camera paths; encoding, reusable player UI, cross-platform runtime and performance acceptance remain unimplemented or unverified.
- **Reusable native code editor** — develop `crates/slint_code_editor` in-repo as an independent Slint/cosmic-text component and migrate OpenQuartz to it. Add WGSL/JSON/plain lexical styling, themes/custom providers, shaped line-number and pointer geometry, selection/clipboard, local undo/redo, find/replace/goto and desktop IME event adaptation without a Slint fork or WebView. Add an independent two-instance demo and CI tests; preserve explicit Shader Apply and read-only catalog sources. Real-window input/undo and multi-color pixel assertions cover dynamic component creation. Full screen-reader text geometry and real IME/Windows/Linux acceptance remain outstanding; no public release is claimed.

- **Native Slint editor restoration** — restore the 7-category/34-entry Shader menu, real shared-GPU thumbnails and resolution labels on eligible node cards, collapsible previews, and per-node runtime status. GPU failures preserve the node ID, mark only the affected LED, clear stale textures, and no longer blanket-fail unrelated ONNX nodes. Replace the old plain editor with native multiline `TextInput`, UTF-8-safe line/goto/find/next navigation, live match counts, independent scrolling, read-only presets, and fixed Apply; syntax highlighting remains unsupported by Slint 1.18. AppKit dialogs and a real shared-GPU error/recovery smoke are verified.
- **Shared Shader catalog** — move Web and Slint menus onto one `catalog.json` source while preserving category, order, source, and template IDs; native factory creation covers all seven categories and 34 presets.
- **Default native desktop** — switch `npm run dev/build` to Slint with host-architecture Windows/macOS/Linux portable packaging, bundled native runtimes and three-platform CI. Web remains independently buildable; Tauri remains an explicit one-version fallback. Mobile Phase 6 is deferred. Packages run the staged executable's `--help` before archiving; macOS is ad-hoc signed, not notarized.
- **Desktop media and output** — acquire video files and enumerate/select cameras through native services; reconcile playback settings in NativeHost, select the displayed Renderer, open the same shared texture in fullscreen with Escape return, request PNG readback explicitly, and export Windows screen savers on the project I/O worker.

- **Slint Phase 2 shell** — extract the framework-neutral `open_quartz_native_host` from the Tauri runtime and add an independent Slint 1.18.1 application shell with SDK-owned project I/O, graph projection, and shared wgpu 30 renderer texture import. Tauri remains the fallback application; the full editor and mobile delivery are not migrated yet.

- **Slint spatial GraphView** — add a scrollable native graph canvas with persisted node coordinates, Bezier edges, single selection, release-only SDK position commits, negative-coordinate reachability, busy-I/O edit guards, and deterministic 100/300-node stress fixtures.

- **Slint connection editing** — project actual typed ports and per-port edge endpoints; add click-to-connect, occupied-input replacement, atomic endpoint reconnection, edge selection/deletion, Cancel, and SDK-backed Undo/Redo. Failed type/cycle edits preserve graph and history, topology changes refresh NativeHost, and layout-only undo leaves rendering running.

- **Slint graph viewport** — add center-anchored 1%–200% manual zoom, 100% reset and resize-aware Fit View over tight graph/Bezier bounds. Scale ports, edge markers and screen-to-world drag conversion consistently, hide labels in low-zoom overviews, and reset session-local view state when opening a project without recording graph edits.

- **Slint graph clipboard** — add Shift multi-selection, Select all and graph-scoped Command/Ctrl+A/C/V. Copy owned node/internal-edge snapshots; paste with new node/port IDs and incremental offsets in one SDK revision/Undo entry, preserving node data and cross-project clipboard state while leaving text-field shortcuts untouched.

- **Slint graph MiniMap** — overlay a bounded all-node overview with a live viewport rectangle; click or drag to center the scaled ScrollView while preserving graph focus and Project state across zoom, Fit View, negative coordinates, resize, and empty graphs.

- **Slint Inspector and WGSL editing** — inspect a single node's identity and ports; edit names, Shader/Constant output sizes, and Shader WGSL through typed SDK commands. Independent Apply/Discard drafts guard graph/project actions, preserve failed edits, and integrate SDK history, Save/Open, parse-error display, and NativeHost refresh. Shader Apply remains reachable in narrow windows; Tauri is unchanged.

- **Slint Shader uniforms** — add independent typed drafts for Float/Int/Uint/Bool, numeric vectors, and Mat2–4 inputs on Shader nodes only. Per-row validation enforces exact arity, finite float and i32/u32 ranges, and bool syntax. Connected rows are read-only and excluded from dirty/mutation checks; unsupported/resource ports are not uniform rows. Apply preserves unrelated drafts, stale uniform keys, and complete SDK node data through `UpdateNodeData`; missing zeros remain no-ops, and matrix text follows WGSL column-major order.

- **Slint Sampling/Framebuffer/Video forms** — add independent SDK-backed configuration drafts. Sampling supports source-node Linear/Nearest and Clamp/Repeat/Mirror and is consumed by NativeHost for generated and uploaded textures. Framebuffer validates seven formats, 1–8192 dimensions, and optional u32 stride; Video validates File/Camera, camera ID, Loop, and 0.1–4.0 rate. Image and framebuffer resource loading are implemented separately; video/camera resource acquisition remains out of scope. Complete NodeData and existing resource paths are preserved.
- **Slint image and framebuffer resources** — add native file selection for Image and raw Framebuffer inputs; decode and validate assets on the file worker, upload decoded RGBA textures through the shared NativeHost bridge, embed source data in SDK projects, and rehydrate resources asynchronously after project Open. Framebuffer loads use the applied format, dimensions, and optional row stride.

- **Slint ONNX metadata** — inspect recorded model/source/path, connection counts, thresholds and parameters separately from saved status/backend/error/download metadata. Preserve missing fields and zero values without treating saved `ready` as a loaded session or mutating the project.

- **Slint native ONNX sessions** — explicitly load a saved/custom local model or catalog download through existing Rust resource-intent policy; show live progress, session backend/tensor names, real texture/structured results, errors, cancellation and unload. Keep resource state outside SDK history/project metadata and invalidate late work across unload, configuration changes and project replacement. Tauri and Slint share a streaming, atomic native model cache with optional digest checks and off-thread session construction.

### Fixes

- **Renderer source invalidation** — discard cached Renderer mappings when their actual upstream changes or disconnects. Reconnecting to an unbound ONNX output now reports the missing GPU texture and clears the preview rather than displaying the previous source.
- **ONNX completion lifetime** — identify each in-flight task independently of graph revision. Unload/reload and graph replacement discard old completions without restoring removed textures, publishing stale events, or clearing a replacement task's pending state; unloaded models no longer trigger unnecessary GPU readback.
- **Native ORT shutdown** — upgrade the Rust ONNX binding to `2.0.0-rc.12` with API 20 compatibility so the global environment is released before native C++ destructors. Remove forced-exit/leaked-session coverage workarounds and exercise ordinary process exit after real inference.
- **Native ONNX CPU integration smoke** — with the official ONNX Runtime 1.21.0 dylib, load `image_identity.onnx`, verify the actual 2×2 NativeHost→ONNX→Renderer RGBA and CPU output event, and exit normally; offline host/cache tests cover stale work, digest/partial download validation and invalid IDs. Workspace passes 286 tests/46 suites, Slint passes 51 tests, release build succeeds. Final macOS Slint session controls remain unverified because macOS reports the GUI session locked; keep this distinct from the completed headless inference smoke.

### Tests

- **macOS Slint validation** — verify the Phase 2/3 shell in a release AppKit window on Apple Silicon/Metal with shared texture output, SDK file paths, spatial editing, subgraph Copy/Paste/Undo, zoom/Fit, 300-node Retina rendering, and VoiceOver actions for node headers, ports, edges, and MiniMap; add a macOS CI compile gate.
- **Slint GPU graph baseline** — benchmark the real 1180×760 Slint tree offscreen through FemtoVG/wgpu 30 on Apple M4 Pro: release steady full-frame p95 is 2.523ms for 100 nodes/150 edges and 3.338ms for 300 nodes/450 edges. Native release AppKit runs stabilize at 60 FPS for both sizes (first intervals: 59 and 51 FPS respectively).
- **Inspector validation** — add field-isolation/history/persistence and connected-port rejection regressions (32 Slint tests passing); exercise actual text/pointer UI paths offscreen at 1180×760 and 800×540, and verify controller-applied 96×48 NativeHost output plus red RGBA pixels on Apple M4 Pro/Metal. Native AppKit AX validation covers name/source Apply, source Undo/Redo, rejected connected-port removal, Discard, 641×360 output, and Save/Open. Fix imported renderer image preferred height starving the Inspector. Complete native pointer/keyboard and 800×540 client-area scrolling acceptance: HID node selection, editor focus, Command+A/text replacement, pointer Apply changing real Metal output from red to blue, independent-draft protection, and Discard preserving committed source.
- **Uniform Inspector validation** — 41 Slint tests and 276 workspace tests / 45 suites pass; release build succeeds. Cover parsing/bounds, invalid-draft atomicity, connected exclusion, missing-zero no-ops, stale keys, independent drafts, schema refresh, single-step history, persistence, and legacy values. Native Metal checks at 1180×760 and 800×540 exercise invalid input, connected read-only rows, scalar/vector Apply with red→blue 96×48 output, Undo/Redo, a Mat2 uniform, narrow-window scrolling, consecutive keyboard input with preserved caret, pointer Apply, and SDK Save/Open.
- **Node-form validation** — 47 Slint tests and 278 workspace tests / 45 suites pass; release build and formatting checks succeed. Cover enum/range/u32 boundaries, invalid/busy atomicity, independent drafts, complete-data/resource preservation, one-step history, persistence, and real-GPU Linear/Nearest plus Clamp/Repeat/Mirror sampling. Native AppKit checks at 1180×760 and 800×540 exercise blue→red Repeat Apply with Undo/Redo and ComboBox synchronization, invalid/repaired Framebuffer and Video fields, narrow-window widgets, and SDK Save/Open.
- **ONNX and editor validation** — 49 Slint tests and 280 workspace tests / 45 suites pass; release build and formatting checks succeed. Native AppKit checks cover saved/live-state separation, catalog/custom/empty nodes, invalid progress, Unicode path wrapping, selection clearing, SDK Save/Open, missing-output errors and recovery at 1180×760 and 800×540. Evaluate a 300-comment-line WGSL source with scrolling, consecutive Unicode keyboard input, Apply and SDK Save; retain native TextEdit without claiming CodeMirror parity, IME composition, or measured editing latency.

- **Complete Rust SDK 100% coverage gate** — enforce 100% production line, function, and region coverage independently for `open_quartz_schema`, `open_quartz_execution`, `open_quartz_host_api`, `open_quartz_sdk`, the `open_quartz` native/WASM facade, and `open_quartz_bindings`; coverage tests now exercise graph transactions, GPU resource/readback failures, native video lifecycle and FFmpeg failures, ONNX preprocessing/inference/postprocessing, host intents, FFI marshalling, and JNI handle lifetimes.
- **Cross-platform SDK matrix** — run the Rust coverage gate on Linux, macOS, and Windows, include Windows DXGI/D3D12 tests in the owning execution crate, run WASM browser contracts separately with headless WebGPU configuration, and compile-check Windows, Linux, WASM, and JNI targets.

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

- **DESIGN.md §10 — Software Architecture (historical)** — new section covering layered architecture, dependency rules, reactive pipeline design, store slicing, service layer, and 4-PR implementation roadmap.

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
