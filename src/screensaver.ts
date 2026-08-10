import type { Edge, Node } from '@xyflow/react';
import type { ProjectFile, ShaderNodeData } from './types';
import { SHADER_TEMPLATES } from './catalog/predefinedShaders';
import { parseWgslShader } from './sdk/wgslParser';

export type ScreenSaverInputKind = 'image' | 'video';
export type ScreenSaverMode = 'run' | 'preview';

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

export function collectScreenSaverGraph(
  project: ProjectFile,
  rendererNodeId: string,
): ProjectFile {
  const keep = new Set<string>([rendererNodeId]);
  const pending = [rendererNodeId];
  while (pending.length > 0) {
    const target = pending.pop()!;
    for (const edge of project.graph.edges) {
      if (edge.target !== target || keep.has(edge.source)) continue;
      keep.add(edge.source);
      pending.push(edge.source);
    }
  }

  return {
    ...project,
    graph: {
      nodes: project.graph.nodes.filter((node) => keep.has(node.id)),
      edges: project.graph.edges.filter((edge) => keep.has(edge.source) && keep.has(edge.target)),
    },
  };
}

export function prepareScreenSaverGraph(
  nodes: Node<ShaderNodeData>[],
  edges: Edge[],
  rendererNodeId: string,
  width: number,
  height: number,
): { nodes: Node<ShaderNodeData>[]; edges: Edge[] } {
  const renderer = nodes.find((node) => node.id === rendererNodeId && node.data.type === 'renderer');
  if (!renderer) throw new Error('The selected Renderer is missing from the exported graph.');
  const rendererEdge = edges.find((edge) => edge.target === rendererNodeId);
  if (!rendererEdge) throw new Error('The selected Renderer has no connected input.');

  const resample = SHADER_TEMPLATES.get('Resample');
  if (!resample) throw new Error('The Resample shader is unavailable.');
  const parsed = parseWgslShader(resample.code);
  const input = parsed.inputs[0];
  const output = parsed.outputs[0];
  if (!input || !output) throw new Error('The Resample shader has an invalid port contract.');

  const nodeId = '__screen_saver_output_resample';
  const resampleNode: Node<ShaderNodeData> = {
    id: nodeId,
    type: 'shader',
    position: renderer.position,
    data: {
      type: 'shader',
      label: 'Screen Resolution',
      templateName: 'Resample',
      shaderTemplateId: 'Resample',
      shaderCode: resample.code,
      inputs: parsed.inputs,
      outputs: parsed.outputs,
      uniforms: {},
      autoSize: false,
      width,
      height,
      resolvedWidth: width,
      resolvedHeight: height,
      outFormat: 'rgba8',
    },
  };

  return {
    nodes: [...nodes, resampleNode],
    edges: [
      ...edges.filter((edge) => edge.id !== rendererEdge.id),
      {
        ...rendererEdge,
        id: '__screen_saver_source_edge',
        target: nodeId,
        targetHandle: input.id,
      },
      {
        id: '__screen_saver_renderer_edge',
        source: nodeId,
        sourceHandle: output.id,
        target: rendererNodeId,
        targetHandle: rendererEdge.targetHandle,
        type: 'bezier',
      },
    ],
  };
}

