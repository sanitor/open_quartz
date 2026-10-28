# OpenQuartz Slint application shell

This native application shell includes Phase 4 Inspector/WGSL editing and Phase 5 ONNX sessions.
It is a standalone Rust workspace, with Slint and its build compiler pinned to
**1.18.1**, using the femtovg wgpu renderer and wgpu 30 shared-device integration.

## Run

From the repository root:

```sh
cargo run --manifest-path apps/open_quartz_slint/Cargo.toml
cargo run --manifest-path apps/open_quartz_slint/Cargo.toml -- --project /absolute/path/project.json
cargo run --manifest-path apps/open_quartz_slint/Cargo.toml -- --project -draft.json
cargo run --manifest-path apps/open_quartz_slint/Cargo.toml -- --graph-stress
cargo run --manifest-path apps/open_quartz_slint/Cargo.toml -- --graph-stress 300
```

The first launch without `--project` creates a small gradient shader → renderer
project through the Rust SDK node factory and graph commands. It is a real
project and can be saved. A supplied project is read and validated asynchronously;
a failed open leaves the current project intact and reports the SDK or file error.
An empty project opens with an empty graph and an explicit idle renderer status.

`--project` accepts relative, absolute, and dash-prefixed paths. `--graph-stress`
creates a deterministic spatial fixture with 100 nodes by default and accepts an
explicit count from 1 through 300. The two options are mutually exclusive.

The path field is used for **both Open and Save**. Press Enter to open that path,
or use the corresponding button. Save writes the SDK's `Project::to_file()` JSON
representation and **overwrites an existing file at the entered path**. Parent
directories must already exist. Relative paths are resolved against the process
working directory. There is no native file dialog or automatic save.

## Graph and application boundary

- A real Slint window with project name, path entry, Open/Save, I/O and runtime
  status, a scrollable spatial graph, node selection and drag-to-move, typed ports,
  editable connection paths, Undo/Redo, zoom/Fit View, MiniMap, graph Copy/Paste, and renderer output.
- `GraphViewModel` projects SDK graph snapshots into node and edge models and
  computes padded scroll content bounds. Its origin offset keeps loaded nodes with
  negative coordinates reachable while preserving graph-space edge coordinates.
- Node moves are committed once on pointer release through the Rust SDK. Motions
  below the three-pixel drag threshold select instead, and unchanged positions do
  not create a graph revision. Graph interaction is disabled while asynchronous
  project I/O is busy.
- Click an **output**, then an **input** to connect. The SDK replaces any existing
  edge on that input atomically. Click a **connected input** or a link's midpoint
  marker to select its edge, then use **Delete edge** or **Reconnect**. Reconnect
  keeps the old edge until a valid input is chosen; choosing another output first
  also changes the source. **Cancel** never mutates the project. SDK type/cycle
  errors preserve the original graph and leave the pending source available.
- **Undo/Redo** use SDK graph history for moves, connections, reconnections,
  deletions, and whole-subgraph pastes. An identical connection is a no-op. A successful new edit clears redo;
  a failed edit does not. Opening a project resets the edit session; generated
  fixture construction is not exposed as user undo history. History is not saved.
  Topology edits and their undo/redo refresh NativeHost; layout-only history does
  not restart rendering.
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
  by another Copy or the app exits. Referenced external media/model resources are
  not duplicated or attached by this shell. Text fields retain their normal text
  clipboard shortcuts. Graph Copy/Paste/Select all are disabled during project I/O;
  graph shortcuts do not run during a node drag.
- `AppController` owns the SDK `Project` and updates Slint properties/models.
  Project parsing, validation, serialization, and graph mutation use the Rust SDK;
  the UI does not interpret project JSON.
- A file worker performs disk I/O and SDK parsing/serialization. A separate render
  worker owns `open_quartz_native_host::NativeHost`; no execution engine logic is
  reproduced in this crate.
- The render bridge creates one wgpu 30 device/queue, clones those same handles
  into `NativeHost::from_shared_wgpu`, and passes the originals to Slint's
  `WGPUConfiguration::Manual`. The host receives the SDK graph, advances playback,
  and provides its output texture. A Slint timer imports the latest texture using
  `slint::Image::try_from`; frames do not go through pixel readback or encoded URLs.
  Graph generations discard stale output after opening another project.
- If GPU/host initialization fails, the shell attempts Slint's default UI backend
  and explicitly reports **Renderer not initialized**. It never synthesizes an
  output image. Host graph/render errors and texture import failures are displayed;
  a render/import failure stops rendering until another open, topology, or executable-data edit.

The shell includes Sampling, Framebuffer, and Video configuration forms plus ONNX
model loading and inference. It does **not** yet load image/raw/video inputs,
enumerate cameras, or adapt gestures for mobile. Sampling is executable: NativeHost
applies each source node's Linear/Nearest filter and Clamp/Repeat/Mirror wrap to
generated or externally uploaded textures. Framebuffer and Video persist configuration
only; absent resources and host failures are reported instead of replaced with
sample output. Only RGBA8 host preview outputs can be imported.

## Inspector and WGSL editing

- Select exactly one node to inspect its ID, kind, and typed input/output ports.
  Empty/multi-selection shows an explicit selection prompt. Shader nodes expose
  uniform rows for Float/Int/Uint/Bool, Vec2–4/Ivec2–4/Uvec2–4, and Mat2–4 inputs.
  Connected rows are read-only because their upstream value wins. Samplers, Auto,
  boolean vectors, and other resource/object types are not uniform rows; the Ports
  list still shows them. Sampling and Input configuration forms are described below.
