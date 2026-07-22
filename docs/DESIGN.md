# OpenQuartz — 实时异构视频管线编辑框架

> Version 0.11.0b — GPU shaders, neural networks, and CPU math in one reactive graph

---

## 1. 项目概述

OpenQuartz 是一个基于 Web 的实时异构视频管线编辑框架，受 Apple Quartz Composer、Shadertoy 和 chaiNNer 启发。

- **异构计算**：GPU shader（WebGL2）、ONNX 神经网络推理（WebGPU/WASM）、CPU 数学运算，三种执行后端统一在一个可视化节点图（DAG）中
- **实时管线**：rAF 驱动的 Host/Compositor 架构，60fps interactive frame rate
- **Shader 即接口声明**：解析 GLSL `uniform` 自动生成输入端口，`out` 生成输出端口
- **Feedback/Accumulator**：shader 通过 `previousFrame` 隐式声明式读取自身上一帧输出，引擎自动启用 ping-pong 双缓冲
- 支持摄像头/视频文件/图片/原始 framebuffer 作为输入源
- 支持**工程文件保存/载入**：`.quartz.json` 格式
- Tauri 2 桌面端（macOS/Windows）+ 纯 Web 浏览器模式
---

## 2. 交互设计原则

- **macOS 菜单栏风格**：顶部工具栏全大写粗体小字，无边文字按钮，hover 变蓝
- **极简白底**：纯白背景 #ffffff，深灰主文字 #1d1d1f，辅助灰 #86868b/#aeaeb2
- **节点卡片式**：白底圆角 + 薄灰边框 + 顶部彩色 header 标识类型（紫=shader，蓝=input，红=output）
- **节点图区**：浅灰底 #e0e0e0 + 十字交叉网格，与右侧白底面板区分
- **实时反馈**：选中节点蓝框 + 阴影，连线贝塞尔曲线，右侧面板即时编辑

---

## 3. 技术栈

| 层 | 选型 | 版本 |
|---|---|---|
| UI 框架 | React | 19 |
| 节点图 | @xyflow/react | 12 |
| Shader 编辑器 | CodeMirror | 6 |
| WebGL 渲染 | Three.js | 最新 |
| State 管理 | Zustand | 最新 |
| 构建 | Vite | 8 |
| TypeScript | — | 6 |
| CSS | Tailwind CSS | 4 |

---

## 4. 核心数据模型

```typescript
type GlslDataType =
  | 'float' | 'int' | 'uint' | 'bool'
  | 'vec2' | 'vec3' | 'vec4'
  | 'ivec2' | 'ivec3' | 'ivec4'
  | 'uvec2' | 'uvec3' | 'uvec4'
  | 'bvec2' | 'bvec3' | 'bvec4'
  | 'mat2' | 'mat3' | 'mat4'
  | 'sampler2D' | 'samplerCube';

// 非 GLSL 逻辑类型，可在 DAG 中流动但不能被 GLSL 采样
type LogicalDataType = 'roi' | 'mesh' | 'json';

type DataType = GlslDataType | LogicalDataType | 'auto';

type InputMode = 'image' | 'framebuffer' | 'video' | 'system';

interface Port {
  id: string;
  label: string;
  dataType: DataType;
  direction: 'input' | 'output';
  defaultValue?: unknown;
}

type NodeType = 'shader' | 'input' | 'constant' | 'onnx' | 'renderer' | 'math';

interface ShaderNodeData {
  type: NodeType;
  label: string;
  shaderCode: string;
  inputs: Port[];
  outputs: Port[];
  uniforms: Record<string, unknown>;
  collapsed?: boolean;
  inputDataType?: DataType;
  inputMode?: InputMode;            // input 节点专用：image / framebuffer / video
  imageDataUrl?: string;            // sampler2D 输入图片
  imageFileName?: string;
  imageWidth?: number;
  imageHeight?: number;
  expanded?: boolean;               // 节点展开/折叠
  // 视频字段
  videoSourceType?: 'camera' | 'file';
  videoUrl?: string;
  videoFileName?: string;
  videoFilePath?: string;           // Tauri: 绝对路径 + convertFileSrc
  videoDeviceId?: string;
  videoLoop?: boolean;
  videoPlaybackRate?: number;
  // ONNX 节点字段
  onnxModelId?: string;
  onnxScoreThreshold?: number;
  onnxIouThreshold?: number;
  onnxTargetSize?: number;
  width?: number;
  height?: number;
  autoSize?: boolean;
  resolvedWidth?: number;
  resolvedHeight?: number;
  // Math 节点字段
  mathOp?: string;                // 运算标识：'add' | 'sin' | 'clamp' | ...
  systemSource?: string;          // system 源：'time' | 'timeDelta' | 'frame' | 'mouse' | 'resolution'
  // Feedback / Accumulator 字段
  feedbackClearColor?: [number, number, number, number]; // ping-pong 缓冲区初始化颜色
}

interface ProjectFile {
  version: string;
  name: string;
  createdAt: string;
  updatedAt: string;
  graph: {
    nodes: Array<{ id: string; type: string; position: { x: number; y: number }; data: ShaderNodeData }>;
    edges: Array<{ id: string; source: string; sourceHandle: string; target: string; targetHandle: string }>;
  };
}
```

---

## 5. 组件架构

