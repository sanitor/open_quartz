import * as THREE from 'three';

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

  setSize(width: number, height: number) {
    this.renderer.setSize(width, height);
  }

  createTarget(id: string, width: number, height: number) {
    const target = new THREE.WebGLRenderTarget(width, height, {
      minFilter: THREE.LinearFilter,
      magFilter: THREE.LinearFilter,
      format: THREE.RGBAFormat,
      type: THREE.UnsignedByteType,
    });
    this.targets.set(id, target);
    return target;
  }

  getTarget(id: string): THREE.WebGLRenderTarget | undefined {
    return this.targets.get(id);
  }

  loadImageTexture(id: string, dataUrl: string): Promise<THREE.Texture> {
    return new Promise((resolve) => {
      const img = new Image();
      img.onload = () => {
        const texture = new THREE.Texture(img);
        texture.needsUpdate = true;
        texture.minFilter = THREE.LinearMipmapLinearFilter;
        texture.magFilter = THREE.LinearFilter;
        this.imageTextures.set(id, texture);
        resolve(texture);
      };
      img.src = dataUrl;
    });
  }

  getImageTexture(id: string): THREE.Texture | undefined {
    return this.imageTextures.get(id);
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
    const pixels = new Uint8Array(w * h * 4);
    this.renderer.readRenderTargetPixels(target, 0, 0, w, h, pixels);

    const canvas = document.createElement('canvas');
    canvas.width = w;
    canvas.height = h;
    const ctx = canvas.getContext('2d')!;
    const imageData = ctx.createImageData(w, h);

    for (let y = 0; y < h; y++) {
      for (let x = 0; x < w; x++) {
        const srcIdx = (y * w + x) * 4;
        const dstIdx = ((h - 1 - y) * w + x) * 4;
        imageData.data[dstIdx] = pixels[srcIdx];
        imageData.data[dstIdx + 1] = pixels[srcIdx + 1];
        imageData.data[dstIdx + 2] = pixels[srcIdx + 2];
        imageData.data[dstIdx + 3] = pixels[srcIdx + 3];
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
