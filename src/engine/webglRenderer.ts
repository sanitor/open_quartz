import * as THREE from 'three';
import type { FramebufferFormat, TextureFilter, TextureWrap } from '../types';

function convertNV12toRGBA(data: Uint8Array, width: number, height: number): Uint8Array {
  const ySize = width * height;
  const rgba = new Uint8Array(width * height * 4);
  for (let j = 0; j < height; j++) {
    for (let i = 0; i < width; i++) {
      const yIdx = j * width + i;
      const uvIdx = ySize + Math.floor(j / 2) * width + (i & ~1);
      const y = data[yIdx];
      const u = data[uvIdx] - 128;
      const v = data[uvIdx + 1] - 128;
      const outIdx = yIdx * 4;
      rgba[outIdx]     = Math.max(0, Math.min(255, y + 1.402 * v));
      rgba[outIdx + 1] = Math.max(0, Math.min(255, y - 0.344 * u - 0.714 * v));
      rgba[outIdx + 2] = Math.max(0, Math.min(255, y + 1.772 * u));
      rgba[outIdx + 3] = 255;
    }
  }
  return rgba;
}

export class WebGLRenderer {
  private renderer: THREE.WebGLRenderer;
  private scene: THREE.Scene;
  private camera: THREE.OrthographicCamera;
  private quad: THREE.Mesh<THREE.PlaneGeometry, THREE.Material>;
  private targets = new Map<string, THREE.WebGLRenderTarget>();
  private imageTextures = new Map<string, THREE.Texture>();

  constructor(canvas: HTMLCanvasElement) {
    this.renderer = new THREE.WebGLRenderer({ canvas, alpha: false });
    this.renderer.setPixelRatio(Math.min(devicePixelRatio, 2));

    this.scene = new THREE.Scene();
    this.camera = new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 1);

