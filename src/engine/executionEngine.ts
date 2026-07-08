import * as THREE from 'three';
import type { Node, Edge } from '@xyflow/react';
import type { ShaderNodeData, FramebufferFormat, TextureFilter, TextureWrap } from '../types';
import type { FrameInputs } from './compositor';
import { WebGLRenderer } from './webglRenderer';
import { compileNodeShader, validateFragmentShader } from './shaderCompiler';
import { topologicalSort } from './graphExecutor';
import { ONNX_MODELS, DEFAULT_ONNX_MODEL_ID, type OnnxModelDescriptor } from './onnxRegistry';
import { OnnxSession, type OnnxDetection } from './onnxSession';
import { drawDetectionOverlay } from './onnxOverlay';

type TextureSource =
  | { kind: 'fbo'; target: THREE.WebGLRenderTarget }
  | { kind: 'image'; texture: THREE.Texture };

const BUILTIN_UNIFORMS = new Set(['iTime', 'iTimeDelta', 'iFrame', 'iDate', 'iMouse', 'iResolution']);

export interface ExecutionPlan {
  sortedIds: string[];
  nodeMap: Map<string, Node<ShaderNodeData>>;
  edges: Edge[];
  materials: Map<string, THREE.ShaderMaterial>;
  upstreamSamplerBindings: Map<string, Map<string, string>>;
  scalarBindings: Map<string, Map<string, unknown>>;
  selfUniforms: Map<string, Record<string, unknown>>;
  targets: Map<string, THREE.WebGLRenderTarget>;
  textureSources: Map<string, TextureSource>;
  outputNodes: string[];
  builtinPorts: Map<string, Set<string>>;
  preambleLines: Map<string, number>;
  defaultW: number;
  defaultH: number;
}

export class ExecutionEngine {
  private renderer: WebGLRenderer | null = null;
  private running = false;

  constructor(canvas: HTMLCanvasElement) {
    try {
      this.renderer = new WebGLRenderer(canvas);
    } catch (e) {
      console.error('Failed to create WebGL renderer:', e);
    }
  }

  isRunning(): boolean {
    return this.running;
  }

