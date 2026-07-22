import type { ExecutionPlan } from '../executionEngine';
import type { FrameInputs } from '../compositor';
import type { ShaderNodeData } from '../../types';
import type { Node } from '@xyflow/react';
import { MATH_OPS } from '../../catalog/mathOps';

export function executeMathNode(
  nodeId: string,
  node: Node<ShaderNodeData>,
  plan: ExecutionPlan,
  builtins: FrameInputs,
): void {
  const op = MATH_OPS[node.data.mathOp ?? 'add'];
  if (!op) return;
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
      if (srcNode?.data.type === 'input') {
        if (srcNode.data.inputMode === 'system' && srcNode.data.systemSource) {
          switch (srcNode.data.systemSource) {
            case 'time': inputs.push(builtins.time); continue;
            case 'timeDelta': inputs.push(builtins.delta); continue;
            case 'frame': inputs.push(builtins.frame); continue;
            default: break;
          }
        }
        const srcLabel = srcNode.data.inputs[0]?.label;
        const val = srcNode.data.uniforms?.[srcLabel ?? ''];
        inputs.push(Number(val) || 0);
        continue;
      }
    }
    const selfVal = node.data.uniforms?.[label];
    inputs.push(Number(selfVal) || 0);
  }
  const result = op.compute(inputs);
  plan.mathValues.set(nodeId, result);
}
