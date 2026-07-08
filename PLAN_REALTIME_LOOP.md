# 实时渲染循环 + 时间系统 + Video 输入 — 实现方案

> OpenQuartz P0 架构升级：从"手动 RUN 单次执行"进化为"rAF 驱动实时合成器"

---

## 现状分析

| 维度 | 当前状态 | 目标状态 |
|---|---|---|
| 执行模型 | Push，手动点 RUN 单次全图执行 | rAF 驱动连续渲染循环 |
| 时间 | 无时间概念 | 宿主注入 `iTime` / `iTimeDelta` / `iFrame` / `iDate` |
| 交互 | 静态参数输入 | `iMouse` 实时跟踪 |
| 帧率 | 无 | FPS 显示 + 可选帧率限制 |
| 视频输入 | 不支持 | 摄像头 + 视频文件作为 sampler2D 纹理源 |
| 预览更新 | 执行完成后一次性 readback | 每帧实时更新预览 canvas |

---

## 架构设计

### 核心原则

1. **宿主-Composition 分离** — 借鉴 QC 的 `QCRenderer` 架构：Composition（节点图）是**无状态纯函数**，给一个 `(graph, time, inputs)` 出一帧图；Host（宿主）负责驱动时间、管理输入源、决定何时求值。两者通过 `Compositor` 接口干净解耦
2. **时间是普通 uniform** — 不是特殊机制，shader 通过 `uniform float iTime;` 声明即可使用，与其他 uniform 一样走端口解析
3. **时间由宿主注入** — Composition 不持有时钟。宿主每帧传入 `time` 参数（QC 的 `renderAtTime:` 模式），不同宿主可以用不同时间策略
4. **无结束时间** — 与 QC / Shadertoy 一致，Composition 没有"总时长"概念，永远跑到宿主停止。未来离线导出时，导出器（另一种宿主）自行指定起止时间
5. **现有 push 模型保留** — rAF 驱动的本质仍是每帧 push 全图，只是从"单次"变"连续"
6. **零分配热路径** — 渲染循环内不创建对象、不触发 GC，uniform 值直接写入已有 buffer

### QC 架构参考

```
Quartz Composer 的宿主模型：

┌─────────────────────┐     renderAtTime:t
│ 宿主 (Host)          │ ──────────────────► ┌──────────────┐
│                     │                      │ Composition  │
│ • QCView            │ ◄────────────────── │ (纯函数)      │
│ • CVDisplayLink     │     返回一帧图        │ .qtz 文件    │
│ • 屏保引擎           │                      └──────────────┘
│ • Cocoa 应用         │
│ • 离线渲染脚本       │
└─────────────────────┘

宿主可以是任何东西。Composition 不管谁在调它、什么时候调、调多久。
QCRenderer.renderAtTime:arguments: 的示例代码：

  for(double t = 0.0; t <= 10.0; t += 1.0/25.0)
      [myRenderer renderAtTime:t arguments:nil];

实时预览：CVDisplayLink 回调时间戳，跟显示器刷新走，无限循环
离线渲染：宿主按固定步长递增（如 +1/30s），不跟墙上时钟走
交互 scrub：直接传拖动位置，不需要渲染循环
```

### Push vs Pull 求值模型

**QC 的 Pull 模型：**

QC 的求值是 pull（拉）模式——宿主从最终输出节点发起求值请求，沿连线向上游递归，每个 patch 被 pull 到时才计算。没有连到输出链路上的 patch 根本不执行。

**OpenQuartz 当前的 Push 模型：**

拓扑排序后从上游到下游逐个执行，全图都会跑。简单直接，小图（几十节点）下性能无差异。

| | Push（当前） | Pull（QC） |
|---|---|---|
| 求值方向 | 源头 → 叶子，全图执行 | 输出节点 → 递归向上拉依赖 |
| 未连接节点 | 照跑（浪费但不影响正确性） | 不执行（天然剪枝） |
| 缓存/跳帧 | 无 | 节点可判断输入未变 → 跳过重算 |
| 复杂度 | O(全图节点数) | O(活跃路径节点数) |
| 适用规模 | 小图（< 50 节点） | 大图 + 条件分支 + Iterator |

**决策：当前保持 Push，不提供切换选项。**

