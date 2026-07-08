# OpenQuartz — ONNX 节点设计 (webonnx)

> Version: draft-1 (2026-07-08)
> Author: caozs
> Scope: 为 OpenQuartz 引入 `onnx` 节点，支持在 WebGL/WebGPU 有向无环图中嵌入 ONNX 推理，并落地 `yolo-detector` 为示例节点 crate（sampler2D → roi + sampler2D overlay）。

---

## 1. 目标与非目标

### 1.1 目标

1. 允许 OpenQuartz DAG 中出现"节点内部持有 ONNX 模型"的推理节点，模型随 app 本地分发（不走 CDN，不走 IPC）。
2. 保持既有 `shader / input / constant` 三种节点不变，`onnx` 是**平级**的新节点类型，与它们共用 React Flow 画布、连线校验、Undo/Redo、工程文件保存/载入。
3. 推理产物类型统一表达：既能是 `roi`（bounding box 列表）、`mesh`、`json` 等非 GLSL 数据，也能是 `sampler2D`（可视化后的 texture），供下游 shader 消费。
4. 首个落地：`rust/crates/yolo-detector/` — 输入 `sampler2D`，输出 `roi` + `sampler2D overlay`。
5. 复用 [`caozisheng/rimeflow-yolov8n`](https://github.com/caozisheng/rimeflow-yolov8n) 上游 crate 的 `postprocess` 和 `ort_bridge`（`wasm_bindgen(inline_js)` → `onnxruntime-web`），仅在本地做一个薄的 `yolo-detector` crate 暴露 `YoloDetectorWasm` façade。
6. **前向兼容**：当 OpenQuartz 未来把 WebGL 换成 WebGPU (`WebGPURenderer`) 或者切到 Tauri native (`ort` Rust crate)，节点契约与 Rust 后处理零改动，仅替换推理层。

### 1.2 非目标

- 训练；节点只做推理。
- 支持全部 ONNX opset；受限于 `onnxruntime-web` 覆盖度，实际支持面等同 ORT-Web 1.22+。
- 多帧时序追踪的 UI 编排（ByteTrack 只作为 Rust 库能力保留，MVP 单帧调用一次）。
- 端到端 zero-copy 到 GLSL：详见 §4。

---

## 2. 术语

| 术语 | 含义 |
| --- | --- |
| ORT | `onnxruntime-web`，浏览器 ONNX 运行时。 |
| ORT-EP | ORT Execution Provider（`webgpu` / `jsep` / `wasm`）。 |
| 桥接层 | 上游 `rimeflow-yolov8n` crate 的 `ort_bridge` 模块，`wasm_bindgen(inline_js)` 内嵌 JS。 |
| 逻辑类型 | `roi` / `mesh` / `json` 等 GLSL 不可绑定但节点之间可传递的数据类型。 |
| GLSL 类型 | `float / vec2 / … / sampler2D`，可被 shader 编译器直接绑定。 |

---

## 3. 架构总览

```
┌────────────────────────── OpenQuartz DAG ──────────────────────────┐
│                                                                    │
│  input(sampler2D)  ──►  onnx(yolov8n)  ──roi──►  shader(overlay)   │
│                              │                                     │
│                              └──sampler2D──►  shader(mask)         │
│                                                                    │
└────────────────────────────────────────────────────────────────────┘
         │                        │                       │
         ▼                        ▼                       ▼
   Three.js texture      OnnxSession (TS)          RawShaderMaterial
   (FBO / Image)         │                         (glsl 300 es)
                         ├── loadModel(url)
                         ├── loadWasm(pkg)   ◄── rust/crates/yolo-detector
                         └── run(canvas) ──► Rust wasm_bindgen 桥接
                                             ├── inline_js
                                             │   └── ORT WebGPU EP
                                             │       (fallback wasm)
                                             └── postprocess::nms
                                                 bytetrack::update
```

数据流最短路径：

```
sampler2D → readTargetToCanvas → ort_detect(canvas) → Float32Array
   → Rust decode_yolo_output + nms → Detection[] → JS 序列化
   → [roi 端口，直接推给下游]
   → [overlay 端口，drawImage + drawBox 到 scratch canvas → Three.Texture → FBO]
```

---

## 4. 与 rimecut 方案的差异（关键设计取舍）

| 维度 | rimecut (原文 §关键技术点) | OpenQuartz | 理由 |
| --- | --- | --- | --- |
| 图像来源 | `HTMLCanvasElement` compositor canvas | `WebGLRenderTarget` FBO 或 `Image` texture | 复用现有渲染管线 |
| 底层 GPU | `wgpu` (WebGPU backend) | Three.js `WebGLRenderer` (v0.5.0b) | 现状 |
| ORT EP | `webgpu`，共享 wgpu 的 `GPUDevice` | `webgpu` **不共享 device**（Three.js WebGL 无 device 可共享），或回退 `wasm` | 见下方 4.1 |
| `capture_webgpu_device()` monkey-patch | 必需 | **省略**，因为无 wgpu | 见下方 4.1 |
| `copyExternalImageToTexture(canvas)` | GPU→GPU，同设备 | GPU→浏览器共享→GPU，跨设备（ORT 自建 GPUDevice） | 见下方 4.1 |
| CPU readback | 只在 fallback 生效 | fallback + 显式路径均可用 | MVP 允许 CPU 路径 |
| 后处理 (decode + NMS + ByteTrack) | Rust，纯 CPU | **同源码复用** ✅ | 零移植 |
| 分发 | ORT 文件复制到 `public/ort/` | 相同 | 直接复用 |

### 4.1 关于 "webgpu + webonnx zero-copy"

用户请求里的 "webgpu+webonnx" 需要拆解：

- **onnxruntime-web 的 WebGPU EP**：`ort.env.webgpu.device` 可注入外部 `GPUDevice`。若不注入，ORT 会 `navigator.gpu.requestAdapter().requestDevice()` 自建。
- **Three.js v0.185 `WebGLRenderer`** 用的是 WebGL2，**不产出 `GPUDevice`**，因此无法与 ORT 共享设备；能做到的最好是把 FBO 内容送到 `OffscreenCanvas`，然后 `device.queue.copyExternalImageToTexture(source)` — 这是**浏览器合成器层的 GPU→GPU 拷贝**，仍旧比 CPU readback 快，但不是"同一设备零拷贝"。
- **只有当 OpenQuartz 迁移到 Three.js `WebGPURenderer` (r167+) 或原生 `wgpu`**，才能像 rimecut 那样 monkey-patch `GPUAdapter.prototype.requestDevice` 并共享设备。

因此本设计文档给出**三档路径**：

| 档位 | 何时启用 | GPU 拓扑 |
| --- | --- | --- |
| **A. CPU 路径 (MVP 保底)** | 无 WebGPU、或用户强制 | `readRenderTargetPixels → putImageData → new OffscreenCanvas → ORT wasm EP` |
| **B. 浏览器桥接 GPU 路径 (MVP 首选，若 `navigator.gpu` 可用)** | 浏览器 & 设备支持 WebGPU | `WebGL FBO → canvas → copyExternalImageToTexture(ORT 自建 device) → ORT WebGPU EP` |
| **C. 同设备零拷贝 (未来)** | OpenQuartz 换 `WebGPURenderer`  | rimecut eidon 模式：monkey-patch + 共享 `GPUDevice` + `Tensor.fromGpuBuffer` |

A、B 由上游 `rimeflow-yolov8n` crate 的 `ort_bridge` 覆盖（内部按 `_capturedDevice`/`navigator.gpu` 分派）；C 只是**追加**一个 `capture_webgpu_device()` 调用，Rust 侧代码无需重写。

### 4.2 输出端口设计

单个 YOLO 节点暴露两个输出端口：

1. `detections : roi` — 结构化数据，`{id, bbox:[x1,y1,x2,y2], score, class_id, class_name}[]`（0..1 归一化坐标）。
2. `overlay : sampler2D` — 在输入图上绘制 bbox 的 texture，可以直接被下游 shader 的 sampler2D 输入连上（Three.Texture 由 scratch canvas 生成）。

**分离的原因**：`roi` 不能被 GLSL 采样，`sampler2D` 不能被下游"数字滤波"节点消费。分开后连线校验完全靠 dataType 匹配，无需引入"任意"类型。

---

## 5. 类型系统扩展

### 5.1 DataType

新增逻辑类型（GLSL 不可用）：

```ts
export type LogicalDataType = 'roi' | 'mesh' | 'json';
export type DataType = /* ...既有 GLSL 类型... */ | LogicalDataType;
```

`GLSL_VALID_TYPES` 保持不变（不含逻辑类型），另外新增：

```ts
export const LOGICAL_TYPES: LogicalDataType[] = ['roi', 'mesh', 'json'];
```

`DATA_TYPE_COLORS` 追加：`roi #ff8a65 · mesh #7986cb · json #ffb74d`。

### 5.2 NodeType

```ts
export type NodeType = 'shader' | 'input' | 'constant' | 'onnx';
```

`ShaderNodeData` 追加可选字段（沿用同一 interface，避免联合类型侵入渲染层）：

```ts
onnxModelId?: string;      // e.g. 'yolov8n'
onnxScoreThreshold?: number;
onnxIouThreshold?: number;
onnxTargetSize?: number;   // model input size, e.g. 640
```

### 5.3 ProjectFile version

从 `0.2.0` 升到 `0.3.0`。老版本载入时报错、拒绝加载（现有策略）。CHANGELOG 记录 breaking。

### 5.4 连线校验

`isConnectionValid` 追加：

```ts
if (LOGICAL_TYPES.includes(sourcePort.dataType) || LOGICAL_TYPES.includes(targetPort.dataType)) {
  return sourcePort.dataType === targetPort.dataType;
}
```

---

## 6. 运行时组件

### 6.1 ORT 本地分发

- **不通过 npm import**：`onnxruntime-web` 装为 devDependency，`scripts/copy-ort.mjs` 把 `dist/ort.min.js` + `dist/ort-wasm-*.wasm` 复制到 `public/ort/`，`index.html` 用 `<script src="/ort/ort.min.js"></script>` 加载。
- 桥接层在 `ort_init` 前设置 `globalThis.ort.env.wasm.wasmPaths = '/ort/'`。
- **模型文件** `yolov8n.onnx` 不入库，由 `scripts/copy-model.mjs` 在 `predev`/`prebuild` 时从 Cargo 拉的 `rimeflow-yolov8n` git checkout（`~/.cargo/git/checkouts/rimeflow-yolov8n-*/*/models/yolov8n.onnx`）复制到 `public/models/`。模型版本随 `yolo-detector/Cargo.toml` 里对上游的 pin 而变。

### 6.2 Rust crate：`rust/crates/yolo-detector`

**thin wrapper**：Cargo 通过 git 依赖 `https://github.com/caozisheng/rimeflow-yolov8n`（`branch = "main"`）拿到上游 `postprocess` + `ort_bridge`，本地只补一个 `YoloDetectorWasm` façade + `COCO_CLASSES` 表。crate 目标：`cdylib` for `wasm32-unknown-unknown`，用 `wasm-pack --target web` 打包到 `rust/crates/yolo-detector/pkg/`。所有 OpenQuartz Rust 节点 crate 都放在 `rust/crates/*` 下，由 `rust/Cargo.toml` 声明的 Cargo workspace 统一管理。

模块结构：

```
rust/                        Cargo workspace 根 (members = ["crates/*"])
├── Cargo.toml
└── crates/
    └── yolo-detector/       git dep: rimeflow-yolov8n (upstream)
        ├── Cargo.toml
        ├── src/lib.rs       pub use upstream::{postprocess, ort_bridge};
        │                    + YoloDetectorWasm façade + COCO_CLASSES
        └── README.md
```

上游 crate 提供：

- `postprocess::{decode_yolo_output, nms, Detection}` — 纯 Rust decode + NMS。
- `ort_bridge::{capture_webgpu_device, ort_init, ort_detect, ort_release, get_output_f32, get_f64}` — 只在 `wasm32` target 上编译；`ort_init` 自动 `<script src="/ort/ort.min.js">` 注入，选 `webgpu` EP 失败回落到 `wasm`。上游今天在 `ort_bridge` 里禁用了 GPU zero-copy 的 `fromGpuBuffer` 路径（ORT 1.27 返回空 tensor），走 CPU letterbox + WebGPU EP 推理。

`Cargo.toml` 关键片段：

```toml
[dependencies]
rimeflow-yolov8n = { git = "https://github.com/caozisheng/rimeflow-yolov8n", branch = "main" }
wasm-bindgen = "0.2"
wasm-bindgen-futures = "0.4"
js-sys = "0.3"
serde = { version = "1", features = ["derive"] }
serde-wasm-bindgen = "0.6"

[dependencies.web-sys]
features = ["HtmlCanvasElement", "OffscreenCanvas", "console"]
```

### 6.3 wasm 交付

- `npm run build:wasm` → `wasm-pack build rust/crates/yolo-detector --target web --release --out-dir pkg`。
- Vite `resolve.alias`：`'@nodes/yolo-detector' -> 'rust/crates/yolo-detector/pkg/yolo_detector.js'`。
- TS 侧静态 `import __wbg_init, { YoloDetectorWasm } from '@nodes/yolo-detector'`；手写 `.d.ts`（`src/types/onnx.d.ts`）避免 tsc 依赖 pkg 存在。
- 升级上游：`cd rust/crates/yolo-detector && cargo update -p rimeflow-yolov8n && wasm-pack build ...`。
### 6.4 `OnnxSession` 抽象（TypeScript）

```ts
interface OnnxDescriptor {
  id: string;
  label: string;
  modelUrl: string;           // '/models/yolov8n.onnx'
  targetSize: number;         // 640
  loader: () => Promise<OnnxSession>;
  taskOutputs: Port[];        // roi + sampler2D overlay
}

interface OnnxSession {
  init(): Promise<void>;
  run(canvas: HTMLCanvasElement | OffscreenCanvas, sourceWidth: number, sourceHeight: number): Promise<OnnxResult>;
  release(): void;
}

interface OnnxResult {
  detections: Detection[];   // normalized 0..1
  raw: Float32Array;
}
```

`onnxRegistry.ts` 集中管理 descriptor；加新模型只加 registry 条目。

### 6.5 ExecutionEngine 集成

在 `executionEngine.run()` 的节点循环里追加 `if (node.data.type === 'onnx')` 分支：

1. 找到接到该节点 sampler2D 输入端口的上游 texture（既有 `textures` map）。
2. 用 `WebGLRenderer.readTargetToOffscreenCanvas(target, w, h)` 把 FBO 内容送到 OffscreenCanvas（新增 API）。
3. 调用 `OnnxSession.run(offscreen, w, h)` 得到 `Detection[]`。
4. 生成 overlay：CPU 上在同一 OffscreenCanvas 之上 `strokeRect` 画框 → 通过 `THREE.CanvasTexture` 注册为 texture，登记到 `textures` map（`kind: 'image', texture: canvasTexture`）供下游 sampler2D 消费。
5. `onOutput(nodeId, dataUrl)` 送预览。
6. 新增 `onOutputData?(nodeId, data)` 送结构化 `roi` 到 store（`outputData` 分状态）。

### 6.6 Store 扩展

新增：

```ts
outputData: Record<string, unknown>;
setOutputData(nodeId: string, data: unknown): void;
addOnnxNode(modelId: string, position?): void;
```

---

## 7. UI

### 7.1 Header 菜单

在 `SHADER / INPUT` 之后新增 `ONNX` dropdown：`YOLOv8n Detector`（后续更多模型）。

### 7.2 OnnxNode 组件

- Header 采用 accent `#ff8a65`（roi 色）。
- 输入端口：默认一个 `image : sampler2D`。
- 输出端口：`detections : roi`, `overlay : sampler2D`。
- 卡片显示预览缩略图（带 bbox 的 overlay）。

### 7.3 SidePanel 面板

- 显示模型 id、模型文件路径、Load Status（`idle / loading / ready / error`）。
- 阈值滑杆：`Score`, `IoU`。
- 输入分辨率（只读，来自 descriptor）。
- 检测结果列表（class_name × N，score × 100）。
- Preview 复用现有预览槽（PNG dataUrl）。

---

## 8. 前向兼容性总表

| 层 | Web (当前) | Web + WebGPURenderer (未来) | Tauri native (更远期) | 移植量 |
| --- | --- | --- | --- | --- |
| 模型文件 | `public/models/yolov8n.onnx` | 同上 | 同上 | 0 |
| ORT 运行时 | `onnxruntime-web` (JS) | 同上 + `capture_webgpu_device` | `ort` Rust crate | 桥接层替换 |
| 预处理 (letterbox + NCHW) | WGSL/JS 或 CPU 路径 | WGSL (同源码) | wgpu native (同源码) | 0（WGSL 复用） |
| decode + NMS | `postprocess.rs` | 同上 | 同上 | 0 |
| ByteTrack | `bytetrack.rs` | 同上 | 同上 | 0 |
| 节点契约 (端口签名) | roi + sampler2D | 同上 | 同上 | 0 |

---

## 9. 交付清单（MVP）

1. `rust/crates/yolo-detector/` 薄 wrapper（git-dep 上游 crate + 本地 `YoloDetectorWasm` façade + `COCO_CLASSES`），Cargo workspace 根位于 `rust/Cargo.toml`。
2. `public/models/yolov8n.onnx`。
3. `public/ort/` 由 `scripts/copy-ort.mjs` 生成。
4. 前端：`onnx` NodeType + `OnnxNode.tsx` + `onnxRegistry.ts` + `onnxSession.ts` + `executionEngine` 分支 + `SidePanel` 面板 + Header 菜单。
5. `docs/ONNX_NODE_DESIGN.md`（本文档）。
6. `CHANGELOG.md` 记录 `[0.6.0b]`。
7. `README.md` 追加 `ONNX Node` 章节。

## 10. 已知限制 & TODO

- **A/B 分派**：上游 `ort_bridge` 通过 `executionProviders: ['webgpu', 'wasm']` 让 ORT 自己选择；`webgpu` EP 在无 `navigator.gpu` 时自动回落 `wasm`。**未来**升级到 WebGPURenderer 后调用 `YoloDetectorWasm.captureWebgpuDevice()` 打通 GPU zero-copy。
- **GPU zero-copy 状态**：上游代码里 `useGpuPreprocess = false`，因为 ORT 1.27 的 `Tensor.fromGpuBuffer` + 共享 `GPUDevice` 组合下会静默返回空 tensor，仍在观察上游修复；今天走 CPU letterbox → WebGPU EP inference，仍避免了 CPU 侧的模型 CPU 计算。
- **Overlay 分辨率**：MVP overlay canvas 与输入 canvas 同分辨率，未走 GPU compute shader；换 Three.js `WebGPURenderer` 或 `wgpu` 后可迁移到 WGSL。
- **多帧时序**：MVP 每次 RUN 独立推理；ByteTrack 未随上游发布，UI 层解锁"实时循环"后按需在此 wrapper 侧补一个 `ByteTracker`。
- **模型热加载**：MVP 靠 `ort_release()` + `ort_init()` 显式切换。
- **wasm 体积**：`yolo_detector_bg.wasm` MVP 无 SIMD，约 200 KB；ORT 核 WASM 约 5 MB（用户装 `onnxruntime-web` 后运行 `copy:ort` 生成）。
