# OpenQuartz vs Quartz Composer — 功能差距分析

> 基于 Apple Quartz Composer 全功能集与 OpenQuartz v0.6.0b 的对比

---

## 已对齐

| QC 功能 | OpenQuartz 实现 |
|---|---|
| 节点图 DAG 编辑（拖拽、连线、排列） | React Flow 画布 + MiniMap + 框选 |
| 端口类型校验 | 连线时类型检查 + 颜色提示 |
| Inspector 面板 | SidePanel：shader 编辑、端口检查、uniform 编辑 |
| 实时预览 / 结果查看 | FBO readback → 缩略图 + Lightbox |
| 图片输入 | sampler2D + Image/Framebuffer 两种模式 |
| 多 pass 渲染管线 | 拓扑排序 + FBO 链 + 分辨率传播 |
| 工程文件保存/加载 | .quartz.json + 版本校验 |
| Undo/Redo | 50 级历史栈 |
| Shader 编辑 + 错误提示 | CodeMirror 6 + GLSL lint + autocompletion |
| 预定义效果模板 | 10 个（Sobel、Blur、Sharpen、Invert 等） |

---

## 核心差距

### 1. 执行模型与时间系统（最大架构差距）

**QC 的 Pull 模式：**

QC 的执行是 **pull（拉）模型** — renderer（宿主）从最终输出节点发起求值请求，沿连线向上游递归，每个 patch 被 pull 到时才计算，计算前先 pull 自己的输入依赖。没有连到输出链路上的 patch 根本不执行。

时间不是内部时钟，而是宿主每帧传入的 `NSTimeInterval` 参数（`renderAtTime:arguments:`）：
- **实时预览**：`CVDisplayLink` 回调时间戳，跟显示器刷新走
- **离线渲染**：宿主按固定步长递增（如 +1/30s），不跟墙上时钟走
- **交互 scrub**：直接传拖动位置，不需要渲染循环

Composition 本身是**无状态纯函数** — 给一个 time，出一帧图。时间只是一个 float 输入，不是特殊机制。Movie Importer 的 `Movie Time`、`Rate`、`Start Time` 都是普通端口，可以外接任意数学 patch 做变速/跳转/乒乓循环。

**OpenQuartz 当前的 Push 模式：**

OpenQuartz 是 **push（推）模型** — 拓扑排序后从上游到下游逐个执行，全图都会跑。手动点 RUN 单次执行，没有实时反馈。

缺少：

- `requestAnimationFrame` 渲染循环
- 宿主驱动的时间注入（`iTime`、`iFrame`、`iTimeDelta`）
- 参数拖拽时实时更新画面
- 帧率显示
- Pull 模式的按需求值（当前 push 模式对全图执行可接受，但 pull 在节点数量大时更高效）

### 2. 内置 Patch 库

QC 有几百个内置 patch，按类别组织。OpenQuartz 只有 10 个 shader 模板。

| QC 类别 | 代表性 Patch | OpenQuartz |
|---|---|---|
| **Generator** | Color, Gradient, Noise (Perlin/Simplex), Checkerboard, Solid Color, Star Shape | ❌ |
| **Filter / Image** | Blur (Gaussian/Motion/Zoom), Color Controls (Brightness/Contrast/Saturation), Hue Rotate, Levels, Threshold, Median, Unsharp Mask, Color Matrix | 部分（10 个模板） |
| **Blend Mode** | Add, Multiply, Screen, Overlay, Difference, Exclusion, Color Dodge/Burn, Hard/Soft Light | ❌ |
| **Distortion** | Twirl, Bump Distortion, Ripple, Pinch, Displacement Map, Vortex | ❌ |
| **Transition** | Dissolve, Swipe, Flash, Mod, CopyMachine, Bars Swipe, Ripple | ❌ |
| **Math** | Add, Multiply, Divide, Min, Max, Interpolation, Math Expression, Round, Mod, Clamp | ❌ |
| **Logic** | If/Then, Multiplexer, Demultiplexer, Boolean Logic (AND/OR/NOT), Conditional | ❌ |
| **Signal** | LFO (Sine/Square/Saw/Triangle), Wave Generator, Pulse, Counter, Sample & Hold, Smooth (Lerp) | ❌ |
| **3D** | Cube, Sphere, Teapot, Mesh Loader, Light (Ambient/Directional/Point/Spot), Camera, 3D Transform | ❌ |
| **Controller / Input** | Mouse, Keyboard, MIDI Controller, Audio Input (Spectrum/Peak), Trackball | ❌ |
| **String** | String Concat, Number Formatter, Date/Time Formatter | ❌ |
| **Network** | Image Downloader, RSS Feed, XML Parser | ❌ |
| **Video / Audio** | Video Input (Camera), Movie Loader, Audio File Player | ❌ |
| **Render in Image** | 将子图渲染到纹理 | ❌（等价于当前 FBO，但缺子图封装） |
| **Core Image Filter** | 直接嵌入 macOS Core Image 滤镜库 | ❌（平台相关，不可移植） |

### 3. 交互输入