```
<App>
  <Header />
    ├── OPENQUARTZ v0.11.0b
    ├── 工程名输入框
    ├── 添加节点：SOURCE / MATH / +SHADER / +ONNX / +RENDERER
    ├── 文件：SAVE / LOAD
    ├── 运行：▶ PLAY / ⏸ PAUSE / ■ STOP / CLEAR
    └── FPS / TIME / FRAME 实时显示
  <main className="flex">
    <NodeGraph />                  ← React Flow 画布 (bg #e0e0e0 + cross grid)
      ├── <ShaderNode />           ← 紫 header，input/output 端口，叶子节点显示输出预览
      ├── <InputNode />            ← 蓝 header，类型选择 + 值输入/图片加载/视频输入
      ├── <OnnxNode />             ← 黄 header，模型选择 + 参数配置
      ├── <RendererNode />         ← 绿 header，显示上游 FBO 预览（mirror canvas blit）
      ├── <MathNode />             ← 橙 header，CPU 运算节点，auto 类型端口
      └── 贝塞尔曲线连线
    <SidePanel />                  ← 白底右侧面板
      ├── 节点信息（类型 + label + Delete）
      ├── <ShaderEditor />         ← CodeMirror 浅色主题
      ├── <PortInspector />        ← 端口列表 + uniform 值编辑 + builtin AUTO 徽章
      └── <OnnxPanel />            ← ONNX 模型参数面板
  </main>
  <canvas ref={canvasRef} />       ← 隐藏的 WebGL 后端画布
</App>
```

---

## 6. Shader 端口自动生成

用户写 GLSL 后实时正则解析：

```glsl
uniform float intensity;       →  Port(float, "intensity")
uniform vec2 resolution;       →  Port(vec2, "resolution")
uniform sampler2D image;       →  Port(sampler2D, "image")
uniform vec4 tint;             →  Port(vec4, "tint")

out vec4 fragColor;            →  Port(vec4, "fragColor")
```

端口颜色按数据类型区分（蓝=float，绿=vec2，红=vec4，黄=sampler2D 等）。

---

## 7. 时间系统

OpenQuartz 的时间管理是**多层正交**的：物理时钟驱动循环调度，循环调度控制帧执行，帧内按节点类型分路径处理。

### 7.1 物理时钟 — `Clock`

**文件**：`src/engine/clock.ts`

```
start() → 清零所有计数器
tick(now) → 产出 TimeState { time, delta, frame, date, fps }
pause()  → 冻结时间
resume() → 补偿暂停时长，保持 time 连续
seek(t)  → 跳转到指定时间点
```

关键语义：
- **`time`**（=`iTime`）：`(now − startTime) / 1000 − pauseElapsed`，连续累计，pause 自动补偿暂停时间，保证 resume 后不跳变
- **`delta`**（=`iTimeDelta`）：raw delta clamped to 0.1s，防止浏览器 tab 休眠后 delta 爆炸导致物理模拟爆炸（spiral-of-death）
- **`frame`**（=`iFrame`）：单调递增整数，每 `tick()` 一次，无论 delta 大小
- FPS 通过 60 样本环形缓冲区滑动平均计算
- `TimeState` 是**单例可复用对象**（mutated in place），每帧零分配

### 7.2 循环调度 — `RealtimeHost`

**文件**：`src/engine/realtimeHost.ts`

```
play()
  → compositor.prepare()   编译 shader、分配 FBO
  → clock.start()          重置时钟
  → mouse.attach()         开始追踪鼠标
  → rAF(tick)              启动渲染循环

tick(now)
  → [编译标志? compositor.prepare() 热更新]
  → clock.tick(now)        推进时钟
  → 收集 video textures
  → 组装 FrameInputs
  → compositor.render()    执行一帧
  → renderToScreen()       mirror canvas blit
  → [动态? rAF(tick) : 停止]
```

三态控制：
| 操作 | Clock | Video | 管线 |
|------|-------|-------|------|
| PLAY | `start()` | 启动采集 | rAF 循环 |
| PAUSE | `pause()` | `pause()` | 冻结 delta=0，buffer 不释放 |
| RESUME | `resume()` | `play()` | 时间连续，buffer 状态保持 |
| STOP | `reset()` | `dispose()` | `clearResources()` 销毁所有 FBO |

### 7.3 Static vs Dynamic 管线

**检测函数**：`isStaticPipeline()` 

扫描所有节点：
- 含有 `iTime` / `iTimeDelta` / `iFrame` / `iMouse` 引用 → **dynamic**
- 含有 `previousFrame` 引用（feedback） → **dynamic**
- 含有 `video` 输入节点 → **dynamic**
- 没有任何动态源 → **static**

| 模式 | 行为 |
|------|------|
| **Static** | 单帧渲染。`rAF` 跑一次即停。适合静态图片处理、纯 shader 链 |
| **Dynamic** | 连续 rAF 循环。每帧 clock 前进，渲染结果不断更新 |

### 7.4 帧内执行 — `ExecutionEngine.runFrame()`

每帧严格按照拓扑序遍历所有节点：

```
tick() 内的时间线 (单帧):
clock → math(CPU同步) → shader/render(同步) → onnx?(跳过/异步) → renderToScreen
                          ↑ feedback: 读 ping-pong A，写 B，swap
```

各节点类型在帧内的时间行为：

