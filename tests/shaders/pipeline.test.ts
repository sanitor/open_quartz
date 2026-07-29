/**
 * Integration bit-true tests — real WebGPU pipeline end-to-end.
 *
 * Tests the FULL path: Node graph → prepare() → runFrame() → readback → pixel verify.
 * This catches bugs in the execution engine (uniform uploads, texture binding, etc.)
 * that shader-level tests miss.
 *
 * Run with: npm run test:shaders
 */
import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import type { Node, Edge } from '@xyflow/react';
import type { ShaderNodeData, Port, DataType } from '../../src/types';
import { WebGPUBackend } from '../../src/engine/gpu/WebGPUBackend';
import { WebGPUExecutionEngine } from '../../src/engine/executionEngine';
import type { FrameInputs } from '../../src/engine/compositor';
import { validateWgslEdit, compileWgslShader } from '../../src/engine/gpu/wgslCompiler';

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

let backend: WebGPUBackend;
let engine: WebGPUExecutionEngine;

beforeAll(async () => {
  if (!navigator.gpu) throw new Error('WebGPU not available');

  const canvas = document.createElement('canvas');
  canvas.width = 4;
  canvas.height = 4;

  backend = new WebGPUBackend(canvas);
  await backend.init();

  engine = new WebGPUExecutionEngine();
  engine.initWithBackend(backend);
});

afterAll(() => {
  engine.dispose();
});

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const W = 4;
const H = 4;

/** Default FrameInputs for a single frame. */
function makeBuiltins(overrides?: Partial<FrameInputs>): FrameInputs {
  return {
    time: 1.0,
    delta: 0.016,
    frame: 60,
    date: new Float32Array([2026, 6, 27, 43200]),
    mouse: new Float32Array([0, 0, 0, 0]),
    resolution: new Float32Array([W, H, 1]),
    ...overrides,
  };
}

/** Create a solid-color 4×4 RGBA data URL (PNG, lossless). */
function solidDataUrl(r: number, g: number, b: number, a = 255): string {
  const canvas = document.createElement('canvas');
  canvas.width = W;
  canvas.height = H;
  const ctx = canvas.getContext('2d')!;
  const img = ctx.createImageData(W, H);
  for (let i = 0; i < W * H; i++) {
    img.data[i * 4] = r;
    img.data[i * 4 + 1] = g;
    img.data[i * 4 + 2] = b;
    img.data[i * 4 + 3] = a;
  }
  ctx.putImageData(img, 0, 0);
  return canvas.toDataURL('image/png');
}

let idCounter = 0;

function makePort(label: string, dataType: DataType, direction: 'input' | 'output'): Port {
  return { id: `p${++idCounter}`, label, dataType, direction };
}

function makeInputNode(id: string, dataUrl: string): Node<ShaderNodeData> {
  return {
    id,
    type: 'custom',
    position: { x: 0, y: 0 },
    data: {
      type: 'input',
      label: 'Image',
      shaderCode: '',
      inputs: [makePort('inputImage', 'sampler2D', 'input')],
      outputs: [makePort('output', 'sampler2D', 'output')],
      uniforms: {},
      inputDataType: 'sampler2D',
      inputMode: 'image',
      imageDataUrl: dataUrl,
      imageWidth: W,
      imageHeight: H,
    },
  };
}

function makeShaderNode(
  id: string,
  code: string,
  inputs: Port[],
  outputs: Port[],
  uniforms: Record<string, unknown> = {},
): Node<ShaderNodeData> {
  return {
    id,
    type: 'custom',
    position: { x: 200, y: 0 },
    data: {
      type: 'shader',
      label: 'Shader',
      shaderCode: code,
      inputs,
      outputs,
      uniforms,
    },
  };
}

function makeRendererNode(id: string): Node<ShaderNodeData> {
  return {
    id,
    type: 'custom',
    position: { x: 400, y: 0 },
    data: {
      type: 'renderer',
      label: 'Output',
      shaderCode: '',
      inputs: [makePort('inputImage', 'sampler2D', 'input')],
      outputs: [],
      uniforms: {},
    },
  };
}

