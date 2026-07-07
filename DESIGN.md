# OpenQuartz — 可视化着色器节点编辑器

> Version 0.0.1b — macOS 风格极简 Web 版 Quartz Composer

---

## 1. 项目概述

OpenQuartz 是一个 Web 版可视化着色器（GLSL）节点编辑器，受 Apple Quartz Composer 启发。

- 用户通过**节点图（DAG）**组织 GLSL shader 片段
- 每个节点是一个可编程的 shader 处理单元
- **Shader 即接口声明**：解析 GLSL `uniform` 自动生成输入端口，`out` 生成输出端口
- 输入输出可互相连接，类型校验
- 支持**工程文件保存/载入**：整个图结构 + 节点状态 + 输入值 + 图片数据序列化为 `.quartz.json` 文件
- 最终实时渲染输出到 WebGL 预览

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
type DataType =
  | 'float' | 'int' | 'bool'
  | 'vec2' | 'vec3' | 'vec4'
  | 'ivec2' | 'ivec3' | 'ivec4'
  | 'mat2' | 'mat3' | 'mat4'
  | 'sampler2D' | 'samplerCube';

interface Port {
  id: string;
  label: string;
  dataType: DataType;
  direction: 'input' | 'output';
  defaultValue?: any;
}

type NodeType = 'shader' | 'input' | 'constant';

interface ShaderNodeData {
  type: NodeType;
  label: string;
  shaderCode: string;
  inputs: Port[];
  outputs: Port[];
  uniforms: Record<string, any>;
  inputDataType?: DataType;       // input 节点专用
  imageDataUrl?: string;          // sampler2D 输入图片
  imageFileName?: string;
}

