import type { Edge, Node } from '@xyflow/react';
import type { ShaderNodeData } from './types';

export type ScreenSaverInputKind = 'image' | 'video';
export type ScreenSaverMode = 'run' | 'preview' | 'configure';

export interface ScreenSaverExposedInput {
  nodeId: string;
  label: string;
  kind: ScreenSaverInputKind;
}


export interface ScreenSaverBootstrap {
  mode: ScreenSaverMode;
  projectJson: string;
  rendererNodeId: string;
  exposedInputs: ScreenSaverExposedInput[];
  settings: Record<string, string>;
}

export interface ScreenSaverExportRequest {
  outputPath: string;
  name: string;
  projectJson: string;
  rendererNodeId: string;
  exposedInputs: ScreenSaverExposedInput[];
}

export function screenSaverInputCandidates(
  nodes: Node<ShaderNodeData>[],
): ScreenSaverExposedInput[] {
  return nodes.flatMap<ScreenSaverExposedInput>((node) => {
    if (node.data.type !== 'input') return [];
    if (node.data.inputMode === 'video') {
      return [{ nodeId: node.id, label: node.data.label, kind: 'video' as const }];
    }
    if (node.data.inputMode === 'image' && node.data.inputDataType === 'sampler2D') {
      return [{ nodeId: node.id, label: node.data.label, kind: 'image' as const }];
    }
    return [];
  });
}

export function rendererCandidates(nodes: Node<ShaderNodeData>[], edges: Edge[]) {
  return nodes
    .filter((node) => node.data.type === 'renderer')
    .map((node) => ({
      id: node.id,
      label: node.data.label,
      connected: edges.some((edge) => edge.target === node.id),
    }));
}
