/**
 * WGSL Shader Parser — extract inputs (bindings) and outputs from WGSL source.
 *
 * Replaces the GLSL shaderParser.ts for the WebGPU pipeline.
 *
 * WGSL binding declarations:
 *   @group(0) @binding(N) var texName: texture_2d<f32>;
 *   @group(0) @binding(N) var sampName: sampler;
 *   @group(0) @binding(N) var<uniform> name: f32;
 *   @group(0) @binding(N) var<uniform> name: vec4f;
 *   @group(0) @binding(N) var<uniform> name: Params;  // struct
 *
 * Output: @location(0) in the fragment return type or struct.
 */

import type { Port, DataType } from '../types';
import type { ParsedShader } from './types';

// ---------------------------------------------------------------------------
// WGSL type mapping
// ---------------------------------------------------------------------------

const WGSL_TYPE_MAP: Record<string, DataType> = {
  'f32': 'float',
  'i32': 'int',
  'u32': 'uint',
  'bool': 'bool',
  'vec2f': 'vec2',
  'vec2<f32>': 'vec2',
  'vec2i': 'ivec2',
  'vec2<i32>': 'ivec2',
  'vec2u': 'uvec2',
  'vec2<u32>': 'uvec2',
  'vec3f': 'vec3',
  'vec3<f32>': 'vec3',
  'vec3i': 'ivec3',
  'vec3<i32>': 'ivec3',
  'vec3u': 'uvec3',
  'vec3<u32>': 'uvec3',
  'vec4f': 'vec4',
  'vec4<f32>': 'vec4',
  'vec4i': 'ivec4',
  'vec4<i32>': 'ivec4',
  'vec4u': 'uvec4',
  'vec4<u32>': 'uvec4',
  'mat2x2f': 'mat2',
  'mat2x2<f32>': 'mat2',
  'mat3x3f': 'mat3',
  'mat3x3<f32>': 'mat3',
  'mat4x4f': 'mat4',
  'mat4x4<f32>': 'mat4',
  'texture_2d<f32>': 'sampler2D',
};

function mapWgslType(raw: string): DataType {
  const t = raw.trim();
  return WGSL_TYPE_MAP[t] ?? 'float';
}

// ---------------------------------------------------------------------------
// Regexes
// ---------------------------------------------------------------------------

/** Match: @group(G) @binding(B) var name: texture_2d<f32>; */
const TEXTURE_RE = /@group\s*\(\s*\d+\s*\)\s*@binding\s*\(\s*(\d+)\s*\)\s*var\s+(\w+)\s*:\s*texture_2d\s*<\s*f32\s*>\s*;/g;

/** Match: @group(G) @binding(B) var<uniform> name: type; */
const UNIFORM_RE = /@group\s*\(\s*\d+\s*\)\s*@binding\s*\(\s*(\d+)\s*\)\s*var\s*<\s*uniform\s*>\s*(\w+)\s*:\s*([\w<>]+)\s*;/g;

/** Match: @location(N) in fragment output */
const OUTPUT_RE = /@location\s*\(\s*(\d+)\s*\)\s*(?:var\s+)?(\w+)?\s*:\s*([\w<>]+)/g;

/** Match: fn main(...) -> @location(0) type */
const FN_OUTPUT_RE = /->\s*@location\s*\(\s*(\d+)\s*\)\s*([\w<>]+)/g;

/** Builtin uniforms that the engine injects — not user ports. */
const BUILTIN_UNIFORMS = new Set([
  'iTime', 'iTimeDelta', 'iFrame', 'iDate', 'iMouse', 'iResolution', 'previousFrame',
]);

// ---------------------------------------------------------------------------
// Port ID generation
// ---------------------------------------------------------------------------

let portCounter = 0;
function nextPortId(): string {
  return `port_${++portCounter}_${Date.now()}`;
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

export function parseWgslShader(
  code: string,
  existingInputs?: Port[],
  existingOutputs?: Port[],
): ParsedShader {
  const existingInputMap = new Map(existingInputs?.map((p) => [p.label, p]));
  const existingOutputMap = new Map(existingOutputs?.map((p) => [p.label, p]));

  const inputs: Port[] = [];
  const outputs: Port[] = [];

  // Parse texture bindings → sampler2D inputs
  let m: RegExpExecArray | null;
  TEXTURE_RE.lastIndex = 0;
  while ((m = TEXTURE_RE.exec(code)) !== null) {
    const label = m[2];
    if (BUILTIN_UNIFORMS.has(label)) continue;
    // Skip companion sampler declarations (e.g. inputImageSampler)
    if (label.endsWith('Sampler') || label.endsWith('_sampler')) continue;
    const existing = existingInputMap.get(label);
    inputs.push({
      id: existing?.id ?? nextPortId(),
      label,
      dataType: 'sampler2D',
      direction: 'input',
    });
  }

  // Parse uniform bindings → scalar/vector inputs
  UNIFORM_RE.lastIndex = 0;
  while ((m = UNIFORM_RE.exec(code)) !== null) {
    const label = m[2];
    const wgslType = m[3];
    if (BUILTIN_UNIFORMS.has(label)) continue;
    const dataType = mapWgslType(wgslType);
    const existing = existingInputMap.get(label);
    inputs.push({
      id: existing?.id ?? nextPortId(),
      label,
      dataType,
      direction: 'input',
    });
  }

  // Parse fragment outputs
  // Try function return type first: fn main(...) -> @location(0) vec4f
  FN_OUTPUT_RE.lastIndex = 0;
  while ((m = FN_OUTPUT_RE.exec(code)) !== null) {
    const wgslType = m[2];
    const dataType = mapWgslType(wgslType);
    const label = 'fragColor'; // conventional name
    const existing = existingOutputMap.get(label);
    outputs.push({
      id: existing?.id ?? nextPortId(),
      label,
      dataType,
      direction: 'output',
    });
  }

  // If no fn return output found, look for @location in struct
  if (outputs.length === 0) {
    OUTPUT_RE.lastIndex = 0;
    while ((m = OUTPUT_RE.exec(code)) !== null) {
      const label = m[2] ?? 'fragColor';
      const wgslType = m[3];
      const dataType = mapWgslType(wgslType);
      const existing = existingOutputMap.get(label);
      outputs.push({
        id: existing?.id ?? nextPortId(),
        label,
        dataType,
        direction: 'output',
      });
    }
  }

  return { inputs, outputs, raw: code };
}