interface ProjectFile {
  version: string;
  name: string;
  createdAt: string;
  updatedAt: string;
  graph: { nodes: SerializedNode[]; edges: SerializedEdge[] };
  images: Record<string, string>; // nodeId → base64 data URL
}
```

---

## 5. 组件架构

```
<App>
  <Header />
    ├── OPENQUARTZ v0.0.1b
    ├── 工程名输入框
    ├── 添加节点：+SHADER / +INPUT / +IMAGE
    ├── 文件：SAVE / LOAD
    └── 运行：▶ RUN / ■ STOP / CLEAR
  <main className="flex">
    <NodeGraph />                  ← React Flow 画布 (bg #e0e0e0 + cross grid)
      ├── <ShaderNode />           ← 紫 header，input/output 端口，叶子节点显示输出预览
      ├── <InputNode />            ← 蓝 header，类型选择 + 值输入/图片加载
      └── 贝塞尔曲线连线
    <SidePanel />                  ← 白底右侧面板
      ├── 节点信息（类型 + label + Delete）
      ├── <ShaderEditor />         ← CodeMirror 浅色主题
      └── <PortInspector />        ← 端口列表 + uniform 值编辑
    <OutputPanel />                ← 底部预览
  </main>
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

## 7. 渲染管线

1. **拓扑排序** — Kahn 算法，按依赖顺序排列节点
2. **编译 Shader**：用户代码 + 注入 `v_uv`/`fragColor`/uniforms，用 `RawShaderMaterial` + `GLSL3`
3. **输入节点**：scalar 类型直接传值，sampler2D 加载图片到纹理
4. **逐节点渲染到 FBO**：Three.js `WebGLRenderTarget`，上游纹理自动绑定到 uniform
5. **OutputNode** 渲染到 screen / 预览 canvas

---

## 8. 工程文件保存/载入

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

## 9. 当前目录结构

```
open-quartz/
├── DESIGN.md
├── index.html
├── package.json
├── vite.config.ts
├── tsconfig.json / tsconfig.app.json / tsconfig.node.json
├── src/
│   ├── main.tsx
│   ├── App.tsx
│   ├── index.css                     ← Tailwind 入口 + React Flow 样式重置
│   ├── version.ts                    ← 版本号
│   ├── components/
│   │   ├── Header.tsx                ← macOS 菜单栏风格
│   │   ├── NodeGraph/
│   │   │   ├── index.tsx             ← ReactFlow 容器
│   │   │   └── nodes/
│   │   │       ├── ShaderNode.tsx
│   │   │       ├── InputNode.tsx
│   │   ├── SidePanel/
│   │   │   ├── index.tsx
│   │   │   ├── ShaderEditor.tsx      ← CodeMirror 6
│   │   │   └── PortInspector.tsx
│   │   └── OutputPanel.tsx
│   ├── engine/
│   │   ├── shaderParser.ts           ← 正则解析 GLSL
│   │   ├── shaderCompiler.ts         ← RawShaderMaterial 编译
│   │   ├── graphExecutor.ts          ← 拓扑排序
│   │   ├── webglRenderer.ts          ← Three.js FBO 管线
│   │   └── executionEngine.ts        ← 执行引擎
│   ├── store/
│   │   └── useGraphStore.ts          ← Zustand 全局状态
│   ├── utils/
│   │   ├── graphUtils.ts
│   │   └── projectIO.ts              ← 序列化/反序列化
│   └── types/
│       └── index.ts                  ← 类型定义 + DATA_TYPE_COLORS
```

---

## 10. 实现状态

| 模块 | 状态 |
|---|---|
| Vite + React + TS + Tailwind 脚手架 | ✅ |
| React Flow 节点图 + 自定义节点 | ✅ |
| 两种节点：Shader / Input（无独立 Output 节点） | ✅ |
| GLSL 正则解析（uniform / out） | ✅ |
| Shader 编译（RawShaderMaterial + GLSL3） | ✅ |
| WebGL FBO 渲染管线 | ✅ |
| 拓扑排序执行引擎 | ✅ |
| CodeMirror shader 编辑器 | ✅ |
| PortInspector uniform 值编辑 | ✅ |
| InputNode 图片加载 + 缩略图 | ✅ |
| 工程文件保存/载入 .quartz.json | ✅ |
| macOS 极简 UI 风格 | ✅ |
| 版本管理（version.ts） | ✅ |

---

## 11. 关键设计决策

| 决策 | 选择 | 理由 |
|---|---|---|
| 节点渲染 | 自定义组件 + Tailwind | 完全控制外观，不依赖 React Flow 默认主题 |
| Shader 编译 | RawShaderMaterial + GLSL3 | 避免 Three.js 自动注入与 #version 冲突 |
| WebGL 上下文管理 | 单个 canvas + CSS overlay | 避免 2D/WebGL 上下文冲突 |
| Handle 定位 | position:relative 父容器 | 确保多端口各占一行，不重叠 |
| 边类型 | bezier | 视觉效果流畅 |
| UI 框架 | 纯 Tailwind，无组件库 | 轻量，macOS 风格自由定制 |
| FBO 管线 | 零冗余 FBO，叶子 shader 即输出点 | 业务性能最优（见下文） |
| 节点架构 | 无独立 Output 节点，shader 直接输出 | 消除 passthrough FBO 拷贝 |
| PixelRatio | 离屏管线固定 pixelRatio=1 | FBO 渲染不需要 DPI 缩放 |

---

## 12. 渲染管线设计原则

**核心原则：零冗余 FBO，业务性能最优。**

- 无独立 Output 节点。DAG 中的叶子 shader 节点（无下游 shader/constant 消费其输出）即为输出点，负责 FBO readback 和预览生成。
- 管线中不创建任何不必要的中间 FBO。每个 FBO 的存在必须有明确的业务语义（输入纹理缓存、或多级 shader 链的中间结果）。
- 所有 FBO 的分辨率由叶子 shader 的输出配置决定（width/height/autoSize），从叶子反向传播到上游节点。shader 在目标分辨率下逐像素执行，不做事后缩放。
- 离屏渲染管线使用 `pixelRatio=1`，不受屏幕 DPI 影响。FBO 尺寸即像素尺寸，所见即所得。
- 工程文件版本号（当前 `0.2.0`）随数据模型变更递增，LOAD 时严格校验版本，不兼容则报错拒绝加载。
