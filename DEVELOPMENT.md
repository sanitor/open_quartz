# OpenQuartz 开发文档

## 单元测试计划

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
