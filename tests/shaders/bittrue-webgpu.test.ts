/**
 * Shader bit-true tests — WebGPU via vitest browser mode.
 *
 * Mirror of bittrue.test.ts (WebGL2) to verify that the WebGPU pipeline
 * produces identical pixel results. Same 4×4 inputs, same expected outputs.
 *
 * Run with: npm run test:shaders
 */
import { describe, it, expect } from 'vitest';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const FULLSCREEN_VERT = /* wgsl */ `
struct VertexOutput {
  @builtin(position) position: vec4f,
  @location(0) v_uv: vec2f,
}

@vertex
fn main(@builtin(vertex_index) vertexIndex: u32) -> VertexOutput {
  let x = f32(i32(vertexIndex) / 2) * 4.0 - 1.0;
  let y = f32(i32(vertexIndex) % 2) * 4.0 - 1.0;
  var out: VertexOutput;
  out.position = vec4f(x, y, 0.0, 1.0);
  out.v_uv = vec2f((x + 1.0) * 0.5, (1.0 - y) * 0.5);
  return out;
}
`;

let _device: GPUDevice | null = null;

async function getDevice(): Promise<GPUDevice> {
  if (_device) return _device;
  if (!navigator.gpu) throw new Error('WebGPU not available');
  const adapter = await navigator.gpu.requestAdapter();
  if (!adapter) throw new Error('No WebGPU adapter');
  _device = await adapter.requestDevice();
  return _device;
}

