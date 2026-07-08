/* tslint:disable */
/* eslint-disable */

/**
 * Stable façade for the OpenQuartz TS side. Wraps upstream's free-function
 * `ort_bridge` API into a session-shaped handle so the engine can hold and
 * release detectors per model.
 *
 * Upstream `ort_detect` returns `{ output, scale, padX, padY, srcW, srcH }`
 * where `output` is the raw YOLOv8 tensor. Decode + NMS run here.
 */
export class YoloDetectorWasm {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Reserved for future WebGPURenderer integration — upstream's
     * `capture_webgpu_device` monkey-patches `GPUAdapter.prototype.requestDevice`
     * so wgpu and ORT share one device. OpenQuartz's Three.js WebGL renderer
     * has no device to intercept today; call this before any WebGPU init
     * once the renderer swap lands.
     */
    static captureWebgpuDevice(): void;
    /**
     * Run one detection. `canvas` accepts both `HTMLCanvasElement` and
     * `OffscreenCanvas` at the JS side because upstream's inline_js uses
     * `canvas.width/height` and passes the value to
     * `copyExternalImageToTexture` / `drawImage`.
     */
    detect(canvas: any, src_w: number, src_h: number): Promise<any>;
    init(): Promise<void>;
    constructor(model_url?: string | null, target_size?: number | null);
    release(): void;
    setThresholds(score: number, iou: number): void;
    readonly initialized: boolean;
    readonly targetSize: number;
}

export function classNames(): any;

export function defaultModelUrl(): string;

export function defaultTargetSize(): number;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_yolodetectorwasm_free: (a: number, b: number) => void;
    readonly classNames: () => any;
    readonly defaultModelUrl: () => [number, number];
    readonly defaultTargetSize: () => number;
    readonly yolodetectorwasm_detect: (a: number, b: any, c: number, d: number) => any;
    readonly yolodetectorwasm_init: (a: number) => any;
    readonly yolodetectorwasm_initialized: (a: number) => number;
    readonly yolodetectorwasm_new: (a: number, b: number, c: number) => number;
    readonly yolodetectorwasm_release: (a: number) => void;
    readonly yolodetectorwasm_setThresholds: (a: number, b: number, c: number) => void;
    readonly yolodetectorwasm_targetSize: (a: number) => number;
    readonly yolodetectorwasm_captureWebgpuDevice: () => void;
    readonly wasm_bindgen__convert__closures_____invoke__hf54f9d6457151012: (a: number, b: number, c: any) => [number, number];
    readonly wasm_bindgen__convert__closures_____invoke__h2494547d955a8308: (a: number, b: number, c: any, d: any) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_destroy_closure: (a: number, b: number) => void;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