理由：
1. Push 和 Pull 是**内部求值策略**，不是用户应该关心的交互概念。用户不需要知道图是怎么求值的
2. 在当前节点规模下（通常 < 20 个节点），push 全图执行的开销可忽略。过早优化为 pull 增加复杂度但无实际收益
3. `Compositor.render()` 的接口不预设求值方向——未来从 push 切换到 pull 对宿主完全透明，是 Compositor 内部的优化
4. 真正需要 pull 的场景（数百节点 + 条件分支 + Iterator 循环）属于 P3 特性，届时再引入

### iMouse 与 Renderer 节点

`iMouse` 的坐标是相对于**输出画面**的像素坐标（原点左下角）。这隐含一个前提：**有一个明确的"用户正在看的输出"**。

- **QC**：`QCView` 是显式的渲染视图，composition 可以有多个 `QCView`，每个有自己的窗口和交互
- **Shadertoy**：只有一个 Image 输出 canvas，天然唯一
- **OpenQuartz 当前**：叶子 shader 节点隐式充当输出点，没有显式的"观察窗口"概念

**解决方案：新增 Renderer 节点类型。**

Renderer 节点对应 QC 的 `QCView` / Shadertoy 的 Image canvas——一个**显式的输出观察点**，接收上游 shader 的输出纹理，提供：
- 可展开的预览窗口（节点内嵌 canvas）
- 该窗口上的 `iMouse` 事件捕获
- 该窗口的分辨率即 `iResolution`

```
┌──────────┐     ┌──────────┐     ┌──────────────────┐
│  Shader  │────►│  Shader  │────►│  🖥 Renderer     │
│  Blur    │     │  Color   │     │                  │
└──────────┘     └──────────┘     │  ┌────────────┐  │
                                  │  │  预览窗口   │  │
                                  │  │  (canvas)   │  │
                                  │  │  iMouse ←── │  │ ← 鼠标事件在此捕获
                                  │  └────────────┘  │
                                  │  512×512  60fps  │
                                  └──────────────────┘

一个图可以有多个 Renderer 节点（多视图）：

  ShaderA ──► 🖥 Renderer 1 (折叠)
  ShaderB ──► 🖥 Renderer 2 (展开) ← 当前活跃，iMouse 来源
  ShaderC ──► 🖥 Renderer 3 (折叠)
```

**设计细节：**

```typescript
// 新增节点类型
export type NodeType = 'shader' | 'input' | 'constant' | 'renderer';

// Renderer 节点专用字段（复用 ShaderNodeData）
// rendererWidth / rendererHeight: 输出分辨率，默认 512×512
// expanded: 预览窗口是否展开
```

**Renderer 节点的行为：**

| 特性 | 行为 |
|---|---|
| 输入端口 | 唯一：`uniform sampler2D inputTexture`（接收上游 shader 的 FBO 输出） |
| 输出端口 | 无（DAG 终点） |
| Shader 代码 | 固定 passthrough（不可编辑）：`fragColor = texture(inputTexture, v_uv);` |
| 预览窗口 | 节点内嵌 canvas，可折叠/展开，展开后显示实时渲染结果 |
| iMouse 捕获 | **仅在展开的预览窗口上**捕获鼠标事件 → 生成 `iMouse` uniform |
| iResolution | 由 `rendererWidth × rendererHeight` 决定 |
| 活跃输出 | **展开预览窗口的 Renderer = 活跃输出**。多个展开时取最后被交互（点击/hover）的 |

**iMouse 的流向：**

```
用户鼠标在 Renderer 2 的预览窗口上移动/点击
    ↓
RealtimeHost 捕获事件 → 更新 MouseState
    ↓
compositor.render(inputs) 中 inputs.mouse = [x, y, clickX, clickY]
    ↓
图中所有声明了 uniform vec4 iMouse 的 shader 收到同一组值
（坐标相对于活跃 Renderer 的分辨率）
```

**与现有叶子节点预览的关系：**

- 现有叶子 shader 节点的 FBO readback 缩略图保留（已有功能，轻量静态预览）
- Renderer 节点提供**交互式实时预览**——独立 canvas、鼠标交互、未来可弹出全屏
- 没有 Renderer 节点的图仍然正常工作，只是没有 `iMouse` 输入（纯数据流处理）
- Renderer 不消耗额外 FBO——直接将上游纹理渲染到自己的屏幕 canvas

### OpenQuartz 的宿主-Composition 分层