| 节点类型 | 执行时机 | 语义 |
|----------|----------|------|
| **Math** | 同步，当前帧 | CPU 运算，结果即时用于下游 scalar uniform |
| **Shader** | 同步，当前帧 | GPU 渲染到 FBO，结果即时用于下游 sampler2D |
| **Feedback** | 同步，当前帧 | 读 ping-pong 读端，写另一端，帧尾 swap |
| **Renderer** | 同步，当前帧末 | blit 上游 FBO 到 mirror canvas |
| **ONNX** | 异步，跨帧 | 跳过 in-flight，1-2 帧后结果可用（见 7.6） |

### 7.5 Feedback / Accumulator 时间语义

**原理**：shader 通过引用 `previousFrame` 标识符隐式声明需要反馈。编译器自动检测，引擎自动创建 ping-pong 双缓冲，每帧自动换手。

```
prepare():
  → 检测 compiled.needsFeedback
  → 分配两个 WebGLRenderTarget: targetA, targetB
  → 存入 feedbackTargets[nodeId] = [targetA, targetB]
  → feedbackReadIndex[nodeId] = 0

第一帧 (feedbackFirstFrame):
  → clearColor 清空双缓冲 ← 初始化状态
  → previousFrame = targetA (刚清空)
  → shader 执行，渲染到 targetB
  → swap: readIndex = 1

第二帧:
  → previousFrame = targetB (有上一帧结果)
  → shader 执行，渲染到 targetA
  → swap: readIndex = 0

后续:         每帧交替读写，互不干扰
暂停:         缓冲区冻结，恢复后从暂停点继续
停止:         clearResources() 销毁双缓冲，下次 play 重新初始化
```

### 7.6 ONNX 异步执行

**当前策略**：best-effort 延迟（非阻塞）

```
Frame N:     ONNX 无 in-flight
             → 读 upstream(shader FBO / image)
             → blit 到 scratch FBO
             → readPixels 回 CPU canvas
             → 启动 async inference
             → this.onnxInFlight.add(nodeId)
             → textureSources 保留旧结果（初始帧无结果时跳过）

Frame N+1:   in-flight → 跳过，下游继续消费旧结果

Frame N+K:   ONNX 完成
             → 更新 textureSources[nodeId] = { kind: 'image', texture: result }
             → 下次 tick 下游节点自动读到新结果
             → 如果 pipeline 是 static，触发 scheduleRerender() 追回
```

**已知问题**：当前 ONNX 启动 inference 后需要等待异步 Promise 完成。由于 JavaScript 的单线程事件循环，inference 至少跨越 1 个 rAF 间隔（~16ms），导致下游至少有 1 帧的 stale 输出。对于大模型（Real-ESRGAN、MiDaS）这个延迟可能扩展到多个帧。

**远期优化方向**：
- ONNX 结果就绪后，在不等待下一帧 tick 的前提下，立即触发一次增量 render pass，仅更新受影响的下游子图
- 当前 `scheduleRerender()` 在 static 模式下已实现了基本版本，但 dynamic 模式下尚未做增量更新

### 7.7 热更新（Playing 中编辑）

```typescript
updateGraph(nodes, edges):
  → this.needsRecompile = true

下一次 tick():
  → if (needsRecompile) compositor.prepare()
  → 重新编译所有 shader，重新分配所有 FBO
  → feedbackFirstFrame 重新填充 → ping-pong 双缓冲从 Clear Color 重新开始
  → ONNX 输出缓存 (onnxOutputCache) 保留，避免模型重新推理
```

---

## 8. 渲染管线

1. **PLAY** — App 创建 `RealtimeHost`，传入隐藏 `<canvas>` 和回调
2. **RealtimeHost** 持有 `Clock`、`MouseState`、`VideoSource` 集合，启动 `requestAnimationFrame` 循环
3. **每帧 tick**：
   - `Clock.tick(now)` → 产生 `TimeState`（iTime, iTimeDelta, iFrame, iDate, fps）
   - 遍历 `VideoSource` 取最新 `THREE.VideoTexture`
   - 组装 `FrameInputs { time, delta, frame, date, mouse, resolution, videoTextures }`
   - 调用 `Compositor.render(inputs)`
4. **Compositor** 包装 `ExecutionEngine`：
   - `prepare(nodes, edges)` — 拓扑排序，编译 shader，分配 FBO
   - `render(inputs)` — 执行 `engine.runFrame(plan, inputs)`
   - 按拓扑序逐节点渲染到各自 FBO；上游纹理自动绑定到 uniform
   - Feedback 节点：自动创建双缓冲，每帧 `previousFrame` + swap
   - ONNX 节点：异步推理，非阻塞，best-effort 延迟
5. **Renderer 节点**（绿色 header，QC 的 QCView 等价物）：
   - 不创建额外 FBO，直接读取上游 shader 的 FBO 纹理
   - `renderRendererToScreen(nodeId)` 将上游 FBO blit 到主画布
   - 通过 mirror canvas（`<canvas id="renderer-mirror-{nodeId}">`）GPU→GPU blit 显示
   - 多个 Renderer 节点各自拥有独立的 mirror canvas
6. **GPU-only 输出** — 实时路径不调用 `readPixels`/`toDataURL`，零 GPU→CPU readback
7. **暂停/恢复** — `Clock` 支持 pause/resume/seek，暂停时冻结 `iTime`
8. **热更新** — 播放中修改节点/连线，`updateGraph()` 标记 `needsRecompile`，下帧重编译

---

## 9. 工程文件保存/载入

### 保存
```
用户点击 SAVE
  → 序列化 graph (nodes, edges, projectName)
  → 收集所有 sampler2D 图片 dataUrl
  → 格式化 ProjectFile JSON
  → 触发下载 .quartz.json
```

