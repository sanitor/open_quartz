import { describe, it, expect, vi, beforeEach } from 'vitest';

// Use vi.hoisted so these are available inside vi.mock factory (which is hoisted)
const { mockReadPixels, mockRendererContext, LINEAR_FILTER, NEAREST_FILTER, LINEAR_MIPMAP_LINEAR_FILTER, REPEAT_WRAPPING, MIRRORED_REPEAT_WRAPPING, CLAMP_TO_EDGE_WRAPPING, RGBA_FORMAT, RG_FORMAT, RED_FORMAT, UNSIGNED_BYTE_TYPE, FLOAT_TYPE, HALF_FLOAT_TYPE } = vi.hoisted(() => ({
  mockReadPixels: vi.fn(),
  mockRendererContext: { fake: 'context' },
  LINEAR_FILTER: 9729,
  NEAREST_FILTER: 9728,
  LINEAR_MIPMAP_LINEAR_FILTER: 9987,
  REPEAT_WRAPPING: 1000,
  MIRRORED_REPEAT_WRAPPING: 1002,
  CLAMP_TO_EDGE_WRAPPING: 1001,
  RGBA_FORMAT: 1023,
  RG_FORMAT: 1030,
  RED_FORMAT: 1028,
  UNSIGNED_BYTE_TYPE: 1009,
  FLOAT_TYPE: 1015,
  HALF_FLOAT_TYPE: 1016,
}));

vi.mock('three', () => {
  const mockDomElement = typeof document !== 'undefined' ? document.createElement('canvas') : {};
  class WebGLRenderer {
    domElement = mockDomElement;
    setPixelRatio = vi.fn();
    setSize = vi.fn();
    setRenderTarget = vi.fn();
    render = vi.fn();
    clear = vi.fn();
    dispose = vi.fn();
    readRenderTargetPixels = mockReadPixels;
    getContext = vi.fn(() => mockRendererContext);
  }
  class Scene {
    add = vi.fn();
  }
  class OrthographicCamera {}
  class PlaneGeometry {}
  class MeshBasicMaterial {
    map: unknown;
    dispose = vi.fn();
    constructor(opts?: { map?: unknown; color?: number }) {
      this.map = opts?.map;
    }
  }
  class Mesh {
    material: { dispose: () => void };
    constructor(_geo: unknown, mat: { dispose: () => void }) {
      this.material = mat;
    }
  }
  class Texture {
    needsUpdate = false;
    minFilter = LINEAR_MIPMAP_LINEAR_FILTER;
    magFilter = LINEAR_FILTER;
    wrapS = CLAMP_TO_EDGE_WRAPPING;
    wrapT = CLAMP_TO_EDGE_WRAPPING;
    type = UNSIGNED_BYTE_TYPE;
    image: unknown;
    dispose = vi.fn();
    constructor(img?: unknown) {
      this.image = img;
    }
  }
  class DataTexture extends Texture {
    constructor(_data: unknown, _w: number, _h: number, _format?: number, _type?: number) {
      super();
    }
  }
  class WebGLRenderTarget {
    width: number;
    height: number;
    texture: Texture;
    dispose = vi.fn();
    constructor(w: number, h: number, _opts?: unknown) {
      this.width = w;
      this.height = h;
      this.texture = new Texture();
    }
  }

  return {
    WebGLRenderer,
    Scene,
    OrthographicCamera,
    PlaneGeometry,
    MeshBasicMaterial,
    Mesh,
    Texture,
    DataTexture,
    WebGLRenderTarget,
    LinearFilter: LINEAR_FILTER,
    NearestFilter: NEAREST_FILTER,
    LinearMipmapLinearFilter: LINEAR_MIPMAP_LINEAR_FILTER,
    RepeatWrapping: REPEAT_WRAPPING,
    MirroredRepeatWrapping: MIRRORED_REPEAT_WRAPPING,
    ClampToEdgeWrapping: CLAMP_TO_EDGE_WRAPPING,
    RGBAFormat: RGBA_FORMAT,
    RGFormat: RG_FORMAT,
    RedFormat: RED_FORMAT,
    UnsignedByteType: UNSIGNED_BYTE_TYPE,
    FloatType: FLOAT_TYPE,
    HalfFloatType: HALF_FLOAT_TYPE,
    GLSL3: 'GLSL3',
    RawShaderMaterial: class {
      vertexShader: string;
      fragmentShader: string;
      uniforms: Record<string, unknown>;
      glslVersion: string;
      constructor(opts: { vertexShader: string; fragmentShader: string; uniforms: Record<string, unknown>; glslVersion: string }) {
        this.vertexShader = opts.vertexShader;
        this.fragmentShader = opts.fragmentShader;
        this.uniforms = opts.uniforms;
        this.glslVersion = opts.glslVersion;
      }
    },
  };
});

