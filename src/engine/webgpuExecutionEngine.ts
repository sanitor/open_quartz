/**
 * WebGPU Execution Engine — renders the node graph using pure WebGPU.
 *
 * Drop-in replacement for the WebGL-based ExecutionEngine.
 * Uses WebGPUBackend for rendering, wgslCompiler for shader compilation,
 * and wgslParser for port extraction.
 *
 * This engine is selected at runtime when WebGPU is available.
 * Falls back to the legacy WebGL engine otherwise.
 */

import type { Node, Edge } from '@xyflow/react';
import type { ShaderNodeData } from '../types';
import type { FrameInputs } from './compositor';
import { WebGPUBackend, type RenderTarget, type TextureHandle } from './gpu/WebGPUBackend';
import { compileWgslShader, type CompiledShader } from './gpu/wgslCompiler';
import { topologicalSort } from './graphExecutor';
import { ONNX_CATALOG } from '../catalog/onnxCatalog';
import { DEFAULT_ONNX_MODEL_ID } from '../catalog/onnxRegistry';
import { modelManager } from '../store/helpers';
import { MATH_OPS } from '../catalog/mathOps';
import { SHADER_TEMPLATES } from '../catalog/predefinedShaders';
import { OnnxInferenceSession, runSuperResolution, runBackgroundRemoval, runDepthEstimation, runGenericImageToImage, runDetection, runSegmentation } from './onnx/inference';
import { COCO_CLASSES } from './onnx/yoloDetectionPostprocess';
import { drawDetectionOverlay, drawSegmentationOverlay } from './onnx/overlay';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

type TextureSource =
  | { kind: 'target'; target: RenderTarget }
  | { kind: 'image'; handle: TextureHandle };

const BUILTIN_UNIFORMS = new Set([
  'iTime', 'iTimeDelta', 'iFrame', 'iDate', 'iMouse', 'iResolution', 'previousFrame',
]);

export interface WebGPUExecutionPlan {
  sortedIds: string[];
  nodeMap: Map<string, Node<ShaderNodeData>>;
  edges: Edge[];
  shaders: Map<string, CompiledShader>;
  upstreamSamplerBindings: Map<string, Map<string, string>>;
  scalarUpstream: Map<string, Map<string, string>>;
  scalarBindings: Map<string, Map<string, unknown>>;
  selfUniforms: Map<string, Record<string, unknown>>;
  targets: Map<string, RenderTarget>;
  textureSources: Map<string, TextureSource>;
  outputNodes: string[];
  builtinPorts: Map<string, Set<string>>;
  resolutionUniforms: Map<string, Float32Array>;
  preambleLines: Map<string, number>;
  defaultW: number;
  defaultH: number;
  mathValues: Map<string, unknown>;
  feedbackTargets: Map<string, [RenderTarget, RenderTarget]>;
  feedbackReadIndex: Map<string, number>;
  feedbackFirstFrame: Set<string>;
  pendingTextures: Promise<void>[];
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

export class WebGPUExecutionEngine {
  private backend: WebGPUBackend | null = null;
  private running = false;
  private onnxInFlight = new Set<string>();
  private onnxOutputCache = new Map<string, TextureSource>();
  private tsOrtSessions = new Map<string, OnnxInferenceSession>();
  private onnxCallbacks: {
    onOutput?: (nodeId: string, dataUrl: string) => void;
    onNodeError?: (nodeId: string, error: string) => void;
    onOutputSize?: (nodeId: string, w: number, h: number) => void;
    onOutputData?: (nodeId: string, data: unknown) => void;
    onOnnxComplete?: () => void;
    onBackendDetected?: (nodeId: string, backend: 'webgpu' | 'wasm') => void;
  } = {};

  async init(canvas: HTMLCanvasElement): Promise<void> {
    this.backend = new WebGPUBackend(canvas);
    await this.backend.init();
  }

  get device(): GPUDevice | null {
    return this.backend?.device ?? null;
  }

  get canvas(): HTMLCanvasElement | null {
    return this.backend?.canvas ?? null;
  }

  isRunning(): boolean {
    return this.running;
  }

  // -----------------------------------------------------------------------
  // Prepare
  // -----------------------------------------------------------------------