  prepare(
    nodes: Node<ShaderNodeData>[],
    edges: Edge[],
    onNodeError?: (nodeId: string, error: string) => void,
    onOutputSize?: (nodeId: string, width: number, height: number) => void,
  ): ExecutionPlan | null {
    if (!this.renderer) return null;

    const sortedIds = topologicalSort(nodes, edges);
    const nodeMap = new Map(nodes.map((n) => [n.id, n]));
    const textureSources = new Map<string, TextureSource>();
    const targets = new Map<string, THREE.WebGLRenderTarget>();
    const materials = new Map<string, THREE.ShaderMaterial>();
    const upstreamSamplerBindings = new Map<string, Map<string, string>>();
    const scalarBindings = new Map<string, Map<string, unknown>>();
    const selfUniforms = new Map<string, Record<string, unknown>>();
    const builtinPorts = new Map<string, Set<string>>();
    const preambleLines = new Map<string, number>();

    let defaultW = 512;
    let defaultH = 512;
    for (const node of nodes) {
      if (node.data.inputMode === 'framebuffer' && node.data.fbWidth && node.data.fbHeight) {
        defaultW = node.data.fbWidth;
        defaultH = node.data.fbHeight;
        break;
      }
      if (node.data.imageWidth && node.data.imageHeight) {
        defaultW = node.data.imageWidth;
        defaultH = node.data.imageHeight;
        break;
      }
    }

    let maxW = defaultW;
    let maxH = defaultH;
    for (const node of nodes) {
      if (node.data.type === 'shader' || node.data.type === 'constant') {
        const ow = node.data.autoSize === false ? ((node.data.width as number) || defaultW) : defaultW;
        const oh = node.data.autoSize === false ? ((node.data.height as number) || defaultH) : defaultH;
        maxW = Math.max(maxW, ow);
        maxH = Math.max(maxH, oh);
      } else if (node.data.type === 'renderer') {
        maxW = Math.max(maxW, node.data.rendererWidth ?? defaultW);
        maxH = Math.max(maxH, node.data.rendererHeight ?? defaultH);
      }
    }
    this.renderer.setSize(maxW, maxH);

    for (const nodeId of sortedIds) {
      const node = nodeMap.get(nodeId);
      if (!node) continue;

      if (node.data.type === 'input') {
        this.prepareInputTexture(node, textureSources, onNodeError);
        continue;
      }

      if (!isRenderableNode(node)) continue;

      const upstreamEdges = edges.filter((e) => e.target === nodeId);
      const upstreamMap = new Map<string, string>();
      const upstreamScalarValues = new Map<string, unknown>();
      const connectedPorts = new Set<string>();

      for (const edge of upstreamEdges) {
        const port = node.data.inputs.find((p) => p.id === edge.targetHandle);
        if (!port) continue;
        connectedPorts.add(port.label);
        upstreamMap.set(port.label, edge.source);
        if (port.dataType !== 'sampler2D' && port.dataType !== 'samplerCube') {
          const srcNode = nodeMap.get(edge.source);
          if (srcNode?.data.type === 'input') {
            const srcLabel = srcNode.data.inputs[0]?.label;
            const v = srcNode.data.uniforms?.[srcLabel ?? ''] ?? port.defaultValue;
            upstreamScalarValues.set(port.label, v);
          }
        }
      }

      const builtin = new Set<string>();
      const missing: string[] = [];
      for (const port of node.data.inputs) {
        if (connectedPorts.has(port.label)) continue;
        if (BUILTIN_UNIFORMS.has(port.label)) {
          builtin.add(port.label);
          continue;
        }
        if (port.dataType === 'sampler2D' || port.dataType === 'samplerCube') {
          missing.push(`'${port.label}'`);
        }
      }
      if (missing.length > 0 && node.data.type !== 'renderer') {
        onNodeError?.(nodeId, `Unconnected input${missing.length > 1 ? 's' : ''}: ${missing.join(', ')}`);
      }

      try {
        const outW = node.data.type === 'renderer'
          ? (node.data.rendererWidth ?? defaultW)
          : (node.data.autoSize === false ? ((node.data.width as number) || defaultW) : defaultW);
        const outH = node.data.type === 'renderer'
          ? (node.data.rendererHeight ?? defaultH)
          : (node.data.autoSize === false ? ((node.data.height as number) || defaultH) : defaultH);
        const outFormat = node.data.outFormat;
        const isFloat = outFormat === 'rgba32f' || outFormat === 'rg32f' || outFormat === 'r32f';
        const target = this.renderer.createTarget(nodeId, outW, outH, isFloat, outFormat);
        if (node.data.texFilter || node.data.texWrap) {
          this.renderer.applyTextureSampling(target.texture, node.data.texFilter, node.data.texWrap);
        }
        targets.set(nodeId, target);
        onOutputSize?.(nodeId, outW, outH);

        if (node.data.type === 'renderer') {
          upstreamSamplerBindings.set(nodeId, upstreamMap);
          continue;
        }

        const compiled = compileNodeShader(node.data.shaderCode, node.data.inputs, upstreamMap);
        const material = compiled.material;
        const gl = this.renderer.getContext();
        const err = validateFragmentShader(gl, material.fragmentShader);
        if (err) throw new Error(err);

        materials.set(nodeId, material);
        upstreamSamplerBindings.set(nodeId, compiled.upstreamSamplers);
        scalarBindings.set(nodeId, upstreamScalarValues);
        selfUniforms.set(nodeId, node.data.uniforms);
        builtinPorts.set(nodeId, builtin);
        preambleLines.set(nodeId, compiled.preambleLines);
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        const formatted = formatShaderError(msg, preambleLines.get(nodeId) ?? 0);
        console.warn(`Shader error for node ${nodeId}:`, formatted);
        onNodeError?.(nodeId, formatted);
      }
    }

    const rendererNodes = nodes.filter((n) => n.data.type === 'renderer').map((n) => n.id);
    const outputNodes = rendererNodes.length > 0
      ? rendererNodes
      : nodes
        .filter((n) => isRenderableNode(n) && !edges.some((e) => e.source === n.id))
        .map((n) => n.id);

    return {
      sortedIds,
      nodeMap,
      edges,
      materials,
      upstreamSamplerBindings,
      scalarBindings,
      selfUniforms,
      targets,
      textureSources,
      outputNodes,
      builtinPorts,
      preambleLines,
      defaultW,
      defaultH,
    };
  }

