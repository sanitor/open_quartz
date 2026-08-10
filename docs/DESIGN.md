# Open Quartz 软件架构设计

> 架构基线：v0.19.0b（2026-08-10）
>
> 本文只描述当前源码中可验证的结构，并单独标出目标边界和已发现的边界漂移。
> `当前`、`遗留`、`目标`不能混写；性能结果不等同于分层正确性。

## 0. 阅读指南

本文优先回答四个问题：

1. TypeScript、WASM、Tauri 和 Rust 各自负责什么，依赖方向是什么？
2. UI 如何通过监听把意图送入 runtime，又如何监听下层状态和数据？
3. Browser 与 Native 两条执行路径在哪里共用 Rust，在哪里仍然分叉？
4. 长期优化后，原有模块切分哪些仍成立，哪些已经漂移？

建议阅读顺序：

- 第 1–3 节：系统总图、模块依赖、对象关系；
- 第 4–6 节：状态监听、JS↔Rust 接口和逐帧数据流；
- 第 7–8 节：Rust 分仓、所有权和优化后的边界审计；
- 第 9–12 节：约束、演进顺序、验证方法和源码索引。

状态标记：

| 标记 | 含义 |
|---|---|
| **当前** | 生产路径已经使用 |
| **部分** | 类型或基础设施存在，但生产路径未完整接入 |
| **遗留** | 源码仍存在，但生产入口不再使用 |
| **目标** | 应收敛到的边界，不代表已实现 |

---

## 1. 系统总览

### 1.1 系统总览：共享核心与对称目标

这一节先画**目标架构**，再画**当前实现**。两者不能合并成一张图：目标图说明两条链路应在哪里汇合，当前图说明现在仍在哪里分叉。

#### 目标架构：两条宿主进入同一个 Rust kernel

```mermaid
flowchart TB
    subgraph Clients[React clients]
        BrowserUI[Browser UI]
        TauriUI[Tauri UI]
    end

    subgraph Bindings[薄 binding / transport]
        WasmBinding[Worker + wasm-bindgen]
        TauriBinding[Tauri command / direct Rust]
    end

    subgraph Kernel[共享 Rust kernel]
        Runtime[Runtime: lifecycle / clock / delivery]
        Engine[Engine: graph execution / plan / work]
        Schema[types / errors / events]
        HostApi[host traits: GPU / media / inference / presenter]
        Runtime --> Engine
        Runtime --> HostApi
        Engine --> HostApi
        Engine --> Schema
    end

    subgraph Hosts[宿主 backend]
        BrowserHost[WebGPU / WebCodecs / ORT-Web]
        NativeHost[wgpu / FFmpeg / ORT / DXGI / WebView2]
    end

    BrowserUI --> WasmBinding --> Runtime
    TauriUI --> TauriBinding --> Runtime
    HostApi -->|Browser implementation| BrowserHost
    HostApi -->|Native implementation| NativeHost
    BrowserHost -. stamped async completion .-> Runtime
    NativeHost -. stamped async completion .-> Runtime
```

目标调用方向：

```text
Browser UI -> Worker/wasm-bindgen -> Runtime
Tauri UI   -> Tauri binding/direct Rust -> Runtime
                                      ↓
                           Engine / host traits
                                      ↓
                    WebGPU/codec/ORT 或 wgpu/FFmpeg/ORT
```

`Runtime` 和 `Engine` 是共享 policy/kernel；GPU、codec、ORT、DXGI、WebView2 是宿主实现。backend 不反向驱动 `Engine`；异步 backend 只把带 `revision`、`generation`、`FrameStamp` 的 completion 返回 Runtime。

#### 当前实现：共享基础模块，但 Runtime 和 host orchestration 仍不对称

```mermaid
flowchart TB
    subgraph UI[React 控制面]
        Components[Header / NodeGraph / SidePanel]
        Store[Zustand GraphState]
        Service[PipelineService]
        Components --> Store
        Store -->|state diff| Service
        Service -->|callback projection| Store
    end

    subgraph Shared[两条链路实际共享的 open_quartz 模块]
        Types[types]
        Graph[graph]
        Wgsl[部分 wgsl]
        Engine[engine / ExecutionPlan / ExecutionCommand]
        Types --> Graph
        Graph --> Engine
        Wgsl --> Engine
    end

    subgraph Browser[Browser current]
        BAdapter[BrowserPipelineRuntime]
        Worker[BrowserRuntimeWorker]
        BRuntime[Rust Runtime via WASM]
        BWork[drainWork / work batch]
        BHost[TS WebGPUExecutionEngine]
        BMedia[HTML video / WebCodecs]
        BOnnx[ORT-Web]
        BAdapter <-->|postMessage| Worker
        Worker --> BRuntime --> BWork --> BHost
        BHost --> BMedia
        BHost --> BOnnx
    end

    subgraph Native[Tauri current]
        NAdapter[NativePipelineRuntime]
        Tauri[Tauri commands / events]
        NThread[native render thread]
        NRuntime[NativeGpuRuntime]
        NGpu[open_quartz GpuExecutor]
        NMedia[NativeVideoSource / FFmpeg]
        NOnnx[open_quartz ONNX / ORT]
        NPresent[DXGI / TextureStream / readback]
        NAdapter <-->|invoke / listen| Tauri
        Tauri --> NThread --> NRuntime
        NRuntime --> Engine
        NRuntime --> NGpu
        NRuntime --> NMedia
        NRuntime --> NOnnx
        NRuntime --> NPresent
    end

    Service --> BAdapter
    Service --> NAdapter
    BRuntime -. uses Engine through WASM .-> Engine
```

当前实现的真实结论：

