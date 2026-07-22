import type * as THREE from 'three';
import type { FrameInputs } from '../compositor';
import type { ExecutionPlan } from '../executionEngine';
import type { WebGLRenderer } from '../webglRenderer';
import type { ShaderNodeData } from '../../types';
import type { Node } from '@xyflow/react';
import { setUniform, normalizeUniformValue } from './types';

export function executeShaderNode(
  nodeId: string,
  node: Node<ShaderNodeData>,
  plan: ExecutionPlan,
  builtins: FrameInputs,
  renderer: WebGLRenderer,
): void {
  const material = plan.materials.get(nodeId);
  if (!material) return;

  // Feedback: determine read/write targets, bind previousFrame, clear on first frame
  const isFeedback = plan.feedbackTargets.has(nodeId);
  let renderTarget: THREE.WebGLRenderTarget;
  if (isFeedback) {
    const fbTargets = plan.feedbackTargets.get(nodeId)!;
    const fbReadIdx = plan.feedbackReadIndex.get(nodeId) ?? 0;
    const fbWriteIdx = 1 - fbReadIdx;

    if (plan.feedbackFirstFrame.has(nodeId)) {
      const clearColor = node.data.feedbackClearColor as [number, number, number, number] | undefined;
      renderer.clearTarget(fbTargets[0], clearColor);
      renderer.clearTarget(fbTargets[1], clearColor);
      plan.feedbackFirstFrame.delete(nodeId);
    }

    setUniform(material, 'previousFrame', fbTargets[fbReadIdx].texture);

    renderTarget = fbTargets[fbWriteIdx];
  } else {
    const target = plan.targets.get(nodeId);
    if (!target) return;
    renderTarget = target;
  }

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
    for (const [key, val] of scalars) {
      const upstreamId = plan.scalarUpstream.get(nodeId)?.get(key);
      if (upstreamId && plan.mathValues.has(upstreamId)) {
        setUniform(material, key, normalizeUniformValue(plan.mathValues.get(upstreamId)));
      } else {
        setUniform(material, key, normalizeUniformValue(val));
      }
    }
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
    if (builtin.has('iResolution')) setUniform(material, 'iResolution', plan.resolutionUniforms.get(nodeId) ?? builtins.resolution);
  }

  renderer.renderWithMaterial(material, renderTarget);
  plan.textureSources.set(nodeId, { kind: 'fbo', target: renderTarget });

  if (isFeedback) {
    const current = plan.feedbackReadIndex.get(nodeId) ?? 0;
    plan.feedbackReadIndex.set(nodeId, 1 - current);
  }
}