  prepare(
    nodes: Node<ShaderNodeData>[],
    edges: Edge[],
    onNodeError?: (nodeId: string, error: string) => void,
    onOutputSize?: (nodeId: string, width: number, height: number) => void,
    onOutputData?: (nodeId: string, data: unknown) => void,
    onOutput?: (nodeId: string, dataUrl: string) => void,
    onOnnxComplete?: () => void,
    prevPlan?: WebGPUExecutionPlan | null,
    onBackendDetected?: (nodeId: string, backend: 'webgpu' | 'wasm') => void,
  ): WebGPUExecutionPlan | null {
    if (!this.backend) return null;
    const device = this.backend.device;
    this.onnxCallbacks = { onOutput, onNodeError, onOutputSize, onOutputData, onOnnxComplete, onBackendDetected };

    const shaders = new Map<string, CompiledShader>();
    const pendingTextures: Promise<void>[] = [];
    const upstreamSamplerBindings = new Map<string, Map<string, string>>();
    const scalarUpstream = new Map<string, Map<string, string>>();
    const scalarBindings = new Map<string, Map<string, unknown>>();
    const selfUniforms = new Map<string, Record<string, unknown>>();
    const builtinPorts = new Map<string, Set<string>>();
    const resolutionUniforms = new Map<string, Float32Array>();
    const preambleLines = new Map<string, number>();

    // Derive default size from inputs
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

    this.backend.setSize(defaultW, defaultH);
    const sortedIds = topologicalSort(nodes, edges);
    const nodeMap = new Map(nodes.map((n) => [n.id, n]));
    const textureSources = new Map<string, TextureSource>();
    const targets = new Map<string, RenderTarget>();
    const feedbackTargets = new Map<string, [RenderTarget, RenderTarget]>();
    const feedbackReadIndex = new Map<string, number>();
    const feedbackFirstFrame = new Set<string>();

    for (const nodeId of sortedIds) {
      const node = nodeMap.get(nodeId);
      if (!node) continue;

      // Input nodes
      if (node.data.type === 'input' && node.data.inputMode !== 'system') {
        if (node.data.inputDataType === 'sampler2D' && node.data.imageDataUrl) {
          const p = this.backend.loadImageTexture(nodeId, node.data.imageDataUrl)
            .then((handle) => {
              textureSources.set(nodeId, { kind: 'image', handle });
            })
            .catch((e: unknown) => {
              const msg = e instanceof Error ? e.message : String(e);
              onNodeError?.(nodeId, msg);
            });
          pendingTextures.push(p);
        }
        continue;
      }

      if (node.data.type === 'input' && node.data.inputMode === 'system') continue;

      // Math nodes
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

      if (node.data.type === 'renderer') {
        const upstreamEdges = edges.filter((e) => e.target === nodeId);
        const upstreamMap = new Map<string, string>();
        for (const edge of upstreamEdges) {
          const port = node.data.inputs.find((p) => p.id === edge.targetHandle);
          if (port) upstreamMap.set(port.label, edge.source);
        }
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
        const upstreamEdges = edges.filter((e) => e.target === nodeId);
        const upstreamMap = new Map<string, string>();
        for (const edge of upstreamEdges) {
          const port = node.data.inputs.find((p) => p.id === edge.targetHandle);
          if (port) upstreamMap.set(port.label, edge.source);
        }
        upstreamSamplerBindings.set(nodeId, upstreamMap);
        continue;
      }

      // Shader / constant nodes → compile WGSL
      if (node.data.type !== 'shader' && node.data.type !== 'constant') continue;

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
            upstreamScalarValues.set(port.label, 0);
          }
        }
      }

      const builtin = new Set<string>();
      for (const port of node.data.inputs) {
        if (connectedPorts.has(port.label)) continue;
        if (BUILTIN_UNIFORMS.has(port.label)) {
          builtin.add(port.label);
        }
      }