### 载入
```
用户点击 LOAD → 选择 .quartz.json
  → FileReader 读取 + JSON.parse
  → 恢复 graph (nodes + edges)
  → 恢复图片数据
  → React Flow 自动重建
```

---

## 10. 软件架构

### 10.1 现状问题

当前代码库 10,320 行 / 54 文件，存在以下架构问题：

| 问题 | 症状 |
|------|------|
| **God Store** | `useGraphStore.ts`（756 行）混合图 CRUD、ONNX 模型管理、undo/redo、播放控制、项目 I/O、UI 选中状态。直接 import 8 个 engine 模块 |
| **God Engine** | `executionEngine.ts`（1,266 行）单体类：shader 编译、4 种 ONNX 推理路径、math 求值、纹理管理、feedback buffer、preview readback、renderer 输出 |
| **Engine → Store 反向依赖** | `executionEngine.ts` import `useGraphStore` 在异步 ONNX 推理中读写 node data（`onnxBackend`）。引擎不应知道 store 存在 |
| **无 pipeline 抽象** | 执行是 topo-sort 后一个 `for` 循环 + `if (type === 'shader') ... else if (type === 'onnx') ...` 分支。无可插拔 executor 接口。ONNX 异步用 `Set<string>` 手动跟踪 |
| **UI 直接 import engine** | Header → shaders/onnxCatalog/mathOps，SidePanel → onnxRegistry/onnxSession。无 service 层隔离 |

### 10.2 目标架构

```
src/
├── types/                  ← 纯类型，零运行时依赖
│
├── catalog/                ← 静态注册表（纯数据，无副作用）
│   ├── shaders/            shader presets: ShaderEntry[]
│   ├── onnx.ts             ONNX 模型目录: CatalogEntry[]
│   └── math.ts             Math 运算库: MathOp[]
│
├── runtime/                ← 执行引擎（纯逻辑，禁止 import store/components）
│   ├── pipeline/
│   │   ├── Pipeline.ts         响应式管线：graph diff → 增量 plan → dirty-set 执行
│   │   ├── NodeExecutor.ts     interface NodeExecutor { prepare, execute, dispose }
│   │   ├── ShaderExecutor.ts   implements NodeExecutor — shader 编译 + FBO 渲染
│   │   ├── OnnxExecutor.ts     implements NodeExecutor — 异步推理 + 结果缓存
│   │   ├── MathExecutor.ts     implements NodeExecutor — CPU 运算
│   │   └── RendererExecutor.ts implements NodeExecutor — blit to screen
│   ├── gpu/
│   │   ├── WebGLBackend.ts     Three.js renderer + render target 生命周期
│   │   └── TexturePool.ts      引用计数的 FBO/texture 缓存
│   ├── onnx/
│   │   ├── ModelManager.ts     模型下载 + buffer 缓存
│   │   ├── InferenceSession.ts ORT 会话管理
│   │   ├── Introspect.ts       模型 I/O 元数据提取
│   │   └── Overlay.ts          检测/分割结果可视化
│   ├── Scheduler.ts            frame loop + async work tracking
│   ├── Clock.ts
│   └── MouseState.ts
│
├── store/                  ← Zustand slices，只存 UI/graph 状态
│   ├── graphSlice.ts       nodes, edges, CRUD, undo/redo
│   ├── transportSlice.ts   play/pause/stop, fps, currentTime
│   ├── projectSlice.ts     projectName, savedFilePath
│   ├── uiSlice.ts          selectedNodeId, activeRendererId, previews, nodeErrors
│   └── index.ts            combine slices → useGraphStore
│
├── services/               ← 胶水层：store ↔ runtime 的唯一桥接
│   ├── PipelineService.ts  subscribe(store) → drive Pipeline
│   └── OnnxService.ts      model download/probe → update store
│
├── components/             ← 纯 UI，只 import store + catalog
│   ├── Header.tsx
│   ├── SidePanel/
│   ├── NodeGraph/
│   └── ImageLightbox.tsx
│
├── utils/
└── App.tsx                 ← 挂载 PipelineService + layout
```

### 10.3 分层依赖规则

```
types  ←  catalog  ←  runtime  ←  services  ←  App
                                      ↕
                                    store  ←  components
```

| 层 | 可 import | 禁止 import | 职责 |
|----|-----------|------------|------|
| **types** | — | 任何运行时模块 | 纯 TypeScript 类型定义 |
| **catalog** | types | runtime, store, components | 静态数据注册表：shader 预设、ONNX 目录、math 运算表 |
| **runtime** | types, catalog | store, components, react | 管线执行引擎：编译、渲染、推理、调度。通过回调接口输出结果，不知道 store 存在 |
| **store** | types, catalog | runtime, components | Zustand 状态切片：图数据、传输控制、项目元数据、UI 状态 |
| **services** | types, catalog, runtime, store | components | 桥接层：订阅 store 变化驱动 runtime，将 runtime 回调写回 store |
| **components** | types, catalog, store | runtime | React UI 组件：只读/写 store，不直接接触引擎 |

### 10.4 响应式管线设计

当前 `ExecutionEngine.runFrame()` 是命令式 for 循环：每帧遍历全部节点，按类型 if/else 分支。目标是改为**增量响应式管线**——只执行变脏的节点，通过可插拔 executor 接口消除类型分支。

#### NodeExecutor 接口