  runFrame(plan: ExecutionPlan, builtins: FrameInputs): void {
    if (!this.renderer) return;

    for (const nodeId of plan.sortedIds) {
      const node = plan.nodeMap.get(nodeId);
      if (!node || !isRenderableNode(node)) continue;

      const target = plan.targets.get(nodeId);
      if (!target) continue;

      if (node.data.type === 'renderer') {
        const rendererInput = plan.upstreamSamplerBindings.get(nodeId)?.values().next().value;
        if (!rendererInput) continue;
        const videoTex = builtins.videoTextures?.get(rendererInput);
        const src = plan.textureSources.get(rendererInput);
        const tex = videoTex ?? (src?.kind === 'fbo' ? src.target.texture : src?.texture);
        if (tex) {
          this.renderer.renderSampler2DInput(tex, target);
          plan.textureSources.set(nodeId, { kind: 'fbo', target });
        }
        continue;
      }

      const material = plan.materials.get(nodeId);
      if (!material) continue;

      const upstreamSamplers = plan.upstreamSamplerBindings.get(nodeId);
      if (upstreamSamplers) {
        for (const [uniformName, sourceNodeId] of upstreamSamplers) {
          const videoTex = builtins.videoTextures?.get(sourceNodeId);
          const src = plan.textureSources.get(sourceNodeId);
          let tex: THREE.Texture | undefined = videoTex;
          if (!tex && src?.kind === 'fbo') tex = src.target.texture;
          if (!tex && src?.kind === 'image') tex = src.texture;
          if (tex) setUniform(material, uniformName, tex);
        }
      }

      const scalars = plan.scalarBindings.get(nodeId);
      if (scalars) {
        for (const [key, val] of scalars) setUniform(material, key, normalizeUniformValue(val));
      }

      const self = plan.selfUniforms.get(nodeId);
      if (self) {
        const upstreamMap = plan.upstreamSamplerBindings.get(nodeId);
        for (const [key, val] of Object.entries(self)) {
          if (!upstreamMap?.has(key)) setUniform(material, key, normalizeUniformValue(val));
        }
      }

      const builtin = plan.builtinPorts.get(nodeId);
      if (builtin) {
        if (builtin.has('iTime')) setUniform(material, 'iTime', builtins.time);
        if (builtin.has('iTimeDelta')) setUniform(material, 'iTimeDelta', builtins.delta);
        if (builtin.has('iFrame')) setUniform(material, 'iFrame', builtins.frame);
        if (builtin.has('iDate')) setUniform(material, 'iDate', builtins.date);
        if (builtin.has('iMouse')) setUniform(material, 'iMouse', builtins.mouse);
        if (builtin.has('iResolution')) setUniform(material, 'iResolution', builtins.resolution);
      }

      this.renderer.renderWithMaterial(material, target);
      plan.textureSources.set(nodeId, { kind: 'fbo', target });
    }
  }

  readOutputs(plan: ExecutionPlan, onOutput: (nodeId: string, dataUrl: string) => void): void {
    if (!this.renderer) return;
    for (const nodeId of plan.outputNodes) {
      const target = plan.targets.get(nodeId);
      if (!target) continue;
      onOutput(nodeId, this.renderer.readTargetToDataURL(target));
    }
  }

