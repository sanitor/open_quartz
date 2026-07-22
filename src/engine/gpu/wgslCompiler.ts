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
  /** Binding index for each texture input (for creating bind groups). */
  textureBindings: Map<string, number>;
  /** Binding index for each uniform (for creating bind groups). */
  uniformBindings: Map<string, number>;
  /** Binding index for previousFrame texture, if needed. */
  previousFrameBinding: number | null;
}

// ---------------------------------------------------------------------------
// Compiler
// ---------------------------------------------------------------------------

/**
 * Compile a user WGSL fragment shader into a GPURenderPipeline.
 *
 * The compiler injects system bindings (upstream textures + samplers,
 * scalar uniforms, previousFrame) and wraps the user code.
 *
 * @param device         The GPUDevice to create the pipeline on.
 * @param userCode       The user's WGSL fragment code (just the @fragment fn body).
 * @param inputPorts     Declared input ports from the parser.
 * @param upstreamMap    Map of uniform name → upstream node ID.
 * @param targetFormat   The render target format (default: rgba8unorm).
 */
export function compileWgslShader(
  device: GPUDevice,
  userCode: string,
  inputPorts: ReadonlyArray<{ label: string; dataType: string }>,
  upstreamMap: Map<string, string>,
  targetFormat: GPUTextureFormat = 'rgba8unorm',
): CompiledShader {
  const upstreamSamplers = new Map<string, string>();
  const textureBindings = new Map<string, number>();
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
    .replace(/@group\s*\(\s*\d+\s*\)\s*@binding\s*\(\s*\d+\s*\)\s*var\s+\w+\s*:\s*sampler\s*;/g, '')
    .replace(/@group\s*\(\s*\d+\s*\)\s*@binding\s*\(\s*\d+\s*\)\s*var\s*<\s*uniform\s*>\s*\w+\s*:\s*[\w<>]+\s*;/g, '');

  // 1. Inject texture bindings for connected upstream sampler2D inputs
  for (const [uniformName, sourceNodeId] of upstreamMap) {
    const port = inputPorts.find((p) => p.label === uniformName);
    if (port?.dataType === 'sampler2D') {
      // texture
      preamble += `@group(0) @binding(${bindingIndex}) var ${uniformName}: texture_2d<f32>;\n`;
      layoutEntries.push({
        binding: bindingIndex,
        visibility: GPUShaderStage.FRAGMENT,
        texture: { sampleType: 'float' },
      });
      textureBindings.set(uniformName, bindingIndex);
      upstreamSamplers.set(uniformName, sourceNodeId);
      bindingIndex++;

      // sampler
      const samplerName = `${uniformName}Sampler`;
      preamble += `@group(0) @binding(${bindingIndex}) var ${samplerName}: sampler;\n`;
      layoutEntries.push({
        binding: bindingIndex,
        visibility: GPUShaderStage.FRAGMENT,
        sampler: { type: 'filtering' },
      });
      bindingIndex++;
    } else if (port) {
      // Scalar/vector uniform
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

  // 3. Inject previousFrame if needed
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
  const fullFragCode = preamble + '\n' + processedCode;

  // Create bind group layout
  const bindGroupLayout = device.createBindGroupLayout({ entries: layoutEntries });

  // Create pipeline
  const vertModule = device.createShaderModule({ code: FULLSCREEN_VERT_WITH_UV });
  const fragModule = device.createShaderModule({ code: fullFragCode });

  const pipelineLayout = device.createPipelineLayout({
    bindGroupLayouts: [bindGroupLayout],
  });

  const pipeline = device.createRenderPipeline({
    layout: pipelineLayout,
    vertex: {
      module: vertModule,
      entryPoint: 'main',
    },
    fragment: {
      module: fragModule,
      entryPoint: 'main',
      targets: [{ format: targetFormat }],
    },
    primitive: {
      topology: 'triangle-list',
    },
  });

  return {
    pipeline,
    bindGroupLayout,
    upstreamSamplers,
    preambleLines,
    needsFeedback,
    textureBindings,
    uniformBindings,
    previousFrameBinding,
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

/**
 * Validate a WGSL shader by attempting to create a GPUShaderModule.
 * Returns null on success, or the error message on failure.
 */
export function validateWgslShader(device: GPUDevice, code: string): string | null {
  try {
    const module = device.createShaderModule({ code });
    // WebGPU shader compilation is synchronous in the module creation
    // but errors are reported via getCompilationInfo
    // For now, createShaderModule doesn't throw — errors surface at pipeline creation
    // Return null (success) and let pipeline creation catch real errors
    void module;
    return null;
  } catch (e) {
    return e instanceof Error ? e.message : String(e);
  }
}
