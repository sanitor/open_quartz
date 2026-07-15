# OpenQuartz vs Quartz Composer — 功能差距分析

> 基于 Apple Quartz Composer 全功能集与 OpenQuartz v0.7.1b 的对比

---

## 已对齐 ✅

| QC 功能 | OpenQuartz 实现 |
|---|---|
| 节点图 DAG 编辑（拖拽、连线、排列） | React Flow 画布 + MiniMap + 框选 |
| 端口类型校验 | 连线时类型检查 + 颜色提示 |
| Inspector 面板 | SidePanel：shader 编辑、端口检查、uniform 编辑 |
| 图片输入 | sampler2D + Image/Framebuffer 两种模式 |
| 多 pass 渲染管线 | 拓扑排序 + FBO 链 + 分辨率传播 |
| 工程文件保存/加载 | .quartz.json + 版本校验 |
| Undo/Redo | 50 级历史栈 |
| Shader 编辑 + 错误提示 | CodeMirror 6 + GLSL lint + autocompletion |
| 预定义效果模板 | 10 个（Sobel、Blur、Sharpen、Invert 等） |
| **实时渲染循环** | **rAF 驱动 Host/Compositor 架构，PLAY/PAUSE/STOP 三态控制** |
| **时间系统** | **Shadertoy 兼容 iTime/iTimeDelta/iFrame/iDate/iMouse/iResolution，声明即注入** |
| **Renderer 节点（QCView）** | **显式输出查看器，多 renderer 独立 mirror canvas，GPU→GPU blit** |
| **全屏预览** | **Renderer fullscreen overlay + SAVE 截帧** |
| **鼠标交互输入** | **iMouse uniform，Shadertoy 约定（底部原点，z/w 点击状态）** |
| **摄像头输入** | **getUserMedia → VideoTexture，实时帧更新** |
| **视频文件输入** | **HTMLVideoElement → VideoTexture，尺寸自动传播到下游 shader** |
| **实时预览** | **GPU-only 输出路径，无 readPixels，mirror canvas blit** |
| **ML 推理节点** | **ONNX 节点（YOLOv8n），异步非阻塞推理，实时路径支持** |

---

## 核心差距

### 1. 执行模型（架构差距，已大幅缩小）

**QC 的 Pull 模式：**

QC 的执行是 **pull（拉）模型** — renderer（宿主）从最终输出节点发起求值请求，沿连线向上游递归，每个 patch 被 pull 到时才计算。没有连到输出链路上的 patch 根本不执行。

**OpenQuartz 当前的 Push 模式：**

OpenQuartz 是 **push（推）模型** — 拓扑排序后从上游到下游逐个执行，全图都会跑。已有 rAF 驱动的实时渲染循环和 Host/Compositor 分离架构。

剩余差距：
- ❌ Pull 模式的按需求值（当前 push 模式对全图执行可接受，但 pull 在节点数量大时更高效）
- ❌ 参数拖拽时实时更新画面（需要 slider/color picker UI）

### 2. 内置 Patch 库

QC 有几百个内置 patch，按类别组织。OpenQuartz 只有 10 个 shader 模板 + 1 个 ONNX 模型。

| QC 类别 | 代表性 Patch | OpenQuartz |
|---|---|---|
| **Generator** | Color, Gradient, Noise (Perlin/Simplex), Checkerboard, Solid Color, Star Shape | ❌ |
| **Filter / Image** | Blur (Gaussian/Motion/Zoom), Color Controls, Hue Rotate, Levels, Threshold | 部分（10 个模板） |
| **Blend Mode** | Add, Multiply, Screen, Overlay, Difference | ❌ |
| **Distortion** | Twirl, Bump Distortion, Ripple, Pinch, Displacement Map | ❌ |
| **Transition** | Dissolve, Swipe, Flash, Mod, Bars Swipe | ❌ |
| **Math** | Add, Multiply, Divide, Min, Max, Interpolation, Math Expression | ❌ |
| **Logic** | If/Then, Multiplexer, Demultiplexer, Boolean Logic | ❌ |
| **Signal** | LFO (Sine/Square/Saw/Triangle), Wave Generator, Pulse, Counter | ❌ |
| **3D** | Cube, Sphere, Teapot, Mesh Loader, Light, Camera, 3D Transform | ❌ |
| **Controller / Input** | ~~Mouse~~, Keyboard, MIDI Controller, Audio Input, Trackball | ✅ 鼠标（iMouse），❌ 其余 |
| **String** | String Concat, Number Formatter, Date/Time Formatter | ❌ |
| **Network** | Image Downloader, RSS Feed, XML Parser | ❌ |
| **Video / Audio** | ~~Video Input (Camera)~~, ~~Movie Loader~~, Audio File Player | ✅ 摄像头 + 视频文件，❌ 音频 |
| **Render in Image** | 将子图渲染到纹理 | ❌（等价于当前 FBO，但缺子图封装） |
| **ML / AI** | 无（QC 时代没有） | ✅ **ONNX 节点（超越 QC）** |