```typescript
interface NodeExecutor {
  /** 编译/初始化（graph 变更时调用）。返回该节点的 port 签名 */
  prepare(node: NodeDescriptor, backend: WebGLBackend): PrepareResult;

  /** 每帧执行。inputs 是上游节点的输出，按 port name 索引 */
  execute(inputs: Map<string, TextureSource | number>, frameInputs: FrameInputs): ExecuteResult;

  /** 释放 GPU 资源 */
  dispose(): void;
}

// 每种节点类型注册一个 executor
const EXECUTORS: Record<NodeType, () => NodeExecutor> = {
  shader:   () => new ShaderExecutor(),
  onnx:     () => new OnnxExecutor(),
  math:     () => new MathExecutor(),
  renderer: () => new RendererExecutor(),
  input:    () => new InputExecutor(),
  constant: () => new ConstantExecutor(),
};
```

#### 增量执行（Dirty-Set）

```typescript
class Pipeline {
  private executors = new Map<string, NodeExecutor>();
  private dirty = new Set<string>();
  private outputs = new Map<string, TextureSource>();
  private topo: string[] = [];        // 缓存的拓扑序

  /** graph 变更 → diff → 增量更新 executor 实例 */
  updateGraph(nodes, edges): void {
    // 1. 新增节点 → 创建 executor + prepare
    // 2. 删除节点 → dispose executor
    // 3. 变更节点 → re-prepare
    // 4. 重算拓扑序
    // 5. 标脏所有受影响节点
  }

  /** 输入变化（视频帧、uniform 编辑）→ 标脏下游 */
  markDirty(nodeId: string): void {
    this.dirty.add(nodeId);
    for (const downstream of this.dependents(nodeId)) {
      this.dirty.add(downstream);
    }
  }

  /** 每帧：只跑 dirty 节点，按拓扑序 */
  tick(inputs: FrameInputs): void {
    for (const id of this.topo) {
      if (!this.dirty.has(id)) continue;
      const executor = this.executors.get(id)!;
      const nodeInputs = this.gatherInputs(id);
      const result = executor.execute(nodeInputs, inputs);
      this.outputs.set(id, result);
    }
    this.dirty.clear();
  }
}
```

**好处**：
- 静态管线（无时间变量）在首帧后 dirty set 为空，零 GPU 开销
- 编辑 uniform 只标脏该节点及其下游，上游不受影响
- ONNX 异步完成后，`markDirty(onnxNodeId)` 触发下游增量更新
- 新增节点类型只需实现 `NodeExecutor`，不改 Pipeline 核心

**vs RxJS**：考虑过 RxJS Observable Graph（每个 node = `Observable<TextureSource>`，edges = `pipe`/`combineLatest`），但 60fps 实时管线中 Observable 的 per-frame allocation 和 WebGL 同步渲染语义不搭。Dirty-set 方案零外部依赖、零分配、复杂度可控。

### 10.5 Store 切片设计

将单体 `useGraphStore`（756 行）拆为 4 个职责单一的 slice：

| Slice | 状态 | 职责 |
|-------|------|------|
| **graphSlice** | `nodes`, `edges`, `undoStack`, `redoStack` | 图 CRUD、undo/redo、节点工厂（addShaderNode 等）。Import catalog 获取预设数据 |
| **transportSlice** | `loopState`, `fps`, `currentTime`, `currentFrame` | 播放控制：play/pause/resume/stop。不依赖 engine |
| **projectSlice** | `projectName`, `savedFilePath` | 工程文件元数据。序列化/反序列化调用 utils/projectIO |
| **uiSlice** | `selectedNodeId`, `activeRendererId`, `outputPreviews`, `nodeErrors`, `outputData` | UI 展示状态。services 层写入预览和错误 |

合并导出：

```typescript
// store/index.ts
export const useGraphStore = create<GraphState>()(
  immer((...args) => ({
    ...graphSlice(...args),
    ...transportSlice(...args),
    ...projectSlice(...args),
    ...uiSlice(...args),
  }))
);
```

对外接口不变——所有 `useGraphStore(s => s.xxx)` 调用无需修改。

### 10.6 Service 层设计

Service 层是 store 与 runtime 的**唯一桥接**，消除当前 engine→store 的反向依赖。

#### PipelineService

```typescript
// services/PipelineService.ts
class PipelineService {
  private pipeline: Pipeline;
  private host: RealtimeHost;

  constructor(canvas: HTMLCanvasElement) {
    this.pipeline = new Pipeline(new WebGLBackend(canvas));
  }

  /** 在 App mount 时调用，订阅 store 变化 */
  attach(): () => void {
    return useGraphStore.subscribe((state, prev) => {
      // graph 变更 → pipeline.updateGraph()
      if (state.nodes !== prev.nodes || state.edges !== prev.edges) {
        this.pipeline.updateGraph(state.nodes, state.edges);
      }
      // transport 变更 → start/stop frame loop
      if (state.loopState !== prev.loopState) {
        this.handleTransport(state.loopState);
      }
    });
  }

  /** pipeline 回调 → 写回 store */
  private onOutput = (nodeId: string, dataUrl: string) => {
    useGraphStore.getState().setOutputPreview(nodeId, dataUrl);
  };
  private onError = (nodeId: string, error: string) => {
    useGraphStore.getState().setNodeError(nodeId, error);
  };
}
```

#### OnnxService

从 `useGraphStore` 中提取的 ONNX 模型下载/WebGPU probe 逻辑：