- `types`、`graph`、`engine` 和部分 `wgsl` 已由 Browser/WASM 与 Native 共同使用；
- Browser 使用 Rust `Runtime` 生成 work batch，再由 TypeScript host 执行；
- Native 尚未使用 `open_quartz::runtime::Runtime`，而是由 `NativeGpuRuntime` 自己编排 clock、Engine、GpuExecutor、video、ONNX 和 presenter；
- 所以当前是“共享 Rust 基础模块 + 两套上层 orchestration”，不是完整对称的共享 Runtime。

| Rust 模块/语义 | Browser/WASM | Tauri/native | 结论 |
|---|---|---|---|
| `types`、`graph`、`engine`、部分 `wgsl` | 通过 WASM 使用 | 直接链接使用 | 已共享 |
| `ffi`/binding | `wasm-bindgen` | Tauri command/direct Rust | transport 不同，语义应相同 |
| `runtime::Runtime` | Browser Worker 生产使用 | Native 尚未使用 | 当前最大分叉 |
| `gpu` | TypeScript WebGPU backend | Rust `GpuExecutor` | 宿主实现分叉；Browser 仍重复部分 policy |
| `media` | DOM/WebCodecs/HTML video | `NativeVideoSource`/FFmpeg | 合理的宿主分叉 |
| `onnx` | ORT-Web + TS task glue | Rust ORT/DirectML | 推理执行分叉；任务语义应共享 |
| `output/presentation` | Rust 类型存在，callback 仍占主导 | native event + presenter | 尚未统一 delivery |

后续收敛目标：

```text
Browser Worker -> wasm-bindgen -> Rust Runtime/Engine -> Browser host backend
Tauri thread   -> direct Rust  -> Rust Runtime/Engine -> Native host backend
```

两边不能共享的只应是 DOM/WebGPU/wgpu/FFmpeg/ORT session、GPU handle、window handle 和 transport；graph semantics、clock、generation、output contract 不应继续留在宿主层。

### 1.2 控制面与数据面

| 平面 | 数据 | 频率 | 所有者 |
|---|---|---:|---|
| UI intent | play/pause/stop、graph edit、selection | 低频 | React + Zustand |
| Runtime control | set graph、resource reconcile、capture | 低频/按编辑 | `PipelineService` + host adapter |
| Frame work | clock tick、dirty commands、GPU submission | 每帧 | Browser Worker 或 native render thread |
| Runtime event | frame metadata、output metadata、error | 合并/限频 | runtime → adapter → Store |
| Pixel stream | GPU texture、TextureStream、preview readback | 高频或按需 | host GPU/runtime；Store 只保存 UI 投影 |
| Persistent project | nodes、edges、资源 descriptor/path | 保存时 | Zustand/project serializer |

### 1.3 宿主选择不变量

`PipelineService.createRuntime()` 只选择一次：

```text
checkIsTauri() == false -> BrowserPipelineRuntime
checkIsTauri() == true  -> NativePipelineRuntime
```

同一会话不得同时启动两套生产 runtime，也不得把 Browser runtime 当作 Native 的隐式 fallback。平台 fallback 只能发生在宿主内部，例如 Native presentation 从 TextureStream 降级为 bounded RGBA readback。


---

## 2. TypeScript 模块依赖

### 2.1 当前依赖图

```mermaid
flowchart TD
    Components[src/components]
    Store[src/store]
    Service[src/services]
    Sdk[src/sdk]
    Worker[BrowserRuntimeWorker]
    Engine[src/engine]
    Catalog[src/catalog]
    Types[src/types]
    Utils[src/utils]
    Wasm[open_quartz WASM]
    Tauri[src-tauri IPC]

    Components --> Store
    Components --> Catalog
    Components --> Utils
    Components --> Sdk
    Store --> Types
    Store --> Catalog
    Store --> Sdk
    Store --> Engine
    Service --> Store
    Service --> Sdk
    Sdk --> Types
    Sdk --> Catalog
    Sdk --> Worker
    Worker --> Engine
    Worker --> Wasm
    Engine --> Types
    Engine --> Catalog
    Engine --> Store
    Sdk --> Tauri
```

这张图同时暴露三个现状：

1. 正常主干是 `components → store → PipelineService → sdk adapter`。
2. Browser Worker 仍依赖 `src/engine` 的 TypeScript GPU/ONNX 实现。
3. `store → engine` 和 `engine → store` 形成概念环：`graphSlice/helpers` 创建 ONNX session/model manager，而 `executionEngine.ts` 又读取 store helper。ES module 不一定形成直接运行时循环，但分层已经不纯。

### 2.2 目录责任

| 目录 | 当前责任 | 禁止新增的责任 |
|---|---|---|
| `src/components` | 展示、用户输入、节点编辑、项目菜单 | graph 调度、GPU/decoder/session 所有权 |
| `src/store` | project/UI state、播放意图、预览和错误投影 | frame clock、runtime lifecycle、GPU handle |
| `src/services` | 监听 Store、串行化 runtime 操作、将 callback 投影回 Store | host-specific GPU/ONNX 算法 |
| `src/sdk` | host adapter、WASM binding、Worker/Tauri transport、协议类型 | 第二份 graph semantics |
| `src/engine` | **当前** Browser Worker 的 WebGPU、媒体、ORT-Web adapter；包含尚未迁出的执行逻辑 | 继续扩展 topology、dirty、clock、generation 等 canonical policy |
| `src/catalog` | UI catalog 与创建节点所需 descriptor | runtime resource/session ownership |
| `src/types` | TypeScript project/node/port schema | 平台对象和运行时资源 |
| `src/utils` | project I/O、Tauri detection、preview helpers | runtime orchestration |

### 2.3 当前生产类与遗留类