  async run(
    nodes: Node<ShaderNodeData>[],
    edges: Edge[],
    onOutput?: (nodeId: string, dataUrl: string) => void,
    onNodeError?: (nodeId: string, error: string) => void,
    onOutputSize?: (nodeId: string, width: number, height: number) => void,
    onOutputData?: (nodeId: string, data: unknown) => void,
  ) {
    if (!this.renderer) return;
    this.running = true;

    const order = topologicalSort(nodes, edges);
    const nodeMap = new Map(nodes.map((n) => [n.id, n]));
    const textures = new Map<string, TextureSource>();

    let defaultW = 512;
    let defaultH = 512;
    for (const node of nodes) {
      if (node.data.imageWidth && node.data.imageHeight) {
        defaultW = node.data.imageWidth;
        defaultH = node.data.imageHeight;
        break;
      }
      if (node.data.inputMode === 'video' && node.data.videoUrl) {
        const videoSize = await new Promise<{ width: number; height: number } | null>((resolve) => {
          const video = document.createElement('video');
          video.preload = 'metadata';
          video.onloadedmetadata = () => resolve({ width: video.videoWidth, height: video.videoHeight });
          video.onerror = () => resolve(null);
          video.src = node.data.videoUrl!;
        });
        if (videoSize) {
          defaultW = videoSize.width;
          defaultH = videoSize.height;
          break;
        }
      }
      if (node.data.inputDataType === 'sampler2D' && node.data.imageDataUrl) {
        const img = await new Promise<HTMLImageElement>((resolve, reject) => {
          const i = new Image();
          i.onload = () => resolve(i);
          i.onerror = () => reject(new Error(`Failed to load image for node ${node.id}`));
          i.src = node.data.imageDataUrl!;
        }).catch(() => null);
        if (!img) break;
        defaultW = img.naturalWidth;
        defaultH = img.naturalHeight;
        break;
      }
      if (node.data.inputMode === 'framebuffer' && node.data.rawDataUrl && node.data.fbWidth && node.data.fbHeight) {
        defaultW = node.data.fbWidth;
        defaultH = node.data.fbHeight;
        break;
      }
    }

    let maxW = defaultW;
    let maxH = defaultH;
    for (const node of nodes) {
      if (node.data.type === 'shader' || node.data.type === 'constant') {
        const ow = node.data.autoSize === false ? ((node.data.width as number) || defaultW) : defaultW;
        const oh = node.data.autoSize === false ? ((node.data.height as number) || defaultH) : defaultH;
        if (ow > maxW) maxW = ow;
        if (oh > maxH) maxH = oh;
      }
    }

    this.renderer.setSize(maxW, maxH);

    for (const nodeId of order) {
      if (!this.running) break;
      const node = nodeMap.get(nodeId);
      if (!node) continue;

      const upstreamEdges = edges.filter((e) => e.target === nodeId);

      if (node.data.type === 'input') {
        if (node.data.inputMode === 'framebuffer' && node.data.rawDataUrl && node.data.fbWidth && node.data.fbHeight) {
          try {
            const b64 = node.data.rawDataUrl.split(',')[1];
            const binary = atob(b64);
            const buf = new ArrayBuffer(binary.length);
            const view = new Uint8Array(buf);
            for (let i = 0; i < binary.length; i++) view[i] = binary.charCodeAt(i);

            const tex = this.renderer.loadRawTexture(
              nodeId, buf,
              (node.data.fbFormat ?? 'rgba8') as FramebufferFormat,
              node.data.fbWidth, node.data.fbHeight,
              node.data.fbStride,
            );
            this.renderer.applyTextureSampling(tex, node.data.texFilter as TextureFilter, node.data.texWrap as TextureWrap);
            const target = this.renderer.createTarget(`raw_${nodeId}`, node.data.fbWidth, node.data.fbHeight, true);
            this.renderer.renderSampler2DInput(tex, target);
            textures.set(nodeId, { kind: 'fbo', target });
            onOutput?.(nodeId, this.renderer.readTargetToDataURL(target));
          } catch (e) {
            const msg = e instanceof Error ? e.message : String(e);
            console.warn(`Raw texture error for node ${nodeId}:`, msg);
            onNodeError?.(nodeId, msg);
          }
        } else if (node.data.inputMode === 'video' && node.data.videoUrl) {
          try {
            const video = document.createElement('video');
            video.muted = true;
            video.playsInline = true;
            video.preload = 'auto';
            video.src = node.data.videoUrl;
            await new Promise<void>((resolve, reject) => {
              const onReady = () => {
                cleanup();
                resolve();
              };
              const onError = () => {
                cleanup();
                reject(new Error(`Failed to load video for node ${nodeId}`));
              };
              const cleanup = () => {
                video.removeEventListener('loadeddata', onReady);
                video.removeEventListener('error', onError);
              };
              video.addEventListener('loadeddata', onReady);
              video.addEventListener('error', onError);
            });
            const tex = new THREE.VideoTexture(video);
            tex.minFilter = THREE.LinearFilter;
            tex.magFilter = THREE.LinearFilter;
            tex.generateMipmaps = false;
            textures.set(nodeId, { kind: 'image', texture: tex });
            if (video.videoWidth > 0 && video.videoHeight > 0) onOutputSize?.(nodeId, video.videoWidth, video.videoHeight);
          } catch (e) {
            const msg = e instanceof Error ? e.message : String(e);
            console.warn(`Video load error for node ${nodeId}:`, msg);
            onNodeError?.(nodeId, msg);
          }
        } else if (node.data.inputDataType === 'sampler2D' && node.data.imageDataUrl) {
          try {
            const tex = await this.renderer.loadImageTexture(nodeId, node.data.imageDataUrl);
            this.renderer.applyTextureSampling(tex, node.data.texFilter as TextureFilter, node.data.texWrap as TextureWrap);
            textures.set(nodeId, { kind: 'image', texture: tex });
            onOutput?.(nodeId, node.data.imageDataUrl);
          } catch (e) {
            const msg = e instanceof Error ? e.message : String(e);
            console.warn(`Image load error for node ${nodeId}:`, msg);
            onNodeError?.(nodeId, msg);
          }
        }
        continue;
      }

      if (node.data.type === 'shader' || node.data.type === 'constant' || node.data.type === 'renderer') {
        const upstreamMap = new Map<string, string>();
        const upstreamScalarValues = new Map<string, unknown>();
        for (const edge of upstreamEdges) {
          const port = node.data.inputs.find((p) => p.id === edge.targetHandle);
          if (port) {
            upstreamMap.set(port.label, edge.source);
            if (port.dataType !== 'sampler2D') {
              const srcNode = nodeMap.get(edge.source);
              if (srcNode?.data.type === 'input') {
                const srcLabel = srcNode.data.inputs[0]?.label;
                const v = srcNode.data.uniforms?.[srcLabel ?? ''] ?? port.defaultValue;
                upstreamScalarValues.set(port.label, v);
              }
            }
          }
        }

        let material: THREE.ShaderMaterial;
        let upstreamSamplers: Map<string, string>;
        let preambleLines = 0;
        try {
          const compiled = compileNodeShader(node.data.shaderCode, node.data.inputs, upstreamMap);
          material = compiled.material;
          upstreamSamplers = compiled.upstreamSamplers;
          preambleLines = compiled.preambleLines;

          const gl = this.renderer!.getContext();
          const fragSrc = material.fragmentShader as string;
          if (fragSrc) {
            const err = validateFragmentShader(gl, fragSrc);
            if (err) throw new Error(err);
          }

          for (const [uniformName, sourceNodeId] of upstreamSamplers) {
            const src = textures.get(sourceNodeId);
            let tex: THREE.Texture | undefined;
            if (src?.kind === 'fbo') tex = src.target.texture;
            else if (src?.kind === 'image') tex = src.texture;
            if (tex) material.uniforms[uniformName] = { value: tex };
          }

          for (const [key, val] of upstreamScalarValues) {
            material.uniforms[key] = { value: normalizeUniformValue(val) };
          }

          for (const [key, val] of Object.entries(node.data.uniforms)) {
            if (!upstreamMap.has(key)) material.uniforms[key] = { value: normalizeUniformValue(val) };
          }

          const outW = node.data.type === 'renderer' ? (node.data.rendererWidth ?? defaultW) : (node.data.autoSize === false ? ((node.data.width as number) || defaultW) : defaultW);
          const outH = node.data.type === 'renderer' ? (node.data.rendererHeight ?? defaultH) : (node.data.autoSize === false ? ((node.data.height as number) || defaultH) : defaultH);
          const outFormat = node.data.outFormat;
          const isFloat = outFormat === 'rgba32f' || outFormat === 'rg32f' || outFormat === 'r32f';
          const target = this.renderer.createTarget(nodeId, outW, outH, isFloat, outFormat);
          if (node.data.texFilter || node.data.texWrap) {
            this.renderer.applyTextureSampling(target.texture, node.data.texFilter, node.data.texWrap);
          }
          this.renderer.renderWithMaterial(material, target);
          textures.set(nodeId, { kind: 'fbo', target });

          const dataUrl = this.renderer.readTargetToDataURL(target);
          onOutput?.(nodeId, dataUrl);
          onOutputSize?.(nodeId, outW, outH);
        } catch (e) {
          const msg = e instanceof Error ? e.message : String(e);
          const formatted = formatShaderError(msg, preambleLines);
          console.warn(`Shader error for node ${nodeId}:`, formatted);
          onNodeError?.(nodeId, formatted);
          continue;
        }
      }
      if (node.data.type === 'onnx') {
        try {
          const modelId = node.data.onnxModelId ?? DEFAULT_ONNX_MODEL_ID;
          const descriptor = ONNX_MODELS[modelId];
          if (!descriptor) throw new Error(`Unknown ONNX model: ${modelId}`);

          const imagePort = node.data.inputs.find((p) => p.dataType === 'sampler2D');
          if (!imagePort) throw new Error(`ONNX node missing sampler2D input`);
          const upstreamEdge = upstreamEdges.find((e) => e.targetHandle === imagePort.id);
          if (!upstreamEdge) throw new Error(`ONNX input '${imagePort.label}' not connected`);

          const source = textures.get(upstreamEdge.source);
          if (!source) throw new Error(`ONNX upstream '${upstreamEdge.source}' produced no texture`);

          // Source dimensions: FBO knows its own size; image texture reads the underlying HTMLImageElement.
          let srcW: number;
          let srcH: number;
          if (source.kind === 'fbo') {
            srcW = source.target.width;
            srcH = source.target.height;
          } else {
            const img: unknown = source.texture.image;
            if (img instanceof HTMLImageElement) {
              srcW = img.naturalWidth || defaultW;
              srcH = img.naturalHeight || defaultH;
            } else {
              srcW = defaultW;
              srcH = defaultH;
            }
          }
          // Render the upstream source into a scratch RGBA8 FBO we can read back.
          const scratchId = `onnx_src_${nodeId}`;
          const scratchTarget = this.renderer.createTarget(scratchId, srcW, srcH, false, 'rgba8');
          if (source.kind === 'fbo') {
            this.renderer.renderSampler2DInput(source.target.texture, scratchTarget);
          } else {
            this.renderer.renderSampler2DInput(source.texture, scratchTarget);
          }
          const sourceCanvas = this.renderer.readTargetToCanvas(scratchTarget);

          // Lazy session cache on the engine instance.
          const session = await this.getOnnxSession(descriptor);
          if (node.data.onnxScoreThreshold !== undefined && node.data.onnxIouThreshold !== undefined) {
            session.setThresholds(node.data.onnxScoreThreshold, node.data.onnxIouThreshold);
          }

          const result = await session.run(sourceCanvas, srcW, srcH);
          const detections: OnnxDetection[] = result.detections;

          // Overlay canvas → CanvasTexture registered for downstream sampler2D consumers.
          const overlay = drawDetectionOverlay(sourceCanvas, srcW, srcH, detections);
          textures.set(nodeId, { kind: 'image', texture: overlay.texture });

          onOutput?.(nodeId, overlay.dataUrl);
          onOutputSize?.(nodeId, srcW, srcH);
          onOutputData?.(nodeId, { detections });
        } catch (e) {
          const msg = e instanceof Error ? e.message : String(e);
          console.warn(`ONNX error for node ${nodeId}:`, msg);
          onNodeError?.(nodeId, msg);
        }
        continue;
      }


    }
  }

