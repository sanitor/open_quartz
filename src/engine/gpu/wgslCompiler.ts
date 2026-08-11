/**
 * WGSL Shader Compiler — compiles user WGSL code into a GPURenderPipeline.
 *
 * Replaces the GLSL shaderCompiler.ts for the WebGPU pipeline.
 *
 * Strategy:
 * - User writes the @fragment fn with @location(0) v_uv input
 * - Compiler wraps with a system preamble (bindings for upstream textures,
 *   uniforms, previousFrame) that the user doesn't need to declare
 * - The fullscreen vertex shader is shared across all fragment shaders
 * - Returns a GPURenderPipeline + bind group layout for the engine to use
 */

import { FULLSCREEN_VERT_WITH_UV } from './WebGPUBackend';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface CompiledShader {
  pipeline: GPURenderPipeline;
  bindGroupLayout: GPUBindGroupLayout;
  /** Mapping: uniform name → upstream node ID (for sampler2D inputs). */
  upstreamSamplers: Map<string, string>;
  /** Number of lines injected before user code (for error line mapping). */
  preambleLines: number;
  /** Whether the shader references `previousFrame` (feedback/accumulator). */
  needsFeedback: boolean;
  /** Binding index for each texture_2d input (for creating bind groups). */
  textureBindings: Map<string, number>;
  /** Binding index for each texture_external input (zero-copy video). */
  externalTextureBindings: Map<string, number>;
  /** Binding index for each uniform (for creating bind groups). */
  uniformBindings: Map<string, number>;
  /** Binding index for previousFrame texture, if needed. */
  previousFrameBinding: number | null;
}

export interface CanonicalCompiledShader {
  fullFragmentCode: string;
  preambleLines: number;
  bindings: Array<{
    binding: number;
    kind: 'texture' | 'externalTexture' | 'sampler' | 'uniform';
    name: string;
    wgslType?: string;
  }>;
  upstreamSamplers: Record<string, string>;
  textureBindings: Record<string, number>;
  externalTextureBindings: Record<string, number>;
  uniformBindings: Record<string, number>;
  previousFrameBinding: number | null;
  needsFeedback: boolean;
  targetFormat: string;
  vertexShader: string;
}

// ---------------------------------------------------------------------------
// Compiler
// ---------------------------------------------------------------------------

/**
 * Compile a user WGSL fragment shader into a GPURenderPipeline.
 *
 * @param device         The GPUDevice to create the pipeline on.
 * @param userCode       The user's WGSL fragment code.
 * @param inputPorts     Declared input ports from the parser.
 * @param upstreamMap    Map of uniform name → upstream node ID.
 * @param targetFormat   The render target format (default: rgba8unorm).
 * @param videoInputs    Set of texture input names from video nodes (use texture_external for zero-copy).
 */
