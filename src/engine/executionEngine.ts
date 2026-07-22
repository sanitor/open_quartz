import * as THREE from 'three';
import type { Node, Edge } from '@xyflow/react';
import type { ShaderNodeData, FramebufferFormat, TextureFilter, TextureWrap } from '../types';
import type { FrameInputs } from './compositor';
import { WebGLRenderer } from './webglRenderer';
import { compileNodeShader, validateFragmentShader } from './shaderCompiler';
import { topologicalSort } from './graphExecutor';
import { ONNX_MODELS, DEFAULT_ONNX_MODEL_ID, type OnnxModelDescriptor } from '../catalog/onnxRegistry';
import { ONNX_CATALOG } from '../catalog/onnxCatalog';
import { modelManager } from '../store/helpers';
import { OnnxSession, type OnnxDetection } from './onnxSession';
import { SemSegSession } from './onnxSegSession';
import { OnnxInferenceSession, runSuperResolution, runBackgroundRemoval, runDepthEstimation, runGenericImageToImage } from './onnxInference';
import { drawDetectionOverlay, drawSegmentationOverlay } from './onnxOverlay';
import { SHADER_TEMPLATES } from '../catalog/predefinedShaders';
import {
  type TextureSource,
  BUILTIN_UNIFORMS,
  normalizeUniformValue,
  formatShaderError,
  isRenderableNode,
  executeShaderNode,
  executeMathNode,
  prepareInputTexture as prepareInputTex,
} from './executors';

export interface ExecutionPlan {
  sortedIds: string[];
  nodeMap: Map<string, Node<ShaderNodeData>>;
  edges: Edge[];
  materials: Map<string, THREE.ShaderMaterial>;
  upstreamSamplerBindings: Map<string, Map<string, string>>;
  scalarUpstream: Map<string, Map<string, string>>; // nodeId -> (uniformName -> upstream nodeId) for scalar connections
  scalarBindings: Map<string, Map<string, unknown>>;
  selfUniforms: Map<string, Record<string, unknown>>;
  targets: Map<string, THREE.WebGLRenderTarget>;
  textureSources: Map<string, TextureSource>;
  outputNodes: string[];
  builtinPorts: Map<string, Set<string>>;
  resolutionUniforms: Map<string, Float32Array>;
  preambleLines: Map<string, number>;
  defaultW: number;
  mathValues: Map<string, unknown>;
  defaultH: number;
  // Feedback / Accumulator fields
  feedbackTargets: Map<string, [THREE.WebGLRenderTarget, THREE.WebGLRenderTarget]>;
  feedbackReadIndex: Map<string, number>;
  feedbackFirstFrame: Set<string>;
  /** Promises for async texture loads (image inputs). Must resolve before first render. */
  pendingTextures: Promise<void>[];
}

export class ExecutionEngine {
  private renderer: WebGLRenderer | null = null;
  private running = false;
  private onnxInFlight = new Set<string>();
  /** Cached ONNX outputs that survive plan rebuilds (recompile). */
  private onnxOutputCache = new Map<string, TextureSource>();
  private onnxCallbacks: {
    onOutput?: (nodeId: string, dataUrl: string) => void;
    onNodeError?: (nodeId: string, error: string) => void;
    onOutputSize?: (nodeId: string, w: number, h: number) => void;
    onOutputData?: (nodeId: string, data: unknown) => void;
    onOnnxComplete?: () => void;
    onBackendDetected?: (nodeId: string, backend: 'webgpu' | 'wasm') => void;
  } = {};

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

  getCanvas(): HTMLCanvasElement | null {
    return this.renderer?.canvas ?? null;
  }