```typescript
// services/OnnxService.ts
// subscribe graph changes → detect new ONNX nodes → trigger download → update store status
// probe WebGPU compatibility → update onnxBackend in store
```

### 10.7 Catalog 抽离

当前 `engine/shaders/`、`engine/onnxCatalog.ts`、`engine/mathOps.ts` 本质上是**纯数据注册表**，不包含运行时逻辑，但因为放在 `engine/` 下导致 components 必须 import engine。

抽离到 `catalog/` 后：
- `components/Header.tsx` import `catalog/shaders` 而非 `engine/shaders`
- `components/SidePanel/OnnxPanel.tsx` import `catalog/onnx` 而非 `engine/onnxCatalog`
- `store/graphSlice.ts` import `catalog/math` 而非 `engine/mathOps`
- `runtime/` import `catalog/` 获取模型描述符

### 10.8 实施路径

| PR | 内容 | 风险 | 依赖 |
|----|------|------|------|
| **PR 1: Store 拆片** | `useGraphStore` → 4 slices，合并导出，外部接口不变 | 低 | — |
| **PR 2: Catalog 抽离** | `engine/shaders/`、`onnxCatalog`、`mathOps` → `catalog/`。更新所有 import | 低 | — |
| **PR 3: Executor 接口** | 定义 `NodeExecutor`，从 executionEngine 提取 ShaderExecutor / OnnxExecutor / MathExecutor / RendererExecutor。`Pipeline` 类替代 `ExecutionEngine` | 中 | PR 2 |
| **PR 4: Service 层** | `PipelineService` + `OnnxService` 桥接 store↔pipeline。消除 engine→store 反向依赖。App.tsx 精简 | 中 | PR 1, PR 3 |

PR 1 和 PR 2 互不依赖，可并行。PR 3 依赖 PR 2（executor 需要从 catalog 读取数据）。PR 4 依赖 PR 1 和 PR 3。

## 11. 实现状态

| 模块 | 状态 |
|---|---|
| Vite + React + TS + Tailwind 脚手架 | ✅ |
| React Flow 节点图 + 自定义节点 | ✅ |
| 六种节点：Shader / Input / Constant / ONNX / Renderer / Math | ✅ |
| GLSL 正则解析（uniform / out） | ✅ |
| Shader 编译（RawShaderMaterial + GLSL3） | ✅ |
| WebGL FBO 渲染管线 | ✅ |
| 拓扑排序执行引擎 | ✅ |
| CodeMirror shader 编辑器 | ✅ |
| PortInspector uniform 值编辑 + builtin AUTO 徽章 | ✅ |
| InputNode 图片加载 + 缩略图 | ✅ |
| 工程文件保存/载入 .quartz.json | ✅ |
| macOS 极简 UI 风格 | ✅ |
| 版本管理（version.ts） | ✅ |
| 实时渲染循环（RealtimeHost + rAF） | ✅ |
| Renderer 节点（mirror canvas blit） | ✅ |
| 视频输入（摄像头 + 文件） | ✅ |
| ONNX 推理节点（异步非阻塞，best-effort） | ✅ |
| 时间系统（iTime/iTimeDelta/iFrame/iDate） | ✅ |
| 多 Renderer 支持（各自独立 mirror canvas） | ✅ |
| PLAY/PAUSE/STOP 三态传输控制 | ✅ |
| Clock 暂停/恢复/seek | ✅ |
| MouseState（Shadertoy iMouse 约定） | ✅ |
| 每节点 iResolution（各 shader 独立 FBO 尺寸） | ✅ |
| 视频尺寸向下游传播 | ✅ |
| Tauri 桌面端支持（可选） | ✅ |
| Math 节点（29 个 CPU 运算） | ✅ |
| System 源节点（Time/Mouse/Resolution） | ✅ |
| Auto 类型推定 + 宽类型连线 | ✅ |
| SOURCE 菜单（重组 INPUT 为 SYSTEM/CONSTANTS/EXTERNAL） | ✅ |
| Feedback/Accumulator（ping-pong 双缓冲 + previousFrame 自动注入） | ✅ |
| 隐式声明式反馈检测（compiler 自动识别 previousFrame） | ✅ |
| FEEDBACK 预设 shader（Gray-Scott Reaction-Diffusion） | ✅ |
| Static/Dynamic 管线自动识别（含 feedback 检测） | ✅ |
| ONNX 异步：最佳努力延迟（已知问题：至少 2 帧延迟，规划优化） | ⚠️ |

---

## 12. 关键设计决策