| 类 | 状态 | 生产入口 | 说明 |
|---|---|---|---|
| `PipelineService` | 当前 | `App`、`ScreenSaverApp` | 唯一 Store↔runtime bridge |
| `BrowserPipelineRuntime` | 当前 | `PipelineService` | main-thread Worker proxy |
| `BrowserRuntimeWorker` | 当前 | `BrowserPipelineRuntime` | Browser clock/work/GPU host |
| `WasmRuntimeContract` | 当前 | Browser Worker | Rust `RuntimeBinding` 的 typed projection |
| `Compositor` | 当前 | Browser Worker | 包装 TS WebGPU execution engine |
| `WebGPUExecutionEngine` | 当前 | `Compositor` | Browser GPU plan/resource/ONNX execution |
| `NativePipelineRuntime` | 当前 | `PipelineService` | Tauri command/event adapter + TS resource reconcile |
| `RealtimeHost` | 遗留 | 无生产引用 | 旧 main-thread rAF host，仅测试仍直接覆盖 |
| `WasmEngineContract` | 部分 | tests/contract usage | 低层 `Engine` binding；生产 Browser 使用 `WasmRuntimeContract` |

`RealtimeHost` 不应再出现在“当前 Browser 拓扑”中。后续应删除或明确移入 legacy test fixture，避免文档和测试继续暗示它是生产入口。

---

## 3. 核心对象关系

### 3.1 跨语言类图

```mermaid
classDiagram
    class GraphState {
        nodes
        edges
        loopState
        selectedNodeId
        outputPreviews
        outputData
        nodeErrors
        fps/currentTime/currentFrame
    }

    class PipelineService {
        runtime: PipelineHostRuntime
        operations: Promise
        attach(canvas)
        detach()
        createRuntime(canvas)
    }

    class PipelineHostRuntime {
        <<interface>>
        initialize(canvas)
        play(nodes, edges)
        updateGraph(nodes, edges)
        pause()
        resume()
        stop()
        setPreviewNode(id)
        captureScreenshot(id)
        close()
    }

    class BrowserPipelineRuntime {
        worker: Worker
        pending: Map
        request(message)
    }

    class NativePipelineRuntime {
        bridge: NativeTauriBridge
        imageResources: Map
        videoResources: Map
        onnxResources: Map
        unlisten: Function[]
    }

    class BrowserRuntimeWorker {
        runtime: WasmRuntimeContract
        compositor: Compositor
        runFrame()
    }

    class WasmRuntimeContract {
        raw: RuntimeBinding
        setGraph()
        advance()
        drainWork()
    }

    class RustRuntime {
        engine: Engine
        clock: CompositionClock
        outputs: OutputRegistry
        presentation: PresentationPlanner
        resources: Map
    }

    class Compositor {
        engine: WebGPUExecutionEngine
        plan
        prepare()
        render()
        readNodeOutput()
    }

    class NativeGpuRuntime {
        engine: Engine
        executor: GpuExecutor
        videos: Map
        onnx_resources: Map
        shared_presenter
        render_next()
    }

    class Engine {
        state
        revision
        node_generations
        executor: ExecutionEngine
    }

    class GpuExecutor {
        backend: GpuBackend
        resources
        execute_commands()
    }

    GraphState --> PipelineService : subscribe / project callbacks
    PipelineService --> PipelineHostRuntime
    PipelineHostRuntime <|.. BrowserPipelineRuntime
    PipelineHostRuntime <|.. NativePipelineRuntime
    BrowserPipelineRuntime --> BrowserRuntimeWorker : postMessage
    BrowserRuntimeWorker --> WasmRuntimeContract
    WasmRuntimeContract --> RustRuntime : wasm-bindgen
    RustRuntime --> Engine
    BrowserRuntimeWorker --> Compositor
    NativePipelineRuntime --> NativeGpuRuntime : Tauri invoke/event
    NativeGpuRuntime --> Engine
    NativeGpuRuntime --> GpuExecutor
```

### 3.2 所有权表

| 对象 | Browser owner | Native owner | 是否进入 Store/项目文件 |
|---|---|---|---:|
| Graph metadata | Worker Rust Runtime + TS GPU plan | `NativeGpuRuntime::Engine` | 只保存可序列化 snapshot |
| Composition clock | Rust `Runtime::CompositionClock` | `NativeGpuRuntime` 的 `Instant/frame` | 仅投影 time/frame |
| GPU device/queue | `WebGPUBackend` | `GpuBackend` | 否 |
| GPU target/texture | `WebGPUExecutionEngine` | `GpuExecutor` | 否 |
| Video decoder | Browser engine/DOM/Web APIs | `NativeVideoSource` | 只保存 source descriptor/path |
| ONNX session | browser inference layer | `NativeOnnxResource` | 只保存 model descriptor/path |
| Output subscription registry | Rust Runtime 存在，生产未完整使用 | 未接入 Native | 否 |
| Preview image | Worker readback → data URL | bounded binary readback | Store 保存最近 UI 投影 |
| Renderer stream | Browser canvas projection | WebView2 TextureStream `MediaStream` | Store 只保存 live UI reference，不序列化 |

需要注意：Native composition clock 尚未使用 `runtime::CompositionClock`；这是共享 runtime 尚未完成的直接证据。

---

## 4. Store 监听与双向数据流

### 4.1 上层意图如何下沉

`App` 只创建一次 `PipelineService`。`PipelineService.attach()` 调用 `useGraphStore.subscribe((state, previous) => ...)`，以状态转换驱动 runtime：