  prepare(
    nodes: Node<ShaderNodeData>[],
    edges: Edge[],
    onNodeError?: (nodeId: string, error: string) => void,
    onOutputSize?: (nodeId: string, width: number, height: number) => void,
    onOutputData?: (nodeId: string, data: unknown) => void,
    onOutput?: (nodeId: string, dataUrl: string) => void,
    onOnnxComplete?: () => void,
    prevPlan?: ExecutionPlan | null,
    onBackendDetected?: (nodeId: string, backend: 'webgpu' | 'wasm') => void,
  ): ExecutionPlan | null {
    if (!this.renderer) return null;
    this.onnxCallbacks = { onOutput, onNodeError, onOutputSize, onOutputData, onOnnxComplete, onBackendDetected };
    const materials = new Map<string, THREE.ShaderMaterial>();
    const pendingTextures: Promise<void>[] = [];
    const upstreamSamplerBindings = new Map<string, Map<string, string>>();
    const scalarUpstream = new Map<string, Map<string, string>>();
    const scalarBindings = new Map<string, Map<string, unknown>>();
    const selfUniforms = new Map<string, Record<string, unknown>>();
    const builtinPorts = new Map<string, Set<string>>();
    const resolutionUniforms = new Map<string, Float32Array>();
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
      }
    }
    this.renderer.setSize(maxW, maxH);
    const sortedIds = topologicalSort(nodes, edges);
    const nodeMap = new Map(nodes.map((n) => [n.id, n]));
    const textureSources = new Map<string, TextureSource>();
    const targets = new Map<string, THREE.WebGLRenderTarget>();
    const feedbackTargets = new Map<string, [THREE.WebGLRenderTarget, THREE.WebGLRenderTarget]>();
    const feedbackReadIndex = new Map<string, number>();
    const feedbackFirstFrame = new Set<string>();

    for (const nodeId of sortedIds) {
      const node = nodeMap.get(nodeId);
      if (!node) continue;

      if (node.data.type === 'input' && node.data.inputMode !== 'system') {
        const pending = prepareInputTex(node, this.renderer!, textureSources, onNodeError);
        if (pending) pendingTextures.push(pending);
        continue;
      }

      // System source nodes: pure value providers, skip shader compilation
      if (node.data.type === 'input' && node.data.inputMode === 'system') {
        continue;
      }

      if (node.data.type === 'math') {
        const upstreamEdges = edges.filter((e) => e.target === nodeId);
        const mathUpstream = new Map<string, string>();
        for (const edge of upstreamEdges) {
          const port = node.data.inputs.find((p) => p.id === edge.targetHandle);
          if (port) mathUpstream.set(port.label, edge.source);
        }
        upstreamSamplerBindings.set(nodeId, mathUpstream);
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
          } else if (srcNode?.data.type === 'math') {
            upstreamScalarValues.set(port.label, 0); // placeholder; real value from mathValues at runtime
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

      if (node.data.type === 'renderer') {
        upstreamSamplerBindings.set(nodeId, upstreamMap);
        const sourceId = upstreamMap.values().next().value;
        if (sourceId) {
          const sourceTarget = targets.get(sourceId);
          const sourceNode = nodeMap.get(sourceId);
          const w = sourceTarget?.width ?? sourceNode?.data.imageWidth ?? sourceNode?.data.resolvedWidth;
          const h = sourceTarget?.height ?? sourceNode?.data.imageHeight ?? sourceNode?.data.resolvedHeight;
          if (w && h) onOutputSize?.(nodeId, w, h);
        }
        continue;
      }

      if (node.data.type === 'onnx') {
        upstreamSamplerBindings.set(nodeId, upstreamMap);
        continue;
      }

      try {
        const outW = node.data.autoSize === false ? ((node.data.width as number) || defaultW) : defaultW;
        const outH = node.data.autoSize === false ? ((node.data.height as number) || defaultH) : defaultH;
        const outFormat = node.data.outFormat;
        const isFloat = outFormat === 'rgba32f' || outFormat === 'rg32f' || outFormat === 'r32f';

        const shaderCode = node.data.shaderTemplateId
          ? (SHADER_TEMPLATES.get(node.data.shaderTemplateId)?.code ?? node.data.shaderCode)
          : node.data.shaderCode;
        const compiled = compileNodeShader(shaderCode, node.data.inputs, upstreamMap);
        const material = compiled.material;
        const gl = this.renderer.getContext();
        const err = validateFragmentShader(gl, material.fragmentShader);
        if (err) throw new Error(err);

        if (compiled.needsFeedback) {
          // Reuse existing feedback targets when shader + size haven't changed
          const prevFb = prevPlan?.feedbackTargets.get(nodeId);
          const canReuse = prevFb
            && prevFb[0].width === outW && prevFb[0].height === outH;

          if (canReuse) {
            feedbackTargets.set(nodeId, prevFb);
            feedbackReadIndex.set(nodeId, prevPlan!.feedbackReadIndex.get(nodeId) ?? 0);
            // Do NOT add to feedbackFirstFrame — preserve accumulated state
            targets.set(nodeId, prevFb[feedbackReadIndex.get(nodeId)!]);
          } else {
            const targetA = this.renderer.createTarget(`${nodeId}_fb0`, outW, outH, true, 'rgba32f');
            const targetB = this.renderer.createTarget(`${nodeId}_fb1`, outW, outH, true, 'rgba32f');
            if (node.data.texFilter || node.data.texWrap) {
              this.renderer.applyTextureSampling(targetA.texture, node.data.texFilter, node.data.texWrap);
              this.renderer.applyTextureSampling(targetB.texture, node.data.texFilter, node.data.texWrap);
            }
            feedbackTargets.set(nodeId, [targetA, targetB]);
            feedbackReadIndex.set(nodeId, 0);
            feedbackFirstFrame.add(nodeId);
            targets.set(nodeId, targetA);
          }
        } else {
          const target = this.renderer.createTarget(nodeId, outW, outH, isFloat, outFormat);
          if (node.data.texFilter || node.data.texWrap) {
            this.renderer.applyTextureSampling(target.texture, node.data.texFilter, node.data.texWrap);
          }
          targets.set(nodeId, target);
        }
        onOutputSize?.(nodeId, outW, outH);
        resolutionUniforms.set(nodeId, new Float32Array([outW, outH, 1]));

        materials.set(nodeId, material);
        upstreamSamplerBindings.set(nodeId, compiled.upstreamSamplers);
        scalarUpstream.set(nodeId, upstreamMap);
        scalarBindings.set(nodeId, upstreamScalarValues);
        const selfVals = { ...node.data.uniforms };
        for (const port of node.data.inputs) {
          if (!upstreamMap.has(port.label) && port.dataType !== 'sampler2D' && port.dataType !== 'samplerCube') {
            if (!(port.label in selfVals) && port.defaultValue !== undefined) {
              selfVals[port.label] = normalizeUniformValue(port.defaultValue);
            }
          }
        }
        selfUniforms.set(nodeId, selfVals);
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
      scalarUpstream,
      scalarBindings,
      selfUniforms,
      targets,
      textureSources,
      outputNodes,
      builtinPorts,
      resolutionUniforms,
      preambleLines,
      defaultW,
      defaultH,
      mathValues: new Map<string, unknown>(),
      feedbackTargets,
      feedbackReadIndex,
      feedbackFirstFrame,
      pendingTextures,
    };
  }

  runFrame(plan: ExecutionPlan, builtins: FrameInputs): void {
    if (!this.renderer) return;

    // Restore cached ONNX outputs into a (possibly fresh) plan.
    for (const [id, src] of this.onnxOutputCache) {
      if (!plan.textureSources.has(id)) {
        plan.textureSources.set(id, src);
      }
    }
    if (builtins.videoTextures) {
      for (const [nodeId, texture] of builtins.videoTextures) {
        plan.textureSources.set(nodeId, { kind: 'image', texture });
      }
    }

    for (const nodeId of plan.sortedIds) {
      const node = plan.nodeMap.get(nodeId);
      if (!node) continue;

      if (node.data.type === 'math') {
        executeMathNode(nodeId, node, plan, builtins);
        continue;
      }

      if (!isRenderableNode(node)) continue;

      if (node.data.type === 'renderer') continue;

      if (node.data.type === 'onnx') {
        if (this.onnxInFlight.has(nodeId)) continue;
        // Skip if model is still downloading / not ready.
        if (node.data.onnxStatus && node.data.onnxStatus !== 'ready') continue;
        // For video-driven pipelines, skip the output-cache check so we
        // re-infer every frame. The previous result stays in textureSources
        // for the downstream renderer to display (1-frame latency).
        const hasVideo = builtins.videoTextures && builtins.videoTextures.size > 0;
        if (!hasVideo && plan.textureSources.has(nodeId)) continue;
        const upstreamMap = plan.upstreamSamplerBindings.get(nodeId);
        const sourceId = upstreamMap?.values().next().value;
        if (!sourceId) continue;
        const src = plan.textureSources.get(sourceId);
        if (!src) continue;
        const tex = src.kind === 'fbo' ? src.target.texture : src.texture;
        const srcW = src.kind === 'fbo' ? src.target.width : plan.defaultW;
        const srcH = src.kind === 'fbo' ? src.target.height : plan.defaultH;
        const scratchId = `onnx_src_${nodeId}`;
        const scratchTarget = this.renderer.createTarget(scratchId, srcW, srcH, false, 'rgba8');
        this.renderer.renderSampler2DInput(tex, scratchTarget);
        const sourceCanvas = this.renderer.readTargetToCanvas(scratchTarget);
        this.onnxInFlight.add(nodeId);
        void this.runOnnxInference(plan, nodeId, node, sourceCanvas, srcW, srcH);
        continue;
      }
      executeShaderNode(nodeId, node, plan, builtins, this.renderer);
    }
  }

  private async runOnnxInference(
    plan: ExecutionPlan,
    nodeId: string,
    node: Node<ShaderNodeData>,
    sourceCanvas: HTMLCanvasElement,
    srcW: number,
    srcH: number,
  ): Promise<void> {
    try {
      const modelId = node.data.onnxModelId ?? DEFAULT_ONNX_MODEL_ID;
      const catalogEntry = ONNX_CATALOG[modelId];

      // Route by task — all image→image tasks share runTsOrtInference.
      if (catalogEntry && catalogEntry.task === 'super-resolution') {
        const isRgb = modelId === 'realesrgan-x4';
        const scale = isRgb ? 4 : 3;
        const modelType = isRgb ? 'rgb' as const : 'ycbcr' as const;
        await this.runTsOrtInference(plan, nodeId, sourceCanvas, srcW, srcH, modelId,
          (s, d, w, h) => runSuperResolution(s, d, w, h, scale, modelType));
        return;
      }
      if (catalogEntry && catalogEntry.task === 'background-removal') {
        await this.runTsOrtInference(plan, nodeId, sourceCanvas, srcW, srcH, modelId,
          (s, d, w, h) => runBackgroundRemoval(s, d, w, h, modelId));
        return;
      }
      if (catalogEntry && catalogEntry.task === 'depth-estimation') {
        await this.runTsOrtInference(plan, nodeId, sourceCanvas, srcW, srcH, modelId,
          (s, d, w, h) => runDepthEstimation(s, d, w, h));
        return;
      }

      if (catalogEntry && catalogEntry.task === 'segmentation') {
        const descriptor = ONNX_MODELS[modelId];
        if (!descriptor) throw new Error(`Unknown ONNX model: ${modelId}`);
        const session = await this.getSemSegSession(descriptor);
        const result = await session.run(sourceCanvas, srcW, srcH);
        const seg = result.segmentation;
        const overlay = drawSegmentationOverlay(sourceCanvas, srcW, srcH, seg.maskRgba, seg.maskW, seg.maskH);
        plan.textureSources.set(nodeId, { kind: 'image', texture: overlay.texture });
        this.onnxOutputCache.set(nodeId, { kind: 'image', texture: overlay.texture });
        this.onnxCallbacks.onOutput?.(nodeId, overlay.dataUrl);
        this.onnxCallbacks.onOutputSize?.(nodeId, srcW, srcH);
        this.onnxCallbacks.onOutputData?.(nodeId, { segmentation: seg });
        return;
      }

      // Custom or generic models: full-image inference, output dims from tensor
      if (node.data.onnxSource === 'custom' || !catalogEntry) {
        await this.runTsOrtInference(plan, nodeId, sourceCanvas, srcW, srcH, modelId,
          (s, d, w, h) => runGenericImageToImage(s, d, w, h));
        return;
      }

      // Catalog detection models: Rust wasm path
      const descriptor = ONNX_MODELS[modelId];
      if (!descriptor) throw new Error(`Unknown ONNX model: ${modelId}`);

      const session = await this.getOnnxSession(descriptor);
      if (node.data.onnxScoreThreshold !== undefined && node.data.onnxIouThreshold !== undefined) {
        session.setThresholds(node.data.onnxScoreThreshold, node.data.onnxIouThreshold);
      }

      const result = await session.run(sourceCanvas, srcW, srcH);
      const detections: OnnxDetection[] = result.detections;

      const overlay = drawDetectionOverlay(sourceCanvas, srcW, srcH, detections);

      plan.textureSources.set(nodeId, { kind: 'image', texture: overlay.texture });
      this.onnxOutputCache.set(nodeId, { kind: 'image', texture: overlay.texture });

      this.onnxCallbacks.onOutput?.(nodeId, overlay.dataUrl);
      this.onnxCallbacks.onOutputSize?.(nodeId, srcW, srcH);
      this.onnxCallbacks.onOutputData?.(nodeId, { detections });
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      console.warn(`ONNX error for node ${nodeId}:`, msg);
      this.onnxCallbacks.onNodeError?.(nodeId, msg);
    } finally {
      this.onnxInFlight.delete(nodeId);
      this.onnxCallbacks.onOnnxComplete?.();
    }
  }

  // -----------------------------------------------------------------------
  // Super-Resolution via generic TS ORT path
  // -----------------------------------------------------------------------

  private tsOrtSessions = new Map<string, OnnxInferenceSession>();

  /**
   * Generic image→image inference via TypeScript ORT.
   * Handles session lifecycle, pixel readback, result→texture, backend reporting, and callbacks.
   */
  private async runTsOrtInference(
    plan: ExecutionPlan,
    nodeId: string,
    sourceCanvas: HTMLCanvasElement,
    srcW: number,
    srcH: number,
    modelId: string,
    infer: (session: OnnxInferenceSession, rgba: Uint8ClampedArray, w: number, h: number) => Promise<{ rgba: Uint8ClampedArray<ArrayBuffer>; width: number; height: number }>,
  ): Promise<void> {
    let session = this.tsOrtSessions.get(modelId);
    if (!session) {
      session = new OnnxInferenceSession();
      const buffer = await modelManager.loadCachedModel(modelId);
      if (!buffer) throw new Error(`Model ${modelId} not downloaded yet`);
      await session.loadFromBuffer(buffer);
      this.tsOrtSessions.set(modelId, session);
    }

    const ctx = sourceCanvas.getContext('2d');
    if (!ctx) throw new Error('Cannot get 2d context from source canvas');
    const imageData = ctx.getImageData(0, 0, srcW, srcH);

    const result = await infer(session, imageData.data, srcW, srcH);

    const backend = session.isWasmFallback ? 'wasm' as const : 'webgpu' as const;
    this.onnxCallbacks.onBackendDetected?.(nodeId, backend);

    const outCanvas = document.createElement('canvas');
    outCanvas.width = result.width;
    outCanvas.height = result.height;
    const outCtx = outCanvas.getContext('2d')!;
    outCtx.putImageData(new ImageData(result.rgba, result.width, result.height), 0, 0);

    const texture = new THREE.CanvasTexture(outCanvas);
    texture.needsUpdate = true;
    texture.flipY = true;
    plan.textureSources.set(nodeId, { kind: 'image', texture });
    this.onnxOutputCache.set(nodeId, { kind: 'image', texture });

    this.onnxCallbacks.onOutput?.(nodeId, outCanvas.toDataURL('image/png'));
    this.onnxCallbacks.onOutputSize?.(nodeId, result.width, result.height);
  }

  readOutputs(plan: ExecutionPlan, onOutput: (nodeId: string, dataUrl: string) => void): void {
    if (!this.renderer) return;
    for (const nodeId of plan.outputNodes) {
      const node = plan.nodeMap.get(nodeId);
      if (node?.data.type === 'renderer') {
        const sourceId = plan.upstreamSamplerBindings.get(nodeId)?.values().next().value;
        if (!sourceId) continue;
        const src = plan.textureSources.get(sourceId);
        if (src?.kind === 'fbo') {
          onOutput(nodeId, this.renderer.readTargetToDataURL(src.target, 512));
          continue;
        }
        if (src?.kind === 'image') {
          const sourceNode = plan.nodeMap.get(sourceId);
          const w = sourceNode?.data.imageWidth ?? sourceNode?.data.resolvedWidth ?? plan.defaultW;
          const h = sourceNode?.data.imageHeight ?? sourceNode?.data.resolvedHeight ?? plan.defaultH;
          let target = plan.targets.get(nodeId);
          if (!target) {
            target = this.renderer.createTarget(nodeId, w, h);
            plan.targets.set(nodeId, target);
          }
          this.renderer.renderSampler2DInput(src.texture, target);
          onOutput(nodeId, this.renderer.readTargetToDataURL(target, 512));
        }
        continue;
      }
      // For feedback nodes, read from textureSources (has latest write target)
      const isFeedback = node ? plan.feedbackTargets.has(nodeId) : false;
      if (isFeedback) {
        const src = plan.textureSources.get(nodeId);
        if (src?.kind === 'fbo') {
          onOutput(nodeId, this.renderer.readTargetToDataURL(src.target, 512));
        }
        continue;
      }
      const target = plan.targets.get(nodeId);
      if (!target) continue;
      onOutput(nodeId, this.renderer.readTargetToDataURL(target, 512));
    }
  }

  /** Read back a single node's render target as a data URL. */
  readNodeOutput(plan: ExecutionPlan, nodeId: string, onOutput: (nodeId: string, dataUrl: string) => void): void {
    if (!this.renderer) return;
    const node = plan.nodeMap.get(nodeId);
    if (!node) return;

    if (node.data.type === 'renderer') {
      const sourceId = plan.upstreamSamplerBindings.get(nodeId)?.values().next().value;
      if (!sourceId) return;
      const src = plan.textureSources.get(sourceId);
      if (src?.kind === 'fbo') {
        onOutput(nodeId, this.renderer.readTargetToDataURL(src.target, 512));
      } else if (src?.kind === 'image') {
        const sourceNode = plan.nodeMap.get(sourceId);
        const w = sourceNode?.data.imageWidth ?? sourceNode?.data.resolvedWidth ?? plan.defaultW;
        const h = sourceNode?.data.imageHeight ?? sourceNode?.data.resolvedHeight ?? plan.defaultH;
        let target = plan.targets.get(nodeId);
        if (!target) {
          target = this.renderer.createTarget(nodeId, w, h);
          plan.targets.set(nodeId, target);
        }
        this.renderer.renderSampler2DInput(src.texture, target);
        onOutput(nodeId, this.renderer.readTargetToDataURL(target, 512));
      }
      return;
    }

    const isFeedback = plan.feedbackTargets.has(nodeId);
    if (isFeedback) {
      const src = plan.textureSources.get(nodeId);
      if (src?.kind === 'fbo') {
        onOutput(nodeId, this.renderer.readTargetToDataURL(src.target, 512));
      }
      return;
    }

    const target = plan.targets.get(nodeId);
    if (!target) return;
    onOutput(nodeId, this.renderer.readTargetToDataURL(target, 512));
  }

  renderRendererToScreen(plan: ExecutionPlan, rendererNodeId: string): void {
    if (!this.renderer) return;
    const node = plan.nodeMap.get(rendererNodeId);
    if (node?.data.type !== 'renderer') return;
    const sourceId = plan.upstreamSamplerBindings.get(rendererNodeId)?.values().next().value;
    if (!sourceId) return;
    const src = plan.textureSources.get(sourceId);
    if (src?.kind === 'fbo') {
      this.renderer.renderToScreen(src.target.texture);
      return;
    }
    if (src?.kind === 'image') {
      this.renderer.renderToScreen(src.texture);
    }
  }

  captureRendererScreenshot(plan: ExecutionPlan, rendererNodeId: string): string | null {
    if (!this.renderer) return null;
    const sourceId = plan.upstreamSamplerBindings.get(rendererNodeId)?.values().next().value;
    if (!sourceId) return null;
    const src = plan.textureSources.get(sourceId);
    if (src?.kind === 'fbo') {
      return this.renderer.readTargetToDataURL(src.target);
    }
    if (src?.kind === 'image') {
      const sourceNode = plan.nodeMap.get(sourceId);
      const w = sourceNode?.data.imageWidth ?? sourceNode?.data.resolvedWidth ?? plan.defaultW;
      const h = sourceNode?.data.imageHeight ?? sourceNode?.data.resolvedHeight ?? plan.defaultH;
      let target = plan.targets.get(rendererNodeId);
      if (!target) {
        target = this.renderer.createTarget(rendererNodeId, w, h);
        plan.targets.set(rendererNodeId, target);
      }
      this.renderer.renderSampler2DInput(src.texture, target);
      return this.renderer.readTargetToDataURL(target);
    }
    return null;
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
          video.crossOrigin = 'anonymous';
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
            video.crossOrigin = 'anonymous';
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

        if (node.data.type === 'renderer') {
          const sourceId = upstreamMap.values().next().value;
          if (sourceId) {
            const src = textures.get(sourceId);
            if (src?.kind === 'fbo') {
              onOutput?.(nodeId, this.renderer.readTargetToDataURL(src.target));
              onOutputSize?.(nodeId, src.target.width, src.target.height);
            }
          }
          continue;
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

          const outW = node.data.autoSize === false ? ((node.data.width as number) || defaultW) : defaultW;
          const outH = node.data.autoSize === false ? ((node.data.height as number) || defaultH) : defaultH;
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
          const catalogEntry = ONNX_CATALOG[modelId];

          // Skip execution if the model is still downloading or hasn't been fetched yet.
          if (node.data.onnxStatus && node.data.onnxStatus !== 'ready') {
            continue;
          }

          const imagePort = node.data.inputs.find((p) => p.dataType === 'sampler2D');
          if (!imagePort) throw new Error(`ONNX node missing sampler2D input`);
          const upstreamEdge = upstreamEdges.find((e) => e.targetHandle === imagePort.id);
          if (!upstreamEdge) throw new Error(`ONNX input '${imagePort.label}' not connected`);

          const source = textures.get(upstreamEdge.source);
          if (!source) throw new Error(`ONNX upstream '${upstreamEdge.source}' produced no texture`);

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
          const scratchId = `onnx_src_${nodeId}`;
          const scratchTarget = this.renderer.createTarget(scratchId, srcW, srcH, false, 'rgba8');
          if (source.kind === 'fbo') {
            this.renderer.renderSampler2DInput(source.target.texture, scratchTarget);
          } else {
            this.renderer.renderSampler2DInput(source.texture, scratchTarget);
          }
          const sourceCanvas = this.renderer.readTargetToCanvas(scratchTarget);

          // Shared helper for image→image TS ORT tasks in the execute path
          const runTsOrt = async (inferFn: (s: OnnxInferenceSession, d: Uint8ClampedArray, w: number, h: number) => Promise<{ rgba: Uint8ClampedArray<ArrayBuffer>; width: number; height: number }>) => {
            let session = this.tsOrtSessions.get(modelId);
            if (!session) {
              session = new OnnxInferenceSession();
              const buffer = await modelManager.loadCachedModel(modelId);
              if (!buffer) throw new Error(`Model ${modelId} not downloaded yet`);
              await session.loadFromBuffer(buffer);
              this.tsOrtSessions.set(modelId, session);
            }
            const ctx = sourceCanvas.getContext('2d');
            if (!ctx) throw new Error('Cannot get 2d context');
            const imageData = ctx.getImageData(0, 0, srcW, srcH);
            const result = await inferFn(session, imageData.data, srcW, srcH);
            const outCanvas = document.createElement('canvas');
            outCanvas.width = result.width;
            outCanvas.height = result.height;
            const outCtx = outCanvas.getContext('2d')!;
            outCtx.putImageData(new ImageData(result.rgba, result.width, result.height), 0, 0);
            const texture = new THREE.CanvasTexture(outCanvas);
            texture.needsUpdate = true;
            texture.flipY = true;
            textures.set(nodeId, { kind: 'image', texture });
            onOutput?.(nodeId, outCanvas.toDataURL('image/png'));
            onOutputSize?.(nodeId, result.width, result.height);
          };

          if (catalogEntry && catalogEntry.task === 'super-resolution') {
            const isRgb = modelId === 'realesrgan-x4';
            const scale = isRgb ? 4 : 3;
            const modelType = isRgb ? 'rgb' as const : 'ycbcr' as const;
            await runTsOrt((s, d, w, h) => runSuperResolution(s, d, w, h, scale, modelType));
          } else if (catalogEntry && catalogEntry.task === 'background-removal') {
            await runTsOrt((s, d, w, h) => runBackgroundRemoval(s, d, w, h, modelId));
          } else if (catalogEntry && catalogEntry.task === 'depth-estimation') {
            await runTsOrt((s, d, w, h) => runDepthEstimation(s, d, w, h));
          } else if (node.data.onnxSource === 'custom' || !catalogEntry) {
            // Custom or generic: full-image inference, output dims from tensor
            await runTsOrt((s, d, w, h) => runGenericImageToImage(s, d, w, h));
          } else {
            // Catalog detection models: Rust wasm path
            const descriptor = ONNX_MODELS[modelId];
            if (!descriptor) throw new Error(`Unknown ONNX model: ${modelId}`);
            const session = await this.getOnnxSession(descriptor);
            if (node.data.onnxScoreThreshold !== undefined && node.data.onnxIouThreshold !== undefined) {
              session.setThresholds(node.data.onnxScoreThreshold, node.data.onnxIouThreshold);
            }
            const result = await session.run(sourceCanvas, srcW, srcH);
            const detections: OnnxDetection[] = result.detections;
            const overlay = drawDetectionOverlay(sourceCanvas, srcW, srcH, detections);
            textures.set(nodeId, { kind: 'image', texture: overlay.texture });
            onOutput?.(nodeId, overlay.dataUrl);
            onOutputSize?.(nodeId, srcW, srcH);
            onOutputData?.(nodeId, { detections });
          }
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
  private onnxBlobUrls = new Map<string, string>();

  private async getOnnxSession(descriptor: OnnxModelDescriptor): Promise<OnnxSession> {
    const existing = this.onnxSessions.get(descriptor.id);
    if (existing) {
      if (existing.status !== 'ready') await existing.init();
      return existing;
    }

    // Try to get model buffer from the model manager (catalog download).
    // If available, create a blob URL so the Rust wasm bridge can fetch it.
    let effectiveDescriptor = descriptor;
    const catalogEntry = ONNX_CATALOG[descriptor.id];
    if (catalogEntry) {
      const buffer = await modelManager.loadCachedModel(descriptor.id);
      if (buffer) {
        const blob = new Blob([buffer], { type: 'application/octet-stream' });
        const blobUrl = URL.createObjectURL(blob);
        this.onnxBlobUrls.set(descriptor.id, blobUrl);
        effectiveDescriptor = { ...descriptor, modelUrl: blobUrl };
      }
    }

    const session = new OnnxSession(effectiveDescriptor);
    this.onnxSessions.set(descriptor.id, session);
    await session.init();
    return session;
  }

  private semSegSessions = new Map<string, SemSegSession>();

  private async getSemSegSession(descriptor: OnnxModelDescriptor): Promise<SemSegSession> {
    const existing = this.semSegSessions.get(descriptor.id);
    if (existing) {
      if (existing.status !== 'ready') await existing.init();
      return existing;
    }

    let modelUrl = descriptor.modelUrl;
    const catalogEntry = ONNX_CATALOG[descriptor.id];
    if (catalogEntry) {
      const buffer = await modelManager.loadCachedModel(descriptor.id);
      if (buffer) {
        const blob = new Blob([buffer], { type: 'application/octet-stream' });
        const blobUrl = URL.createObjectURL(blob);
        this.onnxBlobUrls.set(descriptor.id, blobUrl);
        modelUrl = blobUrl;
      }
    }

    const session = new SemSegSession(modelUrl, descriptor.targetSize);
    this.semSegSessions.set(descriptor.id, session);
    await session.init();
    return session;
  }

  stop() {
    this.running = false;
    for (const s of this.onnxSessions.values()) {
      try { s.dispose(); } catch { /* ignore */ }
    }
    this.onnxSessions.clear();
    for (const s of this.tsOrtSessions.values()) {
      try { s.dispose(); } catch { /* ignore */ }
    }
    this.tsOrtSessions.clear();
    for (const s of this.semSegSessions.values()) {
      try { s.dispose(); } catch { /* ignore */ }
    }
    this.semSegSessions.clear();
    for (const url of this.onnxBlobUrls.values()) {
      URL.revokeObjectURL(url);
    }
    this.onnxBlobUrls.clear();
    this.onnxOutputCache.clear();
    this.renderer?.clearResources();
  }
}
