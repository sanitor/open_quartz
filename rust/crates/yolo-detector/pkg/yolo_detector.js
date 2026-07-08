// CI stub for the wasm-pack output. Committed so that `tsc -b`, `vitest`,
// and Vite's import analysis all resolve `@nodes/yolo-detector` without the
// crate having been built.
//
// `wasm-pack build rust/crates/yolo-detector --target web` (aka
// `npm run build:wasm`) overwrites this file with the real bindings that
// import the per-build `./snippets/<hash>/inline0.js` shim. The stub itself
// must NOT reference `./snippets/` — that directory only exists in a real
// wasm-pack pkg, and Vite would fail its import analysis on CI otherwise.
//
// Runtime behavior: every export throws. Any code path that reaches the stub
// at runtime has skipped `npm run build:wasm` — the error message says so.
// Tests never call these — they mock `@nodes/yolo-detector` via `vi.mock`.

/* @ts-self-types="./yolo_detector.d.ts" */

const NOT_BUILT = new Error(
  '@nodes/yolo-detector wasm bundle not built. Run `npm run build:wasm` first ' +
    '(or `npm run predev` / `npm run prebuild`, which wire it in automatically).',
);

function stub() { throw NOT_BUILT; }

export class YoloDetectorWasm {
  constructor() { throw NOT_BUILT; }
  static captureWebgpuDevice() { throw NOT_BUILT; }
}

export const classNames = stub;
export const defaultModelUrl = stub;
export const defaultTargetSize = stub;
export const initSync = stub;
export default stub;
