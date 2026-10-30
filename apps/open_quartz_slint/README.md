# OpenQuartz Native Desktop (Slint)

Phase 5 and Phase 7 are complete within the agreed macOS scope. This is the default
desktop application; the separate Web application remains independently buildable,
and Tauri remains an explicit fallback for one version. Native package relocation,
real resource dialogs and the Tauri fallback `.app` build are verified. Actual
camera capture, Windows/Linux acceptance and Phase 6 Android/iOS are deferred.
An earlier intermittent winit startup abort remains a known risk. The changes are
still Unreleased; local stage completion does not mean a tagged release was published.

The application is a standalone Rust workspace, with Slint and its build compiler
pinned to **1.18.1**, using the femtovg wgpu renderer and wgpu 30 shared-device integration.

## Run

From the repository root:

```sh
npm ci
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

The macOS ZIP was extracted outside the workspace and launched without ORT/FFmpeg
environment overrides. It used its own `Contents/Resources/runtime/ffmpeg`, rendered
real 96×48 video through Metal, and exited normally with code 0. Extracted-bundle
code signing and `--help` verification passed. `dist-desktop/` is ignored by Git.

The retained Tauri shell also builds via `npm run package:tauri -- --bundles app`;
its macOS bundle lives at the root workspace's `target/release/bundle/macos/`.

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

- The original React/Tauri light visual layout is restored in native Slint: 48px
  icon toolbar with FILE/SOURCE/MATH/SHADER/ONNX menus, a full-bleed cross-grid
  canvas, dark-blue 28px node captions, typed ports, real shared-GPU node
  thumbnails with resolution labels and status LEDs, and a 320px resizable
  sidebar. The sidebar appears on selection; semantic sections and texture
  previews collapse. Runtime and project details are available through FILE →
  RUNTIME STATUS…. At 800×540 the toolbar shortens its brand and hides the
  version to preserve actions.
- The native shader editor uses Slint TextInput, preserving platform caret,
  selection, IME, clipboard, Unicode and read-only behavior. It adds byte-safe
  line numbers, goto-line, UTF-8 find/next selection, live match counts and a
  fixed Apply action. Syntax coloring is intentionally not provided because
  Slint 1.18 has no rich editable text/span API.
- Restoration verification: 66 Slint tests pass and the native application builds.
  Real-window/shared-GPU smoke covers Shader and Renderer thumbnails, resolution
  labels, collapse/expand, categorized shader creation, read-only presets,
  Unicode drafts, live find metadata, exact GPU error status, failed-preview
  clearing and Undo recovery. AppKit accessibility checks also opened and
  cancelled native FILE → Open and Save As dialogs. A remaining limitation is
  that this automation provider cannot reliably synthesize keyboard focus for
  arbitrary TextInput key events; semantic set-value and the in-app smoke cover
  the same model updates.
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
  by another Copy or the app exits. Embedded Image and Framebuffer data URLs are
  copied with their node data; ONNX model files and video/camera sources remain
  external. Text fields retain their normal text clipboard shortcuts. Graph Copy/Paste/
  Select all are disabled during project I/O; graph shortcuts do not run during a node drag.
- `AppController` owns the SDK `Project` and updates Slint properties/models.
  Project parsing, validation, serialization, and graph mutation use the Rust SDK;
  the UI does not interpret project JSON.
- A file worker performs disk I/O, resource decoding, and SDK parsing/serialization.
  A separate render worker owns `open_quartz_native_host::NativeHost`; no execution
  engine logic is reproduced in this crate.
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

The app includes Sampling, Framebuffer, and Video configuration forms plus ONNX
model loading and inference. Image inputs accept PNG/JPEG/BMP/WebP files; raw
Framebuffer inputs accept binary files decoded using the applied format, dimensions,
and optional row stride. Image/Framebuffer resources are embedded in the SDK project
and decoded asynchronously after Open. NativeHost receives decoded RGBA textures;
errors are reported instead of replaced with sample output. Video file paths and
camera IDs are persisted and reconciled by NativeHost, with native camera enumeration,
file selection, loop, and playback rate controls. Video files and cameras remain
external resources and can fail if a file/device is missing or OS permission is
denied. Sampling applies each source node's Linear/Nearest filter and Clamp/Repeat/
Mirror wrap. Only RGBA8 host preview outputs can be imported. Mobile gestures and
mobile camera/file permission adapters are deferred with Phase 6.

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
  deletes the node. Shader/Constant nodes also expose **Auto size**, width/height
  (1–8192), and **Apply output size**. Auto size delegates resolution to the engine.
- Shader nodes expose a native multiline TextInput editor with UTF-8-safe line
  numbers, goto-line and find/next navigation, plus an **Apply shader** action.
  Built-in implementations are not source-editable. There is no syntax
  highlighting, inline GPU linter, or debounced auto-apply.
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
  NativeHost: the affected node's status LED turns red, its stale texture preview
  clears, and the runtime status remains readable. Unrelated attached ONNX nodes
  are not marked failed.
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
  File/Camera source, native file/device selection, Loop, and finite 0.1–4.0 playback
  rate. Apply persists complete SDK `NodeData`; source switches preserve the saved
  file path, and NativeHost reconciles committed source configuration. Raw buffer
  resources are decoded by the file worker rather than on the Slint UI thread.

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

`npm run dev` and packaged builds prepare the compatible **official ONNX Runtime
1.21.0** distribution automatically. They do not use the separate Node ORT version.
Windows uses Microsoft's DirectML package plus its declared DirectML 1.15.4
dependency; macOS/Linux use the CPU distribution. Source URLs and notices are bundled.

Startup resolves runtime resources beside the executable (`runtime/`) or, for a
macOS app, `Contents/Resources/runtime/`. The primary library names are
`onnxruntime.dll`, `libonnxruntime.1.21.0.dylib`, and `libonnxruntime.so.1.21.0`.
Development commands explicitly configure `ORT_DYLIB_PATH`,
`OPEN_QUARTZ_FFMPEG_PATH`, and runtime `PATH`. For direct Cargo launches, an explicit
`ORT_DYLIB_PATH` can point to an [official native 1.21.0 library](https://github.com/microsoft/onnxruntime/releases/tag/v1.21.0):

```sh
ORT_DYLIB_PATH=/absolute/path/libonnxruntime.1.21.0.dylib \
  cargo run --release --manifest-path apps/open_quartz_slint/Cargo.toml -- --project project.json
```

Missing/invalid runtime configuration is a visible error, not fabricated inference.
The shader editor and project I/O do not require a loaded model session.

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

The WGSL editor remains a native multiline `TextInput` with explicit Apply,
UTF-8-safe line/find navigation, and platform caret/selection behavior. The macOS
large-text assessment and decision, including diagnostic/IME/platform limits, are
recorded in [`SLINT_UI_REFACTOR.md` §7.3.1](../../docs/SLINT_UI_REFACTOR.md#731-macos-原生编辑体验评估).

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
