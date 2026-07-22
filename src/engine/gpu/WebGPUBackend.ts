/**
 * WebGPUBackend — pure WebGPU 2D shader rendering layer.
 *
 * Replaces Three.js WebGLRenderer for fullscreen-quad image processing.
 * Manages a GPUDevice, render targets (GPUTexture), shader pipelines,
 * and screen presentation. Designed to share its GPUDevice with ORT
 * and Three.js WebGPURenderer (for future 3D nodes).
 */

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/** A render target (FBO equivalent) — a GPUTexture that can be rendered to and sampled. */
export interface RenderTarget {
  texture: GPUTexture;
  view: GPUTextureView;
  width: number;
  height: number;
  format: GPUTextureFormat;
  /** Sampling: filter + wrap. Applied when creating bind groups. */
  sampler: GPUSampler;
}

/** A loaded image/data texture that can be sampled by shaders. */
export interface TextureHandle {
  texture: GPUTexture;
  view: GPUTextureView;
  width: number;
  height: number;
  sampler: GPUSampler;
}

/** Filter mode for texture sampling. */
export type FilterMode = 'linear' | 'nearest';

/** Wrap mode for texture sampling. */
export type WrapMode = 'clamp' | 'repeat' | 'mirror';

// ---------------------------------------------------------------------------
// Fullscreen triangle vertex shader (shared by all fragment shaders)
// ---------------------------------------------------------------------------

const FULLSCREEN_VERT = /* wgsl */ `
@vertex
fn main(@builtin(vertex_index) vertexIndex: u32) -> @builtin(position) vec4f {
  // Fullscreen triangle: 3 vertices cover the entire clip space
  let x = f32(i32(vertexIndex) / 2) * 4.0 - 1.0;
  let y = f32(i32(vertexIndex) % 2) * 4.0 - 1.0;
  return vec4f(x, y, 0.0, 1.0);
}
`;

// With v_uv output for fragment shaders:
export const FULLSCREEN_VERT_WITH_UV = /* wgsl */ `
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
  // Map clip coords to UV: [-1,1] → [0,1], flip Y for top-left origin
  out.v_uv = vec2f((x + 1.0) * 0.5, (1.0 - y) * 0.5);
  return out;
}
`;

// Simple blit fragment shader (copy texture → output)
const BLIT_FRAG = /* wgsl */ `
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

@fragment
fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  return textureSample(tex, samp, v_uv);
}
`;

// ---------------------------------------------------------------------------
// Backend
// ---------------------------------------------------------------------------

export class WebGPUBackend {
  private _device: GPUDevice | null = null;
  private _context: GPUCanvasContext | null = null;
  private _canvas: HTMLCanvasElement;
  private _presentFormat: GPUTextureFormat = 'bgra8unorm';

  // Caches
  private targets = new Map<string, RenderTarget>();
  private imageTextures = new Map<string, TextureHandle>();
  private pipelineCache = new Map<string, GPURenderPipeline>();

  // Blit pipeline (for renderSampler2DInput / renderToScreen)
  private blitPipeline: GPURenderPipeline | null = null;
  private blitBindGroupLayout: GPUBindGroupLayout | null = null;

  constructor(canvas: HTMLCanvasElement) {
    this._canvas = canvas;
  }

  /** Initialize the WebGPU device. Must be called before any other method. */
  async init(): Promise<void> {
    if (!navigator.gpu) {
      throw new Error('WebGPU not supported in this browser');
    }
    const adapter = await navigator.gpu.requestAdapter();
    if (!adapter) {
      throw new Error('Failed to get WebGPU adapter');
    }
    this._device = await adapter.requestDevice();

    this._context = this._canvas.getContext('webgpu') as GPUCanvasContext;
    this._presentFormat = navigator.gpu.getPreferredCanvasFormat();
    this._context.configure({
      device: this._device,
      format: this._presentFormat,
      alphaMode: 'opaque',
    });

    // Build the blit pipeline
    this.buildBlitPipeline();
  }

  /** The underlying GPUDevice — share with ORT and Three.js. */
  get device(): GPUDevice {
    if (!this._device) throw new Error('WebGPUBackend not initialized');
    return this._device;
  }

  get canvas(): HTMLCanvasElement {
    return this._canvas;
  }

  // -------------------------------------------------------------------------
  // Render targets
  // -------------------------------------------------------------------------

  setSize(width: number, height: number): void {
    this._canvas.width = width;
    this._canvas.height = height;
    if (this._context && this._device) {
      this._context.configure({
        device: this._device,
        format: this._presentFormat,
        alphaMode: 'opaque',
      });
    }
  }

  createTarget(
    id: string,
    width: number,
    height: number,
    float = false,
    _fbFormat?: string,
  ): RenderTarget {
    const device = this.device;
    const format: GPUTextureFormat = float ? 'rgba16float' : 'rgba8unorm';
    const texture = device.createTexture({
      size: { width, height },
      format,
      usage:
        GPUTextureUsage.RENDER_ATTACHMENT |
        GPUTextureUsage.TEXTURE_BINDING |
        GPUTextureUsage.COPY_SRC |
        GPUTextureUsage.COPY_DST,
    });
    const view = texture.createView();
    const sampler = device.createSampler({
      minFilter: 'linear',
      magFilter: 'linear',
      addressModeU: 'clamp-to-edge',
      addressModeV: 'clamp-to-edge',
    });
    const target: RenderTarget = { texture, view, width, height, format, sampler };
    this.targets.set(id, target);
    return target;
  }

