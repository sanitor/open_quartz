# Slint render spike

Phase 1 proof for the [Slint UI refactor](../../docs/SLINT_UI_REFACTOR.md): verify that
Slint 1.18.1 can render with the same wgpu 30 `Device`/`Queue` as the real
`open_quartz_execution` renderer and import its output texture through
`slint::Image::try_from`.

This is an isolated Cargo workspace with its own lockfile and target directory. It uses the
main workspace's `open_quartz_execution` and `open_quartz_schema` crates by path, but does
not change the production Tauri app.

## Scope

- Creates its own wgpu 30 instance/adapter/device/queue.
- Forces DX12 on Windows; uses primary backends elsewhere.
- Builds a real execution graph: `Time` input -> animated WGSL shader -> `Renderer`.
- Runs `ExecutionEngine` and `GpuExecutor` on the shared device/queue every UI tick.
- Imports the actual `Rgba8Unorm` renderer output into `slint::Image`.
- Auto-closes with `--seconds N`, or runs until the window closes.
- `--stress` cycles output sizes 640x360 -> 960x540 -> 1280x720 every 2 s,
  minimize/restore every 4 s, and fullscreen every 7 s while rendering.
- `--solid` replaces the animated shader with `vec4f(0.25, 0.5, 0.75, 1.0)` for color probes.

It does **not** yet prove:

- production native-host wiring, project loading, or command/event flow;
- device-loss/surface-recovery behavior (minimize/restore is covered);
- macOS/Linux/Android/iOS behavior;
- performance parity with the Tauri app.

## Run

```powershell
cargo run --release -- --seconds 10
cargo run --release -- --seconds 20 --stress
cargo run --release -- --seconds 5 --solid
```

Expected output on Windows:

```text
adapter: ... | backend=Dx12 | type=DiscreteGpu
slint: wgpu 30 backend selected with manual device/queue
PASS: imported open_quartz_execution output 640x360 Rgba8Unorm into slint::Image
spike: frames=... elapsed=... fps=... cpu_avg=...ms cpu_max=...ms size=640x360
spike: completed ... frames in ...s avg_fps=... avg_cpu=...ms max_cpu=...ms validation_errors=0
```

With `--stress`, the log also contains:

```text
stress: output resize -> 960x540
stress: minimized=true
stress: fullscreen=true
```

![Windows DX12 execution output](artifacts/slint-phase1b-execution-windows-dx12.png)

## Verified

Environment: Windows, NVIDIA GeForce RTX 3050 Ti Laptop GPU, DX12, Rust 1.96.1,
Slint 1.18.1, wgpu 30.0.1.

- [x] `cargo check`
- [x] Slint selects wgpu 30 with a manually supplied `Device`/`Queue`
- [x] `ExecutionEngine` + `GpuExecutor` run on the shared device/queue
- [x] `Image::try_from` accepts the real renderer output
- [x] Texture updates are visible frame over frame
- [x] Output resize 640x360 -> 960x540 -> 1280x720, with a new graph/target per size
- [x] Minimize/restore and fullscreen toggles while the frame timer is running
- [x] Solid-color probe: shader `0.25/0.5/0.75` reads back as `64,127,191`
- [x] Window auto-closes and process exits cleanly
- [x] No validation error in 5-second, 20-second stress, or 10-minute runs
- [x] Screenshot captured

A 5-second release run observed 288 frames in 5.01 s (~57 FPS) on a 640x360 output. The
20-second stress run observed 1193 frames, 59.6 average FPS, 0.68 ms average CPU per frame
(21.19 ms max, first frame), and 0 validation errors. The 10-minute steady run observed
36682 frames, 61.1 average FPS, 0.36 ms average CPU per frame (16.88 ms max, first frame),
and 0 validation errors. This is a 16 ms UI-timer loop, not a renderer benchmark.

## Findings

`unstable-wgpu-30` alone is not enough with Slint's default femtovg setup. Without the
`renderer-femtovg-wgpu` feature, Slint keeps the OpenGL renderer and fails at runtime with:

```text
WGPU 30.x rendering is not supported with an OpenGL renderer
```

The spike therefore enables both features:

```toml
slint = { version = "=1.18.1", features = ["renderer-femtovg-wgpu", "unstable-wgpu-30"] }
```

Slint's web stack requires newer `wasm-bindgen`/`js-sys`/`web-sys` versions than the main
workspace previously pinned. The execution and SDK crates now pin `wasm-bindgen = "=0.2.129"`;
`cargo check --target wasm32-unknown-unknown` still passes.

The renderer output is `Rgba8Unorm`, which `Image::try_from` accepts as long as the texture
has both `TEXTURE_BINDING` and `RENDER_ATTACHMENT` usage. Both requirements are satisfied by
`open_quartz_execution` render targets. Slint also supports `Rgba8UnormSrgb` imports; float
formats are rejected by `Image::try_from`.

The color probe matters for correctness. A shader output of `(0.25, 0.5, 0.75)` appears as
`(64, 127, 191)` in the composited window, i.e. the raw 8-bit values. Slint does not apply an
extra linear-to-sRGB conversion when sampling the imported `Rgba8Unorm` texture, so graph
output must already be sRGB-encoded bytes. The current renderer does exactly that, matching
the Tauri presentation path.

## Next

1. Extract the Tauri-free native host and drive a real project graph through Slint.
2. Add device-loss/surface-recovery validation and a long stress run.
3. Validate macOS/Linux/Android/iOS, then decide whether to enter Phase 3.
