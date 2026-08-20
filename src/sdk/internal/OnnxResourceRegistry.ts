import type { Port } from '../../types';
import { introspectOnnxModel, metaToDefaultPorts } from '../../engine/onnx/introspect';
import { OnnxInferenceSession } from '../../engine/onnx/inference';
import { OnnxModelManager } from '../../engine/onnx/modelManager';
import type { OnnxModelDescriptor } from '../catalog';

const manager = new OnnxModelManager();

export interface PreparedOnnxModel {
  backend?: 'webgpu' | 'wasm';
  inputs?: Port[];
  outputs?: Port[];
}

export async function prepareCatalogOnnx(
  entry: OnnxModelDescriptor,
  onProgress?: (progress: number) => void,
): Promise<PreparedOnnxModel> {
  let buffer = await manager.loadCachedModel(entry.id);
  if (!buffer) {
    const unsubscribe = onProgress
      ? manager.subscribe(() => onProgress(manager.getState(entry.id).progress))
      : null;
    try {
      buffer = await manager.downloadModel(entry);
    } finally {
      unsubscribe?.();
    }
  }
  return { backend: await probeBackend(buffer, entry.task === 'detection') };
}

export async function prepareCustomOnnx(
  modelId: string,
  buffer: ArrayBuffer,
): Promise<PreparedOnnxModel> {
  manager.cacheBuffer(modelId, buffer);
  const metadata = await introspectOnnxModel(buffer);
  const ports = metaToDefaultPorts(metadata);
  return {
    backend: await probeBackend(buffer, false),
    inputs: ports.inputs,
    outputs: ports.outputs,
  };
}

export function cacheOnnxModel(modelId: string, buffer: ArrayBuffer): void {
  manager.cacheBuffer(modelId, buffer);
}

export async function loadOnnxModel(
  modelId: string | undefined,
  entry: OnnxModelDescriptor | undefined,
  customPath: string | undefined,
): Promise<ArrayBuffer> {
  if (modelId) {
    const cached = await manager.loadCachedModel(modelId);
    if (cached) return cached;
    if (entry) return await manager.downloadModel(entry);
  }
  if (customPath) return await manager.loadLocalModel(customPath);
  throw new Error(`ONNX model ${modelId ?? customPath ?? '<missing>'} is not available`);
}

async function probeBackend(
  buffer: ArrayBuffer,
  skip: boolean,
): Promise<'webgpu' | 'wasm' | undefined> {
  if (skip) return undefined;
  const session = new OnnxInferenceSession();
  try {
    await session.loadFromBuffer(buffer);
    return await session.probeBackend(3);
  } finally {
    session.dispose();
  }
}