```
┌──────────────────────────────────────────────────────────────┐
│                        App.tsx (React)                       │
│         ┌─────────┐  ┌──────────────┐                       │
│         │ Header  │  │  FPS Display │                       │
│         └────┬────┘  └──────────────┘                       │
│              │ ▶ PLAY / ■ STOP / ⏸ PAUSE                   │
├──────────────┼───────────────────────────────────────────────┤
│              ▼                                               │
│  ┌───────────────────────────────────────────────┐           │
│  │              Host 层（宿主）                    │           │
│  │                                               │           │
│  │  ┌─────────────────┐                          │           │
│  │  │  RealtimeHost   │◄─── rAF 驱动              │           │
│  │  │  (实时预览宿主)  │                          │           │
│  │  ├─────────────────┤                          │           │
│  │  │ - clock: Clock  │  持有时钟                 │           │
│  │  │ - mouse: Mouse  │  持有输入源               │           │
│  │  │ - videoSources  │  持有视频源               │           │
│  │  │ - rafId         │  持有 rAF handle          │           │
│  │  └────────┬────────┘                          │           │
│  │           │ 每帧：compositor.render(time, inputs)         │
│  │           ▼                                               │
│  │  ┌─────────────────┐     未来可替换为：        │           │
│  │  │  OfflineHost    │     固定步长离线导出       │           │
│  │  │  EmbedHost      │     iframe 嵌入           │           │
│  │  │  ScrubHost      │     时间轴拖拽            │           │
│  │  └─────────────────┘                          │           │
│  └───────────────────────────────────────────────┘           │
│              │                                               │
│              ▼                                               │
│  ┌───────────────────────────────────────────────┐           │
│  │           Compositor（合成器 = Composition 求值）│           │
│  │                                               │           │
│  │  纯函数接口：                                   │           │
│  │  render(graph, time, inputs) → frame           │           │
│  │                                               │           │
│  │  内部：                                        │           │
│  │  ┌──────────────────┐  ┌──────────────────┐   │           │
│  │  │ ExecutionEngine   │  │  WebGLRenderer   │   │           │
│  │  │ prepare() 编译    │  │  FBO 管线        │   │           │
│  │  │ runFrame() 求值   │  │  纹理管理        │   │           │
│  │  └──────────────────┘  └──────────────────┘   │           │
│  └───────────────────────────────────────────────┘           │
└──────────────────────────────────────────────────────────────┘
```

**关键边界：**

- **Host 拥有**：时钟、输入源（鼠标/视频/摄像头）、rAF 循环、生命周期管理
- **Compositor 拥有**：图求值、shader 编译、FBO 管线、uniform 注入
- **Host → Compositor**：`render(graph, time, inputs)` — 每帧调用，传入时间和所有输入值
- **Compositor → Host**：帧结果（FBO 纹理）、错误回调
- **Compositor 不知道**：谁在调它、rAF 还是固定步长、是否有 UI

这意味着未来可以无缝支持：
| 宿主 | 时间策略 | 场景 |
|---|---|---|
| `RealtimeHost` | rAF + 墙上时钟 | 实时预览（当前） |
| `OfflineHost` | 固定步长 `+1/fps` | 视频导出、序列帧导出 |
| `ScrubHost` | 用户拖拽位置 | 时间轴 scrub |
| `EmbedHost` | 外部传入 | iframe 嵌入到其他应用 |

---

## 模块详细设计

### 1. Clock — 时间系统 `src/engine/clock.ts`

```typescript
export interface TimeState {
  time: number;       // 秒，从 play 开始累计（可暂停）
  delta: number;      // 上一帧耗时（秒）
  frame: number;      // 帧序号（从 0 开始）
  date: Float32Array; // [year, month, day, secondsOfDay] — Shadertoy iDate 兼容
  fps: number;        // 实时 FPS（滑动窗口平均）
}

export class Clock {
  private startTime = 0;
  private lastTime = 0;
  private elapsed = 0;
  private frameCount = 0;
  private paused = false;
  private pauseElapsed = 0;

  // FPS 计算：滑动窗口
  private frameTimes: number[] = [];
  private readonly FPS_WINDOW = 60;

  /** 开始/重置 */
  start(): void;

  /** 暂停 — 冻结 elapsed，rAF 仍在跑（保持视频输入更新等） */
  pause(): void;

  /** 恢复 */
  resume(): void;

  /** 每帧调用，传入 rAF 的 DOMHighResTimeStamp，返回当前帧时间状态 */
  tick(now: DOMHighResTimeStamp): TimeState;

  /** 重置到 t=0 */
  reset(): void;

  /** Seek 到指定时间点（用于未来时间轴 scrub） */
  seek(t: number): void;
}
```

