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
  status, a scrollable spatial graph, node selection and drag-to-move, connection
  paths, and renderer output.
- `GraphViewModel` projects SDK graph snapshots into node and edge models and
  computes padded scroll content bounds. Its origin offset keeps loaded nodes with
  negative coordinates reachable while preserving graph-space edge coordinates.
- Node moves are committed once on pointer release through the Rust SDK. Motions
  below the three-pixel drag threshold select instead, and unchanged positions do
  not create a graph revision. Graph interaction is disabled while asynchronous
  project I/O is busy.
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
  a render/import failure stops that graph's rendering until another open.

This phase does **not** include an Inspector, shader editing, undo/redo, graph zoom
or fit-view controls, media/resource loading, or model controls. Projects requiring
external images, video, or inference resources may open and save, but those
resources are not attached by this shell; resulting host failures are reported
instead of replaced with sample output. Only host preview outputs supported by
`NativeHost::preview_source` (currently RGBA8) can be imported.

## Validation scope

The crate contains unit coverage for spatial projection and edge endpoints,
negative-coordinate content bounds, persisted node movement, no-op revision
preservation, deterministic stress fixtures, CLI option combinations,
dash-prefixed project paths, and the 300-node stress cap.

In debug builds, the package disables debug assertions only for `wgpu-core`. Slint keeps
the manually supplied queue in its thread-local context, while wgpu-core's debug-only
recursive-lock tracer is a separate TLS value; macOS may destroy the tracer first and
otherwise abort when Slint later drops the queue. Release builds never enable that tracer.
The override does not disable assertions in OpenQuartz, Slint, or application code.

Windows DX12 remains covered by the Phase 1 spike. The NativeHost boundary compile-checks
for `aarch64-apple-ios` and selects Metal there, but texture import remains unverified on
iOS hardware. Linux, Android, and mobile runtime validation are deferred.