export function compileWgslShader(
  device: GPUDevice,
  userCode: string,
  inputPorts: ReadonlyArray<{ label: string; dataType: string }>,
  upstreamMap: Map<string, string>,
  targetFormat: GPUTextureFormat = 'rgba8unorm',
  videoInputs: ReadonlySet<string> = new Set(),
): CompiledShader {
  const upstreamSamplers = new Map<string, string>();
  const textureBindings = new Map<string, number>();
  const externalTextureBindings = new Map<string, number>();
  const uniformBindings = new Map<string, number>();
  const layoutEntries: GPUBindGroupLayoutEntry[] = [];

  let bindingIndex = 0;
  let preamble = '';

  // Auto-detect feedback
  const needsFeedback = /\bpreviousFrame\b/.test(userCode);
  let previousFrameBinding: number | null = null;

  // Strip user binding declarations — we'll inject our own
  let processedCode = userCode
    .replace(/@group\s*\(\s*\d+\s*\)\s*@binding\s*\(\s*\d+\s*\)\s*var\s+\w+\s*:\s*texture_2d\s*<\s*f32\s*>\s*;/g, '')
    .replace(/@group\s*\(\s*\d+\s*\)\s*@binding\s*\(\s*\d+\s*\)\s*var\s+\w+\s*:\s*texture_external\s*;/g, '')
    .replace(/@group\s*\(\s*\d+\s*\)\s*@binding\s*\(\s*\d+\s*\)\s*var\s+\w+\s*:\s*sampler\s*;/g, '')
    .replace(/@group\s*\(\s*\d+\s*\)\s*@binding\s*\(\s*\d+\s*\)\s*var\s*<\s*uniform\s*>\s*\w+\s*:\s*[\w<>]+\s*;/g, '');

  // 1. Inject texture bindings for connected upstream sampler2D inputs
  for (const [uniformName, sourceNodeId] of upstreamMap) {
    const port = inputPorts.find((p) => p.label === uniformName);
    if (port?.dataType === 'sampler2D') {
      upstreamSamplers.set(uniformName, sourceNodeId);

      if (videoInputs.has(uniformName)) {
        // Zero-copy video: texture_external + sampler
        preamble += `@group(0) @binding(${bindingIndex}) var ${uniformName}: texture_external;\n`;
        layoutEntries.push({
          binding: bindingIndex,
          visibility: GPUShaderStage.FRAGMENT,
          externalTexture: {},
        });
        externalTextureBindings.set(uniformName, bindingIndex);
        bindingIndex++;

        // Sampler still required for textureSampleBaseClampToEdge
        const samplerName = `${uniformName}Sampler`;
        preamble += `@group(0) @binding(${bindingIndex}) var ${samplerName}: sampler;\n`;
        layoutEntries.push({
          binding: bindingIndex,
          visibility: GPUShaderStage.FRAGMENT,
          sampler: { type: 'filtering' },
        });
        bindingIndex++;

        // Rewrite textureSample → textureSampleBaseClampToEdge (sampler arg stays)
        const sampleRe = new RegExp(
          `textureSample\\s*\\(\\s*${uniformName}\\s*,`,
          'g',
        );
        processedCode = processedCode.replace(
          sampleRe,
          `textureSampleBaseClampToEdge(${uniformName},`,
        );
      } else {
        // Image / render target: texture_2d<f32> + sampler (2 bindings)
        preamble += `@group(0) @binding(${bindingIndex}) var ${uniformName}: texture_2d<f32>;\n`;
        layoutEntries.push({
          binding: bindingIndex,
          visibility: GPUShaderStage.FRAGMENT,
          texture: { sampleType: 'float' },
        });
        textureBindings.set(uniformName, bindingIndex);
        bindingIndex++;

        const samplerName = `${uniformName}Sampler`;
        preamble += `@group(0) @binding(${bindingIndex}) var ${samplerName}: sampler;\n`;
        layoutEntries.push({
          binding: bindingIndex,
          visibility: GPUShaderStage.FRAGMENT,
          sampler: { type: 'filtering' },
        });
        bindingIndex++;
      }
    } else if (port) {
      const wgslType = glslToWgslType(port.dataType);
      preamble += `@group(0) @binding(${bindingIndex}) var<uniform> ${uniformName}: ${wgslType};\n`;
      layoutEntries.push({
        binding: bindingIndex,
        visibility: GPUShaderStage.FRAGMENT,
        buffer: { type: 'uniform' },
      });
      uniformBindings.set(uniformName, bindingIndex);
      bindingIndex++;
    }
  }

  // 2. Inject non-sampler uniforms that aren't connected upstream
  for (const input of inputPorts) {
    if (!upstreamMap.has(input.label) && input.dataType !== 'sampler2D' && input.dataType !== 'samplerCube') {
      const wgslType = glslToWgslType(input.dataType);
      preamble += `@group(0) @binding(${bindingIndex}) var<uniform> ${input.label}: ${wgslType};\n`;
      layoutEntries.push({
        binding: bindingIndex,
        visibility: GPUShaderStage.FRAGMENT,
        buffer: { type: 'uniform' },
      });
      uniformBindings.set(input.label, bindingIndex);
      bindingIndex++;
    }
  }

  // 3. Inject previousFrame if needed (always texture_2d, never video)
  if (needsFeedback) {
    preamble += `@group(0) @binding(${bindingIndex}) var previousFrame: texture_2d<f32>;\n`;
    layoutEntries.push({
      binding: bindingIndex,
      visibility: GPUShaderStage.FRAGMENT,
      texture: { sampleType: 'float' },
    });
    previousFrameBinding = bindingIndex;
    bindingIndex++;

    preamble += `@group(0) @binding(${bindingIndex}) var previousFrameSampler: sampler;\n`;
    layoutEntries.push({
      binding: bindingIndex,
      visibility: GPUShaderStage.FRAGMENT,
      sampler: { type: 'filtering' },
    });
    bindingIndex++;
  }

  const preambleLines = preamble.split('\n').filter(Boolean).length;
  const fullFragCode = preamble + processedCode;

  const bindGroupLayout = device.createBindGroupLayout({ entries: layoutEntries });
  const vertModule = device.createShaderModule({ code: FULLSCREEN_VERT_WITH_UV });
  const fragModule = device.createShaderModule({ code: fullFragCode });
  const pipelineLayout = device.createPipelineLayout({ bindGroupLayouts: [bindGroupLayout] });

  const pipeline = device.createRenderPipeline({
    layout: pipelineLayout,
    vertex: { module: vertModule, entryPoint: 'main' },
    fragment: { module: fragModule, entryPoint: 'main', targets: [{ format: targetFormat }] },
    primitive: { topology: 'triangle-list' },
  });

  return {
    pipeline, bindGroupLayout, upstreamSamplers, preambleLines, needsFeedback,
    textureBindings, externalTextureBindings, uniformBindings, previousFrameBinding,
  };
}

