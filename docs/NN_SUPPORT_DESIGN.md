# chaiNNer 设计分析与 OpenQuartz NN 支持路线图

> Version: 2.0 (2026-07-15)
> 目的：分析 chaiNNer 的设计与实现，对比 OpenQuartz 现状，规划 NN/ONNX 支持的扩展路径。

---

## 1. chaiNNer 架构概览

### 1.1 项目定位

chaiNNer 是一个基于节点图的**图像处理 GUI**，以 AI 超分辨率（upscaling）为核心场景，逐渐扩展为通用图像处理工具。

| 维度 | chaiNNer | OpenQuartz |
|---|---|---|
| 核心场景 | 图像超分/去背景/格式转换（离线批处理） | 实时视觉合成/shader 管线/视频处理（实时交互） |
| 执行模型 | Python 后端，一次性跑完全图（push 模型） | 浏览器内 rAF 实时循环，逐帧执行 DAG |
| 推理框架 | PyTorch / ONNX / NCNN / TensorRT（原生 Python 绑定） | onnxruntime-web（WebGPU/WASM EP，浏览器内） |
| 渲染 | 无 GPU 渲染管线，numpy 处理像素 | Three.js WebGL2 FBO 管线，GPU-only 路径 |
| 桌面框架 | Electron + Python 子进程 | Vite + React + Tauri（可选） |
| 前端 | React + Chakra UI + react-flow | React + @xyflow/react + Tailwind |

### 1.2 架构分层

```
┌──────────────────── chaiNNer ────────────────────┐
│  Electron (前端 UI + react-flow)                   │
│       ↕ IPC (sanic WebSocket)                      │
│  Python 后端                                       │
│    ├── api/        节点注册、类型系统、输入输出声明   │
│    ├── chain/      DAG 构建 + 拓扑排序 + 执行        │
│    ├── nodes/      所有节点实现                      │
│    │   ├── impl/onnx/       ONNX 推理节点           │
│    │   ├── impl/pytorch/    PyTorch 推理节点         │
│    │   ├── impl/ncnn/       NCNN 推理节点            │
│    │   ├── impl/tensorrt/   TensorRT 推理节点        │
│    │   ├── impl/upscale/    通用超分逻辑             │
│    │   ├── impl/rembg/      背景移除                 │
│    │   ├── impl/color/      色彩空间转换             │
│    │   ├── impl/normals/    法线图操作               │
│    │   └── ...              blend/resize/noise/...  │
│    └── dependencies/   Python 包管理               │
└─────────────────────────────────────────────────────┘
```

### 1.3 节点类型系统

chaiNNer 有四种节点 kind：

| Kind | 说明 | OpenQuartz 等价 |
|---|---|---|
| `regularNode` | 普通函数节点（输入→输出，无状态） | `shader` / `math` / `onnx` |
| `generator` | 迭代器源（产出序列，如 Load Images 遍历目录） | 无直接等价（我们用 `input` 节点的 video 模式） |
| `collector` | 迭代器汇（收集序列，如 Save Images） | 无（我们用 `renderer` 节点做输出） |
| `transformer` | 数据转换节点 | `math` 节点 |

### 1.4 NN 推理节点分类

chaiNNer 的 NN 节点按推理框架和任务类型组织：

#### 按框架分