  private onnxSessions = new Map<string, OnnxSession>();

  private async getOnnxSession(descriptor: OnnxModelDescriptor): Promise<OnnxSession> {
    const existing = this.onnxSessions.get(descriptor.id);
    if (existing) {
      if (existing.status !== 'ready') await existing.init();
      return existing;
    }
    const session = new OnnxSession(descriptor);
    this.onnxSessions.set(descriptor.id, session);
    await session.init();
    return session;
  }

  stop() {
    this.running = false;
    for (const s of this.onnxSessions.values()) {
      try { s.dispose(); } catch { /* ignore */ }
    }
    this.onnxSessions.clear();
    try { this.renderer?.dispose(); } catch { /* ignore */ }
    this.renderer = null;
  }

  private prepareInputTexture(
    node: Node<ShaderNodeData>,
    textureSources: Map<string, TextureSource>,
    onNodeError?: (nodeId: string, error: string) => void,
  ): void {
    if (!this.renderer) return;
    if (node.data.inputMode === 'framebuffer' && node.data.rawDataUrl && node.data.fbWidth && node.data.fbHeight) {
      try {
        const b64 = node.data.rawDataUrl.split(',')[1];
        const binary = atob(b64);
        const buf = new ArrayBuffer(binary.length);
        const view = new Uint8Array(buf);
        for (let i = 0; i < binary.length; i++) view[i] = binary.charCodeAt(i);
        const tex = this.renderer.loadRawTexture(
          node.id,
          buf,
          (node.data.fbFormat ?? 'rgba8') as FramebufferFormat,
          node.data.fbWidth,
          node.data.fbHeight,
          node.data.fbStride,
        );
        this.renderer.applyTextureSampling(tex, node.data.texFilter, node.data.texWrap);
        const target = this.renderer.createTarget(`raw_${node.id}`, node.data.fbWidth, node.data.fbHeight, true);
        this.renderer.renderSampler2DInput(tex, target);
        textureSources.set(node.id, { kind: 'fbo', target });
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        onNodeError?.(node.id, msg);
      }
      return;
    }

    const cached = this.renderer.getImageTexture(node.id);
    if (cached) {
      textureSources.set(node.id, { kind: 'image', texture: cached });
      return;
    }
    if (node.data.inputDataType === 'sampler2D' && node.data.imageDataUrl) {
      this.renderer.loadImageTexture(node.id, node.data.imageDataUrl)
        .then((tex) => {
          this.renderer?.applyTextureSampling(tex, node.data.texFilter, node.data.texWrap);
          textureSources.set(node.id, { kind: 'image', texture: tex });
        })
        .catch((e: unknown) => {
          const msg = e instanceof Error ? e.message : String(e);
          onNodeError?.(node.id, msg);
        });
    }
  }
}

