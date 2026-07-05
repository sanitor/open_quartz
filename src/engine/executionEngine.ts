import * as THREE from 'three';
import type { Node, Edge } from '@xyflow/react';
import type { ShaderNodeData } from '../types';
import { WebGLRenderer } from './webglRenderer';
import { compileNodeShader } from './shaderCompiler';
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
  ) {
    if (!this.renderer) return;
    this.running = true;

    const order = topologicalSort(nodes, edges);
    const nodeMap = new Map(nodes.map((n) => [n.id, n]));
    const textures = new Map<string, TextureSource>();

    const w = 512;
    const h = 512;
    this.renderer.setSize(w, h);

    try {
      for (const nodeId of order) {
        if (!this.running) break;
        const node = nodeMap.get(nodeId);
        if (!node) continue;

        const upstreamEdges = edges.filter((e) => e.target === nodeId);

        if (node.data.type === 'input') {
          if (node.data.inputDataType === 'sampler2D' && node.data.imageDataUrl) {
            const tex = await this.renderer.loadImageTexture(nodeId, node.data.imageDataUrl);
            textures.set(nodeId, { kind: 'image', texture: tex });

            const target = this.renderer.createTarget(`img_${nodeId}`, w, h);
            this.renderer.renderSampler2DInput(tex, target);
            textures.set(nodeId, { kind: 'fbo', target });
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
          } catch (e) {
            console.warn(`Shader compile error for node ${nodeId}:`, e);
            continue;
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

          const target = this.renderer.createTarget(nodeId, w, h);
          this.renderer.renderWithMaterial(material, target);
          textures.set(nodeId, { kind: 'fbo', target });
        }

        if (node.data.type === 'output') {
          const upstreamEdge = upstreamEdges[0];
          if (upstreamEdge) {
            const src = textures.get(upstreamEdge.source);
            if (src?.kind === 'fbo') {
              const dataUrl = this.renderer.readTargetToDataURL(src.target);
              onOutput?.(nodeId, dataUrl);
            }
          }
        }
      }
    } catch (e) {
      console.error('Execution engine error:', e);
    }
  }

  stop() {
    this.running = false;
    try { this.renderer?.dispose(); } catch {}
    this.renderer = null;
  }
}
