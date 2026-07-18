import type { Port } from '../types';

export interface ParsedShader {
  inputs: Port[];
  outputs: Port[];
  raw: string;
}

export interface CompiledNode {
  nodeId: string;
  program: WebGLProgram | null;
  outputTexture: WebGLTexture | null;
  dirty: boolean;
}

export interface ExecutionGraph {
  order: string[];
  nodeMap: Map<string, CompiledNode>;
}