```mermaid
sequenceDiagram
    participant UI as React UI
    participant Store as Zustand GraphState
    participant Service as PipelineService
    participant Runtime as PipelineHostRuntime

    UI->>Store: play()/pause()/resume()/stop()
    Store-->>Service: subscribed state + previous state

    alt stopped -> playing
        Service->>Runtime: initialize once
        Service->>Runtime: setPreviewNode(selectedNodeId)
        Service->>Runtime: play(nodes, edges)
    else playing -> paused
        Service->>Runtime: pause()
    else paused -> playing
        Service->>Runtime: resume()
    else any -> stopped
        Service->>Runtime: setPreviewNode(null)
        Service->>Runtime: stop()
    end

    opt graph changed while playing
        Service->>Runtime: updateGraph(nodes, edges)
    end

    opt selection changed while active
        Service->>Runtime: setPreviewNode(selectedNodeId)
    end
```

`PipelineService.operations` 把异步 control 操作串行化，防止 play/update/stop 交错。`generation` 防止 attach/detach 期间过期 runtime 初始化重新挂回服务。

### 4.2 下层状态如何上浮

当前上浮不是 UI 轮询 runtime object，而是 listener/callback 链：

```mermaid
flowchart LR
    Runtime[Worker event / Tauri event / TextureStream callback]
    Adapter[BrowserPipelineRuntime or NativePipelineRuntime]
    Callbacks[PipelineRuntimeCallbacks]
    Service[PipelineService handlers]
    Store[Zustand projection]
    UI[React subscribers]

    Runtime --> Adapter --> Callbacks --> Service --> Store --> UI
```

主要投影：

| 下层信号 | `PipelineService` 处理 | Store 字段 |
|---|---|---|
| frame/time/fps | 最多约 10 Hz 写 UI frame state | `fps/currentTime/currentFrame` |
| preview/output image | 忽略 stopped 后迟到结果 | `outputPreviews[nodeId]` |
| typed output data | 直接投影 | `outputData[nodeId]` |
| output size | 更新输入或输出 resolved size | node `resolvedWidth/resolvedHeight` |
| ONNX backend | 更新可观察 backend | node `onnxBackend/onnxNativeBackend` |
| node/runtime error | 归属 node 或 runtime | `nodeErrors` |
| renderer presented | 500 ms 窗口计算显示 FPS | `rendererFps` |
| TextureStream cadence | 合并 graph/present/display/drop 指标 | `rendererCadence` |

规则：Store 保存 UI 可观察结果，不接管结果资源的所有权。完整 texture、decoder frame、ORT tensor 和 per-frame command 都不能进入 Store。

### 4.3 监听模型的边界

当前监听模型有两层：

1. **应用监听**：Store state diff → runtime command。
2. **runtime 监听**：Worker/Tauri/VideoFrame callback → Store projection。

这使 UI 与底层解耦，但也带来两个必须守护的条件：

- 每个 listener 必须在 `detach()/close()` 中释放；Native 使用 `unlisten[]`，Browser 终止 Worker。
- 所有异步结果必须能识别过期状态。Native ONNX 已使用 graph revision + node generation；preview 通过当前 `previewNodeId` 再检查；通用 callback schema 尚未全部携带 stamp。

---

## 5. JavaScript ↔ Rust 接口

### 5.1 Browser：postMessage + wasm-bindgen

Browser 有两个串联边界：

```text
React main thread
  BrowserPipelineRuntime
    ⇅ structured-clone postMessage
Dedicated Worker
  BrowserRuntimeWorker
    ⇅ typed TS wrapper / JSON payload
  WasmRuntimeContract
    ⇅ wasm-bindgen
Rust RuntimeBinding -> Runtime -> Engine
```

#### Main thread ↔ Worker

请求：

| request | payload | 返回 |
|---|---|---|
| `initialize` | transferred `OffscreenCanvas` | ack |
| `play` | nodes + edges | ack |
| `update-graph` | nodes + edges | ack |
| `pause/resume/stop/close` | 无大 payload | ack |
| `set-preview` | node ID/null | ack |
| `capture` | node ID | data URL/null |

事件：`frame`、`output`、`output-size`、`output-data`、`node-error`、`backend`。

#### Worker ↔ Rust/WASM

生产 Worker 当前使用：

```text
Runtime.setGraph
Runtime.play
Runtime.advance
Runtime.drainWork
Runtime.pause/resume/stop/dispose
```

`RuntimeBinding` 还暴露 resource、output subscription、presentation、completion、delivery 和 event 方法，但 Browser Worker 尚未把这些通用 API 全部接入生产消息流。

### 5.2 Native：Tauri commands + Tauri events + TextureStream

```mermaid
sequenceDiagram
    participant TS as NativePipelineRuntime
    participant Cmd as Tauri command layer
    participant Worker as Native render thread
    participant Core as NativeGpuRuntime
    participant Event as Tauri event bus
    participant Store as PipelineService/Store

    TS->>Event: listen(frame/output/error/fallback)
    TS->>Cmd: native_gpu_initialize
    Cmd->>Core: create Engine + GpuExecutor
    Cmd->>Worker: start_worker

    TS->>Cmd: native_gpu_set_graph
    TS->>Cmd: attach video / upload image / load ONNX
    TS->>Cmd: native_gpu_play

    loop while playing
        Worker->>Core: render_next()
        Core->>Core: video upload -> Engine.run_frame -> GPU/ONNX
        Worker-->>Event: native-runtime-output
        Worker-->>Event: native-runtime-frame
        Event-->>TS: listener callbacks
        TS-->>Store: callback projection
    end
```

#### Control/query command families

| 类别 | 主要命令 | 传输 |
|---|---|---|
| lifecycle | `native_gpu_initialize/play/pause/resume/stop/close` | 小 JSON/空 payload |
| graph | `native_gpu_set_graph` | 去除资源 bytes 的 graph JSON |
| image | `native_gpu_upload_image/remove_texture` | raw RGBA body + headers |
| video | `attach/detach/video_metrics/video_devices` | descriptor/metadata |
| ONNX | `capabilities/load_model/unload_model` | model ID/path/options |
| output | `read_output/read_preview` | binary width+height+RGBA response |
| presentation | `set/take/release_shared_texture` | handles/lease metadata |
| diagnostics | `render_once/drain_events/set_mouse` | 小 payload |