    const geo = new THREE.PlaneGeometry(2, 2);
    const mat = new THREE.MeshBasicMaterial({ color: 0x000000 });
    this.quad = new THREE.Mesh(geo, mat);
    this.scene.add(this.quad);
  }

  get canvas(): HTMLCanvasElement {
    return this.renderer.domElement;
  }

  getContext() {
    return this.renderer.getContext();
  }

  setSize(width: number, height: number) {
    this.renderer.setSize(width, height);
  }

  createTarget(id: string, width: number, height: number, float = false) {
    const target = new THREE.WebGLRenderTarget(width, height, {
      minFilter: THREE.LinearFilter,
      magFilter: THREE.LinearFilter,
      format: THREE.RGBAFormat,
      type: float ? THREE.HalfFloatType : THREE.UnsignedByteType,
    });
    this.targets.set(id, target);
    return target;
  }

  getTarget(id: string): THREE.WebGLRenderTarget | undefined {
    return this.targets.get(id);
  }

  loadImageTexture(id: string, dataUrl: string): Promise<THREE.Texture> {
    return new Promise((resolve, reject) => {
      const img = new Image();
      img.onload = () => {
        const texture = new THREE.Texture(img);
        texture.needsUpdate = true;
        texture.minFilter = THREE.LinearMipmapLinearFilter;
        texture.magFilter = THREE.LinearFilter;
        this.imageTextures.set(id, texture);
        resolve(texture);
      };
      img.onerror = () => reject(new Error(`Failed to load image for ${id}`));
      img.src = dataUrl;
    });
  }

  getImageTexture(id: string): THREE.Texture | undefined {
    return this.imageTextures.get(id);
  }

  applyTextureSampling(tex: THREE.Texture, filter?: TextureFilter, wrap?: TextureWrap) {
    if (filter === 'nearest') {
      tex.minFilter = THREE.NearestFilter;
      tex.magFilter = THREE.NearestFilter;
    } else {
      tex.minFilter = THREE.LinearFilter;
      tex.magFilter = THREE.LinearFilter;
    }
    const wrapMode = wrap === 'repeat' ? THREE.RepeatWrapping
      : wrap === 'mirror' ? THREE.MirroredRepeatWrapping
      : THREE.ClampToEdgeWrapping;
    tex.wrapS = wrapMode;
    tex.wrapT = wrapMode;
    tex.needsUpdate = true;
  }

  loadRawTexture(
    id: string,
    buffer: ArrayBuffer,
    format: FramebufferFormat,
    width: number,
    height: number,
    stride?: number,
  ): THREE.DataTexture {
    let data: ArrayBufferView;
    let threeFormat: THREE.PixelFormat;
    let threeType: THREE.TextureDataType;
    let bpp: number;

    switch (format) {
      case 'rgba8':
        bpp = 4; threeFormat = THREE.RGBAFormat; threeType = THREE.UnsignedByteType;
        break;
      case 'rgba32f':
        bpp = 16; threeFormat = THREE.RGBAFormat; threeType = THREE.FloatType;
        break;
      case 'rg8':
        bpp = 2; threeFormat = THREE.RGFormat; threeType = THREE.UnsignedByteType;
        break;
      case 'rg32f':
        bpp = 8; threeFormat = THREE.RGFormat; threeType = THREE.FloatType;
        break;
      case 'r8':
        bpp = 1; threeFormat = THREE.RedFormat; threeType = THREE.UnsignedByteType;
        break;
      case 'r32f':
        bpp = 4; threeFormat = THREE.RedFormat; threeType = THREE.FloatType;
        break;
      case 'nv12': {
        const rgba = convertNV12toRGBA(new Uint8Array(buffer), width, height);
        const tex = new THREE.DataTexture(rgba, width, height, THREE.RGBAFormat, THREE.UnsignedByteType);
        tex.needsUpdate = true;
        this.imageTextures.set(id, tex);
        return tex;
      }
    }

    const rowBytes = width * bpp;
    const strideBytes = stride ?? rowBytes;

    if (strideBytes === rowBytes) {
      if (threeType === THREE.FloatType) {
        data = new Float32Array(buffer, 0, width * height * (bpp / 4));
      } else {
        data = new Uint8Array(buffer, 0, width * height * bpp);
      }
    } else {
      if (threeType === THREE.FloatType) {
        const out = new Float32Array(width * height * (bpp / 4));
        const src = new Uint8Array(buffer);
        const rowFloats = width * (bpp / 4);
        for (let y = 0; y < height; y++) {
          const srcRow = new Float32Array(src.buffer, y * strideBytes, rowFloats);
          out.set(srcRow, y * rowFloats);
        }
        data = out;
      } else {
        const out = new Uint8Array(width * height * bpp);
        const src = new Uint8Array(buffer);
        for (let y = 0; y < height; y++) {
          out.set(src.subarray(y * strideBytes, y * strideBytes + rowBytes), y * rowBytes);
        }
        data = out;
      }
    }

    const tex = new THREE.DataTexture(data, width, height, threeFormat, threeType);
    tex.needsUpdate = true;
    this.imageTextures.set(id, tex);
    return tex;
  }

  renderWithMaterial(
    material: THREE.Material,
    target?: THREE.WebGLRenderTarget,
  ) {
    this.quad.material.dispose();
    this.quad.material = material;
    this.renderer.setRenderTarget(target ?? null);
    this.renderer.render(this.scene, this.camera);
  }

  renderSampler2DInput(texture: THREE.Texture, target?: THREE.WebGLRenderTarget) {
    const mat = new THREE.MeshBasicMaterial({ map: texture });
    this.renderWithMaterial(mat, target);
  }

  renderToScreen(texture: THREE.Texture) {
    const mat = new THREE.MeshBasicMaterial({ map: texture });
    this.quad.material.dispose();
    this.quad.material = mat;
    this.renderer.setRenderTarget(null);
    this.renderer.render(this.scene, this.camera);
  }

  readTargetToDataURL(target: THREE.WebGLRenderTarget): string {
    const w = target.width;
    const h = target.height;
    const isFloat = target.texture.type !== THREE.UnsignedByteType;

    const canvas = document.createElement('canvas');
    canvas.width = w;
    canvas.height = h;
    const ctx = canvas.getContext('2d')!;
    const imageData = ctx.createImageData(w, h);

    if (isFloat) {
      const floats = new Float32Array(w * h * 4);
      this.renderer.readRenderTargetPixels(target, 0, 0, w, h, floats);
      for (let y = 0; y < h; y++) {
        for (let x = 0; x < w; x++) {
          const srcIdx = (y * w + x) * 4;
          const dstIdx = ((h - 1 - y) * w + x) * 4;
          imageData.data[dstIdx]     = Math.max(0, Math.min(255, Math.round(floats[srcIdx] * 255)));
          imageData.data[dstIdx + 1] = Math.max(0, Math.min(255, Math.round(floats[srcIdx + 1] * 255)));
          imageData.data[dstIdx + 2] = Math.max(0, Math.min(255, Math.round(floats[srcIdx + 2] * 255)));
          imageData.data[dstIdx + 3] = Math.max(0, Math.min(255, Math.round(floats[srcIdx + 3] * 255)));
        }
      }
    } else {
      const pixels = new Uint8Array(w * h * 4);
      this.renderer.readRenderTargetPixels(target, 0, 0, w, h, pixels);
      for (let y = 0; y < h; y++) {
        for (let x = 0; x < w; x++) {
          const srcIdx = (y * w + x) * 4;
          const dstIdx = ((h - 1 - y) * w + x) * 4;
          imageData.data[dstIdx]     = pixels[srcIdx];
          imageData.data[dstIdx + 1] = pixels[srcIdx + 1];
          imageData.data[dstIdx + 2] = pixels[srcIdx + 2];
          imageData.data[dstIdx + 3] = pixels[srcIdx + 3];
        }
      }
    }

    ctx.putImageData(imageData, 0, 0);
    return canvas.toDataURL('image/png');
  }

  clear() {
    this.renderer.setRenderTarget(null);
    this.renderer.clear();
  }

  dispose() {
    for (const t of this.targets.values()) t.dispose();
    for (const t of this.imageTextures.values()) t.dispose();
    this.targets.clear();
    this.imageTextures.clear();
    this.renderer.dispose();
  }
}