**设计决策：**
- `elapsed` 是**逻辑时间**，暂停时不推进，`delta` 为 0
- `fps` 用最近 60 帧的滑动窗口平均，避免单帧抖动
- `date` 用 `Float32Array(4)` 预分配，每帧原地更新，零 GC
- 暴露 `seek()` 为未来时间轴/scrub 预留接口

### 2. MouseState — 鼠标状态 `src/engine/mouseState.ts`

遵循 Shadertoy `iMouse` 约定：`vec4(x, y, clickX, clickY)`

```typescript
export interface MouseUniforms {
  iMouse: Float32Array; // [x, y, clickX, clickY]
}

export class MouseState {
  readonly uniforms: MouseUniforms = {
    iMouse: new Float32Array(4),
  };

  private canvas: HTMLCanvasElement | null = null;
  private readonly onMove: (e: MouseEvent) => void;
  private readonly onDown: (e: MouseEvent) => void;
  private readonly onUp: (e: MouseEvent) => void;

  /** 绑定到 canvas/容器，监听事件 */
  attach(el: HTMLElement): void;

  /** 解绑 */
  detach(): void;
}
```

**坐标约定：**
- `iMouse.xy` — 当前鼠标位置（像素，原点左下角，与 GLSL 对齐）
- `iMouse.zw` — 最近一次点击位置（按下时记录，松开后保持 / 取反）
- 与 Shadertoy 约定一致，用户可直接搬运 Shadertoy 代码

### 3. VideoSource — 视频输入 `src/engine/videoSource.ts`

```typescript
export type VideoSourceType = 'camera' | 'file';

export interface VideoSourceConfig {
  type: VideoSourceType;

  // camera 专用
  deviceId?: string;         // 指定摄像头
  facingMode?: 'user' | 'environment';

  // file 专用
  url?: string;              // blob URL 或 data URL
  loop?: boolean;            // 循环播放，默认 true
  playbackRate?: number;     // 播放速率，默认 1.0
}

export class VideoSource {
  private video: HTMLVideoElement;
  private texture: THREE.VideoTexture | null = null;
  private ready = false;

  constructor(config: VideoSourceConfig);

  /** 异步初始化：请求摄像头权限 / 加载视频文件 */
  async init(): Promise<void>;

  /** 获取 Three.js 纹理（每帧自动更新） */
  getTexture(): THREE.VideoTexture | null;

  /** 视频元数据 */
  getResolution(): { width: number; height: number };
  getDuration(): number;       // 文件视频时长，摄像头返回 Infinity
  getCurrentTime(): number;

  /** 播放控制（文件视频） */
  play(): void;
  pause(): void;
  seek(t: number): void;
  setPlaybackRate(rate: number): void;
  setLoop(loop: boolean): void;

  /** 释放资源 */
  dispose(): void;
}
```

**关键实现细节：**

1. **Three.js VideoTexture** — 基于 `HTMLVideoElement`，Three.js 每帧自动检查 `video.readyState` 并上传到 GPU，无需手动 `texImage2D`
2. **摄像头** — `navigator.mediaDevices.getUserMedia({ video: { deviceId, facingMode } })`，stream 赋给 `video.srcObject`
3. **文件视频** — 用户通过 Input 节点上传视频文件，`URL.createObjectURL(file)` → `video.src`
4. **生命周期** — VideoSource 由 Host 管理（不是 Compositor），Host 创建/销毁。stop 时释放摄像头 stream（`track.stop()`）
</invoke>
### 4. Compositor — 合成器 `src/engine/compositor.ts`

Compositor 是 Composition 的求值引擎——**纯函数语义**，不持有时钟、不管输入源、不知道谁在调它。
对应 QC 中 `QCRenderer.renderAtTime:arguments:` 的被调方。