function isRenderableNode(node: Node<ShaderNodeData>): boolean {
  return node.data.type === 'shader' || node.data.type === 'constant' || node.data.type === 'renderer';
}

function setUniform(material: THREE.ShaderMaterial, key: string, value: unknown): void {
  const uniform = material.uniforms[key];
  if (uniform) {
    uniform.value = value;
  } else {
    material.uniforms[key] = { value };
  }
}

function normalizeUniformValue(value: unknown): unknown {
  if (typeof value === 'number' || typeof value === 'boolean') return value;
  if (typeof value === 'string') {
    const num = Number(value);
    return Number.isNaN(num) ? value : num;
  }
  return value;
}

function formatShaderError(msg: string, preambleLines: number): string {
  const lines = msg.split('\n');
  const relevant = lines.filter(
    (l) => l.includes('ERROR:') || l.includes('WARNING:')
  );
  const result = relevant.length > 0 ? relevant : lines.filter(
    (l) => l.includes('Shader Error') || l.includes('getProgramInfoLog')
  );
  if (result.length === 0) return msg;
  if (preambleLines <= 0) return result.join('\n');
  return result.map((line) =>
    line.replace(/(\d+):(\d+):/g, (_match, strNum, lineNum) => {
      const adjusted = parseInt(lineNum, 10) - preambleLines;
      return `${strNum}:${adjusted > 0 ? adjusted : 1}:`;
    })
  ).join('\n');
}