/** Create a render pipeline from a WGSL fragment shader. */
function createPipeline(
  device: GPUDevice,
  fragCode: string,
  bindGroupLayout: GPUBindGroupLayout,
): GPURenderPipeline {
  return device.createRenderPipeline({
    layout: device.createPipelineLayout({ bindGroupLayouts: [bindGroupLayout] }),
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
}

/** Upload a flat RGBA Uint8Array as a nearest-filtered texture. */
function uploadTexture(device: GPUDevice, w: number, h: number, pixels: Uint8Array): GPUTexture {
  const texture = device.createTexture({
    size: [w, h],
    format: 'rgba8unorm',
    usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_DST,
  });
  device.queue.writeTexture(
    { texture },
    pixels,
    { bytesPerRow: w * 4 },
    [w, h],
  );
  return texture;
}

/** Render a full-screen triangle and read back RGBA pixels. */
async function renderAndRead(
  device: GPUDevice,
  pipeline: GPURenderPipeline,
  bindGroup: GPUBindGroup,
  w: number,
  h: number,
): Promise<Uint8Array> {
  // Render target
  const outTex = device.createTexture({
    size: [w, h],
    format: 'rgba8unorm',
    usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.COPY_SRC,
  });

  const encoder = device.createCommandEncoder();
  const pass = encoder.beginRenderPass({
    colorAttachments: [{
      view: outTex.createView(),
      loadOp: 'clear',
      storeOp: 'store',
      clearValue: { r: 0, g: 0, b: 0, a: 0 },
    }],
  });
  pass.setPipeline(pipeline);
  pass.setBindGroup(0, bindGroup);
  pass.draw(3); // fullscreen triangle
  pass.end();

  // Copy to readback buffer
  const bytesPerRow = Math.ceil(w * 4 / 256) * 256; // align to 256
  const readBuf = device.createBuffer({
    size: bytesPerRow * h,
    usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ,
  });
  encoder.copyTextureToBuffer(
    { texture: outTex },
    { buffer: readBuf, bytesPerRow },
    [w, h],
  );
  device.queue.submit([encoder.finish()]);

  await readBuf.mapAsync(GPUMapMode.READ);
  const mapped = new Uint8Array(readBuf.getMappedRange());

  // Un-pad rows
  const output = new Uint8Array(w * h * 4);
  for (let row = 0; row < h; row++) {
    output.set(
      mapped.subarray(row * bytesPerRow, row * bytesPerRow + w * 4),
      row * w * 4,
    );
  }
  readBuf.unmap();
  return output;
}

/** Make a solid-color 4×4 RGBA input texture. */
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
// Tests — same cases as bittrue.test.ts (WebGL2), same expected results
// ---------------------------------------------------------------------------

describe('Shader bit-true (WebGPU)', () => {

  it('Identity: output equals input pixel-exact', async () => {
    const device = await getDevice();
    const frag = /* wgsl */ `
      @group(0) @binding(0) var inputImage: texture_2d<f32>;
      @group(0) @binding(1) var inputSampler: sampler;

      @fragment
      fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
        return textureSample(inputImage, inputSampler, v_uv);
      }`;

    const input = new Uint8Array([
      100, 150, 200, 255,  50, 100, 150, 255,  200, 50, 100, 255,  25, 75, 125, 255,
      100, 150, 200, 255,  50, 100, 150, 255,  200, 50, 100, 255,  25, 75, 125, 255,
      100, 150, 200, 255,  50, 100, 150, 255,  200, 50, 100, 255,  25, 75, 125, 255,
      100, 150, 200, 255,  50, 100, 150, 255,  200, 50, 100, 255,  25, 75, 125, 255,
    ]);

    const bgl = device.createBindGroupLayout({
      entries: [
        { binding: 0, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: 'float' } },
        { binding: 1, visibility: GPUShaderStage.FRAGMENT, sampler: { type: 'filtering' } },
      ],
    });
    const pipeline = createPipeline(device, frag, bgl);
    const tex = uploadTexture(device, 4, 4, input);
    const sampler = device.createSampler({ magFilter: 'nearest', minFilter: 'nearest' });
    const bg = device.createBindGroup({
      layout: bgl,
      entries: [
        { binding: 0, resource: tex.createView() },
        { binding: 1, resource: sampler },
      ],
    });

    const output = await renderAndRead(device, pipeline, bg, 4, 4);
    expect(Array.from(output)).toEqual(Array.from(input));
  });

  it('Invert: (255,0,0) → (0,255,255)', async () => {
    const device = await getDevice();
    const frag = /* wgsl */ `
      @group(0) @binding(0) var inputImage: texture_2d<f32>;
      @group(0) @binding(1) var inputSampler: sampler;

      @fragment
      fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
        let c = textureSample(inputImage, inputSampler, v_uv);
        return vec4f(1.0 - c.rgb, c.a);
      }`;

    const bgl = device.createBindGroupLayout({
      entries: [
        { binding: 0, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: 'float' } },
        { binding: 1, visibility: GPUShaderStage.FRAGMENT, sampler: { type: 'filtering' } },
      ],
    });
    const pipeline = createPipeline(device, frag, bgl);
    const tex = uploadTexture(device, 4, 4, solidInput(255, 0, 0));
    const sampler = device.createSampler({ magFilter: 'nearest', minFilter: 'nearest' });
    const bg = device.createBindGroup({
      layout: bgl,
      entries: [
        { binding: 0, resource: tex.createView() },
        { binding: 1, resource: sampler },
      ],
    });

    const output = await renderAndRead(device, pipeline, bg, 4, 4);
    for (let i = 0; i < 16; i++) {
      expect(output[i * 4]).toBe(0);
      expect(output[i * 4 + 1]).toBe(255);
      expect(output[i * 4 + 2]).toBe(255);
      expect(output[i * 4 + 3]).toBe(255);
    }
  });

  it('Grayscale: pure green → luminance ≈ 150', async () => {
    const device = await getDevice();
    const frag = /* wgsl */ `
      @group(0) @binding(0) var inputImage: texture_2d<f32>;
      @group(0) @binding(1) var inputSampler: sampler;

      @fragment
      fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
        let c = textureSample(inputImage, inputSampler, v_uv);
        let gray = dot(c.rgb, vec3f(0.299, 0.587, 0.114));
        return vec4f(vec3f(gray), c.a);
      }`;

    const bgl = device.createBindGroupLayout({
      entries: [
        { binding: 0, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: 'float' } },
        { binding: 1, visibility: GPUShaderStage.FRAGMENT, sampler: { type: 'filtering' } },
      ],
    });
    const pipeline = createPipeline(device, frag, bgl);
    const tex = uploadTexture(device, 4, 4, solidInput(0, 255, 0));
    const sampler = device.createSampler({ magFilter: 'nearest', minFilter: 'nearest' });
    const bg = device.createBindGroup({
      layout: bgl,
      entries: [
        { binding: 0, resource: tex.createView() },
        { binding: 1, resource: sampler },
      ],
    });

    const output = await renderAndRead(device, pipeline, bg, 4, 4);
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
    const device = await getDevice();
    const frag = /* wgsl */ `
      @fragment
      fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
        return vec4f(0.5, 0.25, 0.75, 1.0);
      }`;

    const bgl = device.createBindGroupLayout({ entries: [] });
    const pipeline = createPipeline(device, frag, bgl);
    const bg = device.createBindGroup({ layout: bgl, entries: [] });

    const output = await renderAndRead(device, pipeline, bg, 4, 4);
    for (let i = 0; i < 16; i++) {
      expect(output[i * 4]).toBe(128);      // 0.5 * 255 = 127.5 → 128
      expect(output[i * 4 + 1]).toBe(64);   // 0.25 * 255 = 63.75 → 64
      expect(output[i * 4 + 2]).toBe(191);  // 0.75 * 255 = 191.25 → 191
      expect(output[i * 4 + 3]).toBe(255);
    }
  });

  it('Alpha passthrough: input alpha preserved', async () => {
    const device = await getDevice();
    const frag = /* wgsl */ `
      @group(0) @binding(0) var inputImage: texture_2d<f32>;
      @group(0) @binding(1) var inputSampler: sampler;

      @fragment
      fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
        return textureSample(inputImage, inputSampler, v_uv);
      }`;

    const bgl = device.createBindGroupLayout({
      entries: [
        { binding: 0, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: 'float' } },
        { binding: 1, visibility: GPUShaderStage.FRAGMENT, sampler: { type: 'filtering' } },
      ],
    });
    const pipeline = createPipeline(device, frag, bgl);
    const tex = uploadTexture(device, 4, 4, solidInput(100, 200, 50, 128));
    const sampler = device.createSampler({ magFilter: 'nearest', minFilter: 'nearest' });
    const bg = device.createBindGroup({
      layout: bgl,
      entries: [
        { binding: 0, resource: tex.createView() },
        { binding: 1, resource: sampler },
      ],
    });

    const output = await renderAndRead(device, pipeline, bg, 4, 4);
    for (let i = 0; i < 16; i++) {
      expect(output[i * 4]).toBe(100);
      expect(output[i * 4 + 1]).toBe(200);
      expect(output[i * 4 + 2]).toBe(50);
      expect(output[i * 4 + 3]).toBe(128);
    }
  });

  it('Channel swap: RGB → BRG', async () => {
    const device = await getDevice();
    const frag = /* wgsl */ `
      @group(0) @binding(0) var inputImage: texture_2d<f32>;
      @group(0) @binding(1) var inputSampler: sampler;

      @fragment
      fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
        let c = textureSample(inputImage, inputSampler, v_uv);
        return vec4f(c.b, c.r, c.g, c.a);
      }`;

    const bgl = device.createBindGroupLayout({
      entries: [
        { binding: 0, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: 'float' } },
        { binding: 1, visibility: GPUShaderStage.FRAGMENT, sampler: { type: 'filtering' } },
      ],
    });
    const pipeline = createPipeline(device, frag, bgl);
    const tex = uploadTexture(device, 4, 4, solidInput(200, 100, 50));
    const sampler = device.createSampler({ magFilter: 'nearest', minFilter: 'nearest' });
    const bg = device.createBindGroup({
      layout: bgl,
      entries: [
        { binding: 0, resource: tex.createView() },
        { binding: 1, resource: sampler },
      ],
    });

    const output = await renderAndRead(device, pipeline, bg, 4, 4);
    for (let i = 0; i < 16; i++) {
      expect(output[i * 4]).toBe(50);
      expect(output[i * 4 + 1]).toBe(200);
      expect(output[i * 4 + 2]).toBe(100);
      expect(output[i * 4 + 3]).toBe(255);
    }
  });
});