```typescript
/** 宿主每帧传给 Compositor 的全部外部输入 */
export interface FrameInputs {
  time: number;          // 秒（iTime）
  delta: number;         // 上一帧耗时（iTimeDelta）
  frame: number;         // 帧序号（iFrame）
  date: Float32Array;    // [year, month, day, seconds]（iDate）
  mouse: Float32Array;   // [x, y, clickX, clickY]（iMouse）
  resolution: Float32Array; // [width, height, pixelRatio]（iResolution）
  videoTextures?: Map<string, THREE.Texture>; // nodeId → 视频纹理
}

export class Compositor {
  private engine: ExecutionEngine;
  private plan: ExecutionPlan | null = null;

  constructor(canvas: HTMLCanvasElement);

  /**
   * 编译阶段 — 宿主 play 时调用一次，或图变更时重新调用
   * 拓扑排序、编译 shader、分配 FBO
   */
  prepare(
    nodes: Node<ShaderNodeData>[],
    edges: Edge[],
    onNodeError?: (nodeId: string, error: string) => void,
    onOutputSize?: (nodeId: string, w: number, h: number) => void,
  ): boolean;

  /**
   * 求值一帧 — QC 的 renderAtTime: 等价物
   * 纯求值：接收外部输入，渲染全图，不涉及时钟推进
   */
  render(inputs: FrameInputs): void;

  /**
   * 读取输出 — 宿主按需调用（可节流）
   */
  readOutputs(
    onOutput: (nodeId: string, dataUrl: string) => void,
  ): void;

  /** 释放 GPU 资源 */
  dispose(): void;
}
```

**Compositor 的职责边界：**
- ✅ 编译 shader、管理 FBO、注入 builtin uniform、执行渲染
- ✅ 检查端口名是否匹配内置 uniform → 自动注入
- ❌ 不持有 Clock — 时间由宿主传入
- ❌ 不监听鼠标/键盘 — 输入值由宿主传入
- ❌ 不管理 VideoSource — 纹理由宿主传入
- ❌ 不调用 rAF — 宿主决定何时调 `render()`

### 5. RealtimeHost — 实时预览宿主 `src/engine/realtimeHost.ts`

RealtimeHost 是**当前唯一的宿主实现**，负责 rAF 驱动的实时预览。
对应 QC 中使用 `CVDisplayLink` 的 `QCView`。

```typescript
export type HostState = 'stopped' | 'playing' | 'paused';

export interface HostCallbacks {
  onFrame?: (time: TimeState) => void;
  onOutput?: (nodeId: string, dataUrl: string) => void;
  onNodeError?: (nodeId: string, error: string) => void;
  onOutputSize?: (nodeId: string, w: number, h: number) => void;
  onStateChange?: (state: HostState) => void;
}

export class RealtimeHost {
  private compositor: Compositor;
  private clock: Clock;
  private mouse: MouseState;
  private videoSources = new Map<string, VideoSource>();
  private rafId: number | null = null;
  private state: HostState = 'stopped';
  private callbacks: HostCallbacks;

  // 图快照 — play / 图变更时刷新
  private nodes: Node<ShaderNodeData>[] = [];
  private edges: Edge[] = [];

  // 预览节流
  private previewInterval = 1000 / 15; // 15fps readback
  private lastPreviewTime = 0;

  constructor(canvas: HTMLCanvasElement, callbacks: HostCallbacks);

  /** 启动实时循环 */
  play(nodes: Node<ShaderNodeData>[], edges: Edge[]): void {
    // 1. 快照图
    // 2. compositor.prepare(nodes, edges)
    // 3. clock.start()
    // 4. mouse.attach(canvas 容器)
    // 5. 启动 rAF
    // 6. state → 'playing'
  }

  pause(): void;   // 冻结时间，rAF 仍跑
  resume(): void;
  stop(): void;     // 停止循环 + 释放资源

  /** 播放中热更新图 */
  updateGraph(nodes: Node<ShaderNodeData>[], edges: Edge[]): void;

  /** 视频源管理 */
  addVideoSource(nodeId: string, config: VideoSourceConfig): Promise<void>;
  removeVideoSource(nodeId: string): void;

  /** rAF 帧回调 — Host 的核心职责 */
  private frame(now: DOMHighResTimeStamp): void {
    // 1. clock.tick(now) → timeState
    // 2. 收集 videoSource 纹理
    // 3. 组装 FrameInputs
    // 4. compositor.render(inputs)        ← 纯求值
    // 5. 节流 compositor.readOutputs()
    // 6. callbacks.onFrame(timeState)
    // 7. rafId = requestAnimationFrame(frame)
  }

  getState(): HostState;
  getClock(): Clock;
}
```

**未来的其他宿主（无需现在实现，但架构已预留）：**

