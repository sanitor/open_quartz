import type { Port } from '../types';
import type { ParsedShader } from '../engine/types';
import { requireSdk } from './runtime';

let portCounter = 0;

function nextPortId(): string {
  portCounter += 1;
  return `port_${portCounter}_${Date.now()}`;
}

function preservePortIds(parsed: Port[], existing?: Port[]): Port[] {
  const existingByLabel = new Map(existing?.map((port) => [port.label, port.id]));
  return parsed.map((port) => ({
    ...port,
    id: existingByLabel.get(port.label) ?? nextPortId(),
  }));
}

/** Synchronous production parser backed by the initialized Rust WASM SDK. */
export function parseWgslShader(
  code: string,
  existingInputs?: Port[],
  existingOutputs?: Port[],
): ParsedShader {
  const parsed = requireSdk().parseShader<ParsedShader>(code);
  parsed.inputs = preservePortIds(parsed.inputs, existingInputs);
  parsed.outputs = preservePortIds(parsed.outputs, existingOutputs);
  return parsed;
}
