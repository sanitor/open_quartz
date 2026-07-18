# OpenQuartz — 硬件加速图像视频处理图编辑器

> Version 0.11.0b — 受 Apple Quartz Composer 启发的可视化节点图编辑与运行环境

---

## 1. 项目概述

OpenQuartz 是一个基于 Web 的硬件加速图像和视频处理节点图编辑器与运行环境，受 Apple Quartz Composer 和 Shadertoy 启发。
- 用户通过**节点图（DAG）**组织 GLSL shader 片段
- 每个节点是一个可编程的 shader 处理单元
- **Shader 即接口声明**：解析 GLSL `uniform` 自动生成输入端口，`out` 生成输出端口
- 输入输出可互相连接，类型校验
- 支持**工程文件保存/载入**：整个图结构 + 节点状态 + 输入值 + 图片数据序列化为 `.quartz.json` 文件
- **实时渲染循环**：rAF 驱动的 Host/Compositor 架构，PLAY/PAUSE/STOP 三态控制
- **Renderer 节点**：显式输出查看器（QC 的 QCView 等价物），直接读取上游 FBO 显示
- **视频输入**：摄像头与文件视频作为 sampler2D 纹理源
- **ONNX 推理节点**：异步机器学习推理，非阻塞，best-effort 延迟
- **时间系统**：Shadertoy 兼容的 iTime/iTimeDelta/iFrame/iDate/iMouse/iResolution 内置 uniform
- **Feedback/Accumulator**：shader 通过 `previousFrame` 隐式声明式读取自身上一帧输出，引擎自动启用 ping-pong 双缓冲

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

## 10. 当前目录结构

```
open-quartz/
├── DESIGN.md
├── CHANGELOG.md
├── index.html
├── package.json
├── vite.config.ts
├── tsconfig.json / tsconfig.app.json / tsconfig.node.json
├── src-tauri/                        ← Tauri 桌面端（可选）
│   ├── tauri.conf.json
│   └── Cargo.toml
├── src/
│   ├── main.tsx
│   ├── App.tsx                       ← RealtimeHost 生命周期管理
│   ├── index.css                     ← Tailwind 入口 + React Flow 样式重置
│   ├── version.ts                    ← 版本号
│   ├── components/
│   │   ├── Header.tsx                ← macOS 菜单栏 + PLAY/PAUSE/STOP + FPS
│   │   ├── NodeGraph/
│   │   │   ├── index.tsx             ← ReactFlow 容器
│   │   │   └── nodes/
│   │   │       ├── ShaderNode.tsx
│   │   │       ├── InputNode.tsx
│   │   │       ├── OnnxNode.tsx      ← ONNX 推理节点
│   │   │       ├── RendererNode.tsx  ← 输出查看器（mirror canvas blit）
│   │   │       └── MathNode.tsx      ← CPU 运算节点（auto 类型推定）
│   │   ├── SidePanel/
│   │   │   ├── index.tsx
│   │   │   ├── ShaderEditor.tsx      ← CodeMirror 6
│   │   │   ├── PortInspector.tsx     ← uniform 编辑 + builtin AUTO 徽章
│   │   │   └── OnnxPanel.tsx         ← ONNX 模型参数
│   │   └── ImageLightbox.tsx
│   ├── engine/
│   │   ├── realtimeHost.ts           ← rAF 循环 + Clock/Mouse/Video 管理
│   │   ├── compositor.ts             ← 组合器：包装 ExecutionEngine
│   │   ├── clock.ts                  ← 时钟：iTime/iTimeDelta/iFrame/iDate/fps
│   │   ├── mouseState.ts             ← 鼠标状态：iMouse（Shadertoy 约定）
│   │   ├── videoSource.ts            ← 视频源：HTMLVideoElement → THREE.VideoTexture
│   │   ├── executionEngine.ts        ← 执行引擎：编译/FBO 分配/逐帧渲染
│   │   ├── shaderParser.ts           ← 正则解析 GLSL
│   │   ├── shaderCompiler.ts         ← RawShaderMaterial 编译 + previousFrame 自动检测
│   │   ├── graphExecutor.ts          ← 拓扑排序
│   │   ├── webglRenderer.ts          ← Three.js FBO 管线
│   │   ├── mathOps.ts                ← 29 个数学运算注册表
│   │   ├── onnxRegistry.ts           ← ONNX 模型注册表
│   │   ├── onnxSession.ts            ← ONNX Runtime 会话管理
│   │   ├── onnxOverlay.ts            ← ONNX 检测结果叠加渲染
│   │   ├── predefinedShaders.ts      ← 预设 shader
│   │   ├── shaderLinter.ts           ← Shader 语法检查
│   │   └── shaderCompletions.ts      ← Shader 自动补全
│   │   └── shaders/
│   │       ├── index.ts              ← shader 组注册表
│   │       ├── filter.ts
│   │       ├── color.ts
│   │       ├── generator.ts
│   │       ├── blend.ts
│   │       ├── distortion.ts
│   │       ├── templates.ts
│   │       └── feedback.ts           ← 隐式声明式反馈预设 shader
│   ├── store/
│   │   └── useGraphStore.ts          ← Zustand 全局状态
│   ├── utils/
│   │   ├── graphUtils.ts
│   │   ├── projectIO.ts              ← 序列化/反序列化
│   │   ├── rawPreview.ts             ← 原始数据预览
│   │   └── tauri.ts                  ← Tauri 文件路径转换
│   └── types/
│       └── index.ts                  ← 类型定义 + DATA_TYPE_COLORS
```

---

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
