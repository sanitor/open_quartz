# CodeEditor

[English](README.md) | 简体中文

供 Rust/Slint 应用复用的多行代码编辑器，提供 WGSL/JSON 高亮、选区、文档撤销/重做、剪贴板、行号和查找/替换导航。组件使用 Slint 公开元素组合 UI，由 cosmic-text 提供编辑与排版；不依赖 Slint fork、WebView 或 OpenQuartz 业务代码。

本包先在当前仓库开发，之后再迁移独立仓库。目前未发布到 crates.io（`publish = false`），API 仍可能调整。Slint 与 `slint-build` 固定为 **1.18.1**，cosmic-text 固定为 **0.18.2**，声明的最低 Rust 版本为 **1.92**。目前的运行验收平台是 macOS；详见[当前限制](#当前限制)。

本文参照 Slint [TextEdit 参考文档](https://docs.slint.dev/latest/docs/slint/reference/std-widgets/views/textedit/)的示例、属性、回调和函数结构编写。`CodeEditor` 是本项目组件，并非 `std-widgets.slint` 导出的标准控件；可用接口以本文列出的内容为准。

## 示例

```slint
import { CodeEditor, CodeEditorBridge } from "code_editor.slint";
export { CodeEditorBridge }

export component EditorWindow inherits Window {
    width: 800px;
    height: 600px;
    in-out property <string> source: "let value = 1.0;\n";
    callback source-edited(string);

    editor := CodeEditor {
        text <=> root.source;
        language: "wgsl";
        label: "Shader source";
        edited(value) => { root.source-edited(value); }
    }
}
```

此界面需要安装下文的 Rust 桥接。仅导入 `.slint` 文件，不会初始化编辑和渲染能力。

## 快速开始

在 OpenQuartz 仓库根目录运行：

```sh
npm run dev:editor
npm run test:editor
```

独立示例包含 WGSL 和 JSON 两个编辑器。顶部控件可以把 WGSL 编辑器切换为纯文本、切换明暗配色、开启只读模式，并演示宿主提供的 JSON 高亮适配器。当前工具条文字为英文，文档语言不会改变界面语言。

等价的 Cargo 命令，同样从仓库根目录执行：

```sh
cargo run --manifest-path crates/slint_code_editor/Cargo.toml --locked --example editor
cargo test --manifest-path crates/slint_code_editor/Cargo.toml --locked
```

可运行代码见 [Rust 入口](examples/editor.rs)和[示例 UI](ui/demo.slint)。

## 接入 Rust 应用

以下路径假定消费者应用位于本仓的 `apps/editor_example/`，组件位于 `crates/slint_code_editor/`。创建下述四个文件；其中 `apps/editor_example/ui/app.slint` 使用上方[示例](#示例)的内容。

### Cargo.toml

```toml
[package]
name = "editor_example"
version = "0.1.0"
edition = "2021"
publish = false

[workspace]

[dependencies]
slint = "=1.18.1"
slint_code_editor = { path = "../../crates/slint_code_editor" }

[build-dependencies]
slint-build = "=1.18.1"
```

空的 `[workspace]` 让此示例独立于本仓根 workspace。若应用已经属于某个 workspace，沿用其已有配置即可。目录位置不同时，需要调整 dependency 和 include paths；普通 Cargo workspace 同样可以通过 path dependency 消费此组件。

### build.rs

```rust
fn main() {
    let config = slint_build::CompilerConfiguration::new()
        .with_include_paths(vec!["../../crates/slint_code_editor/ui".into()]);
    slint_build::compile_with_config("ui/app.slint", config)
        .expect("Cannot compile editor UI");
}
```

include path 让消费者的 Slint 编译器能够找到 `code_editor.slint`。迁移组件时，需要同时保留 Rust 源码和 `ui/` 资源。消费者的编译器与运行时应共同使用固定版本。

### src/main.rs

```rust
slint::include_modules!();

fn main() -> Result<(), slint::PlatformError> {
    let window = EditorWindow::new()?;
    slint_code_editor::install_editor!(window, CodeEditorBridge);
    window.on_source_edited(|source| {
        println!("Edited {} UTF-8 bytes", source.len());
    });
    window.run()
}
```

从仓库根目录运行：

```sh
cargo run --manifest-path apps/editor_example/Cargo.toml
```

在窗口创建后、运行前，**每个窗口调用一次** `install_editor!`。根 `.slint` 编译单元需要导出 `CodeEditorBridge`，以生成对应 Rust 桥接类型。宏注册回调并创建窗口私有的 `Rc<RefCell<Runtime>>`，同时将它作为返回值提供给高级接入代码；普通用法无需保存返回值。

每个 `CodeEditor` 实例有独立的文档、选区和撤销历史。同一窗口内的实例共享桥接与事件过滤器。UI 实例销毁时，model lease 会释放对应文档。不要重复安装桥接，也不要自行修改桥接的 `ready`、`tick` 或回调处理器。

桌面 IME 适配依赖 `slint::winit_030` 与 winit 后端。`on_winit_window_event` 每个窗口只有一个过滤器槽位，宿主再次安装过滤器会覆盖组件适配，反过来也一样。需要宿主事件过滤器时，应协调事件分发，不能分别注册相互覆盖的过滤器。当前尚未建立 C++、纯解释器、其他窗口后端、移动端或 Web 的受支持接入契约。

## 属性

### text

`string` · **in-out** · 默认值：`""`

已提交的源码文本。使用 `<=>` 与应用状态双向绑定。IME 预编辑只参与显示，在提交之前不会进入该值。

程序赋入不同文本会加载新文档：清除选区、把光标移到开头，并清空文档撤销/重做历史。赋入相同文本不做修改。只读模式仍允许程序替换文本。通过绑定更新源码，本身不会触发 `edited`。

### language

`string` · **in** · 默认值：`"plain"`

选择内置词法高亮器：

| 值 | 行为 |
|---|---|
| `"plain"` | 使用默认文字颜色，不生成内置语法区间 |
| `"wgsl"` | WGSL 注释、属性、关键字、类型、启发式函数分类、数字和标点 |
| `"json"` | JSON 字符串、字面量、数字和标点 |

值区分大小写，未知值按纯文本处理。高亮容忍未完成输入，不校验或编译源码。安装自定义 provider 后，它优先于内置语言，包括 `"plain"`。切换此属性不会修改文本或增加撤销记录。

### read-only

`bool` · **in** · 默认值：`false`

禁止用户修改文本，包括粘贴、剪切、替换和撤销/重做。仍可选择、复制、滚动以及查找导航。外部仍可以给 `text` 赋值。

### enabled

`bool` · **in** · 默认值：`true`

控制编辑器交互。设为 false 时，输入面和编辑操作禁用，源码仍然可见。程序仍可替换文本；此属性不会使应用状态变为不可修改。

### label

`string` · **in** · 默认值：`"Code source"`

编辑面的无障碍标签。应为各实例设置有意义的名称，例如 `"Shader source"` 或 `"Configuration JSON"`。它不会添加可见标题。无障碍能力边界见下文。

### dark

`bool` · **in** · 默认值：`false`

为代码面、行号区、选区和语法颜色选择内置深色配色；false 使用浅色配色。标准工具条控件沿用应用的 Slint widget style，不通过此属性切换配色。

切换配色不改变文本、选区或历史。Rust 的 `theme::Theme` 类型用于底层 `Document` API。对已接入组件的文档直接调用 `Document::set_theme`，不是持久的自定义主题接口：下一次组件绘制会重新根据 `dark` 选择配色。

### find-query

`string` · **in-out** · 默认值：`""`

区分大小写的字面量查找字符串，与工具条 Find 输入框共享。空字符串没有匹配项，匹配项不重叠，不支持正则表达式。点击 **Next** 或在 Find 框中按 Enter，选中下一项；到达末尾后回到第一项。

### show-toolbar

`bool` · **in** · 默认值：`true`

控制 Go、Find/Next、匹配数量和 Replace/All 工具条是否显示。隐藏工具条后，代码面、行号、键盘编辑和右键菜单仍可用；此属性不会另外提供一套外部工具条 API。

### document-handle

`int` · **out** · 初始值：`-1`

此实例在窗口 Rust Runtime 中的文档标识。桥接初始化且组件创建后变为非负值，可通过 `install_editor!` 返回的 `Runtime` 找到文档并安装自定义 provider。标识只在组件存活期间、且只在所属窗口的 Runtime 中有效；不要保存到项目文件，也不要当作跨窗口标识使用。

## 回调

### edited(value: string)

用户操作改变已提交文本后触发，包括文字输入、粘贴、剪切、撤销/重做、替换和 IME 提交。`value` 是完整的更新后源码，不是增量范围。回调不表示保存或编译请求；何时保存、校验或应用，由宿主决定。

仅改变选区、导航、高亮、主题或 IME 预编辑不会触发此回调。直接给 `text` 赋值不视为用户编辑。

```slint
CodeEditor {
    text <=> root.source;
    edited(value) => { root.source-edited(value); }
}
```

## 函数

### focus-editor()

启用时，把键盘焦点移到代码面。工具条 Find 输入框有独立的焦点交互。

### select-all()

选择全部已提交源码，不改变文本，不增加撤销记录。

### set-selection-offsets(anchor: int, head: int)

使用 **UTF-8 字节偏移**设置选区，不是 Unicode 字符数量、可视列或 UTF-16 位置。`anchor` 是固定端，`head` 是光标移动端；`head` 可以小于 `anchor`，表示反向选区。两端相同时只移动光标、不选择文本。此函数不会自动聚焦编辑器。

应传入 `0..=text.len()` 内、位于 UTF-8 字符边界的非负偏移。无效字符边界和超出范围的正偏移会保持原选区不变。位于 CRLF 行尾内部的偏移会定位到前一行末尾。面向用户的选区应优先落在完整 grapheme cluster（字素簇）之间。底层 Rust `Document::select_offsets` 返回布尔值，表示偏移是否被接受。

例如源码 `"a色🙂"` 的字节范围为 `a = 0..1`、`色 = 1..4`、`🙂 = 4..8`；调用 `set-selection-offsets(1, 4)` 选中 `色`。

### refresh()

宿主改变 provider 后，使渲染帧失效并重新绘制。它不会把任意 Rust 文档修改同步回组件的 `text`，不会触发 `edited`，也不会改变文本历史。宿主替换源码应使用绑定的 `text` 属性；派生显示变化使用 `refresh()`。

## 编辑与导航

- 单击放置光标，拖动选择，双击选择单词。行号沿用文字的实际纵向排版，横向滚动时保持固定。
- 使用方向键、Home/End、Page Up/Down 导航，配合 Shift 选择。Tab 缩进，Shift+Tab 减少缩进。代码面不自动换行；使用滚轮或触控板横纵滚动，目前没有可拖动滚动条。
- macOS 使用 Command，其他桌面目标使用 Ctrl，支持 A/C/X/V/Z 和 Shift+Z；Y 也可请求重做。宿主图/项目历史与文档历史分开。未处理的快捷键可以继续传递给宿主。
- **Go** 使用从 1 开始的行号。**Replace** 仅在当前选区等于查找字符串时替换，因此应先用 **Next** 选中匹配。**All** 在一次可撤销事务中替换所有不重叠的字面量匹配。替换内容可以为空。
- 右键菜单提供 Undo/Redo、Cut/Copy/Paste 和 Select all，平台可能采用系统原生菜单。剪贴板失败时，在代码面下方显示错误。

加载源码时保留原行尾。Enter 插入 LF，因此编辑 CRLF 文档可能产生混合行尾。复制到系统剪贴板的是纯文本。组件不自动保存、格式化、编译，也不提供 LSP、补全、折叠或图修改。

## 自定义高亮

宿主可以安装同步 provider，类型为 `Fn(&str) -> Vec<Highlight>`。它接收已提交源码，为半开 UTF-8 字节区间返回语义分类；未覆盖的文字使用默认色。可用分类为 `Keyword`、`Type`、`Attribute`、`Function`、`Number`、`String`、`Comment`、`Boolean` 和 `Punctuation`。

独立示例提供完整接入：

1. UI 回调传入 `right.document-handle` 和启用状态。
2. Rust 处理器调用 `runtime.borrow_mut().document_mut(id)`，再调用 `document.set_provider(Some(Box::new(...)))`。
3. UI 在回调完成后调用 `right.refresh()`。向 `set_provider` 传入 `None`，恢复内置语言高亮器。

代码见 [examples/editor.rs](examples/editor.rs)和 [ui/demo.slint](ui/demo.slint)。示例 provider 重新着色字面量 `true` 的出现位置，用于演示扩展接口，不是另一份 JSON 解析器。

返回区间必须按起点排序、不重叠、非空、终点不超过源码长度，且两端位于 UTF-8 字符边界。无效输出会触发契约断言，不会静默裁剪，也不会转成编译诊断。回调同步运行在 UI 线程上，不应执行阻塞 I/O 或耗时解析。在调用 Slint 回调或运行事件循环期间，不要持有 `Runtime` 的可变借用。

## 当前限制

| 范围 | 当前状态 |
|---|---|
| 平台验收 | 已实跑 macOS 示例和 OpenQuartz 集成；Windows/Linux 运行验收待完成 |
| IME | 已实现公开 winit 预编辑/提交适配；真实系统候选选择、取消和替换尚未完成验收 |
| 无障碍 | 已暴露标签、角色、值、只读状态和输入动作；Slint 1.18.1 没有公开的自定义 text-run/光标/选区报告接口，不支持完整读屏器文本导航 |
| 大文档 | 同步全量高亮和 CPU raster，没有端到端输入延迟或文件大小上限保证 |
| 文字呈现 | 固定等宽样式，字号 14 logical pixels、行距 21 pixels；没有公开字体、换行、tab width、滚动条或任意装饰属性 |
| 历史 | 各实例独立的本地编辑；没有公开事务分组策略、共享文档/多视图编辑或持久化历史 |
| 绑定 | 当前仅有 Rust/Slint 接入；导出 `.slint` 文件不代表已支持 C++/解释器 |

不要推断此组件支持 `TextEdit` 的 `font-family`、`wrap`、`has-focus` 或 `content-x` 等属性。可以通过继承的几何/布局属性设置组件尺寸和位置，但任意继承的视觉属性不一定能改变内部编辑面的样式。

## 开发与验证

从仓库根目录运行：

```sh
cargo fmt --manifest-path crates/slint_code_editor/Cargo.toml -- --check
cargo test --manifest-path crates/slint_code_editor/Cargo.toml --locked
cargo build --manifest-path crates/slint_code_editor/Cargo.toml --locked --example editor
```

可选的 Slint MCP 检查：

```sh
SLINT_MCP_PORT=19472 cargo run --manifest-path crates/slint_code_editor/Cargo.toml --locked --features ui-automation --example editor
```

`ui-automation` feature 启用 Slint 的 MCP 服务。上面的环境变量写法适用于 POSIX shell；PowerShell 中先设置 `$env:SLINT_MCP_PORT = "19472"`，再执行 Cargo 命令。MCP 文本/按键事件不能证明系统输入法或读屏器行为。验证时可使用 `SLINT_NO_MUDA=1` 检查 Slint 绘制的右键菜单；正常使用不需要设置。

OpenQuartz 专属验收报告和架构研究位于包外的[实现记录](../../docs/WGSL_SYNTAX_HIGHLIGHT_RESEARCH.md#11-本仓组件实现与验证)。本文是组件自己的使用参考，拆包时应随组件一起迁移。

## 许可

组件源码按 [Cargo.toml](Cargo.toml) 声明使用 MIT 许可。依赖保留各自许可：Slint 的许可与署名要求需要单独遵守，cosmic-text 使用 MIT OR Apache-2.0。分发应用时，应核对所选 Slint 许可与依赖 notices。目前没有公开发布包或创建独立仓库。
