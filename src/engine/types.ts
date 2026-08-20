import type { Port } from '../types';

export interface ParsedShader {
  inputs: Port[];
  outputs: Port[];
  raw: string;
  /** Syntax error from the Rust/naga parser, if any. */
  parseError?: string;
  /** Shader-level description extracted from leading WGSL comments. */
  description?: string;
}
