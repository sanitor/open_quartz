# OpenQuartz Slint application shell

This is the Phase 3 native application shell. It is a standalone Rust workspace,
with Slint and its build compiler pinned to **1.18.1**, using the femtovg wgpu
renderer and wgpu 30 shared-device integration.

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

## Phase 3 boundary

- A real Slint window with project name, path entry, Open/Save, I/O and runtime
  status, a scrollable spatial graph, node selection and drag-to-move, typed ports,
  editable connection paths, Undo/Redo, and renderer output.
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
- **Undo/Redo** use SDK graph history for moves, connections, reconnections, and
  deletions. An identical connection is a no-op. A successful new edit clears redo;
  a failed edit does not. Opening a project resets the edit session; generated
  fixture construction is not exposed as user undo history. History is not saved.
  Topology edits and their undo/redo refresh NativeHost; layout-only history does
  not restart rendering.
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
  a render/import failure stops rendering until another open or topology edit.

This phase does **not** include an Inspector, shader editing, copy/paste, graph zoom
or fit-view controls, media/resource loading, or model controls. Projects requiring
external images, video, or inference resources may open and save, but those
resources are not attached by this shell; resulting host failures are reported
instead of replaced with sample output. Only host preview outputs supported by
`NativeHost::preview_source` (currently RGBA8) can be imported.

## Validation scope

The crate contains unit coverage for multi-port edge endpoints, spatial bounds,
persisted node movement, no-op revisions, atomic occupied-input replacement and
reconnection, failed type/cycle edits preserving graph/history, deterministic stress
fixtures, CLI option combinations, dash-prefixed paths, and the 300-node cap.

An ephemeral software-rendered Slint smoke exercised actual pointer events for
connection selection/deletion, port connection, reconnection, type rejection,
Cancel, Undo/Redo, busy guards, node dragging, and asynchronous Save/Open. Screenshots
were inspected at 1180×760 and 800×540, plus a 300-node/450-edge canvas. A separate
Apple M4 Pro/Metal smoke observed a disconnected Renderer report its missing input,
clear its image, and resume 640×360 GPU texture output after reconnect and redo.
The current macOS session exposed no native accessibility window, so these software
screenshots do not replace native-window acceptance or establish a 60 FPS result.

In debug builds, the package disables debug assertions only for `wgpu-core`. Slint keeps
the manually supplied queue in its thread-local context, while wgpu-core's debug-only
recursive-lock tracer is a separate TLS value; macOS may destroy the tracer first and
otherwise abort when Slint later drops the queue. Release builds never enable that tracer.
The override does not disable assertions in OpenQuartz, Slint, or application code.

Windows DX12 remains covered by the Phase 1 spike. The NativeHost boundary compile-checks
for `aarch64-apple-ios` and selects Metal there, but texture import remains unverified on
iOS hardware. Linux, Android, and mobile runtime validation are deferred.