QC 支持实时交互驱动 composition：

- ❌ **鼠标位置/点击** → `iMouse` uniform
- ❌ **键盘输入** → 触发事件 / 字符输入
- ❌ **MIDI 控制器** → 参数映射（旋钮/推子/打击垫）
- ❌ **摄像头输入** → 实时视频帧作为纹理
- ❌ **音频输入** → 频谱分析 / 振幅 / 节拍检测
- ❌ **LFO / Wave Generator** → 自动参数动画（Sine/Square/Saw/Triangle）
- ❌ **Trackball** → 3D 旋转交互

### 4. 3D 渲染

QC 有完整的 3D 场景能力：

- ❌ 3D 几何体（Cube, Sphere, Teapot, 自定义 Mesh 导入）
- ❌ 灯光系统（Ambient, Directional, Point, Spot）
- ❌ 摄像机（Perspective, Orthographic, Track Ball）
- ❌ 3D 变换矩阵（Translate, Rotate, Scale）
- ❌ 粒子系统（Particle System patch）
- ❌ Sprite System

### 5. Macro / 子图封装

- ❌ **Macro Patch** — 选中一组节点 → 封装为单个 patch，对外暴露自定义端口
- ❌ **Sub-composition** — 独立 .qtz 文件作为 patch 嵌入到其他 composition
- ❌ **Published Inputs/Outputs** — 将内部参数暴露为 composition 级别的输入输出

### 6. 流程控制

- ❌ **Iterator** — 循环执行子图 N 次（用于批量绘制、粒子等）
- ❌ **Conditional (If/Then)** — 根据条件选择不同处理路径
- ❌ **JavaScript Patch** — 内嵌 JS 脚本处理数据
- ❌ **Queue / Delay** — 时序控制（延迟 N 帧）
- ❌ **Pulse / Gate** — 信号触发
- ❌ **Counter** — 递增/递减计数器
- ❌ **Sample & Hold** — 采样保持

### 7. 图编辑增强

- ❌ **节点搜索** — Cmd+Enter / 双击画布 → 搜索 patch 名称快速添加
- ❌ **节点分组** — 可视化框 + 颜色标记
- ❌ **注释 Notes** — 画布上添加文字说明
- ❌ **节点折叠** — 折叠隐藏端口，只显示 header
- ❌ **连线样式切换** — 直线 / 曲线 / 隐藏
- ❌ **节点对齐 / 自动布局**
- ❌ **右键上下文菜单** — 快速添加 patch、断开连线、查看文档

### 8. 输出能力

- ❌ **全屏预览窗口** — 独立窗口全屏显示渲染结果
- ❌ **视频录制/导出** — 导出为 MOV/MP4/GIF
- ❌ **序列帧导出** — 逐帧导出 PNG 序列
- ❌ **Screen Saver 输出** — macOS 屏保（平台相关）
- ❌ **Syphon / Spout** — 实时视频流共享到其他应用

---

## 部分对齐（有但不完整）

| 功能 | Quartz Composer | OpenQuartz | 差距描述 |
|---|---|---|---|
| 预定义 Patch/Shader | 数百个，按类别组织 | 10 个模板 | 数量级差距，缺 Generator/Blend/Distortion/Math/Logic |
| 参数动画 | 实时 + LFO + 时间轴 | 手动输入数值 | 无任何动画能力 |
| 预览 | 实时 60fps，改参数即刷新 | 手动 RUN 单次执行 | 核心体验差距 |
| 数据类型 | image, number, string, color, boolean, struct, index, virtual | GLSL 标量/向量/矩阵/sampler | 缺 string/color picker/struct/array |
| 参数 UI | Slider, Color Picker, Popup, Text Field, Image Well | Text Field + Checkbox | 缺 Slider 和 Color Picker |
| 复制/粘贴 | 支持跨 composition | 部分支持（图内） | 缺跨工程 |
| 文档/帮助 | 每个 patch 有内置文档 | 无 | 缺 patch 级别文档 |

---

## 建议优先级

| 优先级 | 功能 | 价值 | 工作量 |
|---|---|---|---|
| **P0** | 实时渲染循环 + 时间系统 | QC 核心体验；宿主驱动 rAF 循环 + `iTime`/`iMouse` 注入，解锁动画/交互 | 中 |
| **P1** | 更多内置 shader（Generator + Blend + Distortion） | 用户不用从零写 GLSL | 中 |
| **P1** | 参数 Slider + Color Picker UI | 拖拽调参比输入数字直觉得多 | 小 |
| **P1** | 鼠标/键盘交互输入节点 | 最基本的实时交互驱动 | 小 |
| **P2** | Macro / 子图封装 | 图复杂后必须有 | 大 |
| **P2** | 全屏预览窗口 | 演示展示必备 | 小 |
| **P2** | 摄像头/视频文件输入 | 实时视频处理场景 | 中 |
| **P2** | 节点搜索 + 右键菜单 | 编辑效率 | 小 |
| **P3** | 流程控制（条件/循环/JS） | 高级用户需要 | 大 |
| **P3** | 3D 渲染基础（几何体 + 灯光 + 相机） | 大工程量，视定位而定 | 特大 |
| **P3** | 视频录制/导出 | 成果输出 | 中 |
| **P4** | 音频/MIDI 输入 | 小众但 VJ 场景刚需 | 中 |
| **P4** | LFO / Signal 处理节点 | 配合实时循环有意义 | 中 |
| **P4** | Syphon/Spout 视频流共享 | 专业 VJ 场景 | 大 |