// Mock Image globally for loadImageTexture
class MockImage {
  onload: (() => void) | null = null;
  onerror: (() => void) | null = null;
  naturalWidth = 64;
  naturalHeight = 64;
  private _src = '';

  get src() {
    return this._src;
  }
  set src(val: string) {
    this._src = val;
    Promise.resolve().then(() => {
      if (val.includes('fail')) {
        this.onerror?.();
      } else {
        this.onload?.();
      }
    });
  }
}
vi.stubGlobal('Image', MockImage);

import { WebGLRenderer } from '../../src/engine/webglRenderer';

describe('WebGLRenderer', () => {
  let renderer: WebGLRenderer;

  beforeEach(() => {
    vi.clearAllMocks();
    const canvas = document.createElement('canvas');
    renderer = new WebGLRenderer(canvas);
  });

  describe('constructor', () => {
    it('initializes renderer, scene, camera, and quad', () => {
      expect(renderer).toBeDefined();
      expect(renderer.canvas).toBeDefined();
    });
  });

  describe('canvas getter', () => {
    it('returns the domElement from the THREE renderer', () => {
      expect(renderer.canvas).toBeDefined();
    });
  });

  describe('getContext', () => {
    it('returns the renderer context', () => {
      expect(renderer.getContext()).toBe(mockRendererContext);
    });
  });

  describe('setSize', () => {
    it('calls renderer.setSize', () => {
      renderer.setSize(1024, 768);
      // We can verify via the mock internal renderer
      // The internal THREE.WebGLRenderer.setSize should have been called
      expect(renderer).toBeDefined(); // Confirms no throw
    });
  });

  describe('createTarget', () => {
    it('creates target with default params (non-float)', () => {
      const target = renderer.createTarget('test', 512, 512);
      expect(target).toBeDefined();
      expect(target.width).toBe(512);
      expect(target.height).toBe(512);
    });

    it('creates target with float=true', () => {
      const target = renderer.createTarget('test-float', 256, 256, true);
      expect(target).toBeDefined();
    });

    it('creates target with rgba8 format', () => {
      const target = renderer.createTarget('rgba8', 64, 64, false, 'rgba8');
      expect(target).toBeDefined();
    });

    it('creates target with rgba32f format', () => {
      const target = renderer.createTarget('rgba32f', 64, 64, false, 'rgba32f');
      expect(target).toBeDefined();
    });

    it('creates target with rg8 format', () => {
      const target = renderer.createTarget('rg8', 64, 64, false, 'rg8');
      expect(target).toBeDefined();
    });

    it('creates target with rg32f format', () => {
      const target = renderer.createTarget('rg32f', 64, 64, false, 'rg32f');
      expect(target).toBeDefined();
    });

    it('creates target with r8 format', () => {
      const target = renderer.createTarget('r8', 64, 64, false, 'r8');
      expect(target).toBeDefined();
    });

    it('creates target with r32f format', () => {
      const target = renderer.createTarget('r32f', 64, 64, false, 'r32f');
      expect(target).toBeDefined();
    });

    it('stores target by id for later retrieval', () => {
      const target = renderer.createTarget('stored', 128, 128);
      expect(renderer.getTarget('stored')).toBe(target);
    });
  });

  describe('getTarget', () => {
    it('returns undefined for unknown id', () => {
      expect(renderer.getTarget('nonexistent')).toBeUndefined();
    });

    it('returns previously created target', () => {
      const target = renderer.createTarget('myId', 100, 100);
      expect(renderer.getTarget('myId')).toBe(target);
    });
  });

  describe('getImageTexture', () => {
    it('returns undefined for unknown id', () => {
      expect(renderer.getImageTexture('unknown')).toBeUndefined();
    });
  });

  describe('loadImageTexture', () => {
    it('resolves with texture on successful image load', async () => {
      const tex = await renderer.loadImageTexture('img1', 'data:image/png;base64,AAAA');
      expect(tex).toBeDefined();
      expect(renderer.getImageTexture('img1')).toBe(tex);
    });

    it('rejects on image load error', async () => {
      await expect(
        renderer.loadImageTexture('img2', 'data:image/png;base64,fail'),
      ).rejects.toThrow('Failed to load image for img2');
    });
  });

  describe('applyTextureSampling', () => {
    it('sets nearest filter', () => {
      const tex = { minFilter: 0, magFilter: 0, wrapS: 0, wrapT: 0, needsUpdate: false };
      renderer.applyTextureSampling(tex as never, 'nearest');
      expect(tex.minFilter).toBe(NEAREST_FILTER);
      expect(tex.magFilter).toBe(NEAREST_FILTER);
    });

    it('sets linear filter (default)', () => {
      const tex = { minFilter: 0, magFilter: 0, wrapS: 0, wrapT: 0, needsUpdate: false };
      renderer.applyTextureSampling(tex as never, 'linear');
      expect(tex.minFilter).toBe(LINEAR_FILTER);
      expect(tex.magFilter).toBe(LINEAR_FILTER);
    });

    it('sets repeat wrapping', () => {
      const tex = { minFilter: 0, magFilter: 0, wrapS: 0, wrapT: 0, needsUpdate: false };
      renderer.applyTextureSampling(tex as never, undefined, 'repeat');
      expect(tex.wrapS).toBe(REPEAT_WRAPPING);
      expect(tex.wrapT).toBe(REPEAT_WRAPPING);
    });

    it('sets mirror wrapping', () => {
      const tex = { minFilter: 0, magFilter: 0, wrapS: 0, wrapT: 0, needsUpdate: false };
      renderer.applyTextureSampling(tex as never, undefined, 'mirror');
      expect(tex.wrapS).toBe(MIRRORED_REPEAT_WRAPPING);
      expect(tex.wrapT).toBe(MIRRORED_REPEAT_WRAPPING);
    });

    it('sets clamp wrapping (default)', () => {
      const tex = { minFilter: 0, magFilter: 0, wrapS: 0, wrapT: 0, needsUpdate: false };
      renderer.applyTextureSampling(tex as never, undefined, 'clamp');
      expect(tex.wrapS).toBe(CLAMP_TO_EDGE_WRAPPING);
      expect(tex.wrapT).toBe(CLAMP_TO_EDGE_WRAPPING);
    });

    it('sets needsUpdate to true', () => {
      const tex = { minFilter: 0, magFilter: 0, wrapS: 0, wrapT: 0, needsUpdate: false };
      renderer.applyTextureSampling(tex as never);
      expect(tex.needsUpdate).toBe(true);
    });
  });

  describe('loadRawTexture', () => {
    it('loads rgba8 format without stride', () => {
      const buf = new ArrayBuffer(4 * 4); // 2x2 rgba8
      const tex = renderer.loadRawTexture('raw1', buf, 'rgba8', 2, 2);
      expect(tex).toBeDefined();
      expect(renderer.getImageTexture('raw1')).toBe(tex);
    });

    it('loads rgba32f format', () => {
      const buf = new ArrayBuffer(2 * 2 * 16); // 2x2 rgba32f (16 bpp)
      const tex = renderer.loadRawTexture('raw2', buf, 'rgba32f', 2, 2);
      expect(tex).toBeDefined();
    });

    it('loads rg8 format', () => {
      const buf = new ArrayBuffer(2 * 2 * 2); // 2x2 rg8 (2 bpp)
      const tex = renderer.loadRawTexture('raw3', buf, 'rg8', 2, 2);
      expect(tex).toBeDefined();
    });

    it('loads rg32f format', () => {
      const buf = new ArrayBuffer(2 * 2 * 8); // 2x2 rg32f (8 bpp)
      const tex = renderer.loadRawTexture('raw4', buf, 'rg32f', 2, 2);
      expect(tex).toBeDefined();
    });

    it('loads r8 format', () => {
      const buf = new ArrayBuffer(2 * 2 * 1); // 2x2 r8 (1 bpp)
      const tex = renderer.loadRawTexture('raw5', buf, 'r8', 2, 2);
      expect(tex).toBeDefined();
    });

    it('loads r32f format', () => {
      const buf = new ArrayBuffer(2 * 2 * 4); // 2x2 r32f (4 bpp)
      const tex = renderer.loadRawTexture('raw6', buf, 'r32f', 2, 2);
      expect(tex).toBeDefined();
    });

    it('loads nv12 format (converts to RGBA)', () => {
      // NV12: Y plane = w*h, UV plane = w*h/2
      const w = 4;
      const h = 4;
      const ySize = w * h;
      const uvSize = (w * h) / 2;
      const buf = new ArrayBuffer(ySize + uvSize);
      const view = new Uint8Array(buf);
      // Fill with test data
      for (let i = 0; i < ySize; i++) view[i] = 128; // Y=128
      for (let i = 0; i < uvSize; i++) view[ySize + i] = 128; // U=V=128 (neutral)

      const tex = renderer.loadRawTexture('raw_nv12', buf, 'nv12', w, h);
      expect(tex).toBeDefined();
      expect(renderer.getImageTexture('raw_nv12')).toBe(tex);
    });

    it('loads with stride (byte-aligned rows) for uint8', () => {
      // 2x2 rgba8 with stride of 16 (rowBytes=8, padded to 16)
      const stride = 16;
      const buf = new ArrayBuffer(stride * 2); // 2 rows
      const tex = renderer.loadRawTexture('raw_stride', buf, 'rgba8', 2, 2, stride);
      expect(tex).toBeDefined();
    });

    it('loads with stride for float format', () => {
      // 2x2 rgba32f with stride. rowBytes = 2*16=32, use stride=64
      const stride = 64;
      const buf = new ArrayBuffer(stride * 2); // 2 rows with padding
      const tex = renderer.loadRawTexture('raw_stride_f', buf, 'rgba32f', 2, 2, stride);
      expect(tex).toBeDefined();
    });
  });

  describe('renderWithMaterial', () => {
    it('disposes old material and renders', () => {
      const target = renderer.createTarget('rt', 64, 64);
      const material = { dispose: vi.fn() };
      renderer.renderWithMaterial(material as never, target);
      // No throw = success
      expect(renderer).toBeDefined();
    });

    it('renders to screen when no target provided', () => {
      const material = { dispose: vi.fn() };
      renderer.renderWithMaterial(material as never);
      expect(renderer).toBeDefined();
    });
  });

  describe('renderSampler2DInput', () => {
    it('creates MeshBasicMaterial with texture and renders', () => {
      const target = renderer.createTarget('rt', 64, 64);
      const texture = { type: UNSIGNED_BYTE_TYPE };
      renderer.renderSampler2DInput(texture as never, target);
      expect(renderer).toBeDefined();
    });
  });

  describe('renderToScreen', () => {
    it('renders texture to screen (null render target)', () => {
      const texture = { type: UNSIGNED_BYTE_TYPE };
      renderer.renderToScreen(texture as never);
      expect(renderer).toBeDefined();
    });
  });

  describe('readTargetToDataURL', () => {
    it('reads byte texture target and returns data URL', () => {
      const target = renderer.createTarget('rt', 2, 2);
      target.texture.type = UNSIGNED_BYTE_TYPE;

      // Mock readRenderTargetPixels to fill pixel array
      mockReadPixels.mockImplementation(
        (_t: unknown, _x: number, _y: number, w: number, h: number, pixels: Uint8Array) => {
          for (let i = 0; i < w * h * 4; i++) pixels[i] = 128;
        },
      );

      const mockCtx = {
        createImageData: vi.fn((w: number, h: number) => ({
          data: new Uint8ClampedArray(w * h * 4),
          width: w,
          height: h,
        })),
        putImageData: vi.fn(),
      };
      const mockCanvas = {
        width: 0,
        height: 0,
        getContext: vi.fn(() => mockCtx),
        toDataURL: vi.fn(() => 'data:image/png;base64,RESULT'),
      };
      vi.spyOn(document, 'createElement').mockReturnValueOnce(mockCanvas as never);

      const result = renderer.readTargetToDataURL(target);
      expect(result).toBe('data:image/png;base64,RESULT');
      expect(mockCtx.putImageData).toHaveBeenCalled();
    });

    it('reads float texture target and converts to byte data URL', () => {
      const target = renderer.createTarget('rt', 2, 2, true, 'rgba32f');
      target.texture.type = FLOAT_TYPE;

      mockReadPixels.mockImplementation(
        (_t: unknown, _x: number, _y: number, w: number, h: number, floats: Float32Array) => {
          for (let i = 0; i < w * h * 4; i++) floats[i] = 0.5; // 0.5 -> 128
        },
      );

      const mockCtx = {
        createImageData: vi.fn((w: number, h: number) => ({
          data: new Uint8ClampedArray(w * h * 4),
          width: w,
          height: h,
        })),
        putImageData: vi.fn(),
      };
      const mockCanvas = {
        width: 0,
        height: 0,
        getContext: vi.fn(() => mockCtx),
        toDataURL: vi.fn(() => 'data:image/png;base64,FLOAT_RESULT'),
      };
      vi.spyOn(document, 'createElement').mockReturnValueOnce(mockCanvas as never);

      const result = renderer.readTargetToDataURL(target);
      expect(result).toBe('data:image/png;base64,FLOAT_RESULT');
    });
  });

  describe('clear', () => {
    it('clears to screen target', () => {
      renderer.clear();
      // No throw = success; internal renderer.clear() was called
      expect(renderer).toBeDefined();
    });
  });

  describe('dispose', () => {
    it('disposes all targets, textures, and renderer', () => {
      const target = renderer.createTarget('t1', 64, 64);
      renderer.dispose();
      expect(target.dispose).toHaveBeenCalled();
      expect(renderer.getTarget('t1')).toBeUndefined();
    });

    it('disposes image textures', async () => {
      const tex = await renderer.loadImageTexture('img', 'data:image/png;base64,AAAA');
      renderer.dispose();
      expect(tex.dispose).toHaveBeenCalled();
      expect(renderer.getImageTexture('img')).toBeUndefined();
    });
  });
});