```typescript
/** 离线导出宿主 — 固定步长，不跟墙上时钟 */
class OfflineHost {
  async exportFrames(
    nodes, edges,
    fps: number,           // 如 30
    duration: number,      // 如 10 秒
    onFrame: (frameIndex: number, dataUrl: string) => void,
  ): Promise<void> {
    const compositor = new Compositor(canvas);
    compositor.prepare(nodes, edges);
    const dt = 1 / fps;
    for (let i = 0; i < duration * fps; i++) {
      compositor.render({ time: i * dt, delta: dt, frame: i, ... });
      compositor.readOutputs(onFrame.bind(null, i));
    }
    compositor.dispose();
  }
}

/** Scrub 宿主 — 用户拖拽时间轴 */
class ScrubHost {
  scrubTo(t: number): void {
    compositor.render({ time: t, delta: 0, frame: Math.floor(t * fps), ... });
    compositor.readOutputs(onOutput);
  }
}
```

**预览节流策略：**
- FBO → dataURL readback 是 GPU→CPU 同步操作，开销大
- 渲染每帧都做（60fps），但 readback + 缩略图更新 15fps 即可
- 替代方案：未来可用 `canvas.captureStream()` 或直接在预览区域渲染到屏幕 canvas

### 6. ExecutionEngine 改造

现有 `run()` 是一次性异步方法。新增 **同步帧方法** 供 Compositor 内部调用：

### 7. 内置 Uniform 约定

与 Shadertoy 兼容的内置 uniform 名称：

| Uniform | 类型 | 含义 | 注入条件 |
|---|---|---|---|
| `iTime` | `float` | 播放时间（秒） | shader 声明了 `uniform float iTime` |
| `iTimeDelta` | `float` | 上一帧耗时（秒） | 声明了 `uniform float iTimeDelta` |
| `iFrame` | `int` | 帧序号 | 声明了 `uniform int iFrame` |
| `iDate` | `vec4` | `(year, month, day, seconds)` | 声明了 `uniform vec4 iDate` |
| `iMouse` | `vec4` | `(x, y, clickX, clickY)` | 声明了 `uniform vec4 iMouse` |
| `iResolution` | `vec3` | `(width, height, pixelRatio)` | 声明了 `uniform vec3 iResolution` |

**注入逻辑：**
- `shaderParser.ts` 解析出的 `inputs` 端口中，如果 `label` 匹配上表名称且**未被用户连线**，则由引擎自动注入
- 如果用户手动连线到该端口（比如用数学节点驱动 `iTime`），则用户连线优先
- 这保持了 QC 的设计哲学：时间只是一个 float 输入，可以被任意覆盖

### 8. Video Input 节点扩展

扩展现有 `InputNode`，新增 `inputMode: 'video'`：

```typescript
// types/index.ts 扩展
export type InputMode = 'image' | 'framebuffer' | 'video';

// ShaderNodeData 新增字段
export interface ShaderNodeData {
  // ... 现有字段

  // Video 输入专用
  videoSourceType?: 'camera' | 'file';
  videoUrl?: string;          // 文件视频 blob URL
  videoFileName?: string;
  videoDeviceId?: string;     // 指定摄像头
  videoLoop?: boolean;
  videoPlaybackRate?: number;
}
```

**InputNode UI 扩展：**

```
┌─────────────────────────┐
│ 🔵 Video Input          │
├─────────────────────────┤
│ Mode: [Camera ▼]        │
│                         │
│ ┌─────────────────────┐ │
│ │   📹 摄像头预览      │ │
│ │   (缩略图)           │ │
│ └─────────────────────┘ │
│                         │
│ Resolution: 1280×720    │
│ ○ output ──────────     │
└─────────────────────────┘

┌─────────────────────────┐
│ 🔵 Video Input          │
├─────────────────────────┤
│ Mode: [File ▼]          │
│                         │
│ [选择视频文件]           │
│ video.mp4               │
│ ┌─────────────────────┐ │
│ │   🎬 视频预览        │ │
│ └─────────────────────┘ │
│ Duration: 00:30         │
│ Loop: ✓  Rate: 1.0×     │
│ ○ output ──────────     │
└─────────────────────────┘
```

### 9. Store 扩展 — `useGraphStore.ts`

```typescript
interface GraphState {
  // ... 现有字段

  // 实时循环状态
  loopState: HostState;            // 'stopped' | 'playing' | 'paused'
  fps: number;                     // 实时 FPS
  currentTime: number;             // 当前播放时间（秒）
  currentFrame: number;            // 当前帧号

  // Actions
  play: () => void;
  pause: () => void;
  resume: () => void;
  stop: () => void;
  setFps: (fps: number) => void;
  setCurrentTime: (t: number) => void;
  setCurrentFrame: (frame: number) => void;
}
```

