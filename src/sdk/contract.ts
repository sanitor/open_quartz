export const SDK_API_VERSION = 1;

export interface SdkCapabilities {
  structuredEngine: boolean;
  typedFramePlanning: boolean;
  resourceGenerations: boolean;
  graphPlanning: boolean;
  wgslParsing: boolean;
  wgslCompilation: boolean;
  gpuResourcePrimitives: boolean;
  gpuExecution: boolean;
  onnxPrePostprocessing: boolean;
  nativeOnnxSession: boolean;
  browserOnnxSession: boolean;
}

export interface RuntimePublicSurface {
  apiVersion: number;
  methods: string[];
}

export function decodeRuntimePublicSurface(json: string): RuntimePublicSurface {
  const value = JSON.parse(json) as Partial<RuntimePublicSurface>;
  if (
    value.apiVersion !== SDK_API_VERSION
    || !Array.isArray(value.methods)
    || value.methods.some((method) => typeof method !== 'string')
  ) {
    throw new SdkContractError({
      code: 'invalid-response',
      message: 'Rust SDK returned an invalid runtime public surface',
    });
  }
  return value as RuntimePublicSurface;
}

export type SdkErrorCode =
  | 'disposed'
  | 'invalid-frame'
  | 'invalid-graph'
  | 'invalid-resource'
  | 'invalid-state'
  | 'not-prepared'
  | 'unknown-node'
  | 'protocol-mismatch'
  | 'invalid-response';

export interface SdkErrorPayload {
  code: SdkErrorCode;
  message: string;
  nodeId?: string;
  details?: string;
}

export type EngineState = 'empty' | 'ready' | 'running' | 'paused' | 'stopped' | 'disposed';

export type EngineEvent =
  | { type: 'state'; state: EngineState }
  | { type: 'graph-ready'; revision: number }
  | { type: 'resource-invalidated'; nodeId: string; generation: number }
  | { type: 'resource-released'; nodeId: string; generation: number }
  | {
      type: 'frame-planned';
      frame: number;
      revision: number;
      commandCount: number;
      dirtyNodeCount: number;
    };

export class SdkContractError extends Error {
  readonly code: SdkErrorCode;
  readonly nodeId?: string;
  readonly details?: string;

  constructor(payload: SdkErrorPayload) {
    super(payload.message);
    this.name = 'SdkContractError';
    this.code = payload.code;
    this.nodeId = payload.nodeId;
    this.details = payload.details;
  }
}

export function decodeSdkError(error: unknown): SdkContractError {
  if (error instanceof SdkContractError) return error;
  const source = error instanceof Error ? error.message : error;
  if (typeof source === 'string') {
    try {
      const parsed = JSON.parse(source) as Partial<SdkErrorPayload>;
      if (typeof parsed.code === 'string' && typeof parsed.message === 'string') {
        return new SdkContractError(parsed as SdkErrorPayload);
      }
    } catch {
      return new SdkContractError({ code: 'invalid-response', message: source });
    }
    return new SdkContractError({ code: 'invalid-response', message: source });
  }
  return new SdkContractError({
    code: 'invalid-response',
    message: 'Rust SDK returned a non-structured error',
    details: String(source),
  });
}

export function decodeCapabilities(json: string): SdkCapabilities {
  const value = JSON.parse(json) as Partial<SdkCapabilities>;
  const keys: ReadonlyArray<keyof SdkCapabilities> = [
    'typedFramePlanning',
    'resourceGenerations',
    'structuredEngine',
    'graphPlanning',
    'wgslParsing',
    'wgslCompilation',
    'gpuResourcePrimitives',
    'gpuExecution',
    'onnxPrePostprocessing',
    'nativeOnnxSession',
    'browserOnnxSession',
  ];
  if (!keys.every((key) => typeof value[key] === 'boolean')) {
    throw new SdkContractError({
      code: 'invalid-response',
      message: 'Rust SDK returned invalid capabilities',
    });
  }
  return value as SdkCapabilities;
}

export function decodeEngineEvents(json: string): EngineEvent[] {
  const value: unknown = JSON.parse(json);
  if (!Array.isArray(value)) {
    throw new SdkContractError({
      code: 'invalid-response',
      message: 'Rust SDK returned invalid engine events',
    });
  }
  return value as EngineEvent[];
}
