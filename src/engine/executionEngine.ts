import * as THREE from 'three';
import type { Node, Edge } from '@xyflow/react';
import type { ShaderNodeData, FramebufferFormat, TextureFilter, TextureWrap } from '../types';
import { WebGLRenderer } from './webglRenderer';
import { compileNodeShader, validateFragmentShader } from './shaderCompiler';
import { topologicalSort } from './graphExecutor';

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
  ) {
    if (!this.renderer) return;
    this.running = true;

    const order = topologicalSort(nodes, edges);
    const nodeMap = new Map(nodes.map((n) => [n.id, n]));
    const textures = new Map<string, TextureSource>();

    // Determine resolution from the first sampler2D input with an image or raw data
    let w = 512;
    let h = 512;
    for (const node of nodes) {
      if (node.data.inputDataType === 'sampler2D' && node.data.imageDataUrl) {
        const img = await new Promise<HTMLImageElement>((resolve, reject) => {
          const i = new Image();
          i.onload = () => resolve(i);
          i.onerror = () => reject(new Error(`Failed to load image for node ${node.id}`));
          i.src = node.data.imageDataUrl!;
        }).catch(() => null);
        if (!img) break;
        w = img.naturalWidth;
        h = img.naturalHeight;
        break;
      }
      if (node.data.inputMode === 'framebuffer' && node.data.rawDataUrl && node.data.fbWidth && node.data.fbHeight) {
        w = node.data.fbWidth;
        h = node.data.fbHeight;
        break;
      }
    }
    this.renderer.setSize(w, h);

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
            const target = this.renderer.createTarget(`raw_${nodeId}`, w, h, true);
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

            const target = this.renderer.createTarget(`img_${nodeId}`, w, h, true);
            this.renderer.renderSampler2DInput(tex, target);
            textures.set(nodeId, { kind: 'fbo', target });
            onOutput?.(nodeId, this.renderer.readTargetToDataURL(target));
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
        try {
          const compiled = compileNodeShader(
            node.data.shaderCode,
            node.data.inputs,
            upstreamMap,
          );
          material = compiled.material;
          upstreamSamplers = compiled.upstreamSamplers;

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

          const target = this.renderer.createTarget(nodeId, w, h, true);
          this.renderer.renderWithMaterial(material, target);
          textures.set(nodeId, { kind: 'fbo', target });
        } catch (e) {
          const msg = e instanceof Error ? e.message : String(e);
          const formatted = formatShaderError(msg, node.data.shaderCode);
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

      if (node.data.type === 'output') {
        const upstreamMap = new Map<string, string>();
        for (const edge of upstreamEdges) {
          const port = node.data.inputs.find((p) => p.id === edge.targetHandle);
          if (port) {
            upstreamMap.set(port.label, edge.source);
          }
        }

        let material: THREE.ShaderMaterial;
        let upstreamSamplers: Map<string, string>;
        try {
          const compiled = compileNodeShader(
            node.data.shaderCode,
            node.data.inputs,
            upstreamMap,
          );
          material = compiled.material;
          upstreamSamplers = compiled.upstreamSamplers;

          for (const [uniformName, sourceNodeId] of upstreamSamplers) {
            const src = textures.get(sourceNodeId);
            let tex: THREE.Texture | undefined;
            if (src?.kind === 'fbo') tex = src.target.texture;
            else if (src?.kind === 'image') tex = src.texture;
            if (tex) {
              material.uniforms[uniformName] = { value: tex };
            }
          }

          const outW = (node.data.width as number) || w;
          const outH = (node.data.height as number) || h;
          const target = this.renderer.createTarget(nodeId, outW, outH);
          this.renderer.renderWithMaterial(material, target);
          textures.set(nodeId, { kind: 'fbo', target });

          const dataUrl = this.renderer.readTargetToDataURL(target);
          onOutput?.(nodeId, dataUrl);
          onOutputSize?.(nodeId, outW, outH);
        } catch (e) {
          const msg = e instanceof Error ? e.message : String(e);
          const formatted = formatShaderError(msg, node.data.shaderCode);
          console.warn(`Shader error for node ${nodeId}:`, formatted);
          onNodeError?.(nodeId, formatted);
        }
      }
    }
  }

  stop() {
    this.running = false;
    try { this.renderer?.dispose(); } catch {}
    this.renderer = null;
  }
}

function formatShaderError(msg: string, _shaderCode: string): string {
  const lines = msg.split('\n');
  const relevant = lines.filter(
    (l) => l.includes('ERROR:') || l.includes('WARNING:')
  );
  if (relevant.length > 0) return relevant.join('\n');
  const short = lines.find(
    (l) => l.includes('Shader Error') || l.includes('getProgramInfoLog')
  );
  return short ?? msg;
}
