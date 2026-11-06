# OpenQuartz 开发文档

## 当前开发入口（Rust Slint + 独立 .scr）

当前产品仅为 `apps/open_quartz_slint` 原生桌面与独立 Windows `.scr`；Web、Tauri fallback、TypeScript SDK、WASM 与 bindings API/feature 已退役。`npm run dev` 准备原生运行库并启动 Slint，`npm run build` 生成 `dist-desktop/` 中的 portable 包，`npm run test:desktop` 运行 Slint 应用测试。没有 Vite/Web/Tauri 构建入口。`dist-desktop/` 是可重新生成的忽略产物；Slint 的独立 workspace 构建目录为 `apps/open_quartz_slint/target/`。

本轮 owner 提取与生产调用已完成。最终结果：workspace 234/34 suites、完整 native 315/30 suites、Slint 73、UI 18/18、Shader 34/34、editor 10、boundary 与 macOS release 包通过；独立 ONNX-only 247/30 suites、runtime-only 279/28 suites 通过。两项整体 100% coverage gate 失败，完整矩阵/报告见 [设计 验证基线](DESIGN.md#validation)。相机、Windows/Linux/.scr 和移动端未实测，下文 Web/Tauri 规划仍为历史。

Android/Java/JNI 专用 glue 已移出本仓，不再构建 `open_quartz_jni` 或维护 Java SDK 源码对齐检查。依赖锁文件中的第三方 Android/JNI/browser 包属于跨平台库的目标依赖，不代表本仓保留这些产品或桥接层。

媒体架构与首轮实现见 [跨平台媒体编解码与 Slint 组件架构](MEDIA_CODEC_ARCHITECTURE.md#171-首轮实现与验收)。macOS 文件视频已接入独立 `oq-media` 的进程内 VideoToolbox 硬解及 CoreVideo/Metal GPU 转换；`oq` 仅表示 OpenQuartz 出处，不代表依赖。完整跨平台会话、编码和独立 Slint 播放器组件仍是设计。`npm run prepare:desktop` 在 macOS 准备校验过的 LGPL-only libav SDK；直接运行 Cargo 前也需准备运行库，必要时设置 `OPEN_QUARTZ_FFMPEG_PATH` 指向 FFmpeg executable。

跨平台硬件推理系统的目标分层、设备缓冲契约、执行方案策略与迁移验收见 [推理架构设计](INFERENCE_ARCHITECTURE.md)。当前 ONNX/ORT 读回路径与计划架构明确区分；Core ML、Windows 设备驻留和零 CPU 图像传输在通过各自平台门槛前不视为已实现。

核心仍为单一 open_quartz，根部仅 lib.rs，root 类型保留，无占位 crate。当前 owner：compiler shader/topology/plan；GraphEvaluator dirty/Math/feedback；GraphInstance identity/generation/state；PlaybackSession graph+clock+OutputHub；NativePlayer Slint 协调；OutputHub contract/registry/presentation/resources/backend/capabilities；RenderDevice GPU/readback/Windows presenter；VideoSources 图源 map/lifecycle/upload callback；InferenceTasks sessions/pending/task id/channels/workers/accepted completions。Project/history 与资源规划/模型服务仍分别归 project/resources。

advanced host 路径为 open_quartz::{compiler,evaluation,player,outputs,rendering,inference,media,resources} 与 project::catalog。Slint 使用 NativePlayer；独立 .scr 在自有 Win32 loop 中直接组合 PlaybackSession、evaluation work 与 GpuExecutor。内部 media::sources / inference::tasks 拥有 VideoSources/InferenceTasks，不新增 root alias。实现与生产调用随 owner 迁移，不保留 execution/runtime namespace 或类 alias，实际路径见 [设计 源码索引](DESIGN.md#source-map)。root 类型、既有 feature、独立组件边界保持，WASM/Tauri/bindings/browser 不恢复。

Slint NativePlayer/RenderDevice 保持原 render thread 与同一 Device/Queue；session.validate_completion 在纹理修改之前，候选 graph/clock 与整批 scalar preflight 在 commit 之前，静态数据以 Arc 共享。NativePlayer 选择图输出 present，OutputHub 另支持 backend。PreparedNativeGraph 暂存新纹理/自有视频 handles；stage_video 打开成功后清候选纹理，detach/obsolete video 在可选 image upload 前清 staged texture，失败候选保留旧 source/像素。UI request generation 与 accepted revision 分开。close 释放 descriptors/outputs，活跃 ORT worker 的 Arc session 保留到完成，InferenceTasks Drop join；这些生命周期回归结果见 验证基线。

### 桌面是否需要 FFI？

纯 Rust/Slint 桌面和 `.scr` 直接调用 typed Rust API，不需要额外导出的 SDK C ABI、JNI 或语言 FFI 桥；已移除 JS/WASM adapter 和 Tauri IPC 产品边界，不提供兼容 bindings feature/API。

这里的“无需 SDK FFI”不等于无需外部库 FFI：ORT、FFmpeg、AVFoundation/CoreVideo、Metal、D3D12/COM 等调用仍是原生后端必要的实现细节。typed planner、图校验、compiler/WGSL/catalog/ONNX 语义继续保留，旧 Engine/Runtime 的状态归属见 模块与所有权。

### 核心能力选择与验收

- `default = []`：Graph/Project、compiler、GraphEvaluator/GraphInstance、PlaybackSession/OutputHub 的纯状态；不拉入 wgpu、ORT、FFmpeg、reqwest 或 UI。纯 SDK consumer 与屏保包导出采用无设备配置。
- `gpu`：RenderDevice 的 executor/纹理/readback/presenter；`native-runtime` 仅蕴含 gpu，不自动启用媒体、推理或下载，保留既有 feature 名称与行为。
- `native-video`：平台视频 source/worker，macOS 选择独立 oq-media；`native-video-d3d12` 蕴含 native-video 与 gpu，供 Windows D3D12 路径显式选择。
- `native-onnx`：ORT session/pipeline，可独立于 GPU 和 HTTP 使用。`model-cache`：下载/哈希/文件缓存，可独立于 ORT/GPU 使用。纯 task/pre/postprocess 不随 native-onnx 关闭。

Cargo feature 可加，完整 workspace 构建会因应用重新打开设备依赖；默认无设备闭包与每个独立能力必须分别用 consumer invocation 和目标 normal/build dependency tree 验收。默认纯核心不需要先准备原生运行库；显式 native video/ONNX 的直接 Cargo 调用仍需对应 SDK/运行库。

本次默认 coverage 134 测试、完整 native 201 测试通过，但两项 100% gate 失败：Lines/Functions/Regions 分别为默认 98.53/97.88/98.34%，native 97.70/95.96/96.38%。阈值未放宽，提取的 RenderDevice/InferenceTasks/VideoSources 保持计量，NativePlayer 只沿用旧 facade exclusion。git diff --check 与无旧 namespace/class alias 扫描通过，旧 execution 目录已删除。公开 headless Player driver 与 graph_mut history 限制仍在范围之外，详见设计 验证基线。

### 当前命令与资源索引

```sh
npm ci
npm run prepare:desktop              # 准备原生库；直接 Cargo 前同样需要
npm run dev
npm run build
cargo test --workspace               # 根核心/媒体/exporter/stub workspace
npm run test:desktop                 # Slint 独立 workspace
npm run test:desktop:ui              # 真实原生窗口交互
npm run test:desktop:shaders         # catalog GPU 像素用例
npm run test:editor                 # 独立编辑器 workspace
npm run check:rust-boundaries
npm run test:sdk-coverage
```

图标源为 `apps/open_quartz_slint/icons/`；准备生成的运行库、媒体 SDK、许可证和对应源码在 `apps/open_quartz_slint/runtime/`，不再使用 `src-tauri/` 或 `public/`。`.scr` stub 从 `target/screensaver-stub/` 构建后暂存到 app runtime。macOS numeric bundle version 来自 `package.json`。Shader 唯一源为 `crates/open_quartz/src/project/shader_catalog.json`，原生 catalog 与 Shader automation 共用；根 `tests/fixtures/` 保留原生图像/Shader fixtures。以上是可运行命令与路径，不是本次已通过的检查记录。

### 可复用 Slint 编辑器

`crates/slint_code_editor` 是本仓孵化的独立 Cargo workspace，不依赖 OpenQuartz。`npm run dev:editor` 启动 WGSL/JSON 双实例示例，`npm run test:editor` 运行组件测试。OpenQuartz 通过 path dependency 消费组件；后续拆仓不需要迁移应用的 graph/SDK/GPU 代码。

组件使用说明随包放置：[简体中文](../crates/slint_code_editor/README.zh-CN.md) / [English](../crates/slint_code_editor/README.md)，包含独立应用接入、Slint 属性/回调/函数参考及当前支持边界。

消费者的 `slint-build` include paths 添加该包的 `ui/`，在 `.slint` 中导入 `CodeEditor` 并向根编译单元导出 `CodeEditorBridge`。Rust 在窗口创建后、显示前调用 `slint_code_editor::install_editor!(window, CodeEditorBridge)` 一次。宏返回该窗口的 `Rc<RefCell<Runtime>>`；自定义高亮可通过组件的 `document-handle` 与 `Runtime::document_mut(...).set_provider(...)` 接入，再调用组件 `refresh()`。provider 返回有序、不重叠、非空且位于 UTF-8 字符边界的 `Highlight` 区间；不合法输出会明确触发契约断言，不视为编译诊断。示例含真实 custom provider 开关。

公共 UI 属性：`text`、`language`（`plain`/`wgsl`/`json`）、`dark`、`read-only`、`enabled`、`label`、`find-query`、`show-toolbar`；动作有 `focus-editor`、`select-all`、`set-selection-offsets`、`refresh`，用户文本修改发出 `edited(string)`。程序替换全文建立新文档并清空文本历史；样式切换不进入历史。组件通过 model lease 在动态销毁时释放文档。

运行时使用公开 `slint::winit_030` 接收桌面 IME 事件；`on_winit_window_event` 每窗口只有一个过滤器槽位，宿主不能再无协调地覆盖它。`Image` 是 cosmic-text 同一编辑排版的输出，不是叠在另一份 TextInput 上的镜像。当前 no-wrap，滚轮支持横纵滚动；原有 CRLF 保留，新 Enter 使用 LF；内置词法器全量同步重扫，不声称大文档/长行端到端低延迟。

本轮 macOS 验证不等于跨平台发布验收。真实输入法候选操作尚未验证；读屏器完整 caret/selection/text-run 报告缺少 Slint 公开接口，现有 accessible value/set-value/selection-action 不等于完整文本无障碍。详细证据见 [研究与实现记录](WGSL_SYNTAX_HIGHLIGHT_RESEARCH.md#11-本仓组件实现与验证)。


## 单元测试计划（历史 Web/Tauri 规划）

> **归档范围：本节至文末全部为退役前 Web/Tauri 测试规划。** 当时的“当前”、命令、源码树和覆盖门槛不适用于当前原生产品；不需要安装或运行这里的前端工具。当前入口仅以上文、`package.json` 与活动 CI 为准。

### 1. 目标

- **覆盖率目标：100%**（行覆盖 line / 分支覆盖 branch / 函数覆盖 function / 语句覆盖 statement）
- 所有前端 TypeScript 模块 + Rust 后端模块均需覆盖
- CI 自动化：每次 push / PR 触发测试，覆盖率不达标则阻断合并

---

### 2. 技术选型

| 层 | 工具 | 说明 |
|---|---|---|
| 前端测试框架 | **Vitest** | 与 Vite 原生集成，兼容 Jest API，支持 ESM |
| 组件测试 | **@testing-library/react** + **jsdom** | React 组件渲染 + DOM 交互断言 |
| 覆盖率 | **@vitest/coverage-istanbul** | Istanbul 引擎，支持 line/branch/function/statement 指标 |
| WebGL Mock | **vitest-webgl-canvas-mock** 或手动 mock | 为 shaderCompiler / webglRenderer / shaderLinter 提供 WebGL2 context |
| Rust 测试 | **cargo test** | 内置单元测试框架 |
| Rust 覆盖率 | **cargo-llvm-cov** | 基于 LLVM 的精确覆盖率 |
| CI 平台 | **GitHub Actions** | 已有 release.yml，新增 ci.yml |

---

### 3. 目录结构

```
open-quartz/
├── vitest.config.ts                    ← Vitest 配置（覆盖率阈值、环境、setup）
├── tests/                              ← 所有前端测试集中管理
│   ├── setup.ts                        ← 全局 setup（WebGL mock、DOM mock 等）
│   ├── engine/
│   │   ├── shaderParser.test.ts
│   │   ├── shaderCompiler.test.ts
│   │   ├── shaderLinter.test.ts
│   │   ├── shaderCompletions.test.ts
│   │   ├── graphExecutor.test.ts
│   │   ├── executionEngine.test.ts
│   │   ├── webglRenderer.test.ts
│   │   └── predefinedShaders.test.ts
│   ├── utils/
│   │   ├── graphUtils.test.ts
│   │   ├── projectIO.test.ts
│   │   └── rawPreview.test.ts
│   ├── store/
│   │   └── useGraphStore.test.ts
│   ├── types/
│   │   └── index.test.ts
│   ├── components/
│   │   ├── Header.test.tsx
│   │   ├── ImageLightbox.test.tsx
│   │   ├── NodeGraph.test.tsx
│   │   ├── ShaderNode.test.tsx
│   │   ├── InputNode.test.tsx
│   │   ├── OutputNode.test.tsx
│   │   ├── CustomEdge.test.tsx
│   │   ├── SidePanel.test.tsx
│   │   ├── ShaderEditor.test.tsx
│   │   └── PortInspector.test.tsx
│   └── integration/
│       └── integration.test.ts         ← 跨模块集成测试
├── src-tauri/
│   └── src/
│       ├── lib.rs                      ← #[cfg(test)] mod tests
│       └── main.rs                     ← #[cfg(test)] mod tests
└── .github/
    └── workflows/
        ├── ci.yml                      ← 新增：测试 + 覆盖率 CI
        └── release.yml                 ← 已有
```

---

### 4. 各模块测试清单

#### 4.1 engine/shaderParser.ts

纯函数，无外部依赖，最高优先级。

| 测试用例 | 覆盖目标 |
|---|---|
| 解析单个 `uniform float` | 基本 uniform 解析 |
| 解析多个不同类型 uniform（vec2/vec3/vec4/mat4/sampler2D/samplerCube） | 所有 DataType 映射 |
| 解析 `out vec4 fragColor` | output port 生成 |
| 带默认值的 uniform（`uniform float x = 1.0;`） | defaultValue 提取 |
| 空代码 / 无 uniform / 无 out | 空数组返回 |
| 注释中的 uniform（不应被解析） | 正则边界 |
| existingInputs 保留已有 port id | port id 复用逻辑 |
| existingOutputs 保留已有 port id | port id 复用逻辑 |
| 无效类型回退到 `'float'` | mapType 默认分支 |
| 多个 out 声明 | 多输出端口 |

#### 4.2 engine/graphExecutor.ts

纯函数，Kahn 拓扑排序。

| 测试用例 | 覆盖目标 |
|---|---|
| 线性 DAG (A→B→C) | 基本排序 |
| 菱形 DAG (A→B, A→C, B→D, C→D) | 多入度节点 |
| 单节点无边 | 边界 |
| 多个独立子图 | 不连通图 |
| 有环图（应返回部分排序） | 环检测行为 |
| 空输入 | 空数组 |
| 边引用不存在的节点 | 容错 |

#### 4.3 engine/shaderCompiler.ts

依赖 THREE.js，需 mock `THREE.RawShaderMaterial`。

| 测试用例 | 覆盖目标 |
|---|---|
| `stripInjected` 移除 #version / precision / out 声明 | 内部函数 |
| `compileNodeShader` 基本编译 | material 生成 |
| 连接上游 sampler2D uniform 注入 | upstreamSamplers 映射 |
| 连接上游 scalar uniform 注入 | 非 sampler2D 路径 |
| 未连接的非 sampler 输入自动注入 | 独立 uniform 分支 |
| preambleLines 计算正确性 | 行号偏移 |
| `validateFragmentShader` 编译成功 | WebGL mock |
| `validateFragmentShader` 编译失败返回错误 | 错误路径 |
| `validateFragmentShader` 编译成功但有 warning | warning 路径 |
| `validateFragmentShader` createShader 返回 null | null 保护 |

#### 4.4 engine/shaderLinter.ts

依赖 WebGL2 + CodeMirror EditorView，需 mock。

| 测试用例 | 覆盖目标 |
|---|---|
| 空代码返回空诊断 | 短路路径 |
| WebGL 不可用返回空 | null GL 路径 |
| 编译成功返回空 | 正常路径 |
| 编译失败解析 ERROR 行号 | ERR_RE 正则 |
| `buildFullSource` 剥离 #version / precision | 源码清洗 |
| `buildFullSource` 正确计算 offset / strippedLines | 行号偏移 |
| 多个错误 | 多 diagnostic |
| 无法解析的错误日志回退到整体报错 | fallback 路径 |

#### 4.5 engine/shaderCompletions.ts

依赖 CodeMirror CompletionContext，需 mock。

| 测试用例 | 覆盖目标 |
|---|---|
| 输入 GLSL 关键字前缀触发补全 | 关键字补全 |
| 输入 GLSL 类型前缀触发补全 | 类型补全 |
| 输入 GLSL 函数前缀触发补全 | 函数补全 |
| 内置变量补全 | 内置变量 |
| 用户自定义变量提取 + 补全 | extractUserVariables |
| 无匹配返回 null | 空结果 |
| 光标在注释/字符串中不触发 | 上下文过滤 |

#### 4.6 engine/predefinedShaders.ts

纯数据模块。

| 测试用例 | 覆盖目标 |
|---|---|
| `predefinedShaders` 数组非空 | 数据存在性 |
| 每个模板有 label 和 code | 结构完整性 |
| `CUSTOM_SHADER_CODE` 包含 uniform 和 out 声明 | 代码有效性 |
| `CUSTOM_2IN1_SHADER` 包含两个 sampler2D uniform | 代码有效性 |
| 所有模板代码能被 `parseShader` 正确解析 | 与 parser 集成 |

#### 4.7 engine/executionEngine.ts

核心执行引擎，依赖 WebGL + THREE.js，需大量 mock。

| 测试用例 | 覆盖目标 |
|---|---|
| `isRunning()` 初始为 false | 状态初始化 |
| `run()` 执行后 isRunning 变为 true → 结束后 false | 运行生命周期 |
| `stop()` 中断执行 | 停止逻辑 |
| 处理 input 节点（scalar / image / framebuffer） | 输入类型分支 |
| 处理 shader 节点编译 + 渲染 | shader 管线 |
| 处理 output 节点渲染 + 回调 | output 分支 |
| 节点编译失败调用 onNodeError | 错误回调 |
| `formatShaderError` 行号修正 | 错误格式化 |
| 空图执行 | 边界 |
| autoSize 解析 | output 自适应尺寸 |
| NV12 / 各种 framebuffer 格式 | 格式分支覆盖 |
| texture filter / wrap 配置 | 采样参数 |

#### 4.8 engine/webglRenderer.ts

THREE.js 封装，需 mock WebGL。

| 测试用例 | 覆盖目标 |
|---|---|
| 构造函数初始化 renderer / scene / camera / quad | 初始化 |
| `createTarget` 创建 FBO | render target |
| `getTarget` / `getImageTexture` 查找 | getter |
| `loadImageTexture` 异步加载 | 图片加载 |
| `loadRawTexture` 各种格式（rgba8/rgba32f/rg8/rg32f/r8/r32f/nv12） | 所有格式分支 |
| `applyTextureSampling` filter/wrap 组合 | 采样参数 |
| `renderWithMaterial` / `renderSampler2DInput` / `renderToScreen` | 渲染路径 |
| `readTargetToDataURL` float/byte 读取 | readback |
| `convertNV12toRGBA` 色彩空间转换 | NV12 |
| `clear` / `dispose` 资源释放 | 生命周期 |
| `setSize` | resize |

#### 4.9 utils/graphUtils.ts

纯函数，无外部依赖。

| 测试用例 | 覆盖目标 |
|---|---|
| `getUpstreamEdges` 返回指向目标节点的边 | 过滤逻辑 |
| `getDownstreamEdges` 返回从源节点出发的边 | 过滤逻辑 |
| `findUpstreamNodes` 返回上游节点 id 列表 | 映射逻辑 |
| `getConnectedTypeMap` 构建 targetHandle → source 映射 | Map 构建 |
| 空边列表 | 边界 |
| 无匹配节点 | 空结果 |

#### 4.10 utils/projectIO.ts

序列化/反序列化，部分依赖 DOM（downloadProject）。

| 测试用例 | 覆盖目标 |
|---|---|
| `serializeProject` 生成正确 ProjectFile 结构 | 序列化 |
| `serializeProject` 默认项目名 "Untitled" | 默认值 |
| `serializeProject` node 无 type 时回退 'shader' | 空值处理 |
| `deserializeProject` 正确恢复 nodes + edges | 反序列化 |
| `deserializeProject` 无 version 字段抛错 | 校验逻辑 |
| `deserializeProject` 无效 JSON 抛错 | 错误路径 |
| `downloadProject` 创建下载链接 | DOM mock |
| `saveFileAs` / `saveFile` 委托 downloadProject | 调用转发 |
| 序列化→反序列化 round-trip 一致性 | 端到端 |

#### 4.11 utils/rawPreview.ts

依赖 Canvas 2D（需 mock），数据处理密集。

| 测试用例 | 覆盖目标 |
|---|---|
| rgba8 格式正确渲染 | 基本路径 |
| rgba32f 格式 float→byte 映射 | float 格式 |
| rg8 / rg32f / r8 / r32f 各通道映射 | 所有格式 |
| nv12 YUV→RGB 转换 | 色彩转换 |
| 带 stride 的行对齐 | stride 参数 |
| 无效 dataUrl 返回 null | 异常捕获 |
| `clamp` 函数边界值（负数/超 255） | 辅助函数 |

#### 4.12 store/useGraphStore.ts

Zustand store，需在测试中直接操作 store。

| 测试用例 | 覆盖目标 |
|---|---|
| 初始状态 nodes/edges 为空 | 初始化 |
| `addNode` 添加 shader/input/output 节点 | 节点创建 |
| `addInputNode` 各 DataType | 输入节点 |
| `addShaderNode` 从模板创建 | 模板节点 |
| `removeNode` 删除节点 + 关联边 | 删除逻辑 |
| `removeSelectedElements` 批量删除 | 选中删除 |
| `updateNodeData` 更新节点数据 | 数据更新 |
| `updateNodeInputType` 修改输入类型触发 shader 重解析 | 类型联动 |
| `onConnect` 创建边 | 连线 |
| `onNodesChange` / `onEdgesChange` 应用变更 | React Flow 集成 |
| `pushHistory` / `undo` / `redo` 历史栈 | 撤销/重做 |
| undo 超出栈深度 | 边界 |
| `loadGraph` 恢复图 | 载入 |
| `clearGraph` 清空 | 清空 |
| `setOutputPreview` / `clearOutputPreviews` | 预览管理 |
| `setNodeError` / `clearNodeErrors` | 错误管理 |
| `setProjectName` / `setSavedFilePath` | 元数据 |
| `setRunning` | 运行状态 |
| `syncCounters` 从已有节点恢复计数器 | 计数器同步 |
| `createInputShader` / `createDefaultShaderCode` | shader 生成 |

#### 4.13 types/index.ts

类型定义 + 常量。

| 测试用例 | 覆盖目标 |
|---|---|
| `DATA_TYPE_COLORS` 所有 DataType 都有颜色 | 完整性 |
| `GLSL_VALID_TYPES` 包含所有必要类型 | 完整性 |
| `GLSL_VALID_TYPES` 元素均为有效 DataType | 类型一致性 |

#### 4.14 components/Header.tsx

大型组件，包含菜单逻辑、项目管理、运行控制。

| 测试用例 | 覆盖目标 |
|---|---|
| 渲染标题 + 版本号 | 基本渲染 |
| 项目名显示 + 双击编辑 | 交互 |
| SHADER 下拉菜单显示模板列表 | 菜单逻辑 |
| INPUT 分组子菜单（SCALAR / VECTOR / SAMPLER2D） | 嵌套菜单 |
| 点击 +SHADER / +INPUT / +OUTPUT 添加节点 | store 交互 |
| SAVE / LOAD 按钮 | 文件操作 |
| RUN / STOP 按钮状态切换 | 运行控制 |
| CLEAR 按钮 | 清空操作 |
| UNDO / REDO 快捷键 | 键盘事件 |
| 窗口控制按钮（Windows 最小化/最大化/关闭） | 平台分支 |

#### 4.15 components/ImageLightbox.tsx

灯箱组件，含缩放/拖动/颜色取样。

| 测试用例 | 覆盖目标 |
|---|---|
| 打开/关闭灯箱 | 可见性 |
| 滚轮缩放 | zoom 逻辑 |
| 拖动平移 | pan 逻辑 |
| 双击重置 | 重置 |
| Save as PNG 按钮 | 保存功能 |
| Color Picker 模式切换 + 像素信息显示 | 取色器 |

#### 4.16 components/NodeGraph/index.tsx

React Flow 容器。

| 测试用例 | 覆盖目标 |
|---|---|
| 渲染 ReactFlow 画布 | 基本渲染 |
| 自定义节点类型注册 | nodeTypes |
| 连线校验（类型兼容性） | isValidConnection |
| MiniMap 渲染 | 子组件 |
| 右键/选择事件传递 | 事件处理 |

#### 4.17 components/NodeGraph/nodes/ShaderNode.tsx

| 测试用例 | 覆盖目标 |
|---|---|
| 渲染紫色 header + label | 基本渲染 |
| 输入/输出端口渲染 | Handle 渲染 |
| 端口颜色与 DataType 对应 | 颜色映射 |
| 折叠/展开 | collapsed 状态 |
| 选中高亮 | selected 样式 |

#### 4.18 components/NodeGraph/nodes/InputNode.tsx

最大的节点组件，包含图片加载、framebuffer 配置。

| 测试用例 | 覆盖目标 |
|---|---|
| 不同 DataType 渲染不同 UI | 类型分支 |
| scalar 输入值编辑 | 值输入 |
| vec 输入分量编辑 | 向量编辑 |
| image 模式：图片加载 + 缩略图 | 图片上传 |
| framebuffer 模式：格式/宽/高/stride 配置 | FB 配置 |
| texture filter / wrap 设置 | 采样配置 |
| 拖放图片 | drag & drop |

#### 4.19 components/NodeGraph/nodes/OutputNode.tsx

| 测试用例 | 覆盖目标 |
|---|---|
| 渲染红色 header | 基本渲染 |
| 输入端口渲染 | Handle |
| 预览图显示 | preview |
| Auto Size 复选框 | 自适应尺寸 |
| 手动宽高输入 | 尺寸覆盖 |

#### 4.20 components/NodeGraph/edges/CustomEdge.tsx

| 测试用例 | 覆盖目标 |
|---|---|
| 贝塞尔曲线渲染 | 基本渲染 |
| 路径计算 | 坐标 |

#### 4.21 components/SidePanel/index.tsx

| 测试用例 | 覆盖目标 |
|---|---|
| 无选中节点显示占位提示 | 空状态 |
| 选中节点显示节点信息 | 节点详情 |
| label 编辑 | 交互 |
| Delete 按钮 | 删除 |
| ShaderEditor / PortInspector 条件渲染 | 子组件切换 |
| Output 预览显示 | preview |

#### 4.22 components/SidePanel/ShaderEditor.tsx

| 测试用例 | 覆盖目标 |
|---|---|
| CodeMirror 编辑器渲染 | 基本渲染 |
| 代码变更回调 | onChange |
| GLSL 语法高亮加载 | 扩展 |

#### 4.23 components/SidePanel/PortInspector.tsx

| 测试用例 | 覆盖目标 |
|---|---|
| 端口列表渲染 | 基本渲染 |
| 颜色指示器与 DataType 对应 | 颜色映射 |
| float/int/bool uniform 编辑 | scalar 编辑 |
| vec2/vec3/vec4 分量编辑 | vector 编辑 |
| 已连接端口禁用编辑 | 连接状态 |

#### 4.24 src-tauri/src/lib.rs (Rust)

| 测试用例 | 覆盖目标 |
|---|---|
| `run()` 函数存在且可调用 | 入口点 |
| debug 模式启用 log 插件 | cfg(debug_assertions) 分支 |

#### 4.25 src-tauri/src/main.rs (Rust)

| 测试用例 | 覆盖目标 |
|---|---|
| `main()` 调用 `app_lib::run()` | 入口点 |

---

### 5. 配置方案

#### 5.1 Vitest 配置 (vitest.config.ts)

```typescript
import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  test: {
    globals: true,
    environment: 'jsdom',
    setupFiles: ['./tests/setup.ts'],
    include: ['tests/**/*.test.{ts,tsx}'],
    coverage: {
      provider: 'istanbul',
      reporter: ['text', 'text-summary', 'lcov', 'json-summary'],
      reportsDirectory: './coverage',
      include: ['src/**/*.{ts,tsx}'],
        'tests/**',
        'src/main.tsx',
        'src/vite-env.d.ts',
      ],
      thresholds: {
        lines: 100,
        branches: 100,
        functions: 100,
        statements: 100,
      },
    },
  },
});
```

#### 5.2 测试 setup 文件 (tests/setup.ts)

```typescript
import '@testing-library/jest-dom/vitest';

// Mock WebGL2 context
const mockGL = {
  createShader: vi.fn(() => ({})),
  shaderSource: vi.fn(),
  compileShader: vi.fn(),
  getShaderParameter: vi.fn(() => true),
  getShaderInfoLog: vi.fn(() => ''),
  deleteShader: vi.fn(),
  createProgram: vi.fn(() => ({})),
  // ...扩展所需 WebGL2 方法
};

HTMLCanvasElement.prototype.getContext = vi.fn((type: string) => {
  if (type === 'webgl2' || type === 'webgl') return mockGL;
  if (type === '2d') return {
    createImageData: vi.fn((w: number, h: number) => ({
      data: new Uint8ClampedArray(w * h * 4),
      width: w,
      height: h,
    })),
    putImageData: vi.fn(),
  };
  return null;
}) as unknown as typeof HTMLCanvasElement.prototype.getContext;
```

#### 5.3 npm scripts 新增

```json
{
  "scripts": {
    "test": "vitest run",
    "test:watch": "vitest",
    "test:coverage": "vitest run --coverage",
    "test:ui": "vitest --ui"
  }
}
```

#### 5.4 Rust 测试配置

在 `src-tauri/src/lib.rs` 和 `main.rs` 中添加 `#[cfg(test)]` 模块。

Cargo.toml 无需改动，`cargo test` 已原生支持。

覆盖率命令：

```bash
cd src-tauri
cargo llvm-cov --html       # 生成 HTML 报告
cargo llvm-cov --lcov       # 生成 LCOV 格式（用于 CI 上传）
```

---

### 6. 依赖安装

```bash
# 前端测试依赖
npm install -D vitest @vitest/coverage-istanbul @testing-library/react @testing-library/jest-dom @testing-library/user-event jsdom

# Rust 覆盖率工具
cargo install cargo-llvm-cov
```

---

### 7. GitHub Actions CI 配置 (.github/workflows/ci.yml)

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

permissions:
  contents: read

jobs:
  frontend-test:
    name: Frontend Tests
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Setup Node.js
        uses: actions/setup-node@v4
        with:
          node-version: 22
          cache: npm

      - name: Install dependencies
        run: npm ci

      - name: Lint
        run: npm run lint

      - name: Type check
        run: npx tsc -b --noEmit

      - name: Run tests with coverage
        run: npm run test:coverage

      - name: Upload coverage to Codecov
        if: github.event_name == 'push'
        uses: codecov/codecov-action@v5
        with:
          files: ./coverage/lcov.info
          flags: frontend
          fail_ci_if_error: true

  rust-test:
    name: Rust Tests
    runs-on: ubuntu-latest
    defaults:
      run:
        working-directory: src-tauri
    steps:
      - uses: actions/checkout@v4

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable

      - name: Install system dependencies
        run: |
          sudo apt-get update
          sudo apt-get install -y libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf libgtk-3-dev

      - name: Cache cargo
        uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
            src-tauri/target
          key: ${{ runner.os }}-cargo-${{ hashFiles('src-tauri/Cargo.lock') }}

      - name: Run tests
        run: cargo test --verbose

      - name: Install cargo-llvm-cov
        run: cargo install cargo-llvm-cov

      - name: Run tests with coverage
        run: cargo llvm-cov --lcov --output-path ../coverage/rust-lcov.info

      - name: Upload Rust coverage to Codecov
        if: github.event_name == 'push'
        uses: codecov/codecov-action@v5
        with:
          files: ./coverage/rust-lcov.info
          flags: rust
          fail_ci_if_error: true
```

---

### 8. 测试优先级与执行顺序

按模块复杂度和依赖关系分 4 个阶段：

| 阶段 | 模块 | 理由 |
|---|---|---|
| **P0 — 纯逻辑** | shaderParser, graphExecutor, graphUtils, types/index, predefinedShaders | 无依赖、纯函数，立即可测 |
| **P1 — 数据层** | projectIO, rawPreview, useGraphStore | 需少量 DOM/store mock |
| **P2 — 引擎层** | shaderCompiler, shaderLinter, shaderCompletions, executionEngine, webglRenderer | 需 WebGL/THREE.js/CodeMirror mock |
| **P3 — 组件层** | Header, ImageLightbox, NodeGraph, ShaderNode, InputNode, OutputNode, CustomEdge, SidePanel, ShaderEditor, PortInspector | 需 @testing-library/react + store + 全套 mock |
| **P4 — Rust** | lib.rs, main.rs | Tauri 集成测试需系统依赖 |

---

### 9. Mock 策略

| 依赖 | Mock 方式 |
|---|---|
| **WebGL2RenderingContext** | 全局 setup 中 mock `HTMLCanvasElement.prototype.getContext` |
| **THREE.js** | `vi.mock('three')` 返回 stub 类（WebGLRenderer, WebGLRenderTarget, Texture 等） |
| **@xyflow/react** | `vi.mock('@xyflow/react')` mock ReactFlow 组件 + `applyNodeChanges` / `applyEdgeChanges` / `addEdge` |
| **CodeMirror** | mock `EditorView` / `EditorState` / `CompletionContext` |
| **DOM API** | jsdom 环境已提供；`URL.createObjectURL` / `FileReader` / `document.createElement('a')` 按需 mock |
| **@tauri-apps/api** | `vi.mock('@tauri-apps/api')` 返回 noop |
| **zustand store** | 直接 `useGraphStore.getState()` / `useGraphStore.setState()` 操作；组件测试中正常渲染 |

---

### 10. 覆盖率执行与监控

```bash
# 本地运行覆盖率
npm run test:coverage

# 查看 HTML 报告
open coverage/index.html

# Rust 覆盖率
cd src-tauri && cargo llvm-cov --html --open
```

CI 中覆盖率阈值硬性要求 100%，未达标时 `vitest` 自动返回非零退出码，PR 将被阻断。

Codecov 集成后可在 PR comment 中查看覆盖率变化差异。