#### Push events

| event | payload | 目的 |
|---|---|---|
| `native-runtime-frame` | frame/revision/output node/size | frame heartbeat 与 presentation trigger |
| `native-runtime-output` | ONNX output metadata/data/backend | typed output projection |
| `native-runtime-error` | string | runtime failure |
| `native-runtime-presentation-fallback` | string | 关闭 TextureStream 并启动 readback fallback |

#### Pixel presentation 是独立通道

Windows 主显示路径不是 Tauri JSON event：

```text
GpuOutputHandle
  -> DxgiSharedTextureExporter (leased slot)
  -> main-thread WebView2 present_shared_frame
  -> chrome.webview.getTextureStream("open-quartz-renderer")
  -> MediaStream + HTMLVideoElement
  -> Renderer DOM slot
```

不支持或失败时：

```text
native-runtime-frame event
  -> native_gpu_read_preview(max 960)
  -> binary RGBA
  -> temporary canvas/ImageData
  -> Renderer mirror canvas
```

显式 SAVE/screenshot 使用 `native_gpu_read_output`，不受 preview 尺寸限制。

### 5.3 接口不一致清单

| 语义 | Browser | Native | 结论 |
|---|---|---|---|
| canonical `Runtime` | 生产使用 | 未使用 | **不一致** |
| lifecycle/clock | Rust `Runtime` | Tauri `NativeGpuRuntime` | **重复** |
| graph engine | Rust `Engine` | Rust `Engine` | 共享 |
| work execution | TS `WebGPUExecutionEngine` | Rust `GpuExecutor` | host-specific，但 Browser 仍含 policy |
| resource reconcile | TS browser engine | `NativePipelineRuntime` + `NativeGpuRuntime` | **分散** |
| output subscription | Rust API 存在，Worker 未完整接入 | 未接入 | **部分** |
| state delivery | Worker custom events | Tauri custom events | **两个 schema** |
| presentation | canvas/readback | TextureStream/readback | 合理的平台差异 |

目标不是消灭 transport 差异，而是让 transport 机械投影同一组 runtime command/event schema。

---

## 6. 逐帧执行数据流

### 6.1 Browser frame

```mermaid
flowchart TD
    Timer[Worker setTimeout loop]
    Advance[Wasm Runtime.advance now/date/mouse/resolution]
    Clock[Rust CompositionClock]
    Engine[Rust Engine / ExecutionEngine]
    Work[JSON ExecutionCommand batch]
    Compositor[TS Compositor]
    TsEngine[TS WebGPUExecutionEngine]
    Gpu[WebGPUBackend]
    Onnx[ORT-Web async task]
    Preview[bounded readback/data URL]
    Event[Worker event]

    Timer --> Advance --> Clock --> Engine --> Work
    Work --> Compositor --> TsEngine --> Gpu
    TsEngine --> Onnx
    Onnx -. completion schedules frame .-> Timer
    Gpu --> Preview --> Event
    Event -. callback projection .-> Store[Zustand]
    Gpu --> Timer
```

当前关键切分：Rust 决定 clock 和 dirty command batch；TypeScript `WebGPUExecutionEngine.prepare()` 仍建立自己的 plan/resources，并执行 shader、Math/ONNX 相关 host work。因此 Browser 不是纯 backend executor。

### 6.2 Native frame

```mermaid
flowchart TD
    Tick[render thread about 16 ms]
    Video[NativeVideoSource upload_latest]
    Completion[drain ONNX completions]
    Engine[Engine.run_frame]
    Commands[ExecutionCommand batch]
    Split{command kind}
    Ort[spawn native ORT worker]
    Gpu[GpuExecutor.execute_commands]
    Output[GpuOutputHandle]
    Presenter[SharedTexturePresenter]
    Meta[native-runtime-frame/output]

    Tick --> Video --> Completion --> Engine --> Commands --> Split
    Split -->|onnx| Ort
    Split -->|shader/math/renderer| Gpu
    Ort -. revision+generation completion .-> Completion
    Gpu --> Output
    Output --> Presenter
    Output --> Meta
    Presenter --> TextureStream[WebView2 TextureStream]
    Meta --> Listener[NativePipelineRuntime listeners]
```

Native ONNX 当前需要 `GpuExecutor.read_output_rgba()`，在线程中执行 ORT，再 `upload_rgba()` 回 GPU；capability 明确为 tensor `cpu-copy`，不得描述成 shared wgpu/DirectML zero-copy。

### 6.3 Graph、资源和结果三条数据链

| 链 | 内容 | 生命周期 |
|---|---|---|
| Graph chain | node/edge/port/params | revision 管理；可持久化 |
| Resource chain | image/video/model descriptor → live object | node generation + host ownership；不可持久化对象 |
| Result chain | scalar/json/ROI/texture/presentation | output generation/stamp；Store 仅持 UI projection |

任何优化都必须说明自己改变的是哪条链。把 resource bytes 塞回 graph、把 GPU handle 写进 Store、或用 frame heartbeat 代替 typed output contract，都属于边界回退。

---

## 7. Rust 分仓与依赖关系

### 7.1 实际 package 边界

Rust 不是一个 Cargo workspace 内的多个业务 crate；当前是三个独立 package/宿主边界：

| package | 路径 | 责任 |
|---|---|---|
| `open_quartz` | `crates/open_quartz` | 可复用 Rust SDK、WASM binding、graph/engine/GPU/ONNX primitives |
| `app` | `src-tauri` | Tauri shell、native runtime 组合、FFmpeg、WebView2、screen saver export |
| `open-quartz-screensaver-stub` | `crates/open-quartz-screensaver-stub` | 轻量 `.scr` launcher/config host |

