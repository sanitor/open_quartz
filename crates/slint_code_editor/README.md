# CodeEditor

English | [简体中文](README.zh-CN.md)

A reusable multiline code editor for Rust applications built with Slint. It provides WGSL/JSON highlighting, selection, document undo/redo, clipboard operations, line numbers, and find/replace navigation. The component combines public Slint elements with a cosmic-text editing surface; it does not require a Slint fork, a WebView, or OpenQuartz business code.

This package is being developed in this repository before extraction into a separate repository. It is not published to crates.io (`publish = false`); its API may change. It pins Slint and `slint-build` to **1.18.1**, uses cosmic-text **0.18.2**, and declares Rust **1.92** as its minimum version. Runtime acceptance so far is on macOS; see [Current limitations](#current-limitations).

This guide follows the example, properties, callbacks, and functions structure of Slint's [TextEdit reference](https://docs.slint.dev/latest/docs/slint/reference/std-widgets/views/textedit/). `CodeEditor` is a project component, not a widget exported by `std-widgets.slint`; only the APIs listed here are available.

## Example

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

The UI requires the Rust bridge installation below. Importing the `.slint` file alone does not initialize editing or rendering.

## Quick start

From the OpenQuartz repository root:

```sh
npm run dev:editor
npm run test:editor
```

The independent demo contains two editors for WGSL and JSON. Its controls switch the WGSL editor to plain text, select a light/dark palette, enable read-only mode, and demonstrate a host-supplied JSON highlight provider. The toolbar labels are currently English in both documentation languages.

Equivalent Cargo commands, also from the repository root:

```sh
cargo run --manifest-path crates/slint_code_editor/Cargo.toml --locked --example editor
cargo test --manifest-path crates/slint_code_editor/Cargo.toml --locked
```

See the runnable [Rust entry point](examples/editor.rs) and [demo UI](ui/demo.slint).

## Integrate into a Rust application

The following paths assume a consuming application at `apps/editor_example/` alongside `crates/slint_code_editor/` in this repository. Create the four files below; put the [Example](#example) UI in `apps/editor_example/ui/app.slint`.

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

The empty `[workspace]` makes this example independent of the repository's root workspace. If your application already belongs to a workspace, use its existing workspace configuration instead. Adjust the dependency and include paths when using a different directory layout; a normal Cargo workspace can consume the component through a path dependency too.

### build.rs

```rust
fn main() {
    let config = slint_build::CompilerConfiguration::new()
        .with_include_paths(vec!["../../crates/slint_code_editor/ui".into()]);
    slint_build::compile_with_config("ui/app.slint", config)
        .expect("Cannot compile editor UI");
}
```

The include path makes `code_editor.slint` available to the consumer's Slint compiler. Keep the component's `ui/` resources with its Rust source when moving it. The consumer must use the pinned compiler/runtime version together.

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

Run from the repository root:

```sh
cargo run --manifest-path apps/editor_example/Cargo.toml
```

Call `install_editor!` **once per window**, after creating the window and before running it. Export `CodeEditorBridge` from the root `.slint` compilation unit so that the generated Rust bridge type exists. The macro registers callbacks and creates a window-local `Rc<RefCell<Runtime>>`, which it also returns for advanced integrations. You do not need to keep the return value for ordinary use.

Each `CodeEditor` instance has its own document, selection, and undo history. Instances in the same window share the bridge and event filter. A model lease releases the document when its UI instance is destroyed. Do not install the bridge twice or change its `ready`, `tick`, or callback handlers yourself.

The desktop IME adapter uses `slint::winit_030` and the winit backend. `on_winit_window_event` has a single filter slot per window: another host filter can overwrite this adapter, or vice versa. Applications needing their own filter must coordinate event dispatch rather than register competing filters. C++, interpreter-only consumers, other window backends, mobile, and Web are not supported integration contracts at this stage.

## Properties

### text

`string` · **in-out** · default: `""`

The committed source text. Bind it with `<=>` to application state. IME preedit is a visual composition and is not part of this value until committed.

Assigning different text programmatically loads a new document: it clears the selection, resets the cursor to the start, and clears document undo/redo history. Assigning the same text is a no-op. Read-only mode still permits programmatic replacement. Updating source through a binding does not emit `edited` by itself.

### language

`string` · **in** · default: `"plain"`

Selects the built-in lexical highlighter:

| Value | Behavior |
|---|---|
| `"plain"` | Default text color, no built-in syntax spans |
| `"wgsl"` | WGSL comments, attributes, keywords, types, function heuristics, numbers and punctuation |
| `"json"` | JSON strings, literals, numbers and punctuation |

Values are case-sensitive; unknown values use plain text. Highlighting tolerates incomplete input and does not validate or compile source. A custom provider, if installed, takes precedence over the built-in language, including `"plain"`. Switching this property does not edit text or add an undo entry.

### read-only

`bool` · **in** · default: `false`

Prevents user text mutations, including paste, cut, replace, and undo/redo. Selection, copying, scrolling, and find navigation remain available. External assignments to `text` are still allowed.

### enabled

`bool` · **in** · default: `true`

Enables interaction with the editor. When false, the input surface and edit actions are disabled. The source remains visible. Programmatic text replacement is still possible; this property does not make application state immutable.

### label

`string` · **in** · default: `"Code source"`

The accessibility label of the editing surface. Give each instance a descriptive name, such as `"Shader source"` or `"Configuration JSON"`. It does not add a visible heading. See the accessibility limits below.

### dark

`bool` · **in** · default: `false`

Selects the built-in dark palette for the code surface, gutter, selection, and syntax colors. False selects the light palette. Standard toolbar controls follow the application's Slint widget style; they do not switch palettes through this property.

Changing the palette does not modify text, selection, or history. The `theme::Theme` Rust type is used by the lower-level `Document` API. Calling `Document::set_theme` on an attached document is not a persistent custom-theme hook: the next component frame selects the palette from `dark` again.

### find-query

`string` · **in-out** · default: `""`

The literal, case-sensitive search query, shared with the toolbar's Find field. Empty queries have no matches. Matches do not overlap; regular expressions are not supported. Use **Next** or Enter in the Find field to select the next match, wrapping to the first match at the end.

### show-toolbar

`bool` · **in** · default: `true`

Shows the Go, Find/Next, match count, and Replace/All controls. Hiding the toolbar keeps the code surface, gutter, keyboard editing, and context menu available; it does not provide replacement external toolbar APIs.

### document-handle

`int` · **out** · initially: `-1`

The window-local identifier of this instance's Rust document. It becomes nonnegative after bridge initialization and component creation. Resolve it through the `Runtime` returned by `install_editor!` for custom providers. A handle is valid only while its component exists and only in its own window runtime; do not save it in a project file or use it as a cross-window identifier.

## Callbacks

### edited(value: string)

Emitted after a user action changes the committed text, including text input, paste, cut, undo/redo, replace, and IME commit. `value` contains the complete updated source, not a change range. It is not a save or compile request; the host decides when to save, validate, or apply it.

Selection, navigation, highlighting, theme changes, and IME preedit alone do not emit this callback. Direct assignments to `text` do not represent user edits.

```slint
CodeEditor {
    text <=> root.source;
    edited(value) => { root.source-edited(value); }
}
```

## Functions

### focus-editor()

Transfers keyboard focus to the code surface when enabled. Focusing the toolbar's Find field is a separate interaction.

### select-all()

Selects the entire committed source. Does not change text or add a history entry.

### set-selection-offsets(anchor: int, head: int)

Selects a range using **UTF-8 byte offsets**, not Unicode character counts, visual columns, or UTF-16 positions. `anchor` stays fixed; `head` is the moving cursor end and may precede `anchor`. Equal offsets place the cursor without selecting text. This function does not implicitly focus the editor.

Use nonnegative offsets within `0..=text.len()` that fall on UTF-8 character boundaries. Invalid character boundaries and out-of-range positive offsets leave the selection unchanged. An offset inside a CRLF line ending is placed at the preceding line's end. Prefer boundaries between complete grapheme clusters for user-facing selections. The lower-level Rust `Document::select_offsets` returns a boolean indicating whether the offsets were accepted.

For source `"a色🙂"`, byte ranges are `a = 0..1`, `色 = 1..4`, and `🙂 = 4..8`; call `set-selection-offsets(1, 4)` to select `色`.

### refresh()

Invalidates the rendered frame after a host-side provider change. Does not synchronize arbitrary Rust document mutations back into the component's `text` property, emit `edited`, or change text history. Use the bound `text` property to replace source from the host; use `refresh()` for derived rendering changes.

## Editing and navigation

- Click to place the cursor; drag to select; double-click to select a word. The line-number gutter follows the same vertical layout as the text and stays fixed during horizontal scrolling.
- Use arrows, Home/End, Page Up/Down, and Shift for keyboard selection. Tab indents and Shift+Tab unindents. The surface does not wrap lines; use wheel/trackpad gestures to scroll vertically or horizontally. It currently has no draggable scrollbars.
- Use Command on macOS and Ctrl on other desktop targets for A/C/X/V/Z and Shift+Z; Y also requests redo. Host graph/project history is separate from document history. Unknown shortcuts can propagate to the host.
- **Go** uses one-based line numbers. **Replace** replaces the current selection only when it equals the query; select a match with **Next** first. **All** replaces every non-overlapping literal match in one undoable transaction. The replacement text may be empty.
- Right-click opens Undo/Redo, Cut/Copy/Paste, and Select all. The platform may use a native menu. Clipboard failures appear as an error below the surface.

Original line endings are retained when loading source. Enter inserts LF, so editing a CRLF document can produce mixed endings. Plain text is copied to the system clipboard. There is no automatic save, formatting, compilation, LSP, completion, folding, or graph mutation.

## Custom highlighting

The host can install a synchronous provider of type `Fn(&str) -> Vec<Highlight>`. It receives the committed source and returns semantic categories for half-open UTF-8 byte ranges. Uncovered text uses the default color. Available categories are `Keyword`, `Type`, `Attribute`, `Function`, `Number`, `String`, `Comment`, `Boolean`, and `Punctuation`.

The independent demo shows a complete integration:

1. Its UI exposes a callback carrying `right.document-handle` and an enable flag.
2. Its Rust handler calls `runtime.borrow_mut().document_mut(id)` and `document.set_provider(Some(Box::new(...)))`.
3. The UI calls `right.refresh()` after the callback. Passing `None` to `set_provider` restores the built-in language highlighter.

See [examples/editor.rs](examples/editor.rs) and [ui/demo.slint](ui/demo.slint). The demo provider recolors literal `true` occurrences to illustrate the hook; it is not a replacement JSON parser.

Return sorted, disjoint, nonempty ranges whose ends are within the source and whose boundaries are valid UTF-8 boundaries. Invalid provider output triggers a contract assertion; it is not silently clamped or converted to a compiler diagnostic. Do not run blocking I/O or expensive parsing in this callback: it runs synchronously on the UI thread. Do not retain a mutable `Runtime` borrow while invoking Slint callbacks or running the event loop.

## Current limitations

| Area | Current status |
|---|---|
| Platform acceptance | macOS demo and OpenQuartz integration exercised; Windows/Linux runtime acceptance remains pending |
| IME | Public winit preedit/commit adapter implemented; real system candidate selection, cancellation and replacement have not completed acceptance |
| Accessibility | Label, role, value, read-only state and incoming actions are exposed; Slint 1.18.1 has no public custom text-run/caret/selection reporting hook, so full screen-reader text navigation is not supported |
| Large documents | Synchronous full-source highlighting and CPU rasterization; no end-to-end input-latency or maximum-file-size guarantee |
| Text presentation | Fixed monospace style at 14 logical pixels with 21-pixel line spacing; no public font, wrap, tab-width, scrollbar or arbitrary decoration properties |
| History | Per-instance local edits; no public transaction grouping policy, shared-document/multiview editing, or persistent history |
| Bindings | Rust/Slint integration only; exporting the `.slint` file does not establish C++/interpreter support |

Do not infer that `TextEdit` properties such as `font-family`, `wrap`, `has-focus`, or `content-x` are supported by this component. Inherited geometry/layout properties can size and position it, but arbitrary inherited visual properties do not necessarily style its internal editing surface.

## Development and verification

From the repository root:

```sh
cargo fmt --manifest-path crates/slint_code_editor/Cargo.toml -- --check
cargo test --manifest-path crates/slint_code_editor/Cargo.toml --locked
cargo build --manifest-path crates/slint_code_editor/Cargo.toml --locked --example editor
```

For optional Slint MCP inspection:

```sh
SLINT_MCP_PORT=19472 cargo run --manifest-path crates/slint_code_editor/Cargo.toml --locked --features ui-automation --example editor
```

The `ui-automation` feature enables Slint's MCP server. The environment-variable syntax above is for POSIX shells; in PowerShell, set `$env:SLINT_MCP_PORT = "19472"` before the Cargo command. MCP text/key events do not prove system IME or screen-reader behavior. `SLINT_NO_MUDA=1` can exercise Slint-rendered context menus during verification; it is not required for normal use.

OpenQuartz-specific acceptance reports and architectural research live outside the package in the [implementation record](../../docs/WGSL_SYNTAX_HIGHLIGHT_RESEARCH.md#11-本仓组件实现与验证). This guide is the package's own usage reference and should move with it when the package is extracted.

## License

The component source is MIT-licensed, as declared in [Cargo.toml](Cargo.toml). Dependencies retain their own licenses: Slint's licensing and attribution requirements apply separately, and cosmic-text is MIT OR Apache-2.0. Check the selected Slint license and dependency notices when distributing an application. No public package release or independent repository has been created yet.