      try {
        const outW = node.data.autoSize === false ? ((node.data.width as number) || defaultW) : defaultW;
        const outH = node.data.autoSize === false ? ((node.data.height as number) || defaultH) : defaultH;
        const isFloat = node.data.outFormat === 'rgba32f' || node.data.outFormat === 'rg32f' || node.data.outFormat === 'r32f';

        const shaderCode = node.data.shaderTemplateId
          ? (SHADER_TEMPLATES.get(node.data.shaderTemplateId)?.code ?? node.data.shaderCode)
          : node.data.shaderCode;

        const compiled = compileWgslShader(device, shaderCode, node.data.inputs, upstreamMap);

        if (compiled.needsFeedback) {
          const prevFb = prevPlan?.feedbackTargets.get(nodeId);
          const canReuse = prevFb && prevFb[0].width === outW && prevFb[0].height === outH;

          if (canReuse) {
            feedbackTargets.set(nodeId, prevFb);
            feedbackReadIndex.set(nodeId, prevPlan!.feedbackReadIndex.get(nodeId) ?? 0);
            targets.set(nodeId, prevFb[feedbackReadIndex.get(nodeId)!]);
          } else {
            const targetA = this.backend.createTarget(`${nodeId}_fb0`, outW, outH, true);
            const targetB = this.backend.createTarget(`${nodeId}_fb1`, outW, outH, true);
            feedbackTargets.set(nodeId, [targetA, targetB]);
            feedbackReadIndex.set(nodeId, 0);
            feedbackFirstFrame.add(nodeId);
            targets.set(nodeId, targetA);
          }
        } else {
          const target = this.backend.createTarget(nodeId, outW, outH, isFloat);
          targets.set(nodeId, target);
        }
        onOutputSize?.(nodeId, outW, outH);
        resolutionUniforms.set(nodeId, new Float32Array([outW, outH, 1]));

        shaders.set(nodeId, compiled);
        upstreamSamplerBindings.set(nodeId, compiled.upstreamSamplers);
        scalarUpstream.set(nodeId, upstreamMap);
        scalarBindings.set(nodeId, upstreamScalarValues);
        selfUniforms.set(nodeId, { ...node.data.uniforms });
        builtinPorts.set(nodeId, builtin);
        preambleLines.set(nodeId, compiled.preambleLines);
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        console.warn(`Shader error for node ${nodeId}:`, msg);
        onNodeError?.(nodeId, msg);
      }
    }

    const rendererNodes = nodes.filter((n) => n.data.type === 'renderer').map((n) => n.id);
    const outputNodes = rendererNodes.length > 0
      ? rendererNodes
      : nodes
        .filter((n) => n.data.type === 'shader' || n.data.type === 'constant' || n.data.type === 'renderer' || n.data.type === 'onnx')
        .filter((n) => !edges.some((e) => e.source === n.id))
        .map((n) => n.id);

    return {
      sortedIds, nodeMap, edges, shaders,
      upstreamSamplerBindings, scalarUpstream, scalarBindings, selfUniforms,
      targets, textureSources, outputNodes, builtinPorts, resolutionUniforms,
      preambleLines, defaultW, defaultH,
      mathValues: new Map<string, unknown>(),
      feedbackTargets, feedbackReadIndex, feedbackFirstFrame,
      pendingTextures,
    };
  }

  // -----------------------------------------------------------------------
  // Run frame
  // -----------------------------------------------------------------------

  runFrame(plan: WebGPUExecutionPlan, builtins: FrameInputs): void {
    if (!this.backend) return;
    const device = this.backend.device;

    // Restore ONNX cache
    for (const [id, src] of this.onnxOutputCache) {
      if (!plan.textureSources.has(id)) {
        plan.textureSources.set(id, src);
      }
    }

    // Inject video textures
    // TODO: video textures need WebGPU texture upload

    for (const nodeId of plan.sortedIds) {
      const node = plan.nodeMap.get(nodeId);
      if (!node) continue;

      // Math CPU eval
      if (node.data.type === 'math') {
        const op = MATH_OPS[node.data.mathOp ?? 'add'];
        if (!op) continue;
        const upstreamMap = plan.upstreamSamplerBindings.get(nodeId);
        const inputs: number[] = [];
        const portLabels = ['a', 'b', 'c'];
        for (let i = 0; i < op.inputCount; i++) {
          const label = portLabels[i];
          const sourceId = upstreamMap?.get(label);
          if (sourceId) {
            const mathVal = plan.mathValues.get(sourceId);
            if (mathVal !== undefined) { inputs.push(Number(mathVal)); continue; }
            const srcNode = plan.nodeMap.get(sourceId);
            if (srcNode?.data.type === 'input' && srcNode.data.inputMode === 'system') {
              switch (srcNode.data.systemSource) {
                case 'time': inputs.push(builtins.time); continue;
                case 'timeDelta': inputs.push(builtins.delta); continue;
                case 'frame': inputs.push(builtins.frame); continue;
                default: break;
              }
            }
            if (srcNode?.data.type === 'input') {
              const srcLabel = srcNode.data.inputs[0]?.label;
              inputs.push(Number(srcNode.data.uniforms?.[srcLabel ?? '']) || 0);
              continue;
            }
          }
          inputs.push(Number(node.data.uniforms?.[label]) || 0);
        }
        plan.mathValues.set(nodeId, op.compute(inputs));
        continue;
      }

      if (node.data.type === 'renderer') continue;

      // ONNX async inference
      if (node.data.type === 'onnx') {
        if (this.onnxInFlight.has(nodeId)) continue;
        if (node.data.onnxStatus && node.data.onnxStatus !== 'ready') continue;
        const hasVideo = builtins.videoTextures && builtins.videoTextures.size > 0;
        if (!hasVideo && plan.textureSources.has(nodeId)) continue;
        // ONNX readback will be handled async — skip for now
        // TODO: implement ONNX inference with WebGPU backend readback
        continue;
      }

      // Shader / constant nodes
      const compiled = plan.shaders.get(nodeId);
      if (!compiled) continue;

      const isFeedback = plan.feedbackTargets.has(nodeId);
      let renderTarget: RenderTarget;

      if (isFeedback) {
        const fbTargets = plan.feedbackTargets.get(nodeId)!;
        const fbReadIdx = plan.feedbackReadIndex.get(nodeId) ?? 0;
        const fbWriteIdx = 1 - fbReadIdx;

        if (plan.feedbackFirstFrame.has(nodeId)) {
          const clearColor = node.data.feedbackClearColor as [number, number, number, number] | undefined;
          this.backend.clearTarget(fbTargets[0], clearColor);
          this.backend.clearTarget(fbTargets[1], clearColor);
          plan.feedbackFirstFrame.delete(nodeId);
        }

        renderTarget = fbTargets[fbWriteIdx];
      } else {
        const target = plan.targets.get(nodeId);
        if (!target) continue;
        renderTarget = target;
      }

      // Build bind group entries from upstream textures + uniforms
      const entries: GPUBindGroupEntry[] = [];
      const upstreamSamplers = plan.upstreamSamplerBindings.get(nodeId);

      // Texture bindings
      if (upstreamSamplers) {
        for (const [uniformName, sourceNodeId] of upstreamSamplers) {
          const texBinding = compiled.textureBindings.get(uniformName);
          if (texBinding === undefined) continue;
          const src = plan.textureSources.get(sourceNodeId);
          if (!src) continue;
          const view = src.kind === 'target' ? src.target.view : src.handle.view;
          const sampler = src.kind === 'target' ? src.target.sampler : src.handle.sampler;
          entries.push({ binding: texBinding, resource: view });
          entries.push({ binding: texBinding + 1, resource: sampler });
        }
      }

      // Feedback previousFrame binding
      if (isFeedback && compiled.previousFrameBinding !== null) {
        const fbTargets = plan.feedbackTargets.get(nodeId)!;
        const fbReadIdx = plan.feedbackReadIndex.get(nodeId) ?? 0;
        entries.push({ binding: compiled.previousFrameBinding, resource: fbTargets[fbReadIdx].view });
        entries.push({ binding: compiled.previousFrameBinding + 1, resource: fbTargets[fbReadIdx].sampler });
      }

      // Uniform bindings (scalar values)
      // TODO: create uniform buffers for scalar values and builtins

      // Render
      const bindGroup = device.createBindGroup({
        layout: compiled.bindGroupLayout,
        entries,
      });
      this.backend.renderPass(compiled.pipeline, bindGroup, renderTarget);
      plan.textureSources.set(nodeId, { kind: 'target', target: renderTarget });

      // Swap feedback
      if (isFeedback) {
        const current = plan.feedbackReadIndex.get(nodeId) ?? 0;
        plan.feedbackReadIndex.set(nodeId, 1 - current);
      }
    }
  }

  // -----------------------------------------------------------------------
  // Renderer output
  // -----------------------------------------------------------------------

  renderRendererToScreen(plan: WebGPUExecutionPlan, rendererNodeId: string): void {
    if (!this.backend) return;
    const node = plan.nodeMap.get(rendererNodeId);
    if (node?.data.type !== 'renderer') return;
    const sourceId = plan.upstreamSamplerBindings.get(rendererNodeId)?.values().next().value;
    if (!sourceId) return;
    const src = plan.textureSources.get(sourceId);
    if (!src) return;
    if (src.kind === 'target') {
      this.backend.renderToScreen(src.target);
    } else {
      this.backend.renderToScreen(src.handle);
    }
  }

  async readOutputs(
    plan: WebGPUExecutionPlan,
    onOutput: (nodeId: string, dataUrl: string) => void,
  ): Promise<void> {
    if (!this.backend) return;
    for (const nodeId of plan.outputNodes) {
      const node = plan.nodeMap.get(nodeId);
      if (node?.data.type === 'renderer') {
        const sourceId = plan.upstreamSamplerBindings.get(nodeId)?.values().next().value;
        if (!sourceId) continue;
        const src = plan.textureSources.get(sourceId);
        if (src?.kind === 'target') {
          const dataUrl = await this.backend.readTargetToDataURL(src.target, 512);
          onOutput(nodeId, dataUrl);
        }
        continue;
      }
      const target = plan.targets.get(nodeId);
      if (!target) continue;
      const dataUrl = await this.backend.readTargetToDataURL(target, 512);
      onOutput(nodeId, dataUrl);
    }
  }

  // -----------------------------------------------------------------------
  // Lifecycle
  // -----------------------------------------------------------------------

  stop(): void {
    this.running = false;
    for (const s of this.tsOrtSessions.values()) {
      try { s.dispose(); } catch { /* ignore */ }
    }
    this.tsOrtSessions.clear();
    this.onnxOutputCache.clear();
    this.backend?.clearResources();
  }

  dispose(): void {
    this.stop();
    this.backend?.dispose();
    this.backend = null;
  }
}