依赖方向：

```mermaid
flowchart LR
    App[app / src-tauri] --> SDK[open_quartz]
    Stub[screen saver stub] -. launches installed app .-> App
    SDK -. no dependency .-> App
    Stub -. no Rust link dependency .-> SDK
```

这里的“分仓”主要指 `open_quartz` crate 内模块分层，而不是 Cargo crate 间的强制编译边界。模块可见性较宽，靠文档约束不足以阻止逆向依赖。

### 7.2 `open_quartz` 当前模块图

```mermaid
flowchart TD
    Types[types]
    Graph[graph]
    Wgsl[wgsl]
    Engine[engine]
    Runtime[runtime]
    Gpu[gpu]
    Media[media]
    Onnx[onnx]
    Ffi[ffi]

    Graph --> Types
    Wgsl --> Types
    Wgsl --> Gpu
    Engine --> Types
    Engine --> Graph
    Engine --> Wgsl
    Gpu --> Engine
    Gpu --> Wgsl
    Media --> Gpu
    Runtime --> Types
    Runtime --> Ffi
    Ffi --> Types
    Ffi --> Graph
    Ffi --> Wgsl
    Ffi --> Engine
    Ffi --> Runtime
    Ffi --> Onnx
```

上图是源码 `use crate::...` 的实际方向，不是目标图。两个明显环：

- `runtime ↔ ffi`：`Runtime` 使用 `ffi::Engine/SdkError/EngineState`，而 `ffi::RuntimeBinding` 又包装 `runtime::Runtime`。
- `wgsl ↔ gpu`：GPU executor 使用 WGSL binding descriptor，而 WGSL compiler 又从 GPU 模块读取 fullscreen vertex 常量。

Rust 编译器允许同 crate 模块环，但这种方向会阻碍未来拆 crate 和稳定 SDK kernel。

### 7.3 目标模块图

```mermaid
flowchart TD
    Schema[types + error/event schema]
    Graph[graph]
    Wgsl[wgsl]
    Engine[engine]
    Runtime[runtime]
    HostTraits[host traits: gpu/media/inference/presenter]
    NativeImpl[native implementations]
    Binding[ffi/wasm/c bindings]
    Tauri[Tauri shell]

    Graph --> Schema
    Wgsl --> Schema
    Engine --> Schema
    Engine --> Graph
    Engine --> Wgsl
    Runtime --> Schema
    Runtime --> Engine
    Runtime --> HostTraits
    NativeImpl -. implements .-> HostTraits
    Binding --> Runtime
    Tauri --> Binding
    Tauri --> NativeImpl
```

目标规则：

1. schema/error/event 不依赖 binding。
2. WGSL compiler 不依赖 GPU implementation；共享 shader source 放 WGSL 或独立 shader module。
3. media contract 不依赖 concrete `GpuOutputHandle`；通过 opaque handle/host trait 连接。
4. `ffi` 只依赖 runtime，不得被 runtime 反向依赖。
5. Tauri 不拥有第二套 scheduler/resource reconciliation/output policy。

### 7.4 Rust 核心对象所有权

| 对象 | 当前定义 | 当前 owner |
|---|---|---|
| `ExecutionEngine` | `open_quartz::engine` | `ffi::Engine` 内部 |
| `Engine` | `open_quartz::ffi` | Browser `Runtime` 或 Native `NativeGpuRuntime` |
| `Runtime` | `open_quartz::runtime` | Browser WASM production；Native 未使用 |
| `GpuExecutor` | `open_quartz::gpu` | `src-tauri::NativeGpuRuntime` |
| `OnnxSession` | `open_quartz::onnx` | `src-tauri::NativeOnnxResource` |
| `NativeVideoSource` | `src-tauri::native_video` | `src-tauri::NativeGpuRuntime` |
| `SharedTexturePresenter` | `open_quartz::gpu` | `src-tauri::NativeGpuRuntime` |
| `TextureStream` bridge | `src-tauri::webview_texture_stream` | Tauri main thread/WebView2 |

`Engine` 位于 `ffi` 而承担核心状态，是当前命名和依赖反转的根源之一。目标应把核心 `Engine`/errors/events 移到 kernel 层，binding 只做导出。

---

## 8. 优化后的边界审计

### 8.1 审计结论

性能优化没有破坏 GPU object 的基本所有权，但多次在 Tauri facade 和 TypeScript adapter 中加入 orchestration，使“薄宿主 + 厚 Rust Runtime”的原目标没有成立。现状是：**数据路径优化成功，控制路径分层部分漂移。**

### 8.2 逐项审计