| 框架 | 文件位置 | 依赖 | GPU 支持 |
|---|---|---|---|
| **PyTorch** | `impl/pytorch/` | torch + [Spandrel](https://github.com/chaiNNer-org/spandrel) | CUDA, ROCm, MPS |
| **ONNX** | `impl/onnx/` | onnxruntime | CUDA, DML, TensorRT, CPU |
| **NCNN** | `impl/ncnn/` | ncnn (C++ via Python 绑定) | Vulkan (AMD/Intel/NVIDIA) |
| **TensorRT** | `impl/tensorrt/` | tensorrt | NVIDIA only |

#### 按任务分

| 任务 | 节点 | 输入→输出 | 说明 |
|---|---|---|---|
| **超分辨率（SR）** | Upscale Image | Image → Image (scale×) | 核心功能，支持 ESRGAN/RealESRGAN/SwinIR 等几十种架构 |
| **背景移除** | Remove Background | Image → Image (RGBA) | u2net/isnet 系列 ONNX 模型 |
| **帧插值** | Interpolate Frames | 2×Image → Image | RIFE 模型（PyTorch） |
| **特征匹配** | XFeat | Image → Keypoints | XFeat 特征提取（PyTorch） |
| **模型转换** | Convert to ONNX/NCNN | Model → Model | PyTorch→ONNX→NCNN 转换链 |
| **模型信息** | Model Info | Model → metadata | 提取模型元信息 |

---

## 2. chaiNNer 关键设计模式

### 2.1 模型自省（Model Introspection）

chaiNNer 最重要的设计之一：**加载 ONNX 模型时自动推断能力**。

```python
# load.py — 加载模型时自动检测
def load_onnx_model(model_or_bytes) -> OnnxModel:
    # 1. 通过正则匹配输出层名识别模型子类型（Generic / RemBg）
    # 2. 通过 shape inference 推断：
    #    - input_channels / output_channels
    #    - fixed_input_width / fixed_input_height
    #    - scale_width / scale_height（超分倍率）
    #    - size_req（输入尺寸约束：minimum / multiple_of）
    # 3. 返回 OnnxGeneric 或 OnnxRemBg
```

这让用户只需**拖入一个 .onnx 文件**，节点就能自动知道模型是什么类型、输入输出尺寸、放大倍率。

**对 OpenQuartz 的启发**：我们目前 `onnxRegistry.ts` 是**硬编码**描述符（只有 yolov8n），未来应支持"拖入模型文件 → 自动推断端口签名"。

### 2.2 Auto-Split Tiling（自动分块推理）

为解决大图超出 GPU 显存问题，chaiNNer 实现了智能分块推理：

```python
# auto_split.py
def onnx_auto_split(img, session, change_shape, tiler, size_req, progress):
    # 1. 将图像分割为重叠的 tile
    # 2. 逐 tile 推理
    # 3. 带 blending 拼接结果（避免接缝）
    # 4. OOM 时自动缩小 tile 重试
```

**关键技术点**：
- `SizeReq`：每个模型声明最小输入尺寸和对齐约束（如 `multiple_of=16`）
- `tile_blending`：重叠区域用渐变权重混合，消除拼接边界
- OOM 自动降级：捕获 CUDA OOM，自动切到更小的 tile size

### 2.3 多框架统一抽象

chaiNNer 用**独立 session 层**统一不同推理框架：

```
             ┌─ PyTorch Session (torch)
Model Load ──┤─ ONNX Session (onnxruntime)
             ├─ NCNN Session (ncnn bindings)
             └─ TensorRT Session (tensorrt)
                     │
                     ▼
              upscale/convenient_upscale.py
              (统一的 image→image 管线：通道适配、alpha 处理、进度)
```

`convenient_upscale` 是核心抽象——无论底层用什么框架，上层只关心：
- `model_in_nc` / `model_out_nc`（模型输入输出通道数）
- `upscale(img, progress)` 回调
- 自动处理 alpha 通道、灰度图等边界情况

### 2.4 图像处理节点（非 NN）

chaiNNer 的非 NN 节点全部基于 numpy/OpenCV，CPU 执行：

| 类别 | 节点 | 等价于 OpenQuartz |
|---|---|---|
| Color | Color Space Convert, Hue/Saturation, Levels | `colorShaders` (GPU) |
| Blend | 各种混合模式 | `blendShaders` (GPU) |
| Resize | Bilinear/Bicubic/Lanczos + 自定义比例 | 无（我们靠 shader 或 GL filtering） |
| Noise | Simplex/Blue/Value noise 生成 | `generatorShaders` (GPU) |
| Normals | 法线图生成/编辑 | 无 |
| Tile | 平铺/镜像 | 无 |
| DDS | DirectDraw Surface 格式处理 | 无 |
| Caption | 文字叠加 | 无 |
| FFmpeg | 视频编解码 | 视频输入（部分） |

**关键差异**：chaiNNer 的图像处理是 **CPU + numpy**，我们是 **GPU + GLSL shader**。我们的 GPU 路径天然适合实时场景，chaiNNer 的适合离线批处理。

---

## 3. OpenQuartz 现有 ONNX 支持盘点

### 3.1 当前架构

```
┌──────────── OpenQuartz ONNX 架构 (v0.9) ─────────────┐
│                                                        │
│  onnxCatalog.ts    — 模型目录（3 个内置模型 + 分类）       │
│  onnxModelManager  — 下载管理 + 缓冲缓存 + 进度通知       │
│  onnxInference.ts  — 通用 TS ORT 推理 + 分块引擎          │
│    ├── TileCodec: rgbCodec (ESRGAN), ycbcrCodec (SPCNN) │
│    ├── 自适应 tile sizing (64→32→16→WASM fallback)       │
│    └── runSuperResolution → runTiledInference            │
│  onnxIntrospect.ts — 模型自省 + 任务推断 + 端口生成        │
│  onnxSession.ts    — YOLO 检测（wasm-pack + Rust NMS）    │
│  onnxOverlay.ts    — 检测结果可视化                       │
│  executionEngine   — ONNX 分支 + 输出缓存 + 静态管线优化   │
│  realtimeHost      — 静态管线检测 + ONNX 完成后补帧        │
│                                                        │
│  rust/crates/yolo-detector/                             │
│    └── YoloDetectorWasm (保留用于检测后处理)               │
└────────────────────────────────────────────────────────┘
```

### 3.2 当前能力

| 能力 | 状态 | 说明 |
|---|---|---|
| YOLO 目标检测 | ✅ | yolov8n，80 类 COCO |
| 异步推理 | ✅ | 非阻塞，使用上一帧结果 |
| roi 输出 | ✅ | 结构化 Detection[] |
| overlay 输出 | ✅ | bbox 可视化 → sampler2D |
| **模型目录（Catalog）** | ✅ Phase 1 | 3 个内置模型，按类别分组菜单，自动下载 |
| **自定义模型节点** | ✅ Phase 1 | Custom ONNX 节点框架（文件选择待 UI 接入） |
| **模型下载管理** | ✅ Phase 1 | 后台下载 + 进度通知 + 缓冲缓存 |
| **模型自省** | ✅ Phase 1 | inferTaskFromMeta + metaToDefaultPorts |
| **超分辨率** | ✅ Phase 2 | Sub-pixel CNN 3× + Real-ESRGAN 4× |
| **分块推理（Tiling）** | ✅ Phase 2 | 通用 TileCodec + 自适应 tile size |
| **WebGPU→WASM 降级** | ✅ Phase 2 | GPU 不兼容时自动切 WASM EP + UI 标记 |
| **静态管线优化** | ✅ Phase 2 | 无动画管线只跑一帧，ONNX 完成后补帧 |
| **多模型** | ✅ | 3 个 catalog 模型 + 自定义 |
| 背景移除 | ❌ Phase 3 | 待实现 |
| 模型文件拖入 | ❌ Phase 5 | 待实现 |

### 3.3 执行流程

```
检测管线 (YOLO):
  sampler2D input
    → readTargetToCanvas (GPU→CPU)
    → ort_detect (wasm-pack → ORT web, WebGPU/WASM EP)
    → Float32Array → Rust decode + NMS → Detection[]
    → overlay canvas (CPU drawBox) → THREE.CanvasTexture
    → 下游 shader 消费

超分管线 (Phase 2, 通用 TS ORT):
  sampler2D input
    → readTargetToCanvas (GPU→CPU)
    → TileCodec.encode → Float32Array [1,C,tH,tW]
    → OnnxInferenceSession.run (WebGPU EP / WASM fallback)
    → TileCodec.decode → 拼接 RGBA 输出
    → THREE.CanvasTexture → 下游 shader/renderer 消费
  自适应策略: tile=64 → OOM → tile=32 → OOM → tile=16 → WASM fallback
```

---

## 4. 功能对齐分析

### 4.1 chaiNNer 有而我们缺的 NN 能力

| chaiNNer 能力 | 重要性 | 说明 |
|---|---|---|
| **Image Super-Resolution** | ✅ 已实现 | Phase 2 完成，Sub-pixel CNN 3× + Real-ESRGAN 4× |
| **Background Removal** | ⭐⭐⭐⭐ | u2netp (4.7MB, ~40ms)，实时可行 |
| **Model Introspection** | ✅ 已实现 | inferTaskFromMeta + metaToDefaultPorts |
| **Auto-Split Tiling** | ✅ 已实现 | 通用 TileCodec，自适应 tile sizing |
| **深度估计** | ⭐⭐⭐⭐ | MiDaS small (15MB, ~40ms)，shader DOF/fog 价值高 |
| **人脸/人体/手部追踪** | ⭐⭐⭐ | BlazeFace/MoveNet/MediaPipe，<25ms，体感交互 |
| **风格迁移** | ⭐⭐⭐ | Magenta (8MB, ~60ms)，实时艺术效果 |
| **多推理框架** | ❌ 不做 | PyTorch/NCNN/TensorRT 对浏览器无意义 |
| **帧插值 / OCR / Inpainting** | ❌ 不做 | 非实时场景，不是我们的定位 |

### 4.2 我们有而 chaiNNer 缺的能力

| OpenQuartz 能力 | 说明 |
|---|---|
| **实时渲染循环** | rAF 驱动，60fps 交互式，chaiNNer 是离线批处理 |
| **GPU shader 管线** | GLSL 300 es，FBO 链，GPU-only 路径 |
| **NN 与 shader 混合管线** | ONNX 输出可直接接 shader，chaiNNer 只有 numpy |
| **实时视频流 + NN** | 摄像头/视频 → ONNX → shader 后处理，一个循环 |
| **Shadertoy 兼容** | iTime/iMouse/iResolution |
| **浏览器即运行** | 零安装，chaiNNer 需装 Python + 几百 MB 依赖 |

---

## 5. NN 支持扩展路线图

### 5.1 设计原则

1. **ONNX-first**：浏览器环境下 onnxruntime-web 是唯一实际选择，不做多框架抽象
2. **GPU pipeline 优先**：ONNX 输出回到 GPU（Three.js texture）供 shader 消费，保持实时性
3. **通用 ONNX 节点**：不再为每个模型硬编码 Rust crate，改为通用推理节点 + 任务描述符
4. **渐进增强**：MVP 用 CPU readback 路径，未来 WebGPURenderer 迁移后走 zero-copy
5. **实时优先选型**：只纳入 WebGPU 上有机会实时（<100ms/帧，低分辨率输入）的轻量模型。大模型（>50MB、>500ms/帧）不是我们的场景——chaiNNer 做离线批处理，我们做实时交互

### 5.2 分阶段路线

#### Phase 1: 通用 ONNX 推理引擎（基础层重构） ✅ 已完成 (v0.8.0b)

**目标**：将当前 YOLO 专用的 ONNX 管线泛化为通用推理引擎。

##### 5.2.1 两类 ONNX 节点

ONNX 节点分为两类：

**A. 目录模型节点（Catalog ONNX Node）**

菜单里列出的已知模型（YOLOv8n、Real-ESRGAN、u2netp 等）。模型**不预置在 app 里**。用户从菜单选择添加到画布后：

1. 节点立即出现在画布上，显示模型名称和预设端口轮廓
2. 自动触发后台下载到本地模型目录（`~/.openquartz/models/{id}.onnx`）
3. 节点上显示下载进度条（已下载 / 总大小）
4. 下载完成 → 自省模型 → 用实际 I/O 确认/更新端口 → 节点变为 ready 状态
5. 再次打开工程时，模型已在本地，直接加载

**B. 自定义模型节点（Custom ONNX Node）**

用户选择本地 `.onnx` 文件，直接自省，不需要下载：

1. 用户添加 "Custom ONNX" 节点到画布
2. 节点显示文件选择器（或拖入 .onnx 文件）
3. 读取本地文件 → 自省 → 动态生成输入输出端口
4. 端口签名完全由自省结果决定
5. 工程保存时记录模型文件路径（Tauri 绝对路径）

```
节点状态机:

Catalog 节点:
  [added] → [downloading 45%] → [introspecting] → [ready] → [running]
                                                      ↑
  (再次打开工程，模型已在本地) ─────────────────────────┘

Custom 节点:
  [added] → [select file] → [introspecting] → [ready] → [running]
                                                  ↑
  (再次打开工程，路径有效) ─────────────────────────┘
  (路径失效) → [file missing] → [select file] → ...
```

##### 5.2.2 模型目录注册表（Catalog Registry）

取代当前硬编码的 `onnxRegistry.ts`，引入目录注册表：

```typescript
// ---- 任务类型枚举 ----
type OnnxTask =
  | 'super-resolution'   // 图像超分
  | 'background-removal' // 背景移除
  | 'detection'          // 目标检测（当前 YOLO）
  | 'segmentation'       // 语义分割
  | 'style-transfer'     // 风格迁移
  | 'denoising'          // 降噪
  | 'inpainting'         // 修复
  | 'depth-estimation'   // 深度估计
  | 'generic';           // 通用 image→image（Custom 节点默认）

// ---- 目录模型条目（app 内置的已知模型列表）----
interface CatalogEntry {
  id: string;                // 'yolov8n', 'real-esrgan-x4', 'u2netp'
  label: string;             // 菜单显示名
  task: OnnxTask;
  category: string;          // 菜单分组: 'Detection', 'Super-Resolution', ...
  downloadUrl: string;       // 模型下载地址（GitHub release / HuggingFace）
  fileSize: number;          // 字节数，用于下载进度
  sha256: string;            // 下载完成后校验
  // 预填的 I/O 规格（下载前即可显示端口，下载后自省确认）
  expectedIO: {
    inputs: PortSpec[];
    outputs: PortSpec[];
  };
  // 任务特定默认参数
  defaultParams?: Record<string, ParamDescriptor>;
}

interface PortSpec {
  id: string;
  label: string;
  dataType: DataType;        // 'sampler2D' | 'roi' | 'json' | 'float' | ...
  direction: 'input' | 'output';
}

interface ParamDescriptor {
  type: 'float' | 'int' | 'boolean';
  default: number | boolean;
  min?: number;
  max?: number;
  step?: number;
  label: string;
}

// ---- 自省结果（下载完成或用户选择文件后获得）----
interface OnnxModelMeta {
  opset: number;
  inputs: Array<{ name: string; shape: (number | string)[]; dtype: string }>;
  outputs: Array<{ name: string; shape: (number | string)[]; dtype: string }>;
  inferredTask?: OnnxTask;
  inferredScale?: number;
  inferredInputChannels?: number;
  inferredOutputChannels?: number;
}
```

##### 5.2.2 通用前处理/后处理

将当前 YOLO 专用的 Rust wasm 前后处理抽象为可复用的管线：

```
┌──────────── 通用 ONNX 执行管线 ────────────┐
│                                              │
│  输入 sampler2D                              │
│     ↓                                        │
│  [前处理] (TypeScript, 可选 wasm 加速)         │
│     ├── resize / letterbox / pad             │
│     ├── normalize (0-255 → 0-1)              │
│     ├── channel reorder (HWC → CHW)          │
│     └── dtype cast (f32 / f16)               │
│     ↓                                        │
│  ORT InferenceSession.run(tensor)            │
│     ↓                                        │
│  [后处理] (按 task 分派)                       │
│     ├── SR: CHW→HWC → clamp → texture        │
│     ├── RemBg: sigmoid → alpha mask           │
│     ├── Detection: decode + NMS → roi         │
│     ├── Depth: normalize → grayscale texture  │
│     └── Generic: raw tensor → texture         │
│     ↓                                        │
│  输出端口 (sampler2D / roi / mask / ...)       │
└──────────────────────────────────────────────┘
```

##### 5.2.3 TypeScript ORT 直调

当前架构通过 wasm-pack + Rust ort_bridge 调用 ORT。对于**非 YOLO** 任务，应该在 TypeScript 层直接使用 `onnxruntime-web`，避免每个模型都写 Rust crate：

```typescript
// onnxInference.ts — 通用推理
import * as ort from 'onnxruntime-web';

class OnnxInferenceSession {
  private session: ort.InferenceSession | null = null;

  async load(modelUrl: string, options?: ort.InferenceSession.SessionOptions) {
    this.session = await ort.InferenceSession.create(modelUrl, {
      executionProviders: ['webgpu', 'wasm'],
      ...options,
    });
  }

  async run(input: Float32Array, shape: number[]): Promise<ort.Tensor> {
    const tensor = new ort.Tensor('float32', input, shape);
    const inputName = this.session!.inputNames[0];
    const results = await this.session!.run({ [inputName]: tensor });
    return results[this.session!.outputNames[0]];
  }
}
```

**保留 Rust wasm 路径**的场景：
- YOLO 后处理（decode_yolo_output + NMS 的 Rust 实现性能远优于 JS）
- 未来可能的复杂后处理（ByteTrack、SAM 解码等）

#### Phase 2: 超分辨率（Super-Resolution）节点 ✅ 已完成 (v0.9.0b)

**已实现** — 通用分块推理引擎 + 两个超分模型 + 自适应 tile sizing + WebGPU→WASM 自动降级。

##### 支持的模型

| 模型 | 参数量 | 倍率 | ONNX 可用 | 说明 |
|---|---|---|---|---|
| **Real-ESRGAN x4** | 16.7M | 4× | ✅ | 通用照片超分，效果好 |
| **Real-ESRGAN x2** | 16.7M | 2× | ✅ | 2 倍超分 |
| **ESRGAN (通用)** | 各异 | 各异 | ✅ | [OpenModelDB](https://openmodeldb.info) 社区模型 |
| **SwinIR** | 11.8M | 2×/3×/4× | ✅ | Transformer 架构，效果优 |
| **BSRGAN** | 16.7M | 4× | ✅ | 真实降质场景 |
| **RealSR** | 各异 | 4× | ✅ | 针对真实照片 |

##### 节点设计

```
输入端口:
  image: sampler2D         ← 原图

输出端口:
  upscaled: sampler2D      ← 超分后的图像

参数:
  model: 下拉选择 (Real-ESRGAN x4 / SwinIR / ...)
  scale: 只读显示 (由模型决定)
  tileSize: 分块大小 (auto / 256 / 512 / 1024)
```

##### 分块推理策略（浏览器适配）

chaiNNer 的 auto_split 是 CPU 环境设计的。浏览器 WebGPU EP 有不同的约束：

```
浏览器分块推理设计:
  1. 默认 tile size = 256×256 (WebGPU EP 显存友好)
  2. overlap = 16px (边缘融合)
  3. 分块策略:
     - 小图 (≤512px): 不分块，直接推理
     - 中图 (512-2048px): 4-16 块
     - 大图 (>2048px): 按 tile 网格切分
  4. OOM 处理:
     - 捕获 WebGPU OOM
     - 回退到 WASM EP（CPU, 更慢但不 OOM）
     - 或缩小 tile 重试
```

##### 实时管线集成

超分与当前的实时渲染循环如何集成——关键在于延迟策略：

```
方案 A: 离线模式（推荐初始方案）
  - 用户手动触发 "Upscale" 按钮
  - 读取当前帧 → 推理 → 结果写入 output texture
  - 适合静态图片超分场景

方案 B: 实时模式（高级功能）
  - 类似当前 YOLO：异步推理，使用上一帧结果
  - 但超分的输出分辨率大于输入 → FBO 分辨率需要动态调整
  - 性能问题：Real-ESRGAN 4× 在 WebGPU 上约 200-500ms/帧
  - 只适合低分辨率输入 (如 160×120 → 640×480)
```

#### Phase 3: 背景移除（Background Removal）节点

> **策略**：只选轻量模型（<30MB），WebGPU 上能做到 ~30-80ms/帧，实时可行。
> 大模型（u2net 176MB、isnet 174MB、rmbg-1.4 176MB）不纳入——它们是离线批处理场景，不是我们的定位。

##### 模型

| 模型 | 大小 | 输入 | 延迟 (WebGPU 估计) | 说明 |
|---|---|---|---|---|
| **u2netp** | 4.7MB | 320×320 | ~30-50ms | 轻量版，效果可用，实时可行 |
| **modnet** | 25MB | 512×512 | ~60-100ms | 人像抠图，边界情况 |

##### 节点设计

```
输入端口:
  image: sampler2D         ← 原图

输出端口:
  foreground: sampler2D    ← 前景 RGBA（带 alpha）
  mask: sampler2D          ← alpha mask（灰度）

参数:
  model: 下拉选择 (u2netp / modnet)
  threshold: float 0-1 (alpha 阈值)
```

##### 实时管线价值

背景移除 + shader 后处理是 OpenQuartz 区别于 chaiNNer 的核心实时场景：

```
Camera 320×240 → RemBg (u2netp, ~40ms) → alpha mask
                                             ↓
                              Shader (替换/模糊背景)
                                             ↓
                                       Renderer 输出
```

#### Phase 4: 更多实时视觉任务

> **选型原则**：模型 <30MB，WebGPU 推理 <100ms，输入 ≤640px 时可实时。
> 排除：帧插值、OCR、inpainting、大型分割、降噪（全分辨率推理慢）。

| 任务 | 候选模型 | 大小 | 输入 | 延迟估计 | 输出类型 | 实时管线价值 |
|---|---|---|---|---|---|---|
| **深度估计** | MiDaS v2.1 small | 15MB | 256×256 | ~30-60ms | `sampler2D` (depth) | DOF / parallax / fog shader |
| **深度估计** | Depth-Anything v2 small | 25MB | 518×518 | ~60-100ms | `sampler2D` (depth) | 更高质量深度图 |
| **人脸检测** | BlazeFace short | 0.1MB | 128×128 | ~5ms | `roi` + `json` | 人脸效果/滤镜触发 |
| **人脸关键点** | MediaPipe Face Mesh | 2.7MB | 192×192 | ~10-20ms | `json` (468点) | 面部变形/贴纸 |
| **人体姿态** | MoveNet Lightning | 3MB | 192×192 | ~10-15ms | `json` (17点) | 体感交互/骨骼驱动 |
| **手部追踪** | MediaPipe Hands | 2.7MB | 224×224 | ~15-25ms | `json` (21点) | 手势控制 |
| **风格迁移** | Magenta Arbitrary | 8MB | 256×256 | ~50-80ms | `sampler2D` | 实时艺术风格 |

##### 深度估计 × shader 的典型管线

```
Camera 256×256 → MiDaS small (~40ms) → depth map (sampler2D)
                                              ↓
                                   Shader: DOF / parallax / fog
                                              ↓
                                         Renderer 输出
```

用户一个 ONNX 节点 + 一个 shader = 景深模糊效果，零代码。

##### 不纳入的模型（离线/批处理场景）

| 模型 | 原因 |
|---|---|
| u2net / isnet / rmbg-1.4 (背景移除) | 150-180MB，推理 >500ms |
| RIFE (帧插值) | 需要两帧输入 + ~200ms，不适合实时 |
| LaMa (inpainting) | ~50MB，需额外 mask，交互式但非实时 |
| NAFNet / SwinIR-DN (降噪) | 30-60MB，全分辨率推理慢 |
| 大型分割 (SegFormer-B5) | >80MB |
| PaddleOCR | 多阶段管线，不适合帧级实时 |

#### Phase 5: 模型自省与 Custom ONNX 节点

##### 5.5.1 模型自省引擎（ONNX Metadata Extraction）

Catalog 节点下载完成后、Custom 节点选择文件后，都需要自省模型来确认/生成端口：

```typescript
// 使用 onnxruntime-web 或 protobuf 解码读取模型元信息
async function inspectOnnxModel(buffer: ArrayBuffer): Promise<OnnxModelMeta> {
  // 1. 解码 ONNX protobuf → 提取 graph.input / graph.output
  // 2. 推断 tensor format (NCHW/NHWC)、channels、固定/动态尺寸
  // 3. 尝试推断任务类型（按 output shape 启发式判断）:
  //    - 输出 shape 是输入的 N 倍 → super-resolution (scale=N)
  //    - 输出 channels=1, 同尺寸 → mask (background-removal / depth)
  //    - 输出 shape=[1, N, 6+] → detection
  //    - 输出与输入同 shape → generic image→image
  // 4. Catalog 节点: 用自省结果校验 expectedIO，不匹配则警告
  // 5. Custom 节点: 自省结果直接决定端口签名
}
```

##### 5.5.2 模型存储

```
~/.openquartz/
  └── models/
      ├── yolov8n.onnx            ← Catalog 下载
      ├── real-esrgan-x4.onnx     ← Catalog 下载
      ├── u2netp.onnx             ← Catalog 下载
      └── manifest.json           ← 已下载模型的元信息缓存
          {
            "yolov8n": {
              "sha256": "abc...",
              "downloadedAt": "2026-07-14T...",
              "meta": { /* OnnxModelMeta 缓存，避免每次重新自省 */ }
            }
          }
```

Custom 节点的模型文件不复制到模型目录，直接引用用户选择的路径。工程文件保存绝对路径，打开时若路径失效则提示重新选择。

##### 5.5.3 节点 UI 状态

```
Catalog 节点卡片:
  ┌─────────────────────────────┐
  │ ● Real-ESRGAN x4           │  ← header (任务色)
  ├─────────────────────────────┤
  │ ▓▓▓▓▓▓▓░░░  67% (42/63MB)  │  ← 下载中: 进度条
  │ or                          │
  │ ○ image (sampler2D)    →    │  ← ready: 显示自省端口
  │                    → ● out  │
  │ [Score: 0.25] [IoU: 0.45]   │  ← 任务参数（如有）
  └─────────────────────────────┘

Custom 节点卡片:
  ┌─────────────────────────────┐
  │ ● Custom ONNX               │  ← header
  ├─────────────────────────────┤
  │ [Select .onnx file...]      │  ← 未选文件: 文件选择按钮
  │ or                          │
  │ my_model.onnx (25.3MB)      │  ← 已选: 文件名 + 大小
  │ opset 17 · float32          │  ← 自省信息
  │ ○ input (sampler2D)    →    │  ← 自省端口
  │                    → ● out  │
  └─────────────────────────────┘
```
---

## 6. 技术方案对比：TypeScript ORT 直调 vs Rust wasm-pack

### 当前方案（Rust wasm-pack）

```
优势:
  ✅ 后处理性能好（NMS 等 Rust 实现比 JS 快 5-10x）
  ✅ 与上游 rimeflow-yolov8n 代码复用
  ✅ 类型安全

劣势:
  ❌ 每新增一个模型可能需要新 Rust crate
  ❌ wasm-pack 构建链复杂
  ❌ 调试困难（wasm 断点不友好）
  ❌ 包体积增长（每个 crate ~200KB wasm）
```

### 提议方案（混合架构）

```
┌─────────────────────────────────────────┐
│  TypeScript 通用推理层 (onnxInference)     │
│    ├── 模型加载 + 会话管理                 │
│    ├── 通用前处理 (resize/normalize/CHW)   │
│    └── 简单后处理 (SR/RemBg/Depth/Style)   │
│         │                                 │
│         ├── SR: clamp + channel reorder    │
│         ├── RemBg: sigmoid + threshold     │
│         ├── Depth: normalize range         │
│         └── Style: direct output           │
├─────────────────────────────────────────┤
│  Rust wasm 专用后处理 (保留)               │
│    ├── YOLO: decode_yolo_output + NMS     │
│    ├── 未来: SAM decoder                  │
│    └── 未来: ByteTrack 追踪               │
└─────────────────────────────────────────┘
```

**判断标准**：后处理是否涉及**密集浮点计算**（如 NMS 的 IoU 矩阵、tensor 解码）。简单的 clamp/sigmoid/normalize 直接 TS 做，复杂的走 Rust wasm。

---

## 7. 与 chaiNNer 的差异化定位

### 7.1 不照搬的部分

| chaiNNer 特性 | 不照搬原因 |
|---|---|
| Python 后端 | 我们是浏览器 native，无 Python |
| PyTorch/NCNN/TensorRT 支持 | 浏览器内只有 ORT-web，Tauri native 远期才考虑 ort Rust crate |
| 离线批处理 (Load Images → Save Images) | 我们的核心是实时渲染循环 |
| 依赖管理器 (pip install) | 浏览器无需 |
| 模型格式转换 (PyTorch→ONNX→NCNN) | 浏览器内不切实际 |

### 7.2 OpenQuartz 独有的 NN 价值

```
OpenQuartz = 实时 GPU shader 管线 + ONNX 推理 + 可视化 DAG

  chaiNNer 能做:      Image → ONNX → Image (离线)
  OpenQuartz 能做:    Camera → ONNX → Shader → Shader → ... → Renderer (实时 60fps)
```

**核心差异化**：
1. **NN 输出即 shader 输入**——检测结果驱动后处理 shader，深度图驱动景深 shader，分割图驱动抠像 shader
2. **实时视频流**——摄像头/视频 + NN 推理 + shader 后处理，一个渲染循环
3. **交互式调参**——NN 参数（阈值、置信度）实时滑杆调整，即时看到效果
4. **零安装**——打开浏览器就能用，不需要装 Python/CUDA/几百 MB 依赖

---

## 8. 实施优先级总结

| 阶段 | 内容 | 工作量 | 前置条件 | 价值 |
|---|---|---|---|---|
| **Phase 1** | 通用 ONNX 推理引擎 + Catalog/Custom 双节点 + 自动下载 + 自省 | 中 (2-3w) | 无 | 后续所有任务的基础 |
| **Phase 2** | 超分辨率节点 (Real-ESRGAN) — Catalog 条目 | 中 (1-2w) | Phase 1 | ⭐⭐⭐⭐⭐ chaiNNer 招牌功能对齐 |
| **Phase 3** | 背景移除节点 (u2netp/modnet) — Catalog 条目 | 小 (1w) | Phase 1 | ⭐⭐⭐⭐ 实时抠像是杀手级应用 |
| **Phase 4a** | 深度估计节点 (MiDaS) — Catalog 条目 | 小 (1w) | Phase 1 | ⭐⭐⭐⭐ 深度图 × shader 价值大 |
| **Phase 4b** | 风格迁移 / 降噪 / 分割 — 各加 Catalog 条目 | 各 1w | Phase 1 | ⭐⭐⭐ |
| **Phase 5** | Custom ONNX 节点 + 完整自省 + 模型管理 UI | 大 (2-3w) | Phase 1 | ⭐⭐⭐⭐ 用户带自己的模型 |

---

## 9. 附录：chaiNNer 节点完整分类

以下为 `backend/src/nodes/impl/` 下全部模块，供后续功能对齐参考：

```
impl/
├── blend.py              混合模式 (Add/Multiply/Screen/Overlay/...)
├── caption.py            文字叠加
├── cas.py                Contrast Adaptive Sharpening
├── color/                色彩空间转换 (RGB/HSV/Lab/YCbCr/...)
├── color_transfer/       色彩迁移 (均值/标准差/直方图匹配)
├── dds/                  DirectDraw Surface 格式
├── dithering/            抖动处理
├── ffmpeg.py             FFmpeg 视频编解码
├── gradients.py          渐变生成
├── image_formats.py      图像格式转换
├── image_op.py           基础图像操作框架
├── image_utils.py        图像工具函数
├── ncnn/                 NCNN 推理
├── noise.py              噪声添加
├── noise_functions/      程序化噪声生成 (Simplex/Blue/Value)
├── normals/              法线图操作
├── onnx/                 ONNX 推理
│   ├── auto_split.py     分块推理
│   ├── load.py           模型加载 + 自省
│   ├── model.py          模型数据结构
│   ├── np_tensor_utils   numpy↔tensor 转换
│   ├── session.py        ORT 会话管理
│   └── utils.py          shape 推断工具
├── pytorch/              PyTorch 推理
│   ├── auto_split.py     分块推理
│   ├── convert_to_onnx   格式转换
│   ├── rife/             RIFE 帧插值
│   ├── xfeat/            XFeat 特征匹配
│   └── utils.py
├── rembg/                背景移除 (u2net 系列)
├── resize.py             缩放 (Bilinear/Bicubic/Lanczos/...)
├── tensorrt/             TensorRT 推理
├── tile.py               平铺操作
├── upscale/              超分辨率通用逻辑
│   ├── auto_split.py     智能分块
│   ├── basic_upscale.py  基础超分管线
│   ├── convenient_upscale.py  通道/alpha 自适应
│   ├── exact_split.py    精确分块
│   ├── tile_blending.py  分块边缘融合
│   └── tiler.py          分块策略
└── video.py              视频处理
```

### 与 OpenQuartz 现有 shader 分类的对应关系

| OpenQuartz shader 分类 | chaiNNer 对应 | 差异 |
|---|---|---|
| `filterShaders` (Blur/Sharpen/Sobel/Edge/Emboss/Pixelate/Vignette) | `cas.py`, `resize.py`, `noise.py` | 我们 GPU，他们 CPU numpy |
| `colorShaders` (Grayscale/Sepia/HSL/Brightness/Contrast/Gamma/Threshold/Posterize) | `color/`, `image_utils.py` | 我们 GPU |
| `generatorShaders` (Perlin/Simplex/Worley/Gradient) | `noise_functions/`, `gradients.py` | 我们 GPU |
| `blendShaders` (Multiply/Screen/Overlay/Add/Subtract) | `blend.py` | 我们 GPU |
| `distortionShaders` (Swirl/Ripple/Barrel/Polar) | 无直接对应 | 我们独有 |
| 无 | `normals/`, `dds/`, `dithering/`, `color_transfer/` | 他们独有（CPU 操作，我们可 shader 化） |