---

## Quartz Composer 的历史用途与 .qtz 导出场景

### 早期（2004–2010）：macOS 系统级视觉引擎

QC 不只是独立工具，它是 macOS 的**视觉管线基础设施**：

- **屏幕保护程序** — macOS 内置屏保就是 .qtz 文件
- **Dashboard Widget** — macOS Dashboard 的动效由 QC 驱动
- **Core Image** — QC 是 Core Image 滤镜链的可视化编辑器，输出直接集成进系统渲染管线
- **iTunes Visualizer** — 音频可视化插件可用 QC 制作
- **Photo Booth** — 部分特效底层是 QC composition

这些场景下 .qtz 文件是**系统原生格式**，macOS 直接加载执行，不需要导出成视频。

### 中期（2010–2014）：交互原型设计工具

Facebook 设计团队重度使用 QC 做**交互原型**：

- **Origami**（Facebook 开源）— 基于 QC 的交互设计插件，Paper 和早期 Facebook app 的动效原型都在 QC 里做
- **动效验证** — 在 QC 里实时调交互动画参数（弹簧、缓动曲线），调到满意后交给工程师实现
- **输出方式**：屏幕录制为视频 → 给工程师看效果。不是代码导出，是**设计沟通工具**

### 晚期（2014–废弃）：被替代

- Apple 自己不再维护（最后一次更新随 Xcode 5）
- Origami Studio（Facebook）独立出来，脱离 QC
- Framer / Principle / After Effects 替代了原型设计场景

### .qtz 的实际集成场景

| 场景 | 使用方式 |
|---|---|
| macOS 屏保 | .qtz 放到 ~/Library/Screen Savers，系统直接加载 |
| Cocoa 应用嵌入 | `QCView` 控件加载 .qtz，实时渲染在 app 窗口里 |
| 全屏 Visualizer | 全屏输出到显示器/投影仪（VJ 演出） |
| Core Image 滤镜链 | 导出为 Core Image filter，被 Photos / Final Cut 等调用 |
| iTunes/Music 可视化 | .qtz 作为音频可视化插件 |
| 视频录制 | QC 本身不导出视频，靠第三方插件（v002 Movie Exporter）或屏幕录制 |

**关键事实**：QC 几乎**不导出通用格式**。它的价值是**实时执行**，不是离线渲染。.qtz 只能在 macOS Quartz 框架里跑，离开 Apple 生态没有意义。

### QC 被淘汰的根本原因

- **平台锁定** — .qtz 绑死 macOS，无法跨平台
- **Apple 停止维护** — 最后更新停留在 2014 年
- **封闭生态** — 无法扩展、无社区贡献通道

---

## OpenQuartz 的定位与差异化

### 定位对比

| | Quartz Composer | OpenQuartz |
|---|---|---|
| 本质 | 实时交互式视觉合成器 | 可视化 shader 图编辑器（向实时合成器演进） |
| 平台 | macOS only | Web + 桌面（Tauri），跨平台 |
| 生态 | 封闭，Apple 独占 | 开源，社区驱动 |
| 输出格式 | .qtz（macOS 专属） | .quartz.json（通用 JSON） |
| 渲染技术 | Core Image + OpenGL（已废弃） | WebGL2 / GLSL 300 es（现代标准） |
| 扩展性 | 需 Objective-C 插件 | 用户直接写 GLSL |

### OpenQuartz 应该走的路

QC 的真正价值不是"导出"，是**实时可视化**。OpenQuartz 作为跨平台开源工具，应该：

1. **实时预览**（P0）— QC 的核心体验，也是从"静态工具"变成"实时合成器"的关键转折点
2. **Shadertoy 兼容** — 支持 `iTime`/`iMouse`/`iResolution` uniform 约定，直接吸引 shader 社区用户和内容
3. **通用导出** — QC 缺的能力，OpenQuartz 可以做得更好：
   - 导出 GLSL 代码片段 → 直接用在 Web/游戏项目
   - 导出视频/GIF → 分享和演示
   - 导出 WebGL embed → 嵌入网页
4. **开放扩展** — 用户用 GLSL 而非私有插件 API 扩展功能，零门槛

### 关键转折点

加上**实时渲染循环** + 时间系统（`iTime`/`iMouse`），OpenQuartz 就从"静态 shader 图编辑器"变成"实时视觉合成器"。当前 push 模式可以保留（全图执行在节点数合理时足够快），但需要增加 `requestAnimationFrame` 驱动的渲染循环和宿主时间注入。后续所有交互、动画、视频功能才有意义。这是当前最高优先级的架构变更。