  getTarget(id: string): RenderTarget | undefined {
    return this.targets.get(id);
  }

  // -------------------------------------------------------------------------
  // Texture loading
  // -------------------------------------------------------------------------

  async loadImageTexture(id: string, dataUrl: string): Promise<TextureHandle> {
    const device = this.device;
    const img = new Image();
    img.src = dataUrl;
    await img.decode();

    const bitmap = await createImageBitmap(img);
    const texture = device.createTexture({
      size: { width: bitmap.width, height: bitmap.height },
      format: 'rgba8unorm',
      usage:
        GPUTextureUsage.TEXTURE_BINDING |
        GPUTextureUsage.COPY_DST |
        GPUTextureUsage.RENDER_ATTACHMENT,
    });
    device.queue.copyExternalImageToTexture(
      { source: bitmap },
      { texture },
      { width: bitmap.width, height: bitmap.height },
    );
    bitmap.close();

    const view = texture.createView();
    const sampler = device.createSampler({
      minFilter: 'linear',
      magFilter: 'linear',
      mipmapFilter: 'linear',
    });
    const handle: TextureHandle = {
      texture, view, width: img.naturalWidth, height: img.naturalHeight, sampler,
    };
    this.imageTextures.set(id, handle);
    return handle;
  }

  getImageTexture(id: string): TextureHandle | undefined {
    return this.imageTextures.get(id);
  }

  loadRawTexture(
    id: string,
    buffer: ArrayBuffer,
    _format: string,
    width: number,
    height: number,
    _stride?: number,
  ): TextureHandle {
    const device = this.device;
    // For now, treat all raw textures as rgba8unorm
    const texture = device.createTexture({
      size: { width, height },
      format: 'rgba8unorm',
      usage:
        GPUTextureUsage.TEXTURE_BINDING |
        GPUTextureUsage.COPY_DST,
    });
    device.queue.writeTexture(
      { texture },
      buffer,
      { bytesPerRow: width * 4 },
      { width, height },
    );
    const view = texture.createView();
    const sampler = device.createSampler({
      minFilter: 'linear',
      magFilter: 'linear',
    });
    const handle: TextureHandle = { texture, view, width, height, sampler };
    this.imageTextures.set(id, handle);
    return handle;
  }

  applySampling(handle: TextureHandle | RenderTarget, filter?: FilterMode, wrap?: WrapMode): void {
    const device = this.device;
    const minFilter: GPUFilterMode = filter === 'nearest' ? 'nearest' : 'linear';
    const magFilter: GPUFilterMode = filter === 'nearest' ? 'nearest' : 'linear';
    const addressMode: GPUAddressMode =
      wrap === 'repeat' ? 'repeat'
        : wrap === 'mirror' ? 'mirror-repeat'
          : 'clamp-to-edge';
    handle.sampler = device.createSampler({
      minFilter,
      magFilter,
      addressModeU: addressMode,
      addressModeV: addressMode,
    });
  }

  // -------------------------------------------------------------------------
  // Rendering
  // -------------------------------------------------------------------------

  /**
   * Render a fullscreen quad with the given pipeline and bind group to a target.
   * If target is null, renders to screen.
   */
  renderPass(
    pipeline: GPURenderPipeline,
    bindGroup: GPUBindGroup,
    target: RenderTarget | null,
  ): void {
    const device = this.device;
    const encoder = device.createCommandEncoder();
    const colorAttachment: GPURenderPassColorAttachment = {
      view: target ? target.view : this._context!.getCurrentTexture().createView(),
      loadOp: 'clear',
      storeOp: 'store',
      clearValue: { r: 0, g: 0, b: 0, a: 1 },
    };
    const pass = encoder.beginRenderPass({
      colorAttachments: [colorAttachment],
    });
    pass.setPipeline(pipeline);
    pass.setBindGroup(0, bindGroup);
    pass.draw(3); // fullscreen triangle
    pass.end();
    device.queue.submit([encoder.finish()]);
  }

  /** Blit a texture to a render target (or screen if target is null). */
  blitTexture(
    src: TextureHandle | RenderTarget,
    target: RenderTarget | null,
  ): void {
    if (!this.blitPipeline || !this.blitBindGroupLayout) return;
    const device = this.device;
    const bindGroup = device.createBindGroup({
      layout: this.blitBindGroupLayout,
      entries: [
        { binding: 0, resource: src.view },
        { binding: 1, resource: src.sampler },
      ],
    });
    const targetFormat = target ? target.format : this._presentFormat;
    // Rebuild blit pipeline if format doesn't match
    if (this.blitPipeline.label !== targetFormat) {
      this.buildBlitPipeline(targetFormat);
    }
    this.renderPass(this.blitPipeline!, bindGroup, target);
  }