| 决策 | 选择 | 理由 |
|---|---|---|
| 节点渲染 | 自定义组件 + Tailwind | 完全控制外观，不依赖 React Flow 默认主题 |
| Shader 编译 | RawShaderMaterial + GLSL3 | 避免 Three.js 自动注入与 #version 冲突 |
| WebGL 上下文管理 | 单个隐藏 canvas + mirror blit | 避免多上下文，GPU→GPU 拷贝 |
| Handle 定位 | position:relative 父容器 | 确保多端口各占一行，不重叠 |
| 边类型 | bezier | 视觉效果流畅 |
| UI 框架 | 纯 Tailwind，无组件库 | 轻量，macOS 风格自由定制 |
| FBO 管线 | 零冗余 FBO，叶子 shader 即输出点 | 业务性能最优（见下文） |
| 节点架构 | Renderer 节点读取上游 FBO，无额外渲染 pass | 零拷贝输出，GPU-only |
| PixelRatio | 离屏管线固定 pixelRatio=1 | FBO 渲染不需要 DPI 缩放 |
| Host/Compositor 分离 | RealtimeHost 驱动循环，Compositor 封装渲染 | 关注点分离：时间/输入 vs 图执行 |
| Mirror Canvas Blit | drawImage(glCanvas) GPU→GPU | 多 Renderer 各自独立画布，无需多 WebGL 上下文 |
| ONNX 异步推理 | async inference + 旧帧回退 | 非阻塞，不卡主渲染循环，best-effort |
| GPU-only 输出 | 实时路径零 readPixels | 消除 GPU→CPU readback 开销 |
| Feedback 激活方式 | **隐式声明式**：compiler 检测 `previousFrame` 标识符 | 用户不需要手动开关，shader 声明即启用 |
| Feedback 存储 | ping-pong WebGLRenderTarget 双缓冲 | 避免读/写同一 FBO 的 undefined behavior |
| Feedback 初始化 | clear color 清空 + shader frame 0 自举 | 灵活控制初始状态（Gray-Scott: A=1, B=0） |

---

## 13. 渲染管线设计原则

**核心原则：零冗余 FBO，GPU-only 输出，业务性能最优。**

- 无独立 Output 节点。Renderer 节点直接读取上游 shader 的 FBO，不创建额外渲染 pass。
- 管线中不创建任何不必要的中间 FBO。每个 FBO 的存在必须有明确的业务语义（输入纹理缓存、或多级 shader 链的中间结果、或 ping-pong 反馈对）。
- **每节点 iResolution**：每个 shader 在其自身 FBO 尺寸下执行，分辨率不是全局的。Renderer 节点跟随上游 shader 的 FBO 尺寸。
- 离屏渲染管线使用 `pixelRatio=1`，不受屏幕 DPI 影响。FBO 尺寸即像素尺寸，所见即所得。
- **GPU-only 输出**：实时渲染路径不调用 `readPixels`/`toDataURL`，预览通过 mirror canvas 的 `drawImage` 实现 GPU→GPU blit。
- **Renderer 跟随上游尺寸**：Renderer 节点不定义自己的分辨率，完全继承上游 shader FBO 的宽高。
- 视频输入尺寸自动传播到下游 shader 的默认 FBO 大小。
- 工程文件版本号随数据模型变更递增，LOAD 时严格校验版本，不兼容则报错拒绝加载。

---

## 14. Math 节点设计方案（已实现）

### 背景

QC 的 Math/Logic patch 是纯 CPU 运算节点——接收标量/向量输入，执行数学运算，输出结果。不走 GPU shader 管线。QC 的 Math patch 是宽类型的：一个 `Add` patch 既能加两个 float 也能加两个 vec3。

### 设计原则

1. **Math 是 CPU 节点，不是 shader** — 不编译 GLSL，不分配 FBO，不走 GPU 管线
2. **宽类型匹配** — 端口声明为 `'auto'` 类型，实际类型从连线对端推定
3. **类型提升规则** — 遵循 GLSL 隐式转换：`int → float`，`float → vecN`（broadcast），`vecN + vecN → vecN`（逐分量）
4. **仅对 Math 节点放宽连线规则** — 其他节点类型仍严格匹配

### 数据模型

```typescript
type NodeType = 'shader' | 'input' | 'constant' | 'onnx' | 'renderer' | 'math';

// Math 节点专用字段
interface ShaderNodeData {
  // ... 现有字段
  mathOp?: string;              // 运算标识：'add' | 'multiply' | 'sin' | 'clamp' | ...
}

// 新增特殊 DataType
type DataType = GlslDataType | LogicalDataType | 'auto';
```

### 端口类型推定

Math 节点的端口声明为 `dataType: 'auto'`：

```
未连线时：          连线后：
┌─────────┐        ┌─────────┐
│  Add     │        │  Add     │
│ a: auto ─┤        │ a: vec3 ─┤ ← 上游是 vec3
│ b: auto ─┤        │ b: float─┤ ← 上游是 float
│─ out:auto│        │─ out:vec3│ ← 提升为最宽类型
└─────────┘        └─────────┘
```

推定规则：
1. `prepare()` 时遍历 Math 节点的连线
2. 输入端口类型 = 对端输出端口的 `dataType`
3. 输出端口类型 = 所有输入类型的最宽类型（`float < vec2 < vec3 < vec4`，`int → float`）
4. 未连线的输入用 `float` 作为默认

### 连线规则放宽

`onConnect` 中：
- 如果 source 或 target 的端口是 `'auto'` 类型 → 允许任何标量/向量连接
- 其他节点的端口仍严格类型匹配
- sampler2D / samplerCube 不允许连到 Math 端口（Math 不处理纹理）

### 引擎执行

`runFrame()` 中 Math 节点的处理：

```
1. 收集输入值：
   - 从上游 Input 节点的 uniforms 取值
   - 从上游 System 节点的 FrameInputs 取值
   - 从上游 Math 节点的计算结果取值

2. 类型转换 + broadcast：
   - int → float
   - float → vecN: broadcast 到 [x, x, x, x]
   - vecN → vecM (N < M): 用 0 补齐

3. CPU 计算：
   - 纯 JS 运算，按 mathOp 分发
   - 结果存入 plan 的值映射表

4. 下游 shader 消费时：
   - 和消费 Input 节点标量值一样
   - 通过 scalarBindings 注入到 shader uniform
```

### Math 运算库

分类与 QC 对齐：