/** Materialize the canonical Rust shader descriptor into browser GPU objects. */
export function materializeWgslShader(
  device: GPUDevice,
  descriptor: CanonicalCompiledShader,
): CompiledShader {
  const entries: GPUBindGroupLayoutEntry[] = descriptor.bindings.map((binding) => {
    const entry: GPUBindGroupLayoutEntry = {
      binding: binding.binding,
      visibility: GPUShaderStage.FRAGMENT,
    };
    if (binding.kind === 'texture') {
      entry.texture = {
        sampleType: 'float',
        viewDimension: binding.wgslType?.includes('cube') ? 'cube' : '2d',
      };
    } else if (binding.kind === 'externalTexture') {
      entry.externalTexture = {};
    } else if (binding.kind === 'sampler') {
      entry.sampler = { type: 'filtering' };
    } else {
      entry.buffer = { type: 'uniform' };
    }
    return entry;
  });
  const bindGroupLayout = device.createBindGroupLayout({ entries });
  const pipelineLayout = device.createPipelineLayout({ bindGroupLayouts: [bindGroupLayout] });
  const pipeline = device.createRenderPipeline({
    layout: pipelineLayout,
    vertex: {
      module: device.createShaderModule({ code: descriptor.vertexShader }),
      entryPoint: 'main',
    },
    fragment: {
      module: device.createShaderModule({ code: descriptor.fullFragmentCode }),
      entryPoint: 'main',
      targets: [{ format: descriptor.targetFormat as GPUTextureFormat }],
    },
    primitive: { topology: 'triangle-list' },
  });
  return {
    pipeline,
    bindGroupLayout,
    upstreamSamplers: new Map(Object.entries(descriptor.upstreamSamplers)),
    preambleLines: descriptor.preambleLines,
    needsFeedback: descriptor.needsFeedback,
    textureBindings: new Map(Object.entries(descriptor.textureBindings)),
    externalTextureBindings: new Map(Object.entries(descriptor.externalTextureBindings)),
    uniformBindings: new Map(Object.entries(descriptor.uniformBindings)),
    previousFrameBinding: descriptor.previousFrameBinding,
  };
}

// ---------------------------------------------------------------------------
// GLSL → WGSL type name mapping (for the DataType strings in ports)
// ---------------------------------------------------------------------------

function glslToWgslType(dataType: string): string {
  switch (dataType) {
    case 'float': return 'f32';
    case 'int': return 'i32';
    case 'uint': return 'u32';
    case 'bool': return 'u32'; // WGSL has no bool in uniform buffers
    case 'vec2': return 'vec2f';
    case 'vec3': return 'vec3f';
    case 'vec4': return 'vec4f';
    case 'ivec2': return 'vec2i';
    case 'ivec3': return 'vec3i';
    case 'ivec4': return 'vec4i';
    case 'uvec2': return 'vec2u';
    case 'uvec3': return 'vec3u';
    case 'uvec4': return 'vec4u';
    case 'mat2': return 'mat2x2f';
    case 'mat3': return 'mat3x3f';
    case 'mat4': return 'mat4x4f';
    default: return 'f32';
  }
}

// ---------------------------------------------------------------------------
// Shader validation (compile check)
// ---------------------------------------------------------------------------

export interface WgslCompilationError {
  message: string;
  /** 1-based line in the full (preamble + user) code. */
  line: number;
  /** 0-based column offset. */
  column: number;
  /** Byte offset into the source. */
  offset: number;
  /** Length of the error span in bytes. */
  length: number;
}

/**
 * Validate a WGSL shader via GPUShaderModule.getCompilationInfo().
 * Returns an array of errors (empty on success).
 *
 * @param preambleLines  Number of lines the compiler injected before user code,
 *                       used to map error lines back to user source.
 */