### 3. 交互输入

| 输入类型 | QC | OpenQuartz | 状态 |
|---|---|---|---|
| 鼠标位置/点击 | ✅ | ✅ iMouse uniform | **已对齐** |
| 摄像头输入 | ✅ | ✅ getUserMedia + VideoTexture | **已对齐** |
| 键盘输入 | ✅ | ❌ | 待实现 |
| MIDI 控制器 | ✅ | ❌ | P4 |
| 音频输入 | ✅ | ❌ | P4 |
| LFO / Wave Generator | ✅ | ❌ | P2 |
| Trackball | ✅ | ❌ | P3 |

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

| 输出能力 | QC | OpenQuartz | 状态 |
|---|---|---|---|
| 全屏预览窗口 | ✅ | ✅ Fullscreen overlay | **已对齐** |
| 帧截图导出 | 无内置 | ✅ SAVE PNG | **超越 QC** |
| 视频录制/导出 | 第三方插件 | ❌ | 待实现 |
| 序列帧导出 | 无 | ❌ | 待实现（OfflineHost 架构已预留） |
| Screen Saver 输出 | ✅（macOS 专属） | ❌ | 不适用（跨平台） |
| Syphon / Spout | ✅ | ❌ | P4 |

---

## 部分对齐（有但不完整）

| 功能 | Quartz Composer | OpenQuartz | 差距描述 |
|---|---|---|---|
| 预定义 Patch/Shader | 数百个，按类别组织 | 10 个模板 + 1 ONNX | 数量级差距，缺 Generator/Blend/Distortion/Math/Logic |
| 参数动画 | 实时 + LFO + 时间轴 | iTime 驱动 shader 动画 | 缺 LFO/Signal 节点和参数时间轴 |
| 数据类型 | image, number, string, color, boolean, struct, index, virtual | GLSL 标量/向量/矩阵/sampler + LogicalDataType (roi/mesh/json) | 缺 string/color picker/struct/array |
| 参数 UI | Slider, Color Picker, Popup, Text Field, Image Well | Text Field + Checkbox | 缺 Slider 和 Color Picker |
| 复制/粘贴 | 支持跨 composition | 部分支持（图内） | 缺跨工程 |
| 文档/帮助 | 每个 patch 有内置文档 | 无 | 缺 patch 级别文档 |

---

## 超越 QC 的能力

| 能力 | 说明 |
|---|---|
| **ML 推理节点** | ONNX 节点在浏览器中运行 YOLOv8n，WebGPU/WASM EP，QC 时代不存在 |
| **跨平台** | Web + Tauri 桌面，不绑定 macOS |
| **GPU-only 预览** | 实时路径零 readback，mirror canvas blit |
| **视频文件持久化** | Tauri asset protocol 保存绝对路径，跨会话恢复 |
| **帧截图** | Fullscreen overlay + SAVE 导出当前帧 PNG |
| **Shadertoy 兼容** | iTime/iMouse/iResolution 约定，可直接搬运 Shadertoy 代码 |
| **开源** | 用户直接写 GLSL 或扩展节点类型，无私有 API 门槛 |

---

## 建议优先级（更新）