| 类别 | 运算 | 输入 → 输出 |
|---|---|---|
| **算术** | Add, Subtract, Multiply, Divide, Negate | (a, b) → a⊕b |
| **取整** | Floor, Ceil, Round, Fract, Mod | (a) → a' 或 (a, b) → a' |
| **范围** | Min, Max, Clamp, Saturate, Step, Smoothstep | (a, b) → a' |
| **插值** | Mix (Lerp), Map Range | (a, b, t) → a' |
| **三角** | Sin, Cos, Tan, Asin, Acos, Atan, Atan2 | (a) → a' |
| **指数** | Pow, Sqrt, Exp, Log, Abs, Sign | (a) → a' |
| **向量** | Dot, Cross, Length, Normalize, Distance, Reflect | (a, b) → scalar/vector |

### UI

- **节点外观**：橙色 header，标题显示运算名称（如 "Add"、"Sin"）
- **菜单**：SOURCE 和 SHADER 之间新增 MATH 按钮，按类别分组子菜单
- **SidePanel**：显示端口列表 + 推定类型 + 常量输入编辑（未连线的输入可手动设值）
- **节点上显示运算符号**：如 `+` `×` `sin` 等

### 典型使用场景

```
Time → Math(Multiply, b=0.5) → Shader.speed     # 半速时间
Time → Math(Sin) → Math(Add, b=0.5) → Shader.x   # 正弦振荡 0..1
Mouse.xy → Math(Divide, b=resolution) → Shader.uv # 归一化鼠标坐标
```

### 实施阶段

1. **Phase 1**：✅ 新增 `'math'` 节点类型 + `'auto'` DataType + 连线放宽 + `runFrame` CPU 执行分支 + 基础算术运算（Add/Subtract/Multiply/Divide）
2. **Phase 2**：✅ 完整运算库（29 个运算：三角/指数/范围/插值/取整）+ MathNode UI 组件 + MathSidePanel
3. **Phase 3**：✅ 端口类型推定 + 类型提升 + broadcast 逻辑 + System 源节点（TIME/MOUSE/RESOLUTION）

---

## 15. Feedback / Accumulator 设计方案（已实现）

### 背景

引擎默认每个 shader 节点是纯函数——输入进，输出出，无跨帧状态。这阻止了时间相关效果（运动模糊、反应扩散、流体模拟、递归反馈）。QC 的 **Accumulator patch** 和 **Core Image Accumulator** 提供了帧间存储。

### 设计原则

1. **隐式声明式** — shader 代码中引用 `previousFrame` 标识符即触发 ping-pong 双缓冲，无需用户手动开关
2. **声明即注入** — 编译器自动注入 `uniform sampler2D previousFrame;`，无需用户在 GLSL 中声明
3. **零额外分配** — 使用现有的 `WebGLRenderTarget` 分配器，仅增加一个额外 target
4. **无缝集成** — 不改变 DAG 拓扑；feedback 不是图边，而是引擎自动管理的隐式关联

### 数据流

```
┌─────────────────────────────────────────┐
│            Feedback Node                 │
│                                         │
│  readTarget ──→ uniform previousFrame ──→│
│                                         │
│  writeTarget ←── render pass ───────────│
│                                         │
│  帧尾: swap(readIndex)                  │
└─────────────────────────────────────────┘
```

### 实现架构

```
shaderCompiler.ts:  compileNodeShader()
  → 检测 /\bpreviousFrame\b/
  → 注入 uniform sampler2D previousFrame;
  → 返回 needsFeedback: true

executionEngine.ts: prepare()
  → compiled.needsFeedback
  → createTarget × 2 (ping-pong 对)
  → store in feedbackTargets[nodeId]

executionEngine.ts: runFrame()
  → feedbackTargets.has(nodeId)
  → 首次: clearTarget × 2 (clearColor)
  → bind: setUniform(material, 'previousFrame', readTarget.texture)
  → render: renderWithMaterial(material, writeTarget)
  → swap: feedbackReadIndex[nodeId] ^= 1
```

### Clear Color 语义

Clear Color 决定了 feedback 缓冲区的初始状态。对于 Gray-Scott 反应扩散：
- `(R=1, G=0, B=0, A=0)` — A 化学物质浓度 1.0，B 物质浓度 0.0
- Frame 0 shader 在这个基础上写入种子区域

对于 motion blur trails：
- `(R=0, G=0, B=0, A=0)` — 全黑首次帧，然后每帧混合

### 典型使用场景

| 效果 | Shader 策略 | Clear Color |
|------|-------------|-------------|
| Reaction-Diffusion | 存储 A/B 浓度在 RG 通道，frame 0 写种子 | (1, 0, 0, 0) |
| Motion Blur Trails | `mix(previousFrame, currentFrame, 0.9)` | (0, 0, 0, 0) |
| Video Feedback | 缩放/旋转/混合 previousFrame + live input | (0, 0, 0, 0) |
| Fluid Simulation | 多 buffer（速度场/压力场）在 RG 通道 | (0, 0, 0, 0) |

### 实施阶段

1. **Phase 1**：✅ ping-pong 双缓冲 + `previousFrame` 自动注入 + clear/reset + GLSL 隐式声明检测
2. **Phase 2**：✅ UI 只读指示器 + Clear Color 调色板 + Gray-Scott Reaction-Diffusion 预设
3. **Phase 3**：⏳ Motion Blur Trails 预设
4. **Phase 4**：⏳ Video Feedback 万花筒
