import * as THREE from 'three';
import type { Node, Edge } from '@xyflow/react';
import type { ShaderNodeData, FramebufferFormat, TextureFilter, TextureWrap } from '../types';
import { WebGLRenderer } from './webglRenderer';
import { compileNodeShader, validateFragmentShader } from './shaderCompiler';
import { topologicalSort } from './graphExecutor';
import { ONNX_MODELS, DEFAULT_ONNX_MODEL_ID, type OnnxModelDescriptor } from './onnxRegistry';
import { OnnxSession, type OnnxDetection } from './onnxSession';
import { drawDetectionOverlay } from './onnxOverlay';

type TextureSource =
  | { kind: 'fbo'; target: THREE.WebGLRenderTarget }
  | { kind: 'image'; texture: THREE.Texture };

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

    // Determine default resolution from the first sampler2D input with an image or raw data
    let defaultW = 512;
    let defaultH = 512;
    for (const node of nodes) {
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

    // Renderer canvas sized to the largest shader/constant output
    let maxW = defaultW;
    let maxH = defaultH;
    for (const node of nodes) {
      if (node.data.type === 'shader' || node.data.type === 'constant') {
        const ow = (node.data.width as number) || defaultW;
        const oh = (node.data.height as number) || defaultH;
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

      if (node.data.type === 'shader') {
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
          const compiled = compileNodeShader(
            node.data.shaderCode,
            node.data.inputs,
            upstreamMap,
          );
          material = compiled.material;
          upstreamSamplers = compiled.upstreamSamplers;
          preambleLines = compiled.preambleLines;

          const gl = this.renderer!.getContext();
          const fragSrc = material.fragmentShader as string;
          if (fragSrc) {
            const err = validateFragmentShader(gl, fragSrc);
            if (err) {
              throw new Error(err);
            }
          }

          for (const [uniformName, sourceNodeId] of upstreamSamplers) {
            const src = textures.get(sourceNodeId);
            let tex: THREE.Texture | undefined;
            if (src?.kind === 'fbo') tex = src.target.texture;
            else if (src?.kind === 'image') tex = src.texture;
            if (tex) {
              material.uniforms[uniformName] = { value: tex };
            }
          }

          for (const [key, val] of upstreamScalarValues) {
            const valNum = Number(val);
            material.uniforms[key] = { value: isNaN(valNum) ? val : valNum };
          }

          for (const [key, val] of Object.entries(node.data.uniforms)) {
            if (!upstreamMap.has(key)) {
              const valNum = Number(val);
              material.uniforms[key] = { value: isNaN(valNum) ? val : valNum };
            }
          }

          // Use node's configured size or the default derived from input images
          const outW = (node.data.width as number) || defaultW;
          const outH = (node.data.height as number) || defaultH;
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

        const unconnectedInputs = node.data.inputs.filter(
          (port) => !upstreamEdges.some((e) => e.targetHandle === port.id)
        );
        if (unconnectedInputs.length > 0) {
          const names = unconnectedInputs.map((p) => `'${p.label}'`).join(', ');
          onNodeError?.(nodeId, `Unconnected input${unconnectedInputs.length > 1 ? 's' : ''}: ${names}`);
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