| 优先级 | 功能 | 价值 | 工作量 | 状态 |
|---|---|---|---|---|
| ~~**P0**~~ | ~~实时渲染循环 + 时间系统~~ | ~~QC 核心体验~~ | ~~中~~ | ✅ **已完成** |
| ~~**P2**~~ | ~~摄像头/视频文件输入~~ | ~~实时视频处理场景~~ | ~~中~~ | ✅ **已完成** |
| ~~**P2**~~ | ~~全屏预览窗口~~ | ~~演示展示必备~~ | ~~小~~ | ✅ **已完成** |
| ~~**P1**~~ | ~~鼠标交互输入节点~~ | ~~最基本的实时交互驱动~~ | ~~小~~ | ✅ **已完成** |
| **P1** | 更多内置 shader（Generator + Blend + Distortion） | 用户不用从零写 GLSL | 中 | 待实现 |
| **P1** | 参数 Slider + Color Picker UI | 拖拽调参比输入数字直觉得多 | 小 | 待实现 |
| **P2** | Macro / 子图封装 | 图复杂后必须有 | 大 | 待实现 |
| **P2** | 节点搜索 + 右键菜单 | 编辑效率 | 小 | 待实现 |
| **P2** | LFO / Signal 处理节点 | 配合实时循环做参数动画 | 中 | 待实现 |
| **P2** | 视频录制/导出 | 成果输出（OfflineHost 架构已预留） | 中 | 待实现 |
| **P3** | 流程控制（条件/循环/JS） | 高级用户需要 | 大 | 待实现 |
| **P3** | 3D 渲染基础（几何体 + 灯光 + 相机） | 大工程量，视定位而定 | 特大 | 待实现 |
| **P3** | 键盘输入 | 交互驱动 | 小 | 待实现 |
| **P4** | 音频/MIDI 输入 | 小众但 VJ 场景刚需 | 中 | 待实现 |
| **P4** | Syphon/Spout 视频流共享 | 专业 VJ 场景 | 大 | 待实现 |

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

### QC 被淘汰的根本原因

- **平台锁定** — .qtz 绑死 macOS，无法跨平台
- **Apple 停止维护** — 最后更新停留在 2014 年
- **封闭生态** — 无法扩展、无社区贡献通道
- **没有 ML/AI** — QC 时代没有端侧推理能力

---

## OpenQuartz 的定位与差异化

### 定位对比

| | Quartz Composer | OpenQuartz |
|---|---|---|
| 本质 | 实时交互式视觉合成器 | **硬件加速图像视频处理图编辑器与运行环境** |
| 平台 | macOS only | Web + 桌面（Tauri），跨平台 |
| 生态 | 封闭，Apple 独占 | 开源，社区驱动 |
| 输出格式 | .qtz（macOS 专属） | .quartz.json（通用 JSON） |
| 渲染技术 | Core Image + OpenGL（已废弃） | WebGL2 / GLSL 300 es（现代标准） |
| ML/AI | 无 | ONNX runtime（WebGPU/WASM） |
| 扩展性 | 需 Objective-C 插件 | 用户直接写 GLSL + ONNX 模型 |

### 当前进展总结

v0.7.1b 完成了从"静态 shader 图编辑器"到"实时视觉合成器"的关键转折：

- ✅ 实时渲染循环 + Host/Compositor 架构（QC 的 QCRenderer 等价物）
- ✅ Shadertoy 兼容时间系统（iTime/iMouse/iResolution）
- ✅ Renderer 节点（QC 的 QCView 等价物）+ 多 renderer + fullscreen
- ✅ 摄像头 + 视频文件输入
- ✅ GPU-only 输出路径
- ✅ ONNX ML 推理（超越 QC）
- ✅ 642 tests，78% 覆盖率

### 下一步重点

1. **P1: 更多内置 shader 模板** — Generator/Blend/Distortion 系列，降低用户 GLSL 门槛
2. **P1: 参数 Slider + Color Picker** — 拖拽调参的核心体验
3. **P2: LFO/Signal 节点** — 配合实时循环做参数动画
4. **P2: 视频导出** — OfflineHost 架构已预留，实现导出 MP4/GIF
5. **P2: 节点搜索 + 右键菜单** — 编辑效率