  /** Render to screen (presentation). */
  renderToScreen(src: TextureHandle | RenderTarget): void {
    this.blitTexture(src, null);
  }

  /** Clear a render target with a specific color. */
  clearTarget(target: RenderTarget, color?: readonly [number, number, number, number]): void {
    const device = this.device;
    const encoder = device.createCommandEncoder();
    const pass = encoder.beginRenderPass({
      colorAttachments: [{
        view: target.view,
        loadOp: 'clear',
        storeOp: 'store',
        clearValue: {
          r: color?.[0] ?? 0,
          g: color?.[1] ?? 0,
          b: color?.[2] ?? 0,
          a: color?.[3] ?? 0,
        },
      }],
    });
    pass.end();
    device.queue.submit([encoder.finish()]);
  }

  // -------------------------------------------------------------------------
  // Readback (GPU → CPU)
  // -------------------------------------------------------------------------

  /** Read a render target's pixels back to a canvas (for ONNX / preview). */
  async readTargetToCanvas(target: RenderTarget): Promise<HTMLCanvasElement> {
    const { width, height } = target;
    const device = this.device;
    const bytesPerRow = Math.ceil(width * 4 / 256) * 256; // align to 256
    const bufferSize = bytesPerRow * height;

    const readBuffer = device.createBuffer({
      size: bufferSize,
      usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ,
    });

    const encoder = device.createCommandEncoder();
    encoder.copyTextureToBuffer(
      { texture: target.texture },
      { buffer: readBuffer, bytesPerRow },
      { width, height },
    );
    device.queue.submit([encoder.finish()]);

    await readBuffer.mapAsync(GPUMapMode.READ);
    const data = new Uint8Array(readBuffer.getMappedRange());

    const canvas = document.createElement('canvas');
    canvas.width = width;
    canvas.height = height;
    const ctx = canvas.getContext('2d')!;
    const imageData = ctx.createImageData(width, height);

    // Copy with stride removal (bytesPerRow may include padding)
    for (let y = 0; y < height; y++) {
      const srcOffset = y * bytesPerRow;
      const dstOffset = y * width * 4;
      imageData.data.set(data.subarray(srcOffset, srcOffset + width * 4), dstOffset);
    }

    ctx.putImageData(imageData, 0, 0);
    readBuffer.unmap();
    readBuffer.destroy();

    return canvas;
  }

  /** Read a render target to a PNG data URL (for preview thumbnails). */
  async readTargetToDataURL(target: RenderTarget, maxDimension?: number): Promise<string> {
    // If downscaling needed, blit to a smaller target first
    if (maxDimension && (target.width > maxDimension || target.height > maxDimension)) {
      const scale = maxDimension / Math.max(target.width, target.height);
      const w = Math.max(1, Math.round(target.width * scale));
      const h = Math.max(1, Math.round(target.height * scale));
      const previewTarget = this.createTarget(`_preview_${w}x${h}`, w, h);
      this.blitTexture(target, previewTarget);
      const canvas = await this.readTargetToCanvas(previewTarget);
      return canvas.toDataURL('image/png');
    }
    const canvas = await this.readTargetToCanvas(target);
    return canvas.toDataURL('image/png');
  }

  // -------------------------------------------------------------------------
  // Pipeline creation
  // -------------------------------------------------------------------------

  /** Create a render pipeline from a WGSL fragment shader source. */
  createShaderPipeline(
    fragmentCode: string,
    bindGroupLayout: GPUBindGroupLayout,
    targetFormat: GPUTextureFormat = 'rgba8unorm',
    label?: string,
  ): GPURenderPipeline {
    const device = this.device;
    const vertModule = device.createShaderModule({ code: FULLSCREEN_VERT_WITH_UV });
    const fragModule = device.createShaderModule({ code: fragmentCode });

    const pipelineLayout = device.createPipelineLayout({
      bindGroupLayouts: [bindGroupLayout],
    });

    return device.createRenderPipeline({
      label: label ?? targetFormat,
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
  }

  // -------------------------------------------------------------------------
  // Lifecycle
  // -------------------------------------------------------------------------

  clearResources(): void {
    for (const t of this.targets.values()) t.texture.destroy();
    this.targets.clear();
    for (const t of this.imageTextures.values()) t.texture.destroy();
    this.imageTextures.clear();
    this.pipelineCache.clear();
  }

  dispose(): void {
    this.clearResources();
    this._device?.destroy();
    this._device = null;
    this._context = null;
  }

  // -------------------------------------------------------------------------
  // Internal
  // -------------------------------------------------------------------------

  private buildBlitPipeline(format?: GPUTextureFormat): void {
    const device = this.device;
    const targetFormat = format ?? this._presentFormat;

    this.blitBindGroupLayout = device.createBindGroupLayout({
      entries: [
        { binding: 0, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: 'float' } },
        { binding: 1, visibility: GPUShaderStage.FRAGMENT, sampler: { type: 'filtering' } },
      ],
    });

    this.blitPipeline = this.createShaderPipeline(
      BLIT_FRAG,
      this.blitBindGroupLayout,
      targetFormat,
      targetFormat,
    );
  }
}
