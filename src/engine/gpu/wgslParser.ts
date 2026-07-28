/**
 * WGSL Shader Parser — extract inputs (bindings) and outputs from WGSL source.
 *
 * Uses wgsl_reflect for proper AST-based parsing when bindings are declared,
 * with a regex fallback for catalog shaders that omit declarations.
 *
 * wgsl_reflect throws on invalid WGSL, so callers get syntax errors
 * instead of silent empty results.
 */

import { WgslReflect } from 'wgsl_reflect/wgsl_reflect.module.js';
import type { Port, DataType } from '../types';
import type { ParsedShader } from './types';

// ---------------------------------------------------------------------------
// WGSL type → internal DataType mapping
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
  'texture_2d': 'sampler2D',
  'texture_2d<f32>': 'sampler2D',
};

function mapWgslType(raw: string): DataType {
  return WGSL_TYPE_MAP[raw.trim()] ?? 'float';
}

// ---------------------------------------------------------------------------
// Builtin uniforms injected by the engine — not user ports
// ---------------------------------------------------------------------------

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
// Fallback regexes for shaders without @group/@binding declarations
// ---------------------------------------------------------------------------

/** Detect textureSample(texName, texNameSampler, ...) calls. */
const TEXTURE_SAMPLE_RE = /textureSample\w*\s*\(\s*(\w+)\s*,/g;

/** Detect textureDimensions(texName) calls. */
const TEXTURE_DIMS_RE = /textureDimensions\s*\(\s*(\w+)\s*\)/g;

/** Match: fn main(...) -> @location(0) type */
const FN_OUTPUT_RE = /->\s*@location\s*\(\s*(\d+)\s*\)\s*([\w<>]+)/g;

/** Match: @location(N) name: type (struct output member) */
const OUTPUT_RE = /@location\s*\(\s*(\d+)\s*\)\s*(?:var\s+)?(\w+)?\s*:\s*([\w<>]+)/g;

const WGSL_KEYWORDS = new Set([
  'fn', 'let', 'var', 'return', 'if', 'else', 'for', 'while', 'loop', 'break',
  'continue', 'switch', 'case', 'default', 'struct', 'true', 'false', 'discard',
  'main', 'v_uv', 'position', 'const', 'override', 'enable', 'diagnostic',
  'alias', 'continuing', 'fallthrough',
  // Attribute names (appear as bare identifiers after @)
  'fragment', 'vertex', 'compute', 'location', 'group', 'binding',
  'builtin', 'workgroup_size', 'align', 'size', 'interpolate', 'invariant',
  'id', 'must_use',
]);

const WGSL_BUILTINS = new Set([
  'textureSample', 'textureSampleLevel', 'textureSampleBias', 'textureSampleGrad',
  'textureSampleCompare', 'textureSampleCompareLevel',
  'textureLoad', 'textureStore', 'textureDimensions', 'textureNumLevels', 'textureNumLayers',
  'textureGather', 'textureGatherCompare',
  'vec2f', 'vec3f', 'vec4f', 'vec2i', 'vec3i', 'vec4i',
  'vec2u', 'vec3u', 'vec4u', 'vec2h', 'vec3h', 'vec4h',
  'vec2', 'vec3', 'vec4',
  'mat2x2f', 'mat3x3f', 'mat4x4f', 'mat2x2', 'mat3x3', 'mat4x4',
  'f32', 'f16', 'i32', 'u32', 'bool',
  'abs', 'acos', 'acosh', 'asin', 'asinh', 'atan', 'atan2', 'atanh',
  'ceil', 'clamp', 'cos', 'cosh', 'cross',
  'degrees', 'determinant', 'distance', 'dot',
  'exp', 'exp2',
  'faceForward', 'floor', 'fma', 'fract', 'frexp',
  'inverseSqrt',
  'ldexp', 'length', 'log', 'log2',
  'max', 'min', 'mix', 'modf',
  'normalize',
  'pow',
  'radians', 'reflect', 'refract', 'round',
  'saturate', 'select', 'sign', 'sin', 'sinh', 'smoothstep', 'sqrt', 'step',
  'tan', 'tanh', 'transpose', 'trunc',
  'dpdx', 'dpdxCoarse', 'dpdxFine', 'dpdy', 'dpdyCoarse', 'dpdyFine', 'fwidth',
  'pack4x8snorm', 'pack4x8unorm', 'pack2x16snorm', 'pack2x16unorm', 'pack2x16float',
  'unpack4x8snorm', 'unpack4x8unorm', 'unpack2x16snorm', 'unpack2x16unorm', 'unpack2x16float',
  'storageBarrier', 'workgroupBarrier', 'workgroupUniformLoad',
  'arrayLength', 'atomicLoad', 'atomicStore', 'atomicAdd', 'atomicSub',
  'atomicMax', 'atomicMin', 'atomicAnd', 'atomicOr', 'atomicXor',
  'atomicExchange', 'atomicCompareExchangeWeak',
  'countLeadingZeros', 'countOneBits', 'countTrailingZeros',
  'extractBits', 'firstLeadingBit', 'firstTrailingBit', 'insertBits', 'reverseBits',
  'all', 'any',
]);

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
  const seenInputs = new Set<string>();

  // -----------------------------------------------------------------------
  // Try AST-based extraction with wgsl_reflect
  // -----------------------------------------------------------------------
  let hasBindings = false;
  let parseError: string | null = null;

  try {
    const reflect = new WgslReflect(code);

    // --- Extract inputs from declared bindings ---

    // Textures → sampler2D inputs
    for (const tex of reflect.textures) {
      if (BUILTIN_UNIFORMS.has(tex.name)) continue;
      if (seenInputs.has(tex.name)) continue;
      seenInputs.add(tex.name);
      hasBindings = true;
      const existing = existingInputMap.get(tex.name);
      inputs.push({
        id: existing?.id ?? nextPortId(),
        label: tex.name,
        dataType: 'sampler2D',
        direction: 'input',
      });
    }

    // Uniforms → scalar/vector inputs
    for (const u of reflect.uniforms) {
      if (BUILTIN_UNIFORMS.has(u.name)) continue;
      if (seenInputs.has(u.name)) continue;
      seenInputs.add(u.name);
      hasBindings = true;
      const existing = existingInputMap.get(u.name);
      inputs.push({
        id: existing?.id ?? nextPortId(),
        label: u.name,
        dataType: mapWgslType(u.type.name),
        direction: 'input',
      });
    }

    // --- Extract outputs from fragment entry point ---
    const frag = reflect.entry.fragment[0];
    if (frag) {
      for (const out of frag.outputs) {
        const label = out.name || 'fragColor';
        const existing = existingOutputMap.get(label);
        outputs.push({
          id: existing?.id ?? nextPortId(),
          label,
          dataType: out.type ? mapWgslType(out.type.name) : 'vec4',
          direction: 'output',
        });
      }
    }
  } catch (e) {
    // wgsl_reflect throws on invalid WGSL — store the error but continue
    // with regex fallback so we still extract what we can
    parseError = e instanceof Error ? e.message : String(e);
  }

  // -----------------------------------------------------------------------
  // Fallback: regex-based extraction for shaders without binding declarations
  // (This covers all predefined catalog shaders that omit @group/@binding)
  // -----------------------------------------------------------------------
  if (!hasBindings && parseError === null) {
    // Strip comments so regex doesn't match words in // or /* */ comments.
    // Replace with spaces to preserve character positions for member-access checks.
    const cleanCode = code
      .replace(/\/\/[^\n]*/g, (m) => ' '.repeat(m.length))
      .replace(/\/\*[\s\S]*?\*\//g, (m) => ' '.repeat(m.length));

    // Detect texture inputs from textureSample(texName, ...) calls
    const textureNames = new Set<string>();
    let m: RegExpExecArray | null;

    TEXTURE_SAMPLE_RE.lastIndex = 0;
    while ((m = TEXTURE_SAMPLE_RE.exec(cleanCode)) !== null) {
      textureNames.add(m[1]);
    }
    TEXTURE_DIMS_RE.lastIndex = 0;
    while ((m = TEXTURE_DIMS_RE.exec(cleanCode)) !== null) {
      textureNames.add(m[1]);
    }

    for (const label of textureNames) {
      if (BUILTIN_UNIFORMS.has(label)) continue;
      if (seenInputs.has(label)) continue;
      seenInputs.add(label);
      const existing = existingInputMap.get(label);
      inputs.push({
        id: existing?.id ?? nextPortId(),
        label,
        dataType: 'sampler2D',
        direction: 'input',
      });
    }

    // Detect scalar uniform usage — identifiers used but not declared locally
    const localVars = new Set<string>();
    const LOCAL_RE = /(?:let|var|const)\s+(\w+)/g;
    LOCAL_RE.lastIndex = 0;
    while ((m = LOCAL_RE.exec(cleanCode)) !== null) {
      localVars.add(m[1]);
    }
    // Extract fn names defined in the shader
    const FN_RE = /fn\s+(\w+)\s*\(/g;
    FN_RE.lastIndex = 0;
    while ((m = FN_RE.exec(cleanCode)) !== null) {
      localVars.add(m[1]);
    }
    // Extract fn parameter names (includes @location(N) prefixed params)
    const PARAM_RE = /(?:@\w+(?:\(\d+\))?\s+)*(\w+)\s*:\s*[\w<>]+(?:\s*,|\s*\))/g;
    PARAM_RE.lastIndex = 0;
    while ((m = PARAM_RE.exec(cleanCode)) !== null) {
      localVars.add(m[1]);
    }

    const candidateUniforms = new Set<string>();
    // Match identifiers NOT preceded by '.' (member access).
    // We scan all identifiers but check the character before the match.
    const IDENT_RE = /\b([a-zA-Z_]\w*)\b/g;
    IDENT_RE.lastIndex = 0;
    while ((m = IDENT_RE.exec(cleanCode)) !== null) {
      // Skip member access: if char before match is '.', this is a field/swizzle
      if (m.index > 0 && cleanCode[m.index - 1] === '.') continue;
      // Skip attribute names: if char before match is '@', this is an attribute
      if (m.index > 0 && cleanCode[m.index - 1] === '@') continue;
      const name = m[1];
      if (WGSL_KEYWORDS.has(name)) continue;
      if (WGSL_BUILTINS.has(name)) continue;
      if (BUILTIN_UNIFORMS.has(name)) continue;
      if (seenInputs.has(name)) continue;
      if (localVars.has(name)) continue;
      if (name.endsWith('Sampler') || name.endsWith('_sampler')) continue;
      if (/^[A-Z]/.test(name)) continue; // skip types/constructors
      candidateUniforms.add(name);
    }

    for (const label of candidateUniforms) {
      seenInputs.add(label);
      const existing = existingInputMap.get(label);
      inputs.push({
        id: existing?.id ?? nextPortId(),
        label,
        dataType: 'float', // default to float for inferred uniforms
        direction: 'input',
      });
    }

    // Parse outputs via regex if wgsl_reflect didn't find them
    if (outputs.length === 0) {
      FN_OUTPUT_RE.lastIndex = 0;
      while ((m = FN_OUTPUT_RE.exec(code)) !== null) {
        const wgslType = m[2];
        const dataType = mapWgslType(wgslType);
        const label = 'fragColor';
        const existing = existingOutputMap.get(label);
        outputs.push({
          id: existing?.id ?? nextPortId(),
          label,
          dataType,
          direction: 'output',
        });
      }

      // Fallback to struct-based @location
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
    }
  }

  return { inputs, outputs, raw: code, parseError: parseError ?? undefined };
}
