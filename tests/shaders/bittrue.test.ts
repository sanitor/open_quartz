/**
 * Shader bit-true tests — real WebGPU in a browser via vitest browser mode.
 *
 * Each test compiles a WGSL fragment shader, renders to an offscreen texture,
 * reads pixels back, and verifies exact or near-exact values.
 *
 * Run with: npm run test:shaders
 */
import { describe, it, expect, beforeAll } from 'vitest';

// ---------------------------------------------------------------------------
// Shared GPU device (created once, reused across tests)
// ---------------------------------------------------------------------------

let device: GPUDevice;

beforeAll(async () => {
  if (!navigator.gpu) throw new Error('WebGPU not available');
  const adapter = await navigator.gpu.requestAdapter();
  if (!adapter) throw new Error('No WebGPU adapter');
  device = await adapter.requestDevice();
});

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Fullscreen-triangle vertex shader — identical to the one shipped in WebGPUBackend. */
const FULLSCREEN_VERT = /* wgsl */ `
struct VertexOutput {
  @builtin(position) position: vec4f,
  @location(0) v_uv: vec2f,
}

@vertex
fn main(@builtin(vertex_index) vi: u32) -> VertexOutput {
  let x = f32(i32(vi) / 2) * 4.0 - 1.0;
  let y = f32(i32(vi) % 2) * 4.0 - 1.0;
  var out: VertexOutput;
  out.position = vec4f(x, y, 0.0, 1.0);
  out.v_uv = vec2f((x + 1.0) * 0.5, (1.0 - y) * 0.5);
  return out;
}
`;

/** Upload RGBA pixels to a NEAREST-filtered GPUTexture. */
function uploadTexture(
  w: number, h: number, pixels: Uint8Array,
): { texture: GPUTexture; view: GPUTextureView; sampler: GPUSampler } {
  const texture = device.createTexture({
    size: { width: w, height: h },
    format: 'rgba8unorm',
    usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_DST,
  });
  device.queue.writeTexture(
    { texture },
    pixels,
    { bytesPerRow: w * 4 },
    { width: w, height: h },
  );
  const view = texture.createView();
  const sampler = device.createSampler({
    minFilter: 'nearest',
    magFilter: 'nearest',
    addressModeU: 'clamp-to-edge',
    addressModeV: 'clamp-to-edge',
  });
  return { texture, view, sampler };
}

/**
 * Render a fullscreen triangle with the given WGSL fragment shader,
 * read back the RGBA pixels from the render target.
 *
 * @param fragCode  Complete WGSL fragment source (including @group/@binding).
 * @param w         Render target width.
 * @param h         Render target height.
 * @param layoutEntries  Bind group layout entries (empty → no bindings).
 * @param bindEntries    Bind group entries matching the layout.
 */
async function renderAndRead(
  fragCode: string,
  w: number,
  h: number,
  layoutEntries: GPUBindGroupLayoutEntry[] = [],
  bindEntries: GPUBindGroupEntry[] = [],
): Promise<Uint8Array> {
  // Pipeline
  const bindGroupLayouts: GPUBindGroupLayout[] = [];
  let bindGroup: GPUBindGroup | null = null;

  if (layoutEntries.length > 0) {
    const bgl = device.createBindGroupLayout({ entries: layoutEntries });
    bindGroupLayouts.push(bgl);
    bindGroup = device.createBindGroup({ layout: bgl, entries: bindEntries });
  }

  const pipeline = device.createRenderPipeline({
    layout: device.createPipelineLayout({ bindGroupLayouts }),
    vertex: {
      module: device.createShaderModule({ code: FULLSCREEN_VERT }),
      entryPoint: 'main',
    },
    fragment: {
      module: device.createShaderModule({ code: fragCode }),
      entryPoint: 'main',
      targets: [{ format: 'rgba8unorm' }],
    },
    primitive: { topology: 'triangle-list' },
  });

  // Render target
  const renderTex = device.createTexture({
    size: { width: w, height: h },
    format: 'rgba8unorm',
    usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.COPY_SRC,
  });

  // Encode render + readback
  const encoder = device.createCommandEncoder();
  const pass = encoder.beginRenderPass({
    colorAttachments: [{
      view: renderTex.createView(),
      loadOp: 'clear',
      storeOp: 'store',
      clearValue: { r: 0, g: 0, b: 0, a: 0 },
    }],
  });
  pass.setPipeline(pipeline);
  if (bindGroup) pass.setBindGroup(0, bindGroup);
  pass.draw(3);
  pass.end();

  const bytesPerRow = Math.ceil(w * 4 / 256) * 256;
  const readBuffer = device.createBuffer({
    size: bytesPerRow * h,
    usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ,
  });
  encoder.copyTextureToBuffer(
    { texture: renderTex },
    { buffer: readBuffer, bytesPerRow },
    { width: w, height: h },
  );
  device.queue.submit([encoder.finish()]);

  // Readback — strip row padding
  await readBuffer.mapAsync(GPUMapMode.READ);
  const mapped = new Uint8Array(readBuffer.getMappedRange());
  const output = new Uint8Array(w * h * 4);
  for (let y = 0; y < h; y++) {
    output.set(
      mapped.subarray(y * bytesPerRow, y * bytesPerRow + w * 4),
      y * w * 4,
    );
  }
  readBuffer.unmap();

  // Cleanup
  readBuffer.destroy();
  renderTex.destroy();
  return output;
}

