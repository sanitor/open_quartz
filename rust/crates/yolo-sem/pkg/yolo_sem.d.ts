/* tslint:disable */
/* eslint-disable */

export class YoloSemWasm {
    free(): void;
    [Symbol.dispose](): void;
    static captureWebgpuDevice(): void;
    init(): Promise<void>;
    constructor(model_url?: string | null, target_size?: number | null);
    release(): void;
    segment(canvas: any, src_w: number, src_h: number): Promise<any>;
    readonly initialized: boolean;
    readonly targetSize: number;
}

export function classNames(): any;

export function defaultModelUrl(): string;

export function defaultTargetSize(): number;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_yolosemwasm_free: (a: number, b: number) => void;
    readonly classNames: () => any;
    readonly defaultModelUrl: () => [number, number];
    readonly defaultTargetSize: () => number;
    readonly yolosemwasm_init: (a: number) => any;
    readonly yolosemwasm_initialized: (a: number) => number;
    readonly yolosemwasm_new: (a: number, b: number, c: number) => number;
    readonly yolosemwasm_release: (a: number) => void;
    readonly yolosemwasm_segment: (a: number, b: any, c: number, d: number) => any;
    readonly yolosemwasm_targetSize: (a: number) => number;
    readonly yolosemwasm_captureWebgpuDevice: () => void;
    readonly wasm_bindgen__convert__closures_____invoke__h0c6311c40319b468: (a: number, b: number, c: any) => [number, number];
    readonly wasm_bindgen__convert__closures_____invoke__h2ea592e15bfc293c: (a: number, b: number, c: any, d: any) => void;
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