| 边界 | 原意 | 当前证据 | 状态 | 处理 |
|---|---|---|---|---|
| UI ↔ runtime | component 不直接驱动 runtime | App 只创建 `PipelineService`；常规 graph/play 经 Store 监听 | 保持 | 继续守护；screen saver 专用 invoke 可留在专用 UI |
| Store ↔ runtime | Store 只保存 intent/projection | GPU/decoder/ORT 未进入 Store；但 Store helper 持 model/session manager | 部分漂移 | 将 model/session lifecycle 移出 Store |
| Service ↔ adapter | `PipelineService` 是唯一通用桥 | Browser/Native 都实现 `PipelineHostRuntime` | 保持 | 扩展接口时先定义 canonical command/event |
| Browser main ↔ Worker | 主线程不做逐帧执行 | runtime/Compositor 在 Worker；main 只收事件和绘制投影 | 保持 | 避免把 GPU work 搬回主线程 |
| Rust Runtime ↔ host | 两端共用一个 runtime policy | Browser 使用 `Runtime`；Native 直接使用 `Engine` + 自有 clock/scheduler | **破坏** | Native 先切到 `Runtime`，再删除 facade policy |
| Engine ↔ GPU | Engine 生成 command，GPU executor 只执行 | Native `GpuExecutor` 注释和实现符合；Browser executor 仍自行 prepare/policy | 部分 | Browser backend 收敛为 command executor |
| Graph ↔ resource | graph 只传 metadata | Native strip payload 后独立同步 image/video/ONNX | 保持 | 将 sync policy 从 TS adapter 下沉 runtime |
| Runtime ↔ FFI | binding 依赖 core | `runtime` 反向依赖 `ffi::Engine/SdkError` | **破坏** | 把 Engine/error/event 移出 ffi |
| WGSL ↔ GPU | compiler 不依赖 backend | WGSL compiler 依赖 GPU fullscreen shader 常量 | **破坏** | 常量移入 WGSL/shared shader module |
| Media ↔ GPU | media 定义 frame contract | `media` 直接依赖 `GpuOutputHandle` | 部分 | importer 输出 opaque resource handle/trait |
| Output observation | 所有 output 使用 subscription/delivery | Rust registry 已实现；生产仍用 custom frame/output callbacks | **未切换** | 用统一 delivery batch 替代 host 特例 |
| Presentation | graph output 与显示 transport 分离 | Native presenter/mailbox 已分离；选择仍围绕单 `output_node_id` | 部分 | 接入 `PresentationPlanner/Set` 与多订阅 |
| Native video optimization | decoder surface 不经 CPU | Windows D3D12VA→wgpu YUV import；camera/non-Windows fallback 明确 | 保持 | 不把平台实现提升为 core 依赖 |
| TextureStream optimization | 高频 pixels 不走 IPC | shared texture + WebView2 stream；event 只发 metadata | 保持 | transport 留在 host 层 |
| ONNX async | completion 必须防 stale | Native 检查 revision + generation | 保持 | 改用通用 `AsyncCompletionEnvelope` |
| Legacy host | 单一 Browser production host | `RealtimeHost` 无生产引用但测试仍依赖 | 漂移/债务 | 删除或隔离 legacy tests |

### 8.3 最大的四个结构风险

#### A. Native runtime 是第二个 runtime

`src-tauri/src/native_runtime.rs` 同时负责：

- device/executor 创建；
- clock 和 16 ms worker；
- graph/resource reconcile；
- video lifecycle；
- ONNX scheduling/completion；
- output selection；
- presentation；
- event emission；
- smoke/benchmark。

这已经超出“薄 Tauri adapter”。文件体积不是问题本身；问题是它拥有与 `open_quartz::runtime::Runtime` 重复的 policy。

#### B. Browser 仍有第二份 execution semantics

Rust `Runtime.advance()` 生成 `ExecutionCommand`，但 `WebGPUExecutionEngine.prepare()` 仍执行 topology/plan/resource/ONNX 相关逻辑，并依赖 Store/Catalog。若 Rust plan 与 TS plan 漂移，command batch 可能在错误资源图上执行。

#### C. 通用 output contract 尚未成为生产观察边界

`OutputKey/OutputState/OutputSubscription/OutputDeliveryBatch` 已存在，但 UI 生产路径仍依赖：

```text
onFrame / onOutput / onOutputSize / onOutputData / onBackendDetected
native-runtime-frame / native-runtime-output
```

这些 callback 有用，但应成为通用 delivery 的 UI projection，而不是并列的 canonical protocol。

#### D. 同 crate 模块环隐藏了分层错误

`runtime ↔ ffi` 和 `wgsl ↔ gpu` 在单 crate 内能编译，因此常规测试不会阻止方向恶化。需要架构测试或拆 crate 才能形成编译期边界。

---

## 9. 必须守护的架构规则

### 9.1 允许依赖

```text
components -> store/actions
PipelineService -> PipelineHostRuntime
host adapter -> canonical Runtime binding + platform transport
binding -> Rust Runtime
Rust Runtime -> engine/schema/host traits
platform implementation -> host traits + platform APIs
```

### 9.2 禁止依赖

- component 直接调用 graph/native GPU command；screen saver export/config 等独立 shell capability除外。
- Store 保存 GPU texture、decoder frame、ORT session/tensor 或 per-frame command。
- `runtime` 依赖 `ffi`、Tauri、DOM 或具体 WebView event name。
- WGSL parser/compiler 依赖具体 GPU backend。
- Tauri shell 重建 composition clock、subscription registry、generation 或 presentation policy。
- Browser TS engine 重建 Rust 已定义的 topology/dirty/feedback semantics。
- 高频 pixel/frame 数据通过 JSON event 往返。
- 将目标接口写成“当前已实现”。

### 9.3 Change review 必答题

每个 runtime/性能 PR 必须回答：

1. 新状态由谁拥有？
2. 它属于 graph、resource 还是 result chain？
3. Browser 和 Native 是否产生第二套同义 policy？
4. 跨 JS/Rust 的是 command、event、stream 还是 bytes？频率和大小是多少？
5. listener/worker/session 在 stop/close/revision change 时如何释放或失效？
6. 是否改变 revision、generation、frame/content stamp？
7. fallback 是否通过 capability 显式可观察？
8. 是否新增逆向模块依赖？

---

## 10. 收敛顺序

### Phase 1：修正 Rust 内部依赖方向

1. 把 `Engine`、`EngineState`、`EngineEvent`、`SdkError` 从 `ffi` 移到 core schema/engine 层。
2. 让 `ffi` 单向包装 `runtime`。
3. 把 fullscreen WGSL 常量移出 `gpu`。
4. 把 media contract 对 concrete GPU handle 的依赖改为 host trait/opaque resource。