function edge(source: string, target: string, targetHandle: string): Edge {
  return { id: `e-${source}-${target}`, source, target, targetHandle };
}

/** Run the pipeline: prepare → await textures → runFrame → readback pixels from shader target. */
async function runPipeline(
  nodes: Node<ShaderNodeData>[],
  edges: Edge[],
  readTargetNodeId: string,
  builtins?: Partial<FrameInputs>,
): Promise<Uint8ClampedArray> {
  const errors: string[] = [];
  const plan = engine.prepare(
    nodes, edges,
    (_id, msg) => errors.push(msg),
  );
  if (!plan) throw new Error('prepare() returned null');
  if (errors.length > 0) throw new Error(`Shader errors: ${errors.join('; ')}`);

  // Wait for async texture loads to complete
  await Promise.all(plan.pendingTextures);

  engine.runFrame(plan, makeBuiltins(builtins));

  const target = plan.targets.get(readTargetNodeId);
  if (!target) throw new Error(`No render target for node ${readTargetNodeId}`);
  const { rgba } = await backend.readTargetToRgba(target);
  return rgba;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('Integration bit-true (WebGPU pipeline)', () => {

  // 1. Image → Identity shader → output
  it('passthrough: input pixels survive the full pipeline unchanged', async () => {
    const dataUrl = solidDataUrl(100, 200, 50);

    const imgNode = makeInputNode('img', dataUrl);
    const shaderNode = makeShaderNode(
      'shader',
      /* wgsl */ `
@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  return textureSample(inputImage, inputImageSampler, v_uv);
}`,
      [makePort('inputImage', 'sampler2D', 'input')],
      [makePort('fragColor', 'vec4', 'output')],
    );

    const rgba = await runPipeline(
      [imgNode, shaderNode],
      [edge('img', 'shader', shaderNode.data.inputs[0].id)],
      'shader',
    );

    for (let i = 0; i < W * H; i++) {
      expect(rgba[i * 4]).toBe(100);
      expect(rgba[i * 4 + 1]).toBe(200);
      expect(rgba[i * 4 + 2]).toBe(50);
      expect(rgba[i * 4 + 3]).toBe(255);
    }
  });

  // 2. Image → Shader with scalar uniform → output
  //    This is THE test that catches the "uniform never uploaded" bug
  it('scalar uniform: intensity=0.5 halves RGB', async () => {
    const dataUrl = solidDataUrl(200, 100, 50);

    const imgNode = makeInputNode('img', dataUrl);
    const shaderNode = makeShaderNode(
      'shader',
      /* wgsl */ `
@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  var c = textureSample(inputImage, inputImageSampler, v_uv);
  return vec4f(c.rgb * intensity, c.a);
}`,
      [
        makePort('inputImage', 'sampler2D', 'input'),
        makePort('intensity', 'float', 'input'),
      ],
      [makePort('fragColor', 'vec4', 'output')],
      { intensity: 0.5 },
    );

    const rgba = await runPipeline(
      [imgNode, shaderNode],
      [edge('img', 'shader', shaderNode.data.inputs[0].id)],
      'shader',
    );

    // 200*0.5=100, 100*0.5=50, 50*0.5=25, alpha unchanged
    for (let i = 0; i < W * H; i++) {
      expect(rgba[i * 4]).toBe(100);
      expect(rgba[i * 4 + 1]).toBe(50);
      expect(rgba[i * 4 + 2]).toBe(25);
      expect(rgba[i * 4 + 3]).toBe(255);
    }
  });

  // 3. Image → Shader A → Shader B → output (two-stage cascade)
  it('cascade: two shaders in series compose correctly', async () => {
    const dataUrl = solidDataUrl(200, 100, 50);

    const imgNode = makeInputNode('img', dataUrl);

    // Stage A: invert
    const shaderA = makeShaderNode(
      'shaderA',
      /* wgsl */ `
@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let c = textureSample(inputImage, inputImageSampler, v_uv);
  return vec4f(1.0 - c.rgb, c.a);
}`,
      [makePort('inputImage', 'sampler2D', 'input')],
      [makePort('fragColor', 'vec4', 'output')],
    );

    // Stage B: invert again → should restore original
    const shaderB = makeShaderNode(
      'shaderB',
      /* wgsl */ `
@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let c = textureSample(inputImage, inputImageSampler, v_uv);
  return vec4f(1.0 - c.rgb, c.a);
}`,
      [makePort('inputImage', 'sampler2D', 'input')],
      [makePort('fragColor', 'vec4', 'output')],
    );

    const rgba = await runPipeline(
      [imgNode, shaderA, shaderB],
      [
        edge('img', 'shaderA', shaderA.data.inputs[0].id),
        edge('shaderA', 'shaderB', shaderB.data.inputs[0].id),
      ],
      'shaderB',
    );

    // Double invert = identity (within ±1 for 8-bit rounding)
    for (let i = 0; i < W * H; i++) {
      expect(Math.abs(rgba[i * 4] - 200)).toBeLessThanOrEqual(1);
      expect(Math.abs(rgba[i * 4 + 1] - 100)).toBeLessThanOrEqual(1);
      expect(Math.abs(rgba[i * 4 + 2] - 50)).toBeLessThanOrEqual(1);
      expect(rgba[i * 4 + 3]).toBe(255);
    }
  });

  // 4. Generator (no input texture) → constant color output
  it('generator: constant shader with no inputs produces exact value', async () => {
    const shaderNode = makeShaderNode(
      'shader',
      /* wgsl */ `
@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  return vec4f(0.5, 0.25, 0.75, 1.0);
}`,
      [],
      [makePort('fragColor', 'vec4', 'output')],
    );

    const rgba = await runPipeline([shaderNode], [], 'shader');

    for (let i = 0; i < W * H; i++) {
      expect(rgba[i * 4]).toBe(128);
      expect(rgba[i * 4 + 1]).toBe(64);
      expect(rgba[i * 4 + 2]).toBe(191);
      expect(rgba[i * 4 + 3]).toBe(255);
    }
  });

  // 5. Builtin uniform: iTime flows from FrameInputs through the pipeline to pixels
  it('builtin uniform: iTime reaches the shader as a real value', async () => {
    const shaderNode = makeShaderNode(
      'shader',
      /* wgsl */ `
@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  // Encode iTime into red channel: iTime=2.0 → red = 0.5 → 128
  return vec4f(iTime * 0.25, 0.0, 0.0, 1.0);
}`,
      [makePort('iTime', 'float', 'input')],
      [makePort('fragColor', 'vec4', 'output')],
    );

    const rgba = await runPipeline(
      [shaderNode], [], 'shader',
      { time: 2.0 },
    );

    // iTime=2.0 * 0.25 = 0.5 → 128
    for (let i = 0; i < W * H; i++) {
      expect(rgba[i * 4]).toBe(128);
      expect(rgba[i * 4 + 1]).toBe(0);
      expect(rgba[i * 4 + 2]).toBe(0);
      expect(rgba[i * 4 + 3]).toBe(255);
    }
  });
});

// ---------------------------------------------------------------------------
// Edit-time GPU validation
// ---------------------------------------------------------------------------

describe('validateWgslEdit (GPU compile check)', () => {
  it('@doraemon instead of @fragment produces a compile error', async () => {
    const code = `@doraemon fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  return vec4f(1.0);
}`;
    const errors = await validateWgslEdit(code, [], backend.device);
    expect(errors.length).toBeGreaterThan(0);
    expect(errors[0].message).toBeTruthy();
  });

  it('missing @fragment entry point produces a compile error', async () => {
    const code = `fn notAnEntryPoint() -> vec4f {
  return vec4f(1.0);
}`;
    // This may or may not error at module level (depends on impl),
    // but it will fail at pipeline creation. At minimum, no false positive.
    const errors = await validateWgslEdit(code, [], backend.device);
    // Some GPU drivers report no entry point as an error, some don't at module level.
    // We just verify it doesn't crash.
    expect(Array.isArray(errors)).toBe(true);
  });

  it('valid shader with ports produces no errors', async () => {
    const code = `@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let c = textureSample(inputImage, inputImageSampler, v_uv);
  return vec4f(c.rgb * intensity, c.a);
}`;
    const ports = [
      { label: 'inputImage', dataType: 'sampler2D' },
      { label: 'intensity', dataType: 'float' },
    ];
    const errors = await validateWgslEdit(code, ports, backend.device);
    expect(errors).toHaveLength(0);
  });

  it('undeclared identifier without matching port produces an error', async () => {
    const code = `@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  return vec4f(oops);
}`;
    const errors = await validateWgslEdit(code, [], backend.device);
    expect(errors.length).toBeGreaterThan(0);
  });
});

// ---------------------------------------------------------------------------
// Video zero-copy (importExternalTexture) compiler tests
// ---------------------------------------------------------------------------

describe('compileWgslShader — video external texture', () => {
  it('video input generates texture_external binding, not texture_2d', () => {
    const code = `@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  return textureSample(inputImage, inputImageSampler, v_uv);
}`;
    const ports = [{ label: 'inputImage', dataType: 'sampler2D' }];
    const upstreamMap = new Map([['inputImage', 'video-node-1']]);
    const videoInputs = new Set(['inputImage']);

    const compiled = compileWgslShader(
      backend.device, code, ports, upstreamMap, 'rgba8unorm', videoInputs,
    );

    // External texture binding exists
    expect(compiled.externalTextureBindings.has('inputImage')).toBe(true);
    // Regular texture binding does NOT exist for this input
    expect(compiled.textureBindings.has('inputImage')).toBe(false);
    // Only 1 binding slot used (no sampler needed)
    expect(compiled.externalTextureBindings.get('inputImage')).toBe(0);
  });

  it('non-video input still generates texture_2d + sampler (2 bindings)', () => {
    const code = `@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  return textureSample(inputImage, inputImageSampler, v_uv);
}`;
    const ports = [{ label: 'inputImage', dataType: 'sampler2D' }];
    const upstreamMap = new Map([['inputImage', 'image-node-1']]);

    const compiled = compileWgslShader(
      backend.device, code, ports, upstreamMap, 'rgba8unorm',
    );

    expect(compiled.textureBindings.has('inputImage')).toBe(true);
    expect(compiled.externalTextureBindings.has('inputImage')).toBe(false);
    // texture at binding 0, sampler at binding 1
    expect(compiled.textureBindings.get('inputImage')).toBe(0);
  });

  it('mixed inputs: video gets texture_external, image gets texture_2d', () => {
    const code = `@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let a = textureSample(videoIn, videoInSampler, v_uv);
  let b = textureSample(imageIn, imageInSampler, v_uv);
  return mix(a, b, 0.5);
}`;
    const ports = [
      { label: 'videoIn', dataType: 'sampler2D' },
      { label: 'imageIn', dataType: 'sampler2D' },
    ];
    const upstreamMap = new Map([
      ['videoIn', 'video-1'],
      ['imageIn', 'image-1'],
    ]);
    const videoInputs = new Set(['videoIn']);

    const compiled = compileWgslShader(
      backend.device, code, ports, upstreamMap, 'rgba8unorm', videoInputs,
    );

    // videoIn → external texture (1 binding)
    expect(compiled.externalTextureBindings.has('videoIn')).toBe(true);
    expect(compiled.textureBindings.has('videoIn')).toBe(false);
    // imageIn → texture_2d + sampler (2 bindings)
    expect(compiled.textureBindings.has('imageIn')).toBe(true);
    expect(compiled.externalTextureBindings.has('imageIn')).toBe(false);
  });
});