export async function validateWgslShader(
  device: GPUDevice,
  code: string,
  preambleLines = 0,
): Promise<WgslCompilationError[]> {
  try {
    const module = device.createShaderModule({ code });
    const info = await module.getCompilationInfo();
    const errors: WgslCompilationError[] = [];
    for (const msg of info.messages) {
      if (msg.type === 'error') {
        errors.push({
          message: msg.message,
          line: Math.max(1, msg.lineNum - preambleLines),
          column: msg.linePos,
          offset: msg.offset,
          length: msg.length,
        });
      }
    }
    return errors;
  } catch (e) {
    return [{
      message: e instanceof Error ? e.message : String(e),
      line: 1,
      column: 0,
      offset: 0,
      length: 0,
    }];
  }
}

// ---------------------------------------------------------------------------
// Edit-time validation (no pipeline, just shader compile check)
// ---------------------------------------------------------------------------

/** Lazy-initialized lightweight GPUDevice for edit-time validation only. */
let validationDevice: GPUDevice | null = null;
let validationDevicePromise: Promise<GPUDevice | null> | null = null;

async function getValidationDevice(): Promise<GPUDevice | null> {
  if (validationDevice) return validationDevice;
  if (validationDevicePromise) return validationDevicePromise;
  validationDevicePromise = (async () => {
    try {
      if (!navigator.gpu) return null;
      const adapter = await navigator.gpu.requestAdapter();
      if (!adapter) return null;
      validationDevice = await adapter.requestDevice();
      validationDevice.lost.then(() => {
        validationDevice = null;
        validationDevicePromise = null;
      });
      return validationDevice;
    } catch {
      return null;
    }
  })();
  return validationDevicePromise;
}

/**
 * Validate user-edited WGSL code by building the same preamble the compiler
 * would inject and running it through createShaderModule + getCompilationInfo.
 *
 * Uses a dedicated lightweight GPUDevice — no dependency on the render engine.
 * Returns an array of errors (empty on success). Error line numbers are mapped
 * back to the user's source (preamble lines subtracted).
 */
export async function validateWgslEdit(
  userCode: string,
  inputPorts: ReadonlyArray<{ label: string; dataType: string }>,
  device?: GPUDevice,
): Promise<WgslCompilationError[]> {
  const dev = device ?? await getValidationDevice();
  if (!dev) return []; // No WebGPU — can't validate, degrade gracefully

  let bindingIndex = 0;
  let preamble = '';

  // Strip user binding declarations (same regex as compileWgslShader)
  const processedCode = userCode
    .replace(/@group\s*\(\s*\d+\s*\)\s*@binding\s*\(\s*\d+\s*\)\s*var\s+\w+\s*:\s*texture_2d\s*<\s*f32\s*>\s*;/g, '')
    .replace(/@group\s*\(\s*\d+\s*\)\s*@binding\s*\(\s*\d+\s*\)\s*var\s+\w+\s*:\s*sampler\s*;/g, '')
    .replace(/@group\s*\(\s*\d+\s*\)\s*@binding\s*\(\s*\d+\s*\)\s*var\s*<\s*uniform\s*>\s*\w+\s*:\s*[\w<>]+\s*;/g, '');

  // Build preamble from parsed ports (mirrors compileWgslShader logic)
  for (const port of inputPorts) {
    if (port.dataType === 'sampler2D') {
      preamble += `@group(0) @binding(${bindingIndex}) var ${port.label}: texture_2d<f32>;\n`;
      bindingIndex++;
      preamble += `@group(0) @binding(${bindingIndex}) var ${port.label}Sampler: sampler;\n`;
      bindingIndex++;
    } else if (port.dataType !== 'samplerCube') {
      const wgslType = glslToWgslType(port.dataType);
      preamble += `@group(0) @binding(${bindingIndex}) var<uniform> ${port.label}: ${wgslType};\n`;
      bindingIndex++;
    }
  }

  // Inject previousFrame if referenced
  if (/\bpreviousFrame\b/.test(userCode)) {
    preamble += `@group(0) @binding(${bindingIndex}) var previousFrame: texture_2d<f32>;\n`;
    bindingIndex++;
    preamble += `@group(0) @binding(${bindingIndex}) var previousFrameSampler: sampler;\n`;
    bindingIndex++;
  }

  const preambleLines = preamble.split('\n').filter(Boolean).length;
  const fullCode = preamble + processedCode;

  return validateWgslShader(dev, fullCode, preambleLines);
}