验收：目标模块图无 `runtime → ffi`、`wgsl → gpu`、`media → concrete gpu`。

### Phase 2：Native 使用 canonical Runtime

1. `NativeGpuRuntime` 内部从 `Engine` 切到 `Runtime`。
2. composition clock、lifecycle、revision/generation、async completion、output/presentation policy 由 Runtime 负责。
3. Tauri 保留 wgpu/FFmpeg/ORT/WebView2 host implementation 和 command/event transport。

验收：Browser/Native 对同一 control surface 有一一对应的方法和状态转换。

### Phase 3：统一 output delivery

1. UI 选择、Renderer、Math、ONNX 都注册 `OutputSubscription`。
2. custom output callbacks 改为 `OutputDeliveryBatch` 的投影。
3. frame heartbeat 只报告 cadence，不携带 output semantics。
4. preview/capture/native-present 成为 transport policy，不是节点特例。

验收：新增 output type 不需要新增 Tauri event name 或 `PipelineRuntimeCallbacks` 字段。

### Phase 4：Browser backend 变薄

1. Rust Runtime 生成完整 per-port work/resource reconciliation contract。
2. Browser TS 只持 WebGPU/WebCodecs/ORT-Web 对象并执行 work。
3. 删除 TS topology/dirty/clock/generation 的重复实现。
4. 删除或隔离 `RealtimeHost`。

验收：同一 graph 的 Browser/Native work batch contract 可比较，TS engine 不再依赖 Store。

### Phase 5：用编译边界固化分仓

只有在前四步完成后再考虑拆 crate：

```text
open_quartz_schema
open_quartz_kernel
open_quartz_host_api
open_quartz_native_gpu
open_quartz_bindings
```

不要先机械拆 crate；当前环依赖未清除时拆分只会制造 facade 和 re-export。

---

## 11. 性能与正确性不变量

### 11.1 高频路径

Browser：

```text
Worker timer -> Rust Runtime.advance -> work batch -> WebGPU submit
```

Native：

```text
render thread -> video upload -> Engine/Runtime -> GpuExecutor -> presenter
```

两条路径都禁止每帧从 UI 发 command。

### 11.2 Copy budget

| 路径 | 当前 copy 特征 |
|---|---|
| Browser shader | GPU texture/render pass |
| Browser preview | 显式 bounded GPU readback → data URL |
| Native Windows file video | D3D12VA YUV surface import；无 CPU decoded RGBA 主路径 |
| Native ONNX | GPU readback RGBA → CPU/DirectML ORT → GPU upload |
| Native Renderer TextureStream | GPU shared texture bridge；无 CPU pixel IPC |
| Native fallback preview | bounded RGBA binary readback |
| Native screenshot | 显式 full-resolution RGBA readback |

### 11.3 生命周期

- graph revision 改变时，旧 async completion 不得发布。
- node generation 改变时，旧 resource/result 不得复用。
- position-only edit 不应重建 GPU/feedback resource。
- STOP/close 必须停止 worker/listener/video/session/presenter lease。
- TextureStream fallback 不得启动第二个 graph runtime。

---

## 12. 架构验证与源码索引

### 12.1 验证方法

文档更新后至少执行：

1. Mermaid fence 成对、图类型可解析；
2. 文档中的路径、类名、command/event 名可在源码定位；
3. TypeScript diagnostics 无新增问题；
4. 若只改文档，不需要重复运行 GPU/模型/发布测试。

长期应增加两类自动检查：

- dependency policy：禁止 `runtime -> ffi` 等逆向边；
- protocol parity：Rust public surface、WASM binding、Worker protocol、Tauri adapter 的方法/事件映射一致。

### 12.2 关键源码入口

| 主题 | 源码 |
|---|---|
| App/service 挂载 | `src/App.tsx` |
| Store 监听桥 | `src/services/PipelineService.ts` |
| Host facade | `src/sdk/PipelineRuntime.ts` |
| Browser main-thread adapter | `src/sdk/BrowserPipelineRuntime.ts` |
| Browser Worker | `src/sdk/BrowserRuntimeWorker.ts` |
| Worker protocol | `src/sdk/browserWorkerProtocol.ts` |
| WASM binding projection | `src/sdk/WasmSdkClient.ts` |
| Browser compositor/GPU | `src/engine/compositor.ts`、`src/engine/executionEngine.ts` |
| Native TS adapter/listeners | `src/sdk/NativePipelineRuntime.ts` |
| Tauri command registration | `src-tauri/src/lib.rs` |
| Native runtime/worker/events | `src-tauri/src/native_runtime.rs` |
| Native video | `src-tauri/src/native_video.rs` |
| WebView2 TextureStream | `src-tauri/src/webview_texture_stream.rs` |
| Rust public modules | `crates/open_quartz/src/lib.rs` |
| Runtime policy | `crates/open_quartz/src/runtime` |
| Engine/plan/commands | `crates/open_quartz/src/engine` |
| Native GPU/presenter | `crates/open_quartz/src/gpu` |
| ONNX | `crates/open_quartz/src/onnx` |
| Binding | `crates/open_quartz/src/ffi` |

### 12.3 当前基线总结

- **已成立**：单一 `PipelineService` 桥、Store listener 驱动、Browser Worker、共享 Rust graph/engine、Native GPU ownership、异步 ONNX stale protection、GPU TextureStream 主显示路径。
- **部分成立**：Rust Runtime、output subscription、presentation planner、media host contract。
- **尚未成立**：Browser/Native 统一 Runtime、统一 delivery schema、薄 Tauri adapter、纯 host-only Browser engine、无环 Rust module dependency。

这份结论是后续重构的起点。任何优化若改变上述状态，必须同时更新本文件的总图、依赖图、接口表和边界审计。
