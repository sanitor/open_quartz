# Phase 7 Change-set 5 Catalog Report

## Implemented

- Added Rust catalog descriptors and behavior in `crates/open_quartz/src/catalog.rs`.
- Added Rust catalog contract tests in `crates/open_quartz/tests/catalog_contract_test.rs`.
- Moved math execution from the private executor table to `crate::catalog::evaluate_math`.
- Exposed catalog JSON over FFI as `catalog()` and added it to the runtime public surface manifest.
- Added TypeScript SDK catalog proxy types/helpers in `src/sdk/catalog.ts` and exported `Catalog` from `src/sdk/index.ts`.
- Added Java `Catalog` proxy placeholder and expanded public proxy parity to 9 objects.
- Converted `src/catalog/mathOps.ts`, `src/catalog/onnxCatalog.ts`, and `src/catalog/predefinedShaders.ts` into metadata projections.
- Moved executable shader templates from `src/catalog/shaders/*` to `src/shaders/*`.
- Migrated direct ONNX/full-descriptor consumers to SDK descriptor helpers: graph store ONNX creation/download, model manager, ONNX resource registry, native host, browser inference provider, OpenQuartz client, OnnxPanel, and tests.
- Kept BrowserInferenceProvider task execution and store mutation flow in place.

## Retained TypeScript Catalog Metadata

- Math catalog: `id`, `label`, `category`, `inputCount`, category group ordering, and generated UI ports from arity.
- ONNX catalog: `id`, `label`, `category`, optional `icon`, and display-only `taskLabel`.
- Shader catalog: `category`, template `id`, `label`, and declared input/output port metadata.

## Deleted From TypeScript Catalog Layer

- ONNX model execution/download policy: `downloadUrl`, `fileSize`, `sha256`, `expectedIO`, and `defaultParams`.
- Math formula strings and math evaluator behavior.
- Shader WGSL materialization, `CUSTOM_SHADER_CODE`, `CUSTOM_2IN1_SHADER`, and executable shader groups.

## Audit

- `rg -n "downloadUrl|fileSize|sha256|expectedIO|defaultParams|formula|code:|CUSTOM_SHADER_CODE|CUSTOM_2IN1_SHADER" src/catalog` returns no matches.
- Remaining TS ONNX pre/postprocess is outside the catalog layer under `src/engine/onnx` and was intentionally not migrated in this task.
- Remaining executable shader source is outside the catalog layer under `src/shaders`.

## Verification

- `cargo test -p open_quartz --test catalog_contract_test`
- `cargo test -p open_quartz --test executor_test math_nodes_cover_all_operations_and_boundary_values`
- `npm run check:public-proxy`
- `npx vitest run --pool=forks tests/engine/mathOps.test.ts tests/engine/onnxCatalog.test.ts tests/engine/predefinedShaders.test.ts tests/store/onnxStore.test.ts tests/store/mathStore.test.ts tests/components/Header.test.tsx tests/components/OnnxPanel.test.tsx tests/sdk/WasmSdkClient.test.ts tests/sdk/languageConformance.test.ts`
- Direct generated-wasm catalog parity check: Rust `bindings.catalog()` equals TS `Catalog.snapshot()` after stable key ordering.

Skipped full workspace tests, linters, formatters, `npm test`, and performance work as requested.