### 10. Header UI 改造

现有按钮：`▶ RUN` / `■ STOP` / `CLEAR`

改为三态控制：

```
停止状态:   [▶ PLAY]  [CLEAR]
播放状态:   [⏸ PAUSE] [■ STOP]  [CLEAR]    FPS: 60
暂停状态:   [▶ RESUME] [■ STOP]  [CLEAR]    FPS: --
```

- **PLAY** — 启动 rAF 循环，连续渲染
- **PAUSE** — 冻结时间，保持循环（视频输入仍刷新但不推进逻辑时间）
- **STOP** — 停止循环，释放资源，回到 t=0
- **CLEAR** — 清除预览（仅停止状态可用）
- **RUN** — 保留为 `Shift+Enter` 快捷键，单次执行模式（不启动循环）

FPS 显示：右上角小字，播放时实时更新。

### 11. App.tsx 改造

```typescript
export default function App() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const hostRef = useRef<RealtimeHost | null>(null);

  // 监听 loopState 变化，驱动 RealtimeHost 生命周期
  useEffect(() => {
    const unsub = useGraphStore.subscribe((state, prev) => {
      const canvas = canvasRef.current;
      if (!canvas) return;

      // play — 创建 RealtimeHost（内部创建 Compositor）
      if (state.loopState === 'playing' && prev.loopState === 'stopped') {
        const host = new RealtimeHost(canvas, {
          onFrame: (ts) => {
            useGraphStore.getState().setFps(ts.fps);
            useGraphStore.getState().setCurrentTime(ts.time);
            useGraphStore.getState().setCurrentFrame(ts.frame);
          },
          onOutput: useGraphStore.getState().setOutputPreview,
          onNodeError: (id, err) => {
            useGraphStore.getState().setNodeError(id, err);
            useGraphStore.getState().setSelectedNode(id);
          },
          onOutputSize: (id, w, h) => {
            useGraphStore.getState().updateNodeData(id, {
              resolvedWidth: w, resolvedHeight: h,
            });
          },
        });
        hostRef.current = host;
        host.play(state.nodes, state.edges);
      }

      // pause / resume
      if (state.loopState === 'paused' && prev.loopState === 'playing') {
        hostRef.current?.pause();
      }
      if (state.loopState === 'playing' && prev.loopState === 'paused') {
        hostRef.current?.resume();
      }

      // stop — 销毁 Host（内部销毁 Compositor）
      if (state.loopState === 'stopped' && prev.loopState !== 'stopped') {
        hostRef.current?.stop();
        hostRef.current = null;
      }

      // 图变更时热更新
      if (state.loopState === 'playing' &&
          (state.nodes !== prev.nodes || state.edges !== prev.edges)) {
        hostRef.current?.updateGraph(state.nodes, state.edges);
      }
    });
    return () => unsub();
  }, []);

  // ... render
}
```

---

## 文件清单

| 操作 | 文件路径 | 说明 |
|---|---|---|
| **新增** | `src/engine/clock.ts` | 时间系统 |
| **新增** | `src/engine/mouseState.ts` | 鼠标状态追踪 |
| **新增** | `src/engine/videoSource.ts` | 视频输入源（摄像头 + 文件） |
| **新增** | `src/engine/compositor.ts` | 合成器 — Composition 求值（纯函数） |
| **新增** | `src/engine/realtimeHost.ts` | 实时预览宿主 — rAF + Clock + 输入源管理 |
| **改造** | `src/engine/executionEngine.ts` | 新增 `prepare()` + `runFrame()` + `readOutputs()`，供 Compositor 内部调用 |
| **改造** | `src/types/index.ts` | 扩展 `NodeType`（+renderer）、`InputMode`（+video）、`ShaderNodeData` 字段 |
| **改造** | `src/store/useGraphStore.ts` | 新增 `loopState` / `fps` / `play()` / `pause()` / `addRendererNode()` |
| **改造** | `src/App.tsx` | 接入 `RealtimeHost` 生命周期 |
| **改造** | `src/components/Header.tsx` | PLAY/PAUSE/STOP 三态按钮 + FPS 显示 + `+RENDERER` 按钮 |
| **新增** | `src/components/NodeGraph/nodes/RendererNode.tsx` | Renderer 节点组件（内嵌 canvas + 展开/折叠） |
| **改造** | `src/components/NodeGraph/nodes/InputNode.tsx` | Video 模式 UI |
| **改造** | `src/components/SidePanel/index.tsx` | Renderer 参数面板 + Video 参数面板 |
| **改造** | `src/components/SidePanel/PortInspector.tsx` | 内置 uniform 标注 |