/** Convenience: render with a single input texture (binding 0 = texture, 1 = sampler). */
async function renderWithTexture(
  fragCode: string, w: number, h: number, inputPixels: Uint8Array,
): Promise<Uint8Array> {
  const { view, sampler, texture } = uploadTexture(w, h, inputPixels);
  const result = await renderAndRead(
    fragCode, w, h,
    [
      { binding: 0, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: 'float' } },
      { binding: 1, visibility: GPUShaderStage.FRAGMENT, sampler: { type: 'filtering' } },
    ],
    [
      { binding: 0, resource: view },
      { binding: 1, resource: sampler },
    ],
  );
  texture.destroy();
  return result;
}

/** Make a solid-color 4×4 RGBA input. */
function solidInput(r: number, g: number, b: number, a = 255): Uint8Array {
  const pixels = new Uint8Array(4 * 4 * 4);
  for (let i = 0; i < 16; i++) {
    pixels[i * 4] = r;
    pixels[i * 4 + 1] = g;
    pixels[i * 4 + 2] = b;
    pixels[i * 4 + 3] = a;
  }
  return pixels;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('Shader bit-true (WebGPU)', () => {

  it('Identity: output equals input pixel-exact', async () => {
    const fs = /* wgsl */ `
@group(0) @binding(0) var inputImage: texture_2d<f32>;
@group(0) @binding(1) var inputImageSampler: sampler;

@fragment
fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  return textureSample(inputImage, inputImageSampler, v_uv);
}`;

    // All 4 rows identical — immune to Y-flip differences
    const input = new Uint8Array([
      100, 150, 200, 255,  50, 100, 150, 255,  200, 50, 100, 255,  25, 75, 125, 255,
      100, 150, 200, 255,  50, 100, 150, 255,  200, 50, 100, 255,  25, 75, 125, 255,
      100, 150, 200, 255,  50, 100, 150, 255,  200, 50, 100, 255,  25, 75, 125, 255,
      100, 150, 200, 255,  50, 100, 150, 255,  200, 50, 100, 255,  25, 75, 125, 255,
    ]);

    const output = await renderWithTexture(fs, 4, 4, input);
    expect(Array.from(output)).toEqual(Array.from(input));
  });

  it('Invert: (255,0,0) → (0,255,255)', async () => {
    const fs = /* wgsl */ `
@group(0) @binding(0) var inputImage: texture_2d<f32>;
@group(0) @binding(1) var inputImageSampler: sampler;

@fragment
fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let c = textureSample(inputImage, inputImageSampler, v_uv);
  return vec4f(1.0 - c.rgb, c.a);
}`;

    const output = await renderWithTexture(fs, 4, 4, solidInput(255, 0, 0));
    for (let i = 0; i < 16; i++) {
      expect(output[i * 4]).toBe(0);
      expect(output[i * 4 + 1]).toBe(255);
      expect(output[i * 4 + 2]).toBe(255);
      expect(output[i * 4 + 3]).toBe(255);
    }
  });

  it('Grayscale: pure green → luminance ≈ 150', async () => {
    const fs = /* wgsl */ `
@group(0) @binding(0) var inputImage: texture_2d<f32>;
@group(0) @binding(1) var inputImageSampler: sampler;

@fragment
fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let c = textureSample(inputImage, inputImageSampler, v_uv);
  let gray = dot(c.rgb, vec3f(0.299, 0.587, 0.114));
  return vec4f(vec3f(gray), c.a);
}`;

    const output = await renderWithTexture(fs, 4, 4, solidInput(0, 255, 0));
    const expected = Math.round(0.587 * 255); // 150
    for (let i = 0; i < 16; i++) {
      expect(output[i * 4]).toBeGreaterThanOrEqual(expected - 2);
      expect(output[i * 4]).toBeLessThanOrEqual(expected + 2);
      expect(output[i * 4 + 1]).toBe(output[i * 4]); // R=G=B
      expect(output[i * 4 + 2]).toBe(output[i * 4]);
      expect(output[i * 4 + 3]).toBe(255);
    }
  });

  it('Constant output: shader ignoring input produces exact value', async () => {
    const fs = /* wgsl */ `
@fragment
fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  return vec4f(0.5, 0.25, 0.75, 1.0);
}`;

    const output = await renderAndRead(fs, 4, 4);
    for (let i = 0; i < 16; i++) {
      expect(output[i * 4]).toBe(128);     // 0.5 * 255 = 127.5 → 128
      expect(output[i * 4 + 1]).toBe(64);  // 0.25 * 255 = 63.75 → 64
      expect(output[i * 4 + 2]).toBe(191); // 0.75 * 255 = 191.25 → 191
      expect(output[i * 4 + 3]).toBe(255);
    }
  });

  it('Alpha passthrough: input alpha preserved', async () => {
    const fs = /* wgsl */ `
@group(0) @binding(0) var inputImage: texture_2d<f32>;
@group(0) @binding(1) var inputImageSampler: sampler;

@fragment
fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  return textureSample(inputImage, inputImageSampler, v_uv);
}`;

    const output = await renderWithTexture(fs, 4, 4, solidInput(100, 200, 50, 128));
    for (let i = 0; i < 16; i++) {
      expect(output[i * 4]).toBe(100);
      expect(output[i * 4 + 1]).toBe(200);
      expect(output[i * 4 + 2]).toBe(50);
      expect(output[i * 4 + 3]).toBe(128);
    }
  });

  it('Channel swap: RGB → BRG', async () => {
    const fs = /* wgsl */ `
@group(0) @binding(0) var inputImage: texture_2d<f32>;
@group(0) @binding(1) var inputImageSampler: sampler;

@fragment
fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let c = textureSample(inputImage, inputImageSampler, v_uv);
  return vec4f(c.b, c.r, c.g, c.a);
}`;

    const output = await renderWithTexture(fs, 4, 4, solidInput(200, 100, 50));
    for (let i = 0; i < 16; i++) {
      expect(output[i * 4]).toBe(50);
      expect(output[i * 4 + 1]).toBe(200);
      expect(output[i * 4 + 2]).toBe(100);
      expect(output[i * 4 + 3]).toBe(255);
    }
  });

  it('Uniform scaling: intensity=0.5 halves brightness', async () => {
    const fs = /* wgsl */ `
@group(0) @binding(0) var inputImage: texture_2d<f32>;
@group(0) @binding(1) var inputImageSampler: sampler;
@group(0) @binding(2) var<uniform> intensity: f32;

@fragment
fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  let c = textureSample(inputImage, inputImageSampler, v_uv);
  return vec4f(c.rgb * intensity, c.a);
}`;

    const { view, sampler, texture } = uploadTexture(4, 4, solidInput(200, 100, 50));

    // Create a uniform buffer with intensity = 0.5
    const uniformData = new Float32Array([0.5]);
    const uniformBuf = device.createBuffer({
      size: 16, // WebGPU min uniform buffer size
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });
    device.queue.writeBuffer(uniformBuf, 0, uniformData);

    const output = await renderAndRead(
      fs, 4, 4,
      [
        { binding: 0, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: 'float' } },
        { binding: 1, visibility: GPUShaderStage.FRAGMENT, sampler: { type: 'filtering' } },
        { binding: 2, visibility: GPUShaderStage.FRAGMENT, buffer: { type: 'uniform' } },
      ],
      [
        { binding: 0, resource: view },
        { binding: 1, resource: sampler },
        { binding: 2, resource: { buffer: uniformBuf } },
      ],
    );
    texture.destroy();
    uniformBuf.destroy();

    // 200 * 0.5 = 100, 100 * 0.5 = 50, 50 * 0.5 = 25, alpha unchanged
    for (let i = 0; i < 16; i++) {
      expect(output[i * 4]).toBe(100);
      expect(output[i * 4 + 1]).toBe(50);
      expect(output[i * 4 + 2]).toBe(25);
      expect(output[i * 4 + 3]).toBe(255);
    }
  });
});
