# OpenQuartz Native Desktop (Slint)

This is the native Rust Slint desktop application. Windows screen saver export
produces `.scr` files with an independent stub. Web and Tauri are retired; there
is no browser fallback. Windows/Linux runtime and screen saver acceptance, live
camera capture, and Android/iOS delivery remain unverified or deferred. An earlier
intermittent winit startup abort remains a known risk.

The application is a standalone Rust workspace, with Slint and its build compiler
pinned to **1.18.1**, using the femtovg wgpu renderer and wgpu 30 shared-device integration.

The application consumes the single `open_quartz` core through root typed APIs
and explicit advanced modules. `src/lib.rs` remains the only source root file,
and public `Project`, `Graph`, `Player`, `Resource`, `Output` and other root types
are preserved. The concrete owners assign shader/topology/plan to `compiler`,
pure dirty/Math/feedback to `evaluation::GraphEvaluator`, and applied graph
identity/revision/generation/state to `evaluation::GraphInstance`.

Slint's production coordinator is `open_quartz::player::NativePlayer` on the
existing render worker. It owns the realtime CompositionClock and a shared
`execution::FramePipeline`, not a separate PlaybackSession execution loop.
`FrameExecutionSession` contains GraphInstance, OutputHub and the last evaluation
stamp; `PlaybackSession` combines that same clock-free graph/output owner with a
composition clock. The pipeline owns the session, RenderDevice, VideoSources,
InferenceTasks, bounded stable-frame pool and admitted-request state.
`outputs::OutputHub` owns contracts, subscriptions/registry, presentation planning,
resources/backend/capabilities. `rendering::RenderDevice` owns GPU execution/readback/
Windows presentation and uses the **same wgpu Device/Queue instance** as Slint.
The internal `media::sources::VideoSources` owns graph-source map/lifecycle/upload
callbacks; `inference::tasks::InferenceTasks` owns resident graph-task identity,
session and output adaptation. Independent `oq-inference/gpu`
owns the real ONNX GPU compiler and GpuImageSession, with no OpenQuartz/NodeId/UI
dependency; wgpu is optional under its gpu feature. ORT named FP32 Tensor APIs
remain explicit reference/diagnostics only, never a production fallback. Project/history/catalog remain in
`project`; resource intents and background preparation remain in `resources::{plan,models}`.

`inspect_onnx(model)` inspects source declarations without ORT; session `source_interface` preserves IR/producer/model versions, domain, graph name, model/graph/value descriptions/custom properties and graph opset imports. Opsets are source-parsed, not universally unavailable under ORT API 21. Recursive Tensor/SparseTensor/Sequence/Map/Optional/Opaque/Unknown types retain exact dtype codes, scalar vs UnknownRank, dimension symbols/denotations, and Required/Optional/OverridableInitializer inputs. Dense/sparse initializer and external-data references are described without copying/loading weights. Runtime ValueKind/tensor taxonomy/metadata and artifact hash/size/format are separate; runtime version sentinels are not source versions, and producer_version is source-filled.

`ImageInterfaceReport` reports structural compatibility, not task semantics. Typed `ImageTaskContract` records exact named inputs/selected outputs, NCHW layout, decoder, preprocessing/resize/color and declared/concrete shape constraints. Declarations are checked before frame access and concrete outputs before decoding/delivery. Detection `[1,84,N]` is structurally valid, while the COCO decoder requires `[1,84,8400]`. JSON separates `model` and `taskContract`; retained contracts do not expand resident backend support.

