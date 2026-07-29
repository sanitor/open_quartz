import type { Port } from '../types';

export interface ParsedShader {
  inputs: Port[];
  outputs: Port[];
  raw: string;
  /** Syntax error from wgsl_reflect, if any. */
  parseError?: string;
  /** Shader-level description extracted from leading WGSL comments. */
  description?: string;
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
