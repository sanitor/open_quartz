/**
 * Live WGSL editor validation helpers.
 *
 * Production shader compilation/materialization is handled by the Rust
 * descriptor pipeline; this file is kept only for browser edit-time checks.
 */

// ---------------------------------------------------------------------------
// GLSL to WGSL type name mapping (for the DataType strings in ports)
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
 * Uses a dedicated lightweight GPUDevice with no dependency on the render engine.
 * Returns an array of errors (empty on success). Error line numbers are mapped
 * back to the user's source (preamble lines subtracted).
 */
export async function validateWgslEdit(
  userCode: string,
  inputPorts: ReadonlyArray<{ label: string; dataType: string }>,
  device?: GPUDevice,
): Promise<WgslCompilationError[]> {
  const dev = device ?? await getValidationDevice();
  if (!dev) return []; // No WebGPU: can't validate, degrade gracefully

  let bindingIndex = 0;
  let preamble = '';

  // Strip user binding declarations before injecting edit-time validation bindings.
  const processedCode = userCode
    .replace(/@group\s*\(\s*\d+\s*\)\s*@binding\s*\(\s*\d+\s*\)\s*var\s+\w+\s*:\s*texture_2d\s*<\s*f32\s*>\s*;/g, '')
    .replace(/@group\s*\(\s*\d+\s*\)\s*@binding\s*\(\s*\d+\s*\)\s*var\s+\w+\s*:\s*texture_external\s*;/g, '')
    .replace(/@group\s*\(\s*\d+\s*\)\s*@binding\s*\(\s*\d+\s*\)\s*var\s+\w+\s*:\s*sampler\s*;/g, '')
    .replace(/@group\s*\(\s*\d+\s*\)\s*@binding\s*\(\s*\d+\s*\)\s*var\s*<\s*uniform\s*>\s*\w+\s*:\s*[\w<>]+\s*;/g, '');

  // Build the lightweight validation preamble from parsed ports.
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