Image-task contracts retain exact output bindings: U²Net-P keeps all seven outputs `1959`–`1965` and binds mask `1959`; unknown multi-output models require a declared binding. The Metal backend executes the seven catalog profiles through the shared production coordinator; custom models/other platforms reject before frame access. `npm run test:desktop:inference` exercises all seven through production worker, independent CPU reference and native-window UI. See [the regression contract](../../docs/DEVELOPMENT.md#七模型推理-e2e-回归) for fixed artifacts, hardware prerequisites and pass/fail reports.

The SR3 path is GPU texture → GPU BT.601/resize224 → full four-layer FP32 Conv/ReLU → PixelShuffle3/RGB reconstruction → resident RGBA → Slint/downstream Shader, on the **same Metal wgpu30 Device/Queue**. Its network is fixed 224→672 with source-size ×3 reconstruction, not tiling or full-resolution large-image quality. The other catalog models use their original native GPU graphs and typed pre/postprocessing; RealESRGAN-x4 uses source dimensions ×4. Source pixels never enter CPU; no per-frame host Tensor, CPU preprocessing, readback or result upload. Model/weights preparation and scalar controls are explicit boundaries; diagnostic readback is opt-in.

Source-only JSON: `cargo run -p oq-inference --example inspect_model -- MODEL.onnx`. Explicit resident/reference diagnostic: `ORT_DYLIB_PATH=... cargo run --release -p oq-inference --features gpu,ort --example resident_compare -- MODEL.onnx REPORT_DIR`. See [architecture and evidence](../../docs/INFERENCE_ARCHITECTURE.md); ORT/CoreML EP examples remain diagnostic only.

Phase 2 uses `FramePipeline::submit(FrameDemand)` with explicit request/revision,
FrameEvaluationInput/FrameStamp, target output, and optional versioned external
StableFrame inputs. Accepted demands progress only through `progress` on the owning
render worker; `poll_completion` drains a terminal result without executing work or
advancing a clock. One pending demand **or undrained terminal** blocks admission;
WouldBlock/Rejected return the unaccepted demand and its leases. NativePlayer commits
its candidate clock only after admission. The clock-free session preflights the
candidate graph and whole scalar output batch; shared Arc graph/plan data avoids
per-frame WGSL deep copies.

The pipeline submits upstream GPU work before resident inference, validates its
completion identity/contracts/capacity before changing textures, and waits for the
same demand's input stamp before running downstream Shader/Renderer commands.
All dependencies must resolve before the bounded pool captures a stable output.
Submitted(StableFrame) means graph work and the detach copy are enqueued; physical
GPU completion is separate in `frame.ready`. Pool saturation retains the demand
until capacity returns. NativePlayer consumes completion and asks RenderDevice to
present; OutputHub retains its independent backend presentation path. Current media
still uses sequential sources and blocking GPU import: precise source-time selection,
prefetch, credits and multiple in-flight frames remain future phases.
PreparedNativeGraph stages images in fresh textures and owns prepared video handles;
UI request generation differs from accepted graph revision.

Close invalidates pending publication and releases descriptors, not live GPU leases.
TextureHandle/GpuOutputHandle/RenderUpdate and Slint's current handles retain the output
owner through consumer use. The output pool has at most three slots: retained leases
prevent reuse; saturation is explicit backpressure, release permits reuse. Same-queue
order/fences preserve producer/consumer safety. Candidate graph failures preserve old
source/pixels; request generation differs from accepted revision. Independent `.scr`
keeps its own loop but inference export/stub execution is unavailable (CPU forbidden);
non-inference Shader/hardware-video export remains available.

The `execution` module now contains this shared production implementation; it does
not restore the former empty execution/runtime facades, class aliases or duplicate
media frame contracts. No extra crate, device or default device dependency is added.
Confirmed files are in the [source index](../../docs/DESIGN.md#source-map).
The default remains device-free; Slint explicitly selects `native-runtime`
(enables gpu only), `native-video`, `native-onnx`, `model-cache`, and Windows
`native-video-d3d12`, retaining the existing feature names and behavior.
oq-media, the editor, exporter and standalone stub remain
independent. NativePlayer is the real native production coordinator; the public
headless `Player` driver is unchanged and `Project::graph_mut` can still bypass
revision/history. See [Design 模块与所有权](../../docs/DESIGN.md#module-ownership).

The shared core now also has a macOS hardware H.264/MP4 `execution::ExportJob` and runnable `export_video` example. This is headless infrastructure, **not a new File → Export Video UI** in this application. It uses the same FramePipeline with GPU NV12 conversion, bounded native input-consumption credits and atomic final-file commit; no audio or CPU image fallback. Commands, SDK source/patch distribution and actual evidence are in the [media README](../../crates/oq-media/README.md#硬件h264mp4文件任务).

### Does the desktop need FFI?

The pure Rust desktop uses the typed API directly; it needs no SDK C ABI, JNI
bridge, or desktop language bridge. Native backend FFI remains necessary for
ONNX Runtime, FFmpeg, AVFoundation/CoreVideo, Metal and D3D12/COM.

Resident-cutover verification on Apple Metal: desktop 79, workspace 270 and
native/backend 372 passed; real cached-model/reference tests 2 passed. Production
SR3 ran with a nonexistent ORT library path and displayed 24×24 shared textures
in the native window. UI 18/18, Shader pixels 34/34, dependency boundaries and
macOS portable packaging passed. Both unchanged 100% coverage gates failed;
[Design validation](../../docs/DESIGN.md#validation) records percentages and reports.
Camera/Windows/Linux/mobile resident capabilities are not established by Apple
Metal results; historical sections below are not new runs.

## Run

From the repository root:

```sh
npm ci
npm run prepare:desktop
npm run dev
npm run dev -- --project /absolute/path/project.quartz.json
npm run build
```

The default Node entry point prepares native FFmpeg and official ONNX Runtime
1.21.0, then builds/launches Rust directly. It does **not** build WASM, start Vite,
or require a browser. Platform C++ build tools and Rust 1.92+ are required; see the
[root README](../../README.md#getting-started--native-desktop) for Linux dependencies
and Windows LLVM/VC++ redistributable requirements.

Release output lives in `dist-desktop/`: a macOS `.app` plus ZIP, Windows portable
ZIP, or Linux tarball. Packages include architecture-matched runtime libraries and
notices; missing resources fail the build. macOS bundles have the icon and camera
usage description and are ad-hoc signed, not notarized. Windows packages are unsigned;
extract the whole directory, including `runtime/` and executable-adjacent DLLs.
Windows also includes the independently built screen saver stub.

The app owns its icons and prepared runtime under `apps/open_quartz_slint/icons/`
and `apps/open_quartz_slint/runtime/`. The shader catalog source is
`crates/open_quartz/src/project/shader_catalog.json`. The Windows stub is built
under `target/screensaver-stub/` and staged in the app runtime for packaging.
The macOS bundle uses a numeric version derived from root `package.json`.
`dist-desktop/` is ignored by Git.

Windows FFmpeg uses a dated BtbN FFmpeg 9.0.2 snapshot pinned by SHA-256 (ABI 63).
Upstream snapshot retention is not guaranteed: keep the download cache for repeat/
offline builds. Missing downloads or mismatched bytes fail explicitly rather than
falling back to a partial package or a system decoder.

Direct Cargo commands remain useful if native runtime paths are configured:

```sh
cargo run --manifest-path apps/open_quartz_slint/Cargo.toml
cargo run --manifest-path apps/open_quartz_slint/Cargo.toml -- --project /absolute/path/project.json
cargo run --manifest-path apps/open_quartz_slint/Cargo.toml -- --project -draft.json
cargo run --manifest-path apps/open_quartz_slint/Cargo.toml -- --graph-stress
cargo run --manifest-path apps/open_quartz_slint/Cargo.toml -- --graph-stress 300
```

Launching without `--project` opens an empty Untitled project and a blank canvas,
with playback stopped. A supplied project is read and validated asynchronously;
a failed open leaves the current project intact and reports the SDK or file error.
Opening or creating a project leaves playback stopped. Adding, connecting or
editing nodes preserves the current transport state; press **Play** explicitly
to start rendering.

`--project` accepts relative, absolute, and dash-prefixed paths. `--graph-stress`
creates a deterministic spatial fixture with 100 nodes by default and accepts an
explicit count from 1 through 300. The two options are mutually exclusive.

Use **FILE → OPEN… / SAVE / SAVE AS…** for native project dialogs. Save writes
the SDK's `Project::to_file()` JSON to the current path; Save As chooses a new
path and the platform dialog handles overwrite confirmation. Save is manual.
Double-click the project title and press Enter to rename it. NEW starts an empty
project; CLEAR removes the graph as one undoable edit. Neither automatically saves.

## Graph and application boundary

- The native Slint layout has a 48px
  icon toolbar with FILE/SOURCE/MATH/SHADER/INFERENCE menus, a full-bleed cross-grid
  canvas, dark-blue 28px node captions, typed ports, real shared-GPU node
  thumbnails with resolution labels and status LEDs, and a 320px resizable
  sidebar. The sidebar appears on selection; semantic sections and texture
  previews collapse. Runtime and project details are available through FILE →
  RUNTIME STATUS…. At 800×540 the toolbar shortens its brand and hides the
  version to preserve actions.
- The native shader editor uses the independent `crates/slint_code_editor`
  component with live WGSL highlighting. Its editing and accessibility limits are
  described under [Inspector and WGSL editing](#inspector-and-wgsl-editing).
- `GraphViewModel` projects SDK graph snapshots into node and edge models and
  computes padded scroll content bounds. Its origin offset keeps loaded nodes with
  negative coordinates reachable while preserving graph-space edge coordinates.
- Node moves are committed once on pointer release through the Rust SDK. Motions
  below the three-pixel drag threshold select instead, and unchanged positions do
  not create a graph revision. Graph interaction is disabled while asynchronous
  project I/O is busy.
- Manual canvas projection preserves the world-to-screen translation when graph
  bounds change. A lone node stays at its drop position instead of recentering;
  explicit **Fit View** still centers the graph. Pointer smoke covers positive
  and negative drags, Undo, zoomed dragging and dragging after Fit.
- Drag between an **output** and an **input** to connect in either direction;
  the wire follows the pointer and commits once on release. The full port circle
  is interactive, including outside the node border. Blank/same-direction drops
  cancel without an edit; Escape cancels the gesture. Click an **output**, then
  an **input** remains available. The SDK replaces any existing input edge atomically.
  Click a **connected input** or a link's midpoint
  marker to select its edge, then use **Delete edge** or **Reconnect**. Reconnect
  keeps the old edge until a valid input is chosen; choosing another output first
  also changes the source. **Cancel** never mutates the project. SDK type/cycle
  errors preserve the original graph and leave the pending source available.
- **Undo/Redo** use SDK graph history for moves, connections, reconnections,
  deletions, and whole-subgraph pastes. An identical connection is a no-op. A successful new edit clears redo;
  a failed edit does not. Opening a project resets the edit session; generated
  fixture construction is not exposed as user undo history. History is not saved.
  Topology edits and their undo/redo refresh NativePlayer's accepted graph; layout-only
  history does not restart rendering.
- **− / +** change zoom by a factor of 1.25, with a manual range of **1%–200%**.
  **100%** resets scale while preserving the viewport center where bounds allow.
  Scrollbars/trackpad scrolling navigate the scaled canvas. **Fit View** centers
  the entire graph (including backwards Bezier control bounds) without enlarging
  past 100%; while active it adapts to window and graph bounds changes. Manual zoom
  exits Fit mode. Empty graphs disable Fit View. Opening a project resets to 100%
  and clears scroll offsets. View state is session-local: it neither changes graph
  revisions/history nor gets saved to the project.
- Nodes, ports, edge markers and drag previews use the same scale. Drag deltas are
  converted back to graph coordinates; the 3px threshold remains in screen pixels.
  Below 25%, labels are omitted for an overview; geometry and hit areas still scale.
  Geometry is scaled explicitly rather than relying on backend transforms, keeping
  the software fallback and femtovg-wgpu hit testing consistent.
- The bottom-right **MiniMap** projects every node over the same padded world bounds
  as the canvas. Its highlighted rectangle follows scroll, zoom, Fit View and window
  resizing. Click or drag in the MiniMap to center the main viewport; navigation
  clamps at graph edges, restores graph keyboard focus, and never changes Project
  data/history. The MiniMap hides for empty graphs or whenever the complete graph is
  already visible (including Fit View), and leaves room for scrollbars.
- Click a node header to select it; **Shift-click** toggles its selection.
  **Select all** or **Command/Ctrl+A** selects all nodes while the graph has focus.
  Dragging still moves only the dragged node and selects that node on release.
- **Copy / Paste** and graph-focused **Command/Ctrl+C / V** use an owned Rust graph
  clipboard, not the OS text clipboard. Copy captures node data and only connections
  with both endpoints selected; boundary connections are not duplicated. Paste
  remaps node/port IDs, preserves node data/labels and relative layout, selects the
  pasted nodes, and commits the entire subgraph as one SDK revision/Undo entry.
  Each successful paste adds another 40 graph units of offset from the snapshot;
  copying again resets that offset. Failed pastes preserve project/history and
  the next offset. Copy with no selection leaves the existing clipboard intact.
- The graph clipboard survives project Open and source edits/removal until replaced
  by another Copy or the app exits. Embedded Image and Framebuffer data URLs are
  copied with their node data; inference model files and video/camera sources remain
  external. Text fields retain their normal text clipboard shortcuts. Graph Copy/Paste/
  Select all are disabled during project I/O; graph shortcuts do not run during a node drag.
- `AppController` owns the SDK `Project` and updates Slint properties/models.
  Project parsing, validation, serialization, and graph mutation use the Rust SDK;
  the UI does not interpret project JSON.
- A file worker performs disk I/O, resource decoding, and SDK parsing/serialization.
  A separate render worker owns `open_quartz::player::NativePlayer`;
  its realtime clock feeds the shared FramePipeline, which owns graph/GPU/video/inference execution.
  The former native host crate and PlayerHost forwarding layer have been removed.
- The render bridge creates one wgpu 30 device/queue, clones those same handles
  into `NativePlayer::from_shared_wgpu`, and passes the originals to Slint's
  `WGPUConfiguration::Manual`. NativePlayer receives the SDK graph, advances playback,
  and provides its output texture. A Slint timer imports the latest texture using
  `slint::Image::try_from`; frames do not go through pixel readback or encoded URLs.
  Graph generations discard stale output after opening another project.
  Phase 1's bounded stable-frame pool (three slots / 256 MiB by default) remains.
  Phase 2 returns stable demand output through that pool; main/selected preview
  reuses the completed snapshot or captures another output when needed. GPU detach
  copies preserve pixels while asynchronous consumers retain them; node cards stay live.
  On macOS, a Metal texture ownership guard follows Slint's actual texture clones,
  including screenshot/cache use, and pool reuse waits for queue completion.
  Paused preview requests retry capacity exhaustion without resuming playback.
  Historical Phase 1 evidence is not Phase 2 verification; other platforms have not
  received this phase's hardware acceptance. See the
  [phase implementation and evidence records](../open_quartz_video_editor/DESIGN.md#144-当前实施记录与后续范围).
- Model cache/download/checksum/session preparation lives in `open_quartz::resources::models`, and runs on model workers. Ticket/generation validation still precedes session attachment; the render loop performs no downloads. Capabilities are enabled by the Slint app's explicit core features.
  Historical acceptance records below use the former NativeHost implementation name.
- If GPU/host initialization fails, the shell attempts Slint's default UI backend
  and explicitly reports **Renderer not initialized**. It never synthesizes an
  output image. Host graph/render errors and texture import failures are displayed;
  a render/import failure stops rendering until another open, topology, or executable-data edit.

The app includes Sampling, Framebuffer, and Video configuration forms plus
inference model loading and execution. Image inputs accept PNG/JPEG/BMP/WebP files; raw
Framebuffer inputs accept binary files decoded using the applied format, dimensions,
and optional row stride. Image/Framebuffer resources are embedded in the SDK project
and decoded asynchronously after Open. NativePlayer receives decoded RGBA textures;
errors are reported instead of replaced with sample output. Video file paths and
camera IDs are persisted and reconciled by NativePlayer, with native camera enumeration,
file selection, loop, and playback rate controls. Video files and cameras remain
external resources and can fail if a file/device is missing or OS permission is
denied. Sampling applies each source node's Linear/Nearest filter and Clamp/Repeat/
Mirror wrap. Only RGBA8 host preview outputs can be imported. Mobile gestures and
mobile camera/file permission adapters are deferred with Phase 6.

Choosing a video file while stopped or paused decodes one real frame for the node
thumbnail and selected Inspector preview. Source replacement updates the preview
even when dimensions match; it preserves the UI playback state. This does not
execute the graph or advance the starting playback position. Explicit Play starts
normal decoding. Preview polling ends on success, error, timeout or source removal.

The toolbar shows composition playback time (`HH:MM:SS.t`) from NativePlayer's
actual session timeline, not a second UI wall clock. Pause freezes it, Resume
continues, and Stop/New/Open reset it. Renderer cards, the Renderer Inspector
preview and fullscreen show **Render FPS**: successful graph output ticks per
wall-clock measurement window (500ms), published with the time at most every
100ms. This is the graph render-loop rate, not unique source video FPS, GPU
completion throughput, or monitor scan-out FPS. Paused/stopped render rate is
zero; a static output may still be presented on successive render ticks.

A standalone external `sampler2D` source can be the graph's terminal texture output:
Video Play does not require adding a Renderer first. An explicit Renderer still
takes precedence when present. Native Choose File acceptance with a real 1080p
H.264 Baseline MP4 covers a conflicting container BT.709 / SPS BT.470BG source,
thumbnail and Inspector preview, standalone Play/Pause/Resume, and user confirmation.

On macOS, H.264/HEVC file video first uses `oq-media`'s in-process
VideoToolbox backend. Retained CVPixelBuffer planes are imported through Metal
into the shared wgpu device and converted to SDR sRGB RGBA on the GPU, without
CPU frame transfer. The bundled, checksum-pinned LGPL-only FFmpeg 8.1.1 SDK
includes its corresponding source, hardware-required patch and build options.
Backend or unsupported presentation failures are explicit errors, never FFmpeg
RGBA software fallback in the real-time graph. NV12/P010 and GPU colors plus
real-window stopped/Play/Pause/Resume/Stop behavior have been exercised on Apple
Silicon, including a relocated production package. BT.470BG/PAL primaries are
preserved separately from BT.601 YUV coefficients and converted to BT.709 in
linear light before sRGB output. HDR, other primaries, rotation/crop and non-square
pixels remain unsupported by this path. Unsupported cameras/Linux and Windows
without D3D12 refuse rather than CPU-upload frames. Device-specific 8K measurements
in the [oq-media README](../../crates/oq-media/README.md#8k-性能与-opengl-兼容慢路径) do not establish inference throughput or cross-platform support.

For the small shared-GPU video demo, prepare the runtime first, then run
`cargo run --manifest-path apps/open_quartz_slint/Cargo.toml --example video_preview -- VIDEO`.
It exposes explicit Play/Pause/Stop controls and decoder/CPU-upload metrics;
it is an integration example, not a stabilized standalone player-component API.

Renderer controls select an explicit output and reuse its shared texture in the
normal preview or **Fullscreen**; **Close · Esc** returns without readback or a
second decoder. PNG capture requests full-resolution readback only on user action.
Windows screen saver export runs on the project I/O worker, uses an explicit
Renderer, and exposes eligible Image/Video inputs in the exported settings. Camera
graphs are rejected as nondeterministic sources. Raw framebuffer sources are baked
into PNG assets in the export copy; the editor project is unchanged. Exported
`.scr` files include required media/models, shared codec/VC++ DLLs and runtime
resources; their MSVC launcher uses a static CRT and delay-loads FFmpeg until after
extraction. Independent Windows execution still requires platform acceptance.

### Historical macOS package and dialog acceptance

The following observations apply to the earlier migration build, not the current
native-only retirement. Current results and remaining gates are in [Design 验证基线](../../docs/DESIGN.md#validation).

The macOS production Controller/NativeHost smoke used the packaged FFmpeg and
ORT 1.21.0 libraries: advancing 96×48 video, exact 2×2 blue/red PNGs, Renderer
selection while paused, fullscreen/Escape, Stop/Play, Save/Open, ONNX cancellation,
inference and unload all passed with actual FemtoVG window snapshots and ordinary
process exit. After unlocking the session, real AppKit dialogs exposed a Controller
borrow held across a nested modal loop; the fix releases that borrow and keeps
project edits busy-guarded. Native Video cancel/select, Image loading, raw 2×1
red/green framebuffer loading, and PNG save/cancel now pass in the rebuilt package.
The saved video PNG is 80×40; the user also confirms fullscreen button behavior.
Camera enumeration returned no devices; live camera permission/capture and
Windows/Linux runtime gates remain. Windows cross-check reached bindgen with real,
checksum-verified FFmpeg headers but lacks Windows SDK `errno.h`. An earlier
LaunchServices launch aborted inside winit; a later logged launch rendered normally.
Its intermittent cause is unconfirmed, not covered by the modal-loop fix.

## Inspector and WGSL editing

- Select exactly one node to inspect its kind, name, and typed input/output ports.
  Empty selection hides the sidebar; multi-selection shows an explicit prompt.
  Shader nodes expose uniform rows for Float/Int/Uint/Bool, Vec2–4/Ivec2–4/Uvec2–4, and Mat2–4 inputs.
  Connected rows are read-only because their upstream value wins. Samplers, Auto,
  boolean vectors, and other resource/object types are not uniform rows; the Ports
  list still shows them. Sampling and Input configuration forms are described below.
- Image and Framebuffer inputs provide native file selection in the Inspector. Image
  dimensions are detected from the file. For raw Framebuffer data, apply its format,
  width, height, and optional row stride before choosing the file. A successful
  import updates SDK node data as one undoable edit; Save includes the embedded data
  URL, and Open revalidates and uploads it asynchronously.
- Edit the name in the sidebar header and press Enter or **Apply**. Its trash icon
  deletes the node. Shader/Constant nodes expose **Auto size**, width/height
  (1–8192), and **Apply output size**. Checked: follow the first connected sampler
  input in declared port order, including the actual uploaded Image/Video texture
  size and upstream Filter outputs; unrelated graph resources do not override it.
  Unchecked: use the specified width/height. Live source replacements propagate
  dimensions through automatic filters on the next executed frame, while manual
  targets remain fixed. Builtin `iResolution` matches each allocated GPU target.
  With no connected sampler, automatic targets retain the graph-default size.
- Shader nodes use the reusable `crates/slint_code_editor` component, with a
  cosmic-text document/layout, live WGSL colors, line numbers, goto-line,
  find/next, replace/all, mouse/keyboard selection and document undo/redo.
  Built-in implementations remain read-only; **Apply shader** remains explicit.
  No inline GPU linter, completion or debounced auto-apply is provided.
  Desktop IME events use public Slint/winit APIs; real input-method acceptance
  remains unverified. Public Slint APIs cannot report custom caret/selection
  geometry to screen readers, so this is not full text accessibility parity.
- All fields are session-local drafts until their respective Apply action. Name,
  uniform values, Sampling, Framebuffer/Video configuration, output size, and WGSL
  source are independent groups, so applying one preserves drafts in every other group.
  Use **Discard changes** to restore all fields without creating a revision.
- While drafts differ from the project, graph mutations/selection/history and
  Open/Save are disabled and rejected by the controller. Apply or discard before
  switching nodes/projects. Project I/O disables Inspector edits and Apply too.
  Drafts are not saved automatically and are lost on application close.
- Names, uniform values, Sampling, Framebuffer/Video configuration, and output sizes
  use SDK `UpdateNodeData` with complete existing data; WGSL uses SDK `UpdateShaderCode`.
  Uniform values require the exact scalar/vector/matrix component count and valid
  finite/ranged literals. Vector values are comma-separated; matrix values use WGSL
  column-major order. Invalid drafts remain local and disable Apply without changing
  the project or other drafts. Bool accepts `true`, `false`, `1`, or `0` and displays
  canonical `true`/`false` after Apply. Integers must fit i32/u32; floats must be finite
  f32-range values. Missing values display zero/false without writing defaults;
  returning a missing value to zero is a no-op, and applying another row does not
  seed unrelated zeros. Untouched invalid legacy values show validation without
  dirty-locking the graph. Failed connected-port source changes preserve the graph,
  revision, history, and draft. SDK syntax errors can be committed when the graph
  remains valid; the stored `parseError` is shown in the panel. Apply valid source to
  clear it. GPU compilation failures carry their structured node ID back from
  NativePlayer: the affected node's status LED turns red, its stale texture preview
  clears, and the runtime status remains readable. Unrelated attached inference nodes
  are not marked failed.
- Each changed Apply is one SDK history entry. No-op Apply creates no revision.
  Undo/Redo refreshes Inspector values. Name-only edits do not restart rendering;
  source/uniform/output-size edits and their history refresh the runtime graph.
  Committed fields and parse errors round-trip through SDK Save/Open.
- Sampling is its own draft group on Shader/Constant and sampler2D Input nodes.
  Filter supports Linear/Nearest; wrap supports Clamp/Repeat/Mirror. Apply rebuilds
  the runtime graph and takes effect on the source sampler; Undo/Redo refreshes
  both the form and output.
- Framebuffer Inputs expose all schema formats, required 1–8192 width/height, and
  optional positive-u32 stride (`blank` or `0` means auto). Video Inputs expose
  File/Camera source, native file/device selection, Loop, and finite 0.1–4.0 playback
  rate. Apply persists complete SDK `NodeData`; source switches preserve the saved
  file path, and NativePlayer reconciles committed source configuration. Raw buffer
  resources are decoded by the file worker rather than on the Slint UI thread.

### Inference model metadata

- Select one Inference node to inspect its recorded source, model/catalog IDs, custom
  path, input connection count, thresholds, target size, and numeric/bool parameters.
  Missing values say **Not recorded**; a recorded zero remains zero. A custom path
  is explicitly **not checked**. The generic Ports section lists its typed ports.
- A separate **Saved metadata · not live session state** section shows any saved
  file name, status, backend/native backend, error, and download progress. Progress
  accepts only numbers in 0–1; invalid saved values are identified, not clamped or
  treated as a current download. Saved `ready` never means Slint has loaded a model.
- Until explicitly loaded, the live section reports no attached session. Session
  and saved project metadata are separate. Inspection does not seed defaults or
  mutate the graph; empty/multi-selection clears the old panel.
- Renderer source mappings are invalidated when their actual upstream changes. A
  unbound inference output reports the missing native texture and clears the stale image.

### Native inference sessions

Production inference uses Metal resident GPU execution, not a dynamically loaded ORT
session. The packaging scripts still prepare official ORT 1.21.0 and notices for
explicit reference/diagnostic consumers; `ORT_DYLIB_PATH` selects that real runtime,
not a production fallback. Shader editing and project I/O need no loaded model.

- Select one Inference node and explicitly **Load model**. A temporary local path
  overrides the saved custom path only for the current session. Without an
  override, the shell uses the custom path or the catalog download/cache planned by
  the existing Rust host-intent API.
- Catalog downloads stream into a unique temporary file under the
  `com.openquartz.app/models` cache and become visible only after completion.
  A persistent per-model advisory lock coordinates reads/downloads across processes;
  the OS releases a lock when the process exits, so a stale lock file is harmless.
  Optional catalog SHA-256 is verified, including cache hits; an empty catalog
  digest means **unverified**, not validated. Failures/cancellation remove partial
  files. Model IDs are validated before any cache or lock path is constructed.
- Model file/weights parsing is preparation work, not per-frame I/O; initial weights
  upload is permitted. The GPU compiler validates original topology, weights and
  reshape layout, never approximates the network. GpuImageSession scratch is STORAGE-only.
  Live state reports backend, interfaces and actual outputs, not fabricated results.
- **Cancel load / Unload model** invalidates the ticket and clears native output.
  Cancellation blocks late publication but does not promise immediate interruption.
  Opening a project tears down its resource sessions even if a node keeps its ID.
  Edits retain a session only while the shared model/resource plan is unchanged;
  an updated task/model/path requires an explicit reload. Superseded native
  inference completions cannot publish textures/events or clear a newer task.
- Loads, progress, backend and inference results are session state, not SDK history
  or saved node data. Open never loads/downloads models implicitly. Resource
  operations are guarded by unapplied inspector drafts.
- UI policy is **Metal GPU / Auto Hardware** only: MetalGpu / Metal, GPU placement,
  Device path, RequireResident + AcceleratorOnly + Forbid CPU. Auto never selects
  ORT/CPU/CoreML EP. Legacy project parsing cannot weaken these production constraints.
- `.onnx` remains the real artifact format and menus remain task-centric; catalog
  presence and successful introspection do not imply resident model support.
- Independent sessions support named multiple FP32 inputs/outputs. The graph image
  adapter validates one input and a task-selected named output; catalog U²Net-P
  explicitly binds `1959` while retaining all seven outputs in introspection.
  Unknown multi-output models reject without a declared binding. Other dtypes fail explicitly.
- Completed-inference rates differ from Render FPS. No GPU time is fabricated from
  host wall time. On Apple M4 Pro Metal, resident_compare's GPU-generated 8×8 source
  (no COPY_DST), full-network 24×24 output matched explicit ORT CPU reference with
  max/mean byte error 0/0, host image bytes 0, six dispatches, scratch 26,292,224 B,
  weights 238,628 B. Wrong-device before-submit rejection and pool-three saturation/
  released-lease reuse were observed. First completion wall 69.494292 ms includes
  diagnostic copy and initial pipeline compilation, not throughput or GPU time.
  Capture/reference/test readback is opt-in; one-time image/framebuffer CPU decode
  and initial upload is the explicit ingress boundary, never a per-frame fallback.
- Save emits canonical `inference` / `inference*`. Legacy `onnx` / `onnx*` records remain
  readable without rewriting old IDs, ports, edges or project version. Live sessions
  are not persisted; see the [node contract](../../docs/ONNX_NODE_DESIGN.md).

The independent editor has explicit Apply and UTF-8-safe editing and navigation.
The earlier macOS text-input assessment is historical; see
[`SLINT_UI_REFACTOR.md` §7.3.1](../../docs/SLINT_UI_REFACTOR.md#731-macos-原生编辑体验评估).

## UI automation

Run `npm run test:desktop:ui` to build the optional `ui-automation` feature and
drive the compiled application through Slint 1.18.1's embedded MCP server. It uses
Node's built-in APIs; no Python testing license/token or external MCP client is
required. The runner chooses a local port, launches its own application instances,
and terminates them when finished. Normal `dev`/release builds omit this feature.

The suite exercises empty/stopped startup, the real Shader/Renderer menus and
ports, node dragging at manual zoom and after Fit, Undo/Redo, Inspector edits,
explicit Play/Stop, and Save/relaunch with the same project. It asserts live UI
state and node geometry and decodes screenshots to check blue pixels inside the
Renderer preview. It requires a working GPU/window backend, including Vulkan
with Mesa under Xvfb on Linux; `SLINT_BACKEND=headless` is not a substitute for
the shared-wgpu rendering acceptance.
Menus, toolbar buttons, Apply actions, ports, and node movement use simulated
pointer input through the real Slint event loop. Node selection and editing
use accessibility actions/values; this is not an OS keyboard/IME test.

Current real-window acceptance passed 18/18; see the [current report](../../test-results/slint-ui/interaction-2026-11-07T08-00-55-697Z/report.html). Shader pixels passed 34/34; see the [Shader report](../../test-results/slint-ui/shaders-2026-11-07T08-01-12-929Z/report.html).
Other results and remaining gates are in [Design 验证基线](../../docs/DESIGN.md#validation); older local acceptance below remains historical.


Each run writes a readable **`report.html`**, PNG screenshots, application logs,
a saved project, and JSON reports under `test-results/slint-ui/<timestamp>/`.
Open `report.html` for the complete Chinese case list, expected behavior,
pass/fail/not-run results, durations, screenshot previews, and failure reasons.
Search or filter by status; raw technical details remain collapsible. The terminal
also prints each case as it runs and a final summary. Keep the report folder intact
when downloading CI artifacts so relative screenshot/log links remain available.
Failures retain UI-tree and screenshot diagnostics when the process remains responsive. Set
`SLINT_UI_OUTPUT_DIR` to choose the output directory, or `SLINT_UI_TIMEOUT_MS`
to change the default 180-second suite deadline (build time is separate).
After an automation build, rerun without rebuilding using
`node scripts/slint-ui-test.mjs apps/open_quartz_slint/target/debug/open_quartz_slint`.

To convert an existing run without rerunning the application:
`npm run report:desktop:ui -- test-results/slint-ui/<timestamp>`.
The report distinguishes later cases that never ran from the failing case and an
interrupted case without a final result. A startup failure leaves the cases
unexecuted and displays the application error. Report regressions can be checked
with `node --test scripts/slint-ui-report.test.mjs`.

The `auto-size-playback-time-renderer-fps` interaction case loads real embedded
non-square PNG inputs, including a differently sized unconnected image, and
checks Filter→Renderer resolution through Auto→manual→Auto UI commits. It also
checks positive render FPS and advancing time, frozen paused time, resumed time,
zero paused FPS and reset time/FPS after Stop; screenshots retain actual GPU
preview pixels. Run it with the ordinary `npm run test:desktop:ui` suite.
The case checks actual preview pixels; render FPS assertions require a positive
rate, not a machine-specific performance threshold.


### All catalog shaders with real image fixtures

Run `npm run test:desktop:shaders` to exercise every entry in
`crates/open_quartz/src/project/shader_catalog.json` in a native Slint window. The suite loads edge-free
projects containing the original catalog source, explicit non-degenerate uniforms,
and real 64×64 PNG image resources. It then clicks every sampler/Renderer port,
starts playback, pauses the real GPU preview for pixel inspection, and stops.
Single-input effects receive one PNG; blending and displacement receive two
different PNGs. Generators have no sampler and do not receive artificial edges.

Inputs are original, deterministic color/gradient/edge/geometric test charts in
`tests/fixtures/shaders/`, not pure-color arrays or downloaded assets. Each case
retains input images, an independent CPU reference image, the sampled Renderer
image, native-window screenshots, parameters, and per-stage results in its HTML
report. Spatial/color formulas account for linear preview scaling at physical
pixel centers, with a maximum RGB error of 6/255. Noise's GPU sine/hash is
platform-dependent: it checks grayscale, range, mean, variance, and neighboring
pixel RMS gradients against the independent reference instead of equating random
pixels. Wrong-frequency and spatially shuffled noise are rejected. Gray-Scott
requires two distinct rendered frames, then matches the paused output against
independent frames 1–200 (8/255 tolerance), quantizing its seed and every feedback
step to the runtime's RGBA8 format. Pixelate uses binary-exact 8×4 blocks to avoid
platform-dependent nearest-sampler ties after division.

Earlier macOS/Metal catalog acceptance is historical; it does not establish the
current native-only result or cross-platform acceptance.

Every catalog entry must have a reference; adding a new entry fails preparation
until its oracle is defined. Ordinary case failures are isolated and collected
across the full catalog; an application crash or global cancellation aborts the
run. The Shader suite has a default 600-second deadline, configurable through
`SLINT_UI_TIMEOUT_MS`. To reuse an automation binary without building, append
`--shaders` to `node scripts/slint-ui-test.mjs <binary> [output-directory]`.

This suite covers normal project/image decoding, graph connections, parameter
projection, native GPU rendering and display. It does not claim that preloading
a project tests native file dialogs or every Shader creation-menu item. The
separate interaction suite covers menu creation and Save/reopen behavior.


For interactive inspection, use `npm run dev:desktop:ui` (optionally followed by
`-- --project /absolute/path/project.quartz.json`). It exposes the MCP endpoint
at `http://127.0.0.1:8080/mcp`; `SLINT_MCP_PORT` selects another port. Connect a
Streamable HTTP MCP client to inspect elements, send input, and take screenshots.
UI metadata is embedded by `build.rs` whenever `ui-automation` is enabled.

CI runs the suite on Linux/Xvfb and macOS and uploads `slint-ui-<os>` artifacts
even on failure. Windows compiles the optional feature with
`npm run build:desktop:ui` and retains normal build/unit coverage; its UI run is not
enabled until an interactive Windows runner is verified. This suite does not
cover native OS file-dialog interaction, IME composition, camera permission,
or broad screenshot-baseline comparisons. Save uses a preassigned project path;
reopen launches the actual application with `--project`.

## Historical validation and migration records

The following section preserves results from earlier builds and migration stages.
Counts, paths, implementation names and acceptance statements are historical, not
results for the native-only retirement; current results and remaining gates are in
[Design 验证基线](../../docs/DESIGN.md#validation).

### Validation scope and prior acceptance

The crate contains unit coverage for multi-port edge endpoints, spatial bounds,
persisted node movement, no-op revisions, atomic occupied-input replacement and
reconnection, failed type/cycle edits preserving graph/history, deterministic stress
fixtures, CLI option combinations, dash-prefixed paths, the 300-node cap, and tight
translation-invariant fit bounds including backwards Bezier connections. Clipboard
coverage includes multi-selection, internal-only edges, custom port ID remapping,
deep snapshot independence, repeated ID allocation, single-step history, persisted
copies, and rejected pastes preserving destination, redo, and offset.

The Inspector/editor suite adds regressions for independent field commits, no-op
revisions, port ID/edge preservation, single-edit history, output-size boundaries
and Auto transitions, rejected connected-port changes, persisted parse errors and
repair, plus typed uniform scalar/vector/matrix parsing, bounds/exact arity, connected
exclusion, invalid-draft atomicity, schema refresh, missing-zero no-ops, complete-node-data
preservation, independent drafts, single Content history, and SDK persistence.
The historical Inspector/ONNX validation snapshot at this point passed **49 Slint
tests** and **280 workspace tests / 45 suites**; those counts predate the editor
and graph-preview restoration below.

The uniform slice was also exercised in the release AppKit window on Apple M4
Pro/Metal at 1180×760 and an 800×540 client area. Invalid `NaN` showed a per-row
error and guarded file/graph actions; connected `wired` stayed disabled with an
upstream-precedence explanation. Applying scalar `gain` and vector `tint` changed
real 96×48 output from red to blue; one Undo restored red and Redo restored blue.
The fixture also executed a Mat2 uniform. In the narrow window, HID scrolling
reached the fields and Apply; keyboard replacement and consecutive `0.7` then `5`
inputs produced `0.75` without losing focus/caret, and a pointer Apply succeeded.
SDK Save/Open retained `gain = 0.75`, blue `tint`, untouched matrix/integer/connected
values, and both connections. No readback was added to production presentation.

Sampling/Framebuffer/Video forms were exercised in the release AppKit window at
1180×760 and an 800×540 client area. NativeHost changed a 96×48 nearest/clamp sample
from blue to red after Repeat Apply; Undo restored blue and the Clamp ComboBox,
Redo restored red/Repeat. Framebuffer rejected width 0, then committed 64×16 with
stride 256. Video rejected rate 9, then committed Camera `camera-native-1`, Loop off,
and rate 2.5. Narrow-window scrolling, popups, checkbox, text fields and Apply were
all operated. SDK Save/Open retained sampling, framebuffer, video, original video
file path, and both graph edges. Resource loading was not claimed or synthesized.

Headless verification: `cargo test --workspace` passed **288 tests across 46 suites**;
the Slint suite passed **51 tests** and the release build succeeded. An integration
test loaded the `image_identity.onnx` fixture in NativeHost with the official ONNX
Runtime 1.21.0 CPU dylib, passed its expected 2×2 RGBA through the downstream
Renderer, emitted the CPU output event, and exited normally. Native model cache
tests cover streamed publication, digest validation, cancellation, and invalid IDs.
These are historical headless ONNX-slice results, not evidence for the current
packaging/media/capture cutover. The current combined Phase 5/7 native-window,
catalog/custom load/cancel/unload, and portable package acceptance are pending the
final local smoke/tests. Earlier lock-screen observations are not a current blocker.

The ONNX slice was exercised in the release AppKit window at 1180×760 and an
800×540 client area with catalog, custom, and empty ONNX nodes. Saved `ready`,
backend, thresholds (including zero), parameters, saved errors, invalid progress,
Unicode/long paths, connection counts, and selection clearing were checked. SDK
Save/Open preserved the entire fixture graph without defaults or live-state writes.
After switching Renderer to an unbound ONNX output, the real Host reported a missing
texture and cleared the image; reopening the valid graph restored 96×48 shared
Metal output. The GPU regression covers reconnection and subsequent disconnection.

WGSL assessment used 300 long Unicode comment lines (66,706-byte source), native
scrolling and consecutive keyboard input ending in `αβ`, Apply, and SDK Save.
Screenshots confirmed the source, fixed Apply, and actual green output at both
window sizes. This is not a latency benchmark or IME-composition validation.

A temporary software-rendered smoke exercised real name/source text inputs,
Command/Ctrl+A isolation, pointer Apply, draft protection during selection/history/
file actions, per-field Apply, Undo/Redo, multi-selection, asynchronous Save/Open,
busy guards, and parse-error display/repair. Screenshots were inspected at
1180×760 and 800×540; Apply shader stays reachable in the narrow window. Using
the same controller actions with the real RenderBridge/NativeHost on Apple M4
Pro/Metal produced 96×48 output after a size edit and RGBA `[255, 0, 0, 255]`
after applying a red shader (one-pixel readback for this smoke only). Temporary
smoke code was removed; production presentation still uses shared GPU textures.

Native AppKit validation after unlocking exercised accessibility selection, name
Apply, WGSL Apply, source Undo/Redo, rejected connected-port removal with the draft
retained, Discard, width stepping to 641×360, and SDK Save/Open. The saved JSON
preserved the name, red source, dimensions, port IDs, and connection. Screenshots
showed real red shared-texture output. This run also fixed renderer image preferred
height starving the Inspector: both panels now use explicit preferred heights,
with a 120px renderer minimum, so imported textures do not determine their split.

The remaining native pointer/keyboard and narrow-window checks also passed at an
800×540 client area (800×572 including the AppKit title bar). Direct macOS HID
mouse events selected Shader/Renderer nodes, focused text controls, clicked Apply
and Discard, and scrolled the Inspector both ways to reach the WGSL editor.
Command+A and keyboard text input replaced the name draft and then the WGSL
source without triggering graph Select all. Applying a blue shader with a real
pointer click changed the shared Metal output from red to blue at 641×360, while
the independent name draft continued to guard graph/file actions. Discard removed
that name draft without reverting the committed blue source; switching away and
back confirmed it. Screenshots confirmed the visible source and fixed-bottom
Apply alongside the blue output. The earlier automation focus blocker was bypassed
with direct HID events and correctly targeted scroll coordinates; no production
input workaround was added. Native acceptance for this first Inspector/editor
slice is complete. The Phase 2/3 results below predate this Inspector slice.

An ephemeral software-rendered Slint smoke exercised actual pointer events for
connection selection/deletion, port connection, reconnection, type rejection,
Cancel, Undo/Redo, busy guards, node dragging, and asynchronous Save/Open. Screenshots
were inspected at 1180×760 and 800×540, plus a 300-node/450-edge canvas. A separate
Apple M4 Pro/Metal smoke observed a disconnected Renderer report its missing input,
clear its image, and resume 640×360 GPU texture output after reconnect and redo.

A separate viewport pointer smoke verified 80%/125% drag conversion, the screen-pixel
selection threshold, scaled port/edge hit testing, graph Undo, center preservation
after scrolling at 200% then zooming to 160%, and unchanged project data during
view-only operations. Screenshots covered manual 1%/200%, a translated 300-node graph
in Fit View at 1180×760 and 800×540, resize adaptation, empty graphs, and asynchronous
Open resetting Fit mode to 100%. The first and last stress nodes remained selectable
in the fitted overview. The temporary smoke code was removed after verification.

A clipboard pointer/key smoke exercised Shift toggling, graph Select all/Copy/Paste,
one-step Undo/Redo, repeat offsets, text-field shortcut isolation, busy guards,
SDK Save/Open, and cross-project pasting. Screenshots were inspected at 1180×760
and 800×540. A pasted shader/renderer pair in a fresh project produced a real
640×360 Apple M4 Pro/Metal GPU texture through NativeHost using the remapped ports.
Temporary smoke code was removed after verification.

A MiniMap pointer smoke over the deterministic 300-node/450-edge graph verified
bottom-right and top-left navigation, continuous drag navigation, viewport updates
across manual zoom and Fit View, negative-coordinate translation, 1180×760 and
800×540 resize, empty-graph hiding, keyboard focus restoration, and Project/revision
invariance. The first and last nodes remained selectable after navigation.

An ephemeral offscreen benchmark rendered the actual Slint tree through the same
FemtoVG/wgpu 30 backend into a 1180×760 Metal texture, with 30 synchronized steady
full frames per graph after one warm-up. On Apple M4 Pro release: 100 nodes/150 edges
projected in 75µs, rendered at 2.326ms mean / 2.523ms p95 (429.9 FPS); 300 nodes/450
edges projected in 271µs, rendered at 2.825ms mean / 3.338ms p95 (354.0 FPS). Debug
was 20.574ms mean (48.6 FPS) and 28.127ms (35.6 FPS), illustrating optimization
impact. Release clears the 60 FPS frame-time gate in this deterministic offscreen
case. A separate native AppKit/FemtoVG-wgpu release run below verifies the swapchain/
event-loop path on this Mac; results still do not establish other hardware/platforms.

The final macOS Phase 3 candidate was packaged as an app bundle and observed through
ScreenCaptureKit plus the native accessibility tree at Retina scale. The 1180×760
release window exposed the graph controls, rendered the shared 640×360 Metal texture,
selected/copied/pasted a 100-node subgraph (100→200 nodes), undid it in one step,
and exercised zoom/Fit. In the executable sample graph, native accessibility actions
selected nodes and edges, deleted the renderer edge (NativeHost stopped and cleared
the image), restored it with Undo (texture output resumed), and activated output/input
ports without creating a no-op revision. Node headers, ports, edges, and MiniMap now
provide VoiceOver default actions without hiding nested port controls; busy state is
also reflected in node accessibility.

With `SLINT_DEBUG_PERFORMANCE=refresh_full_speed,console`, the native release window
reached a steady **60 FPS** for both 100 nodes/150 edges and 300 nodes/450 edges on
Apple M4 Pro. The first reported intervals were 59 FPS and 51 FPS respectively before
stabilizing at 60, so the macOS native 60 FPS acceptance gate is considered passed.
The 300-node Retina screenshot showed the complete Fit View at 10% with no renderer
substitution. macOS Phase 3 is accepted; Linux validation is intentionally deferred.

In debug builds, the package disables debug assertions only for `wgpu-core`. Slint keeps
the manually supplied queue in its thread-local context, while wgpu-core's debug-only
recursive-lock tracer is a separate TLS value; macOS may destroy the tracer first and
otherwise abort when Slint later drops the queue. Release builds never enable that tracer.
The override does not disable assertions in OpenQuartz, Slint, or application code.

Development builds optimize dependencies (`profile.dev.package."*".opt-level = 2`)
while retaining application debug information and assertions. A native pointer
drag probe on Apple M4 Pro/Metal measured about 33ms of UI-thread frame encoding
with unoptimized dependencies versus 4–5ms after optimization, with typical
frame intervals around 16.6ms. These timings exclude snapshot/readback work and
do not measure GPU execution time or establish results for every graph/device.
The grid is one repeated image rather than thousands of rectangle items. Shared
GPU previews reuse texture identities and notify node rows only when the image,
resolution, availability or error state changes; in-place pixel updates still
request redraw. Slint 1.18's generic `Image` equality does not recognize identical
WGPU/empty images, so the cache compares GPU handles explicitly. A real Time-driven
shader probe verified that pixels keep changing with the same cached texture and
one Undo reverses a full drag. All 68 application tests and the dev build pass.

Windows DX12 remains covered by the earlier Phase 1 spike, not by a completed
Phase 5/7 package acceptance. CI now builds/tests the Slint desktop on Windows,
macOS, and Linux, with Linux desktop/GPU dependencies installed. Results from the
new matrix are pending. The NativeHost boundary previously compile-checked for
`aarch64-apple-ios`; that is not an iOS app or hardware acceptance. Android/iOS
delivery and mobile runtime/gesture validation are explicitly deferred by user decision.

Current native UI restoration verification (2026-10-29): offline Slint build
passed and all **66 application tests** passed. A real shared-GPU window smoke
verified Shader/Renderer thumbnails and resolution labels, preview collapse and
expand, the 34-entry categorized Shader catalog, preset read-only behavior,
Unicode drafts and live find metadata, precise GPU failure status, stale preview
clearing, and Undo recovery to a real texture. Native FILE → Open and Save As
dialogs opened and cancelled. The automation provider did not reliably deliver
arbitrary keyboard focus to a TextInput, so this run does not claim native typing
or IME-composition acceptance; the existing TextInput implementation retains the
platform editor's input behavior.