---

## 实施阶段

### Phase 1 — 核心循环（最小可运行）

1. 实现 `Clock`
2. 实现 `Compositor` — 封装 ExecutionEngine 的 `prepare()` + `render(inputs)` + `readOutputs()`
3. 实现 `RealtimeHost` — rAF 驱动，持有 Clock，每帧调用 `compositor.render()`
4. 改造 `ExecutionEngine` — 新增同步帧方法
5. 改造 Store — `loopState` / `play()` / `stop()`
6. 改造 Header — PLAY / STOP 按钮
7. 改造 App.tsx — 接入 RealtimeHost
**验证：** 写一个使用 `iTime` 的 shader（如颜色随时间变化），点 PLAY 后画面实时动起来。

### Phase 2 — Renderer 节点 + 鼠标

1. 新增 `RendererNode.tsx` — 内嵌 canvas、展开/折叠、上游纹理直接渲染到屏幕
2. 改造 `NodeType` — 新增 `'renderer'`
3. Header 新增 `+RENDERER` 按钮
4. Store 新增 `addRendererNode()`
5. 实现 `MouseState` — 绑定到活跃 Renderer 的预览 canvas
6. RealtimeHost 注入 `iMouse`（收集后传给 `compositor.render()`）
7. Clock 补全 `pause()` / `resume()` / `seek()`
8. Header 补全 PAUSE / RESUME + FPS 显示
9. Store 补全 `pause()` / `resume()`

**验证：** 添加 Renderer 节点，连接 shader 输出，展开预览窗口。shader 中使用 `iMouse`，在预览窗口上移动鼠标改变画面。暂停后时间冻结。

### Phase 3 — Video 输入

1. 实现 `VideoSource`
2. 扩展 `InputMode` → `'video'`
3. `InputNode` UI — Camera / File 选择
4. RealtimeHost 管理 videoSources，纹理通过 `FrameInputs.videoTextures` 传给 Compositor
5. SidePanel 视频参数面板
6. 工程文件序列化（视频文件不序列化原始数据，只保存配置）

**验证：** 添加 Camera Input 节点，连接到 shader，实时显示摄像头画面 + shader 处理效果。

### Phase 4 — 打磨

1. 内置 uniform 端口在 PortInspector 中特殊标注（自动注入 badge）
2. 预览节流优化
3. 热重编译 — 播放中编辑 shader 即时生效
4. 错误恢复 — shader 编译失败不中断循环，跳过该节点
5. 内存泄漏检查 — FBO / texture / video stream 清理

---

## 关键风险与对策

| 风险 | 影响 | 对策 |
|---|---|---|
| readback 阻塞主线程 | 卡顿 | 预览 15fps 节流；未来可用 `OffscreenCanvas` + Worker |
| 视频纹理上传开销 | 掉帧 | Three.js `VideoTexture` 自动优化；大分辨率可降采样 |
| 播放中编辑图导致竞态 | 崩溃 | `updateGraph()` 用 immutable 快照 + 下一帧生效 |
| 摄像头权限被拒 | 功能不可用 | 优雅降级，显示错误提示，不影响其他节点 |
| shader 编译错误中断循环 | 画面冻结 | 跳过出错节点，继续渲染其余链路 |
| WebGL context lost | 全黑 | 监听 `webglcontextlost`，自动重建 renderer |

---

## Shadertoy 兼容示例

实现完成后，用户可以直接使用 Shadertoy 风格的 shader：

```glsl
#version 300 es
precision highp float;

uniform float iTime;
uniform vec3 iResolution;
uniform vec4 iMouse;
uniform sampler2D iChannel0;  // ← 连接 Video Input 节点

in vec2 v_uv;
out vec4 fragColor;

void main() {
    vec2 uv = v_uv;

    // 视频纹理采样
    vec4 video = texture(iChannel0, uv);

    // 时间驱动效果
    float wave = sin(uv.x * 10.0 + iTime * 2.0) * 0.5 + 0.5;

    // 鼠标位置驱动
    vec2 mouse = iMouse.xy / iResolution.xy;
    float dist = distance(uv, mouse);

    fragColor = mix(video, vec4(wave, 0.5, 1.0 - wave, 1.0), smoothstep(0.2, 0.0, dist));
}
```