- Edit the name and use **Apply name**. Shader/Constant nodes also expose
  **Auto size**, width and height (1–8192), and **Apply output size**. Auto size
  delegates resolution to the engine; applying it clears explicit dimensions.
- Shader nodes expose a plain, scrollable WGSL `TextEdit` and a fixed-bottom
  **Apply shader** button. Built-in node implementations are not source-editable.
  There is no syntax highlighting, inline GPU linter, or debounced auto-apply.
- All fields are session-local drafts until their respective Apply action. Name,
  uniform values, Sampling, Framebuffer/Video configuration, output size, and WGSL
  source are independent groups, so applying one preserves drafts in every other group.
  Use **Discard inspector edits** to restore all fields without creating a revision.
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
  clear it. NativeHost/GPU errors are reported in runtime status, not disguised as
  successful output.
- Each changed Apply is one SDK history entry. No-op Apply creates no revision.
  Undo/Redo refreshes Inspector values. Name-only edits do not restart rendering;
  source/uniform/output-size edits and their history refresh the NativeHost graph.
  Committed fields and parse errors round-trip through SDK Save/Open.
- Sampling is its own draft group on Shader/Constant and sampler2D Input nodes.
  Filter supports Linear/Nearest; wrap supports Clamp/Repeat/Mirror. Apply rebuilds
  the NativeHost graph and takes effect on the source sampler; Undo/Redo refreshes
  both the form and output.
- Framebuffer Inputs expose all schema formats, required 1–8192 width/height, and
  optional positive-u32 stride (`blank` or `0` means auto). Video Inputs expose
  File/Camera source, free-form camera ID, Loop, and finite 0.1–4.0 playback rate.
  These forms explicitly say configuration-only because this shell does not load
  raw buffers or open video/camera resources. Changing source does not erase a
  preserved file path; all groups clone complete SDK `NodeData`.

### Read-only ONNX metadata

- Select one ONNX node to inspect its recorded source, model/catalog IDs, custom
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
  unbound ONNX output reports the missing native texture and clears the stale image.

### Native ONNX sessions

Set a compatible ONNX Runtime library **before launching** the shell:

```sh
ORT_DYLIB_PATH=/absolute/path/libonnxruntime.1.21.0.dylib \
  cargo run --release --manifest-path apps/open_quartz_slint/Cargo.toml -- --project project.json
```

Use the platform CPU library (`onnxruntime.dll`, `.dylib`, or `.so`) from an
[official native ONNX Runtime release](https://github.com/microsoft/onnxruntime/releases/tag/v1.21.0).
Missing configuration and invalid paths are reported in the live model status;
the shader shell and project I/O work without an attached session. Runtime bundling
is not automatic in this app.

- Select one ONNX node and explicitly **Load model**. A temporary local path
  overrides the saved custom path only for the current session. Without an
  override, the shell uses the custom path or the catalog download/cache planned by
  the existing Rust host-intent API; task and effective parameters use that same
  plan instead of duplicating the Web catalog/threshold logic.
- Catalog downloads stream into a unique temporary file under the shared Tauri/
  Slint `com.openquartz.app/models` cache and become visible only after completion.
  A persistent per-model advisory lock coordinates reads/downloads across processes;
  the OS releases a lock when the process exits, so a stale lock file is harmless.
  Optional catalog SHA-256 is verified, including cache hits; an empty catalog
  digest means **unverified**, not validated. Failures/cancellation remove partial
  files. Model IDs are validated before any cache or lock path is constructed.
- File reads and ORT session construction run off the UI/render threads. The live
  section reports byte progress, load errors, session backend, tensor names, and
  delivered inference output. Tasks with structured data display that real JSON;
  image-only tasks report texture output. The application never synthesizes
  detection results, claims inference GPU support, or persists a temporary path.
- **Cancel load / Unload model** invalidates the load ticket and clears the native
  output. Download cancellation is cooperative; an active ORT session constructor
  cannot be interrupted, but its late completion cannot attach after cancellation.
  Opening a project tears down its resource sessions even if a node keeps its ID.
  Edits retain a session only while the shared model/resource plan is unchanged;
  an updated task/model/path requires an explicit reload. Superseded native
  inference completions cannot publish textures/events or clear a newer task.
- Loads, progress, backend and inference results are session state, not SDK history
  or saved node data. Open never loads/downloads models implicitly. Resource
  operations are guarded by unapplied inspector drafts.
- Native inference performs GPU readback → CPU ONNX tensor execution → GPU texture
  upload. Slint presentation imports the shared GPU texture; inference itself is not
  zero-copy. Windows requests DirectML with the existing CPU fallback; macOS/Linux
  use the existing CPU execution provider.

The WGSL editor remains native `TextEdit` with explicit Apply. The macOS large-text
assessment and decision, including diagnostic/IME/platform limits, are recorded in
[`SLINT_UI_REFACTOR.md` §7.3.1](../../docs/SLINT_UI_REFACTOR.md#731-macos-原生编辑体验评估).

## Validation scope

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
All **49 Slint tests** and **280 workspace tests / 45 suites** pass; the release
build and formatting checks succeed.

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
The target macOS session later became screen-locked before the final Slint AppKit
window acceptance. **Load/cancel/unload and live panel behavior still need native
window verification after the workstation session is unlocked.**

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

Windows DX12 remains covered by the Phase 1 spike. The NativeHost boundary compile-checks
for `aarch64-apple-ios` and selects Metal there, but texture import remains unverified on
iOS hardware. Linux, Android, and mobile runtime validation are deferred.
